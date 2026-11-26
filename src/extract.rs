//! Étape 4 : extraction des ressources.
//! - flux RLE (graphismes) trouvés par balayage de la zone de données ;
//! - ressources du bloc 0x08260000 (format encore inconnu), sauvegardées brutes.

use crate::emu::Cpu;
use crate::packed;
use crate::png;
use crate::rle;
use crate::rom::{Rom, ROM_BASE};
use std::collections::BTreeSet;
use std::fmt::Write as _;

const GFX_START: usize = 0x0003_8000;
const GFX_END: usize = 0x0026_0000;
const PACKED_DIR: std::ops::Range<usize> = 0x0025_0000..0x0026_0000;
/// Fonction du jeu qui décode une ressource du bloc (dest, src).
const LOAD_PACKED: u32 = 0x0802_EE7D;
/// Tampon de sortie en EWRAM, loin des variables du décodeur (0x02001xxx).
const EMU_DEST: u32 = 0x0200_8000;
const PACKED_DATA: std::ops::Range<u32> = 0x0826_0000..0x0835_0000;

pub fn run(rom: &Rom) -> std::io::Result<String> {
    let mut report = String::new();
    std::fs::create_dir_all("extract/rle")?;
    std::fs::create_dir_all("extract/packed")?;

    // --- Flux RLE ---
    let mut index = String::from("adresse     taille  compressé  ratio\n");
    let (mut count, mut total_in, mut total_out) = (0usize, 0usize, 0usize);
    let mut off = GFX_START;
    while off < GFX_END {
        let Some(d) = rle::plausible(&rom.data[off..]) else {
            off += 1;
            continue;
        };
        let addr = ROM_BASE + off as u32;
        std::fs::write(format!("extract/rle/{addr:08X}.bin"), &d.data)?;
        let (w, h, px) = png::tiles_4bpp(&d.data, 16);
        std::fs::write(format!("extract/rle/{addr:08X}.png"), png::indexed(w, h, &px, &png::gray16()))?;
        writeln!(
            index,
            "{addr:08X}  {:>6}  {:>9}  {:>4.0}%",
            d.data.len(),
            d.consumed,
            d.consumed as f64 * 100.0 / d.data.len() as f64
        )
        .unwrap();
        count += 1;
        total_in += d.consumed;
        total_out += d.data.len();
        off += d.consumed;
    }
    std::fs::write("extract/rle/index.txt", index)?;
    writeln!(report, "Flux RLE extraits : {count} ({} Ko -> {} Ko)", total_in / 1024, total_out / 1024).unwrap();

    // --- Bloc compressé inconnu : sauvegarde brute pour étude ---
    let ptrs: BTreeSet<u32> = PACKED_DIR
        .step_by(4)
        .filter_map(|o| rom.u32(o))
        .filter(|v| PACKED_DATA.contains(v))
        .collect();
    let ptrs: Vec<u32> = ptrs.into_iter().collect();
    let mut index = String::from("adresse     décompressé  compressé\n");
    for (i, &p) in ptrs.iter().enumerate() {
        let o = (p - ROM_BASE) as usize;
        let end = ptrs.get(i + 1).map(|&n| (n - ROM_BASE) as usize).unwrap_or(o + 8 + rom.u32(o + 4).unwrap_or(0) as usize);
        std::fs::write(format!("extract/packed/{p:08X}.bin"), &rom.data[o..end.min(rom.len())])?;
        writeln!(index, "{p:08X}  {:>11}  {:>9}", rom.u32(o).unwrap_or(0), rom.u32(o + 4).unwrap_or(0)).unwrap();
    }
    std::fs::write("extract/packed/index.txt", index)?;
    writeln!(report, "Ressources du bloc 0x08260000 sauvegardées brutes : {}", ptrs.len()).unwrap();

    // --- Décodage par émulation de la vraie routine du jeu (LoadPacked, 0x0802EE7C) ---
    std::fs::create_dir_all("extract/unpacked")?;
    let (mut ok, mut fail, mut steps, mut exact) = (0, 0, 0u64, 0);
    let tables = packed::Tables::from_rom(rom);
    let (mut same, mut differ) = (0, 0);
    let mut decoded: std::collections::BTreeMap<u32, Vec<u8>> = Default::default();
    for &p in &ptrs {
        let size = rom.u32((p - ROM_BASE) as usize).unwrap_or(0) as usize;
        let mut cpu = Cpu::new(rom);
        match cpu.call(LOAD_PACKED, &[EMU_DEST, p], 2_000_000_000) {
            Ok(_) => {
                let o = (EMU_DEST & 0x3FFFF) as usize;
                let reference = &cpu.ewram[o..o + size];
                std::fs::write(format!("extract/unpacked/{p:08X}.bin"), reference)?;
                // Oracle : la réécriture native doit donner exactement la même sortie.
                match packed::decompress(&tables, &rom.data[(p - ROM_BASE) as usize..]) {
                    Some(n) if n.data == reference => {
                        same += 1;
                        decoded.insert(p, n.data);
                    }
                    Some(n) => {
                        if differ < 5 {
                            let at = n.data.iter().zip(reference).position(|(a, b)| a != b);
                            writeln!(report, "  natif ≠ émulateur {p:08X} : premier écart à l'octet {at:?}").unwrap();
                        }
                        differ += 1;
                    }
                    None => differ += 1,
                }
                ok += 1;
                steps += cpu.steps;
                // Vérification : le décodeur a-t-il lu tout le flux, sans le dépasser ?
                let rd = |cpu: &Cpu, a: usize| u32::from_le_bytes(cpu.ewram[a..a + 4].try_into().unwrap());
                let (byte, bit) = (rd(&cpu, 0x1B64), rd(&cpu, 0x1B68));
                let consumed = byte + (bit > 0) as u32;
                let packed = rom.u32((p - ROM_BASE) as usize + 4).unwrap_or(0);
                if consumed.abs_diff(packed) <= 1 {
                    exact += 1;
                } else if fail + ok - exact < 6 {
                    writeln!(report, "  {p:08X} : lu {consumed} octets sur {packed}").unwrap();
                }
            }
            Err(e) => {
                if fail < 5 {
                    writeln!(report, "  échec {p:08X} : {e:?}").unwrap();
                }
                fail += 1;
            }
        }
    }
    writeln!(
        report,
        "Décodées par émulation : {ok} (échecs : {fail}), {} millions d'instructions exécutées
         Flux consommé exactement (à 1 octet près) : {exact}/{ok}
         Décodeur natif identique à l'émulateur : {same}/{ok} (différents : {differ})",
        steps / 1_000_000
    )
    .unwrap();
    report.push_str(&catalog(rom, &decoded)?);
    report.push_str(&crate::levels::run(rom, &tables, &decoded)?);
    report.push_str(&crate::sound::run(rom)?);
    Ok(report)
}

/// Rôle d'une ressource dans un enregistrement de décor (répertoire 0x0825xxxx, 7 mots :
/// tuiles, carte, attributs, extra, dimensions, nombre de tuiles, 6).
const ROLES: [&str; 4] = ["tuiles", "carte", "attributs", "extra"];

/// Catalogue : rôle, dimensions et export PNG des jeux de tuiles.
fn catalog(rom: &Rom, decoded: &std::collections::BTreeMap<u32, Vec<u8>>) -> std::io::Result<String> {
    let mut role: std::collections::BTreeMap<u32, String> = Default::default();
    let is_res = |v: u32| PACKED_DATA.contains(&v);
    let mut records = 0;
    let mut off = PACKED_DIR.start;
    while off + 28 <= PACKED_DIR.end {
        let w: Vec<u32> = (0..7).map(|i| rom.u32(off + i * 4).unwrap_or(0)).collect();
        let ok = is_res(w[0]) && is_res(w[1]) && is_res(w[2]) && (w[3] == 0 || is_res(w[3])) && w[6] == 6;
        if !ok {
            off += 4;
            continue;
        }
        let (width, height, tiles) = (w[4] & 0xFFFF, w[4] >> 16, w[5]);
        let rec = ROM_BASE + off as u32;
        for (i, &p) in w[..4].iter().enumerate().filter(|(_, p)| **p != 0) {
            let note = if i == 0 { format!("{tiles} tuiles") } else { format!("{width}x{height}") };
            role.entry(p).or_insert_with(|| format!("décor {rec:08X} : {} ({note})", ROLES[i]));
        }
        records += 1;
        off += 28;
    }

    std::fs::create_dir_all("extract/decors")?;
    let mut out = String::from("ressource   taille  rôle
");
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for (&p, data) in decoded {
        let r = if data.iter().all(|&b| b == 0) {
            "vide".to_string()
        } else {
            role.get(&p).cloned().unwrap_or_else(|| match data.len() {
                1282 => "hors répertoire : carte probable (écran 30x20)".into(),
                n if n % 32 == 0 => format!("hors répertoire : tuiles probables ({} tuiles)", n / 32),
                _ => "hors répertoire : inconnu".into(),
            })
        };
        let kind = if r == "vide" {
            "vide"
        } else {
            ROLES.iter().find(|k| r.contains(&format!(": {k} "))).copied().unwrap_or("autre")
        };
        *counts.entry(kind).or_default() += 1;
        let mut r = r;
        if kind == "tuiles" {
            let bpp8 = png::coherence(data, true) > png::coherence(data, false);
            let png = if bpp8 {
                r = r.replace(" tuiles", " tuiles 8 bpp").replace(&format!("({} tuiles 8 bpp)", data.len() / 32), &format!("({} tuiles 8 bpp)", data.len() / 64));
                let (w, h, px) = png::tiles_8bpp(data, 16);
                png::indexed(w, h, &px, &png::gray256())
            } else {
                let (w, h, px) = png::tiles_4bpp(data, 16);
                png::indexed(w, h, &px, &png::gray16())
            };
            std::fs::write(format!("extract/decors/{p:08X}_tuiles.png"), png)?;
        }
        writeln!(out, "{p:08X}  {:>7}  {r}", data.len()).unwrap();
    }
    std::fs::write("extract/catalogue.txt", out)?;
    let mut rep = format!("Enregistrements de décor : {records}
Catalogue (extract/catalogue.txt) :
");
    for (k, n) in counts {
        writeln!(rep, "  {k:<10} {n}").unwrap();
    }
    Ok(rep)
}
