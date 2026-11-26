//! Générateur du code recompilé (commande `recomp`) : lit le désassemblage
//! `asm/ladysia.s`, découpe le code de la ROM (Thumb et ARM) en blocs et écrit, pour
//! chaque bloc, une fonction Rust qui exécute ses instructions avec `Cpu::thumb_insn` /
//! `Cpu::arm_insn` (opcode et adresse constants : le compilateur n'en garde que le
//! travail utile). Le code que le jeu copie en IWRAM (`IWRAM_COPIES`) est recompilé à
//! son adresse d'exécution. Fichiers écrits dans `src/recomp/code/`.

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Code copié de la ROM en IWRAM au démarrage (adresse en IWRAM, source, taille) :
/// `IntrMain`, le répartiteur d'interruptions (ARM), recopié par `crt0`.
pub const IWRAM_COPIES: [(u32, u32, u32); 1] = [(0x0300_09C0, 0x0800_012C, 0x104)];

#[derive(Clone, Copy)]
struct Insn {
    /// Adresse d'exécution.
    addr: u32,
    op: u32,
    arm: bool,
}

/// Thumb : peut sauter (le bloc s'arrête si c'est le cas).
fn may_branch(op: u32) -> bool {
    match op >> 11 {
        8 if op & 0x400 != 0 => {
            let sub = (op >> 8) & 3;
            let d = (op & 7) | ((op >> 4) & 8);
            sub == 3 || (sub != 1 && d == 15)
        }
        22 | 23 => op & 0xFF00 == 0xBD00,
        26 | 27 => (op >> 8) & 15 != 14,
        28 | 31 => true,
        _ => false,
    }
}

/// Thumb : calcul pur, sans accès mémoire ni saut ni registre pc en destination
/// (décalages, additions, opérations logiques, `add rX, pc/sp`, `add sp`, préfixe de `bl`).
fn pure(op: u32) -> bool {
    match op >> 11 {
        0..=7 | 20 | 21 | 30 => true,
        8 => op & 0x400 == 0,
        22 => op & 0xFF00 == 0xB000,
        _ => false,
    }
}

/// Thumb : fin de bloc obligatoire (saut inconditionnel, retour, appel, SWI).
fn ends_block(op: u32) -> bool {
    match op >> 11 {
        8 if op & 0x400 != 0 => may_branch(op),
        22 | 23 => op & 0xFF00 == 0xBD00,
        26 | 27 => (op >> 8) & 15 == 15,
        28 | 31 => true,
        _ => false,
    }
}

/// Thumb : écriture en mémoire (une écriture de HALTCNT arrête le processeur tout de suite).
fn stores(op: u32) -> bool {
    match op >> 11 {
        10 | 11 => {
            let k = (op >> 10) & 3;
            if op & 0x200 == 0 { k <= 1 } else { k == 0 }
        }
        12..=19 => op & 0x800 == 0,
        22 | 23 => op & 0x600 == 0x400 && op & 0x800 == 0,
        24 | 25 => op & 0x800 == 0,
        _ => false,
    }
}

/// ARM : fin de bloc obligatoire (instruction inconditionnelle qui écrit pc, SWI, MSR).
fn arm_ends_block(op: u32) -> bool {
    if op >> 28 != 14 {
        return false;
    }
    let rd15 = (op >> 12) & 15 == 15;
    op & 0x0E00_0000 == 0x0A00_0000
        || op & 0x0FFF_FFF0 == 0x012F_FF10
        || op & 0x0F00_0000 == 0x0F00_0000
        || (op & 0x0E10_0000 == 0x0810_0000 && op & 0x8000 != 0)
        || (op & 0x0C10_0000 == 0x0410_0000 && rd15)
        || (op & 0x0C00_0000 == 0 && rd15)
        || op & 0x0DB0_F000 == 0x0120_F000
}

/// Empreinte FNV-1a de la zone de code (le code recompilé ne vaut que pour cette ROM).
pub fn code_hash(rom: &[u8]) -> u64 {
    rom[..rom.len().min(0x4_0000)]
        .iter()
        .fold(0xCBF2_9CE4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01B3))
}

fn rom16(rom: &[u8], a: u32) -> u32 {
    let o = (a - 0x0800_0000) as usize;
    u16::from_le_bytes([rom[o], rom[o + 1]]) as u32
}

fn rom32(rom: &[u8], a: u32) -> u32 {
    let o = (a - 0x0800_0000) as usize;
    u32::from_le_bytes([rom[o], rom[o + 1], rom[o + 2], rom[o + 3]])
}

/// Code que le jeu recopie en IWRAM `(destination, source, longueur)` : le répartiteur
/// d'interruptions et les deux routines du mixeur (réécrites à la main dans `jeu/`).
/// Dans les données sans code, ces copies sont vides ; une sauvegarde instantanée faite
/// avec la ROM complète en contient le code d'origine, remplacé au chargement.
pub const STATE_COPIES: [(u32, u32, u32); 3] =
    [(0x0300_09C0, 0x0800_012C, 0x104), (0x0300_0DC0, 0x0803_5480, 0x18C), (0x0300_0F4C, 0x0803_4EC8, 0x98)];

pub fn fnv(d: &[u8]) -> u64 {
    d.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01B3))
}

/// Instructions du jeu (désassemblage + code trouvé par les pointeurs + saut d'entrée de
/// la cartouche), étiquettes de début de bloc, nombre d'instructions trouvées par pointeur.
struct Parsed {
    insns: Vec<Insn>,
    starts: BTreeSet<u32>,
    found: usize,
    /// Octets répertoriés comme constantes (`.word`) : jamais effacés, même quand une
    /// instruction les recouvre (le gestionnaire 0x0802D726 commence au milieu d'une
    /// constante : ses 2 premiers octets sont à la fois une adresse et une instruction).
    data: BTreeSet<u32>,
}

/// Octets de code gardés dans les données parce que le jeu les lit comme des nombres : son
/// générateur aléatoire (0x0802DA28, tableau de 312 mots en 0x030003F0) lit 25 mots après
/// la fin de son tableau, soit le début de la copie du répartiteur d'interruptions
/// (0x030009C0-0x03000A23, recopié de 0x0800012C). Jamais exécutés par le portage.
const RNG_READS: std::ops::Range<u32> = 0x0800_012C..0x0800_0190;

/// Routines ARM du mixeur (plusieurs variantes selon la qualité, recopiées en IWRAM) :
/// le portage n'utilise que ses versions Rust (`jeu/sound.rs`), elles sont effacées en
/// entier, constantes comprises.
const MIXERS: std::ops::Range<u32> = 0x0803_4DB4..0x0803_5804;

/// Décalages dans la ROM des octets d'instruction à effacer.
fn code_bytes(p: &Parsed) -> impl Iterator<Item = usize> + '_ {
    p.insns
        .iter()
        .flat_map(|i| i.addr..i.addr + if i.arm { 4 } else { 2 })
        .filter(|a| !p.data.contains(a) && !RNG_READS.contains(a))
        .chain(MIXERS)
        .map(|a| (a - 0x0800_0000) as usize)
}

/// Données du jeu sans code machine : la ROM dont chaque instruction recompilée et
/// l'en-tête de cartouche (logo compris) sont mis à zéro. Il reste les graphismes, les
/// niveaux, les sons, les textes et les constantes (tables, adresses) mêlées au code.
pub fn data_blob(rom: &[u8], listing: &str) -> Result<Vec<u8>, String> {
    let txt = std::fs::read_to_string(listing).map_err(|e| format!("{listing} : {e}"))?;
    let p = parse(rom, &txt)?;
    Ok(blob_of(rom, &p))
}

fn blob_of(rom: &[u8], p: &Parsed) -> Vec<u8> {
    let mut d = rom.to_vec();
    d[..0xC0].fill(0);
    for o in code_bytes(p) {
        d[o] = 0;
    }
    d
}

/// Vrai pour chaque octet de la ROM effacé dans les données sans code (contrôle
/// `LADYSIA_AUDIT` : aucun de ces octets ne doit être lu comme une donnée).
pub fn data_mask(rom: &[u8], listing: &str) -> Result<Vec<bool>, String> {
    let txt = std::fs::read_to_string(listing).map_err(|e| format!("{listing} : {e}"))?;
    let p = parse(rom, &txt)?;
    let mut m = vec![false; rom.len()];
    m[..0xC0].fill(true);
    for o in code_bytes(&p) {
        m[o] = true;
    }
    Ok(m)
}

pub fn run(rom: &[u8], listing: &str, out_dir: &str) -> Result<String, String> {
    let txt = std::fs::read_to_string(listing).map_err(|e| format!("{listing} : {e}"))?;
    let parsed = parse(rom, &txt)?;
    let blob = blob_of(rom, &parsed);
    let Parsed { mut insns, mut starts, found, data } = parsed;

    // valeur de « bus ouvert » d'une instruction : ce que la console relirait après elle
    // (`[pc + 4]` en Thumb sur les deux moitiés du mot, `[pc + 8]` en ARM), prise dans la
    // ROM d'origine pour ne plus dépendre du code à l'exécution
    let open = |src: u32, arm: bool| -> u32 {
        let o = (src - 0x0800_0000) as usize + if arm { 8 } else { 4 };
        if arm {
            rom.get(o..o + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).unwrap_or(0)
        } else {
            let h = rom.get(o..o + 2).map(|w| u16::from_le_bytes([w[0], w[1]]) as u32).unwrap_or(0);
            h | (h << 16)
        }
    };
    let mut ob: std::collections::HashMap<u32, u32> = insns.iter().map(|i| (i.addr, open(i.addr, i.arm))).collect();

    // contrôle : toute cible de saut ou d'appel direct du code recompilé doit être recompilée
    let all: BTreeSet<u32> = insns.iter().map(|i| i.addr).collect();
    let mut missing: BTreeSet<u32> = BTreeSet::new();
    for (k, i) in insns.iter().enumerate() {
        if i.arm {
            continue;
        }
        let (a, op) = (i.addr, i.op);
        let top = op >> 11;
        let t = if (26..=27).contains(&top) && (op >> 8) & 15 < 14 {
            Some(a.wrapping_add(4).wrapping_add(((op & 0xFF) as i8 as i32 * 2) as u32))
        } else if top == 28 {
            Some(a.wrapping_add(4).wrapping_add((((op & 0x7FF) << 21) as i32 >> 20) as u32))
        } else if top == 31 && k > 0 && insns[k - 1].addr + 2 == a && insns[k - 1].op >> 11 == 30 {
            let hi = insns[k - 1].op;
            Some((a - 2).wrapping_add(4).wrapping_add((((hi & 0x7FF) << 21) as i32 >> 9) as u32).wrapping_add((op & 0x7FF) << 1))
        } else {
            None
        };
        // instruction suivante (si l'exécution continue en séquence)
        let next = (!ends_block(op) || top == 31 || (26..=27).contains(&top)).then_some(a + 2);
        for t in t.into_iter().chain(next) {
            // (une cible dans une constante vient d'une suite décodée dans des données)
            if (0x0800_0000..0x0804_0000).contains(&t) && !all.contains(&t) && !data.contains(&t) {
                missing.insert(t);
            }
        }
    }
    if !missing.is_empty() {
        let l: Vec<String> = missing.iter().take(20).map(|a| format!("{a:08X}")).collect();
        return Err(format!("{} cible(s) de saut ou suite(s) de code non recompilée(s) : {}", missing.len(), l.join(" ")));
    }

    // copies en IWRAM : mêmes instructions, à l'adresse d'exécution
    for (dst, src, len) in IWRAM_COPIES {
        let copied: Vec<Insn> = insns
            .iter()
            .filter(|i| (src..src + len).contains(&i.addr))
            .map(|i| Insn { addr: i.addr - src + dst, ..*i })
            .collect();
        for i in &copied {
            ob.insert(i.addr, open(i.addr - dst + src, i.arm));
        }
        let labels: Vec<u32> = starts.range(src..src + len).map(|a| a - src + dst).collect();
        starts.extend(labels);
        starts.insert(dst);
        insns.extend(copied);
    }

    // vecteur d'interruption du mini-BIOS de l'émulateur (0x18), avec son point de retour
    for (k, &op) in crate::gba::bus::BIOS_IRQ.iter().enumerate() {
        insns.push(Insn { addr: 0x18 + 4 * k as u32, op, arm: true });
    }
    starts.insert(0x18);
    starts.insert(0x28);

    // blocs : coupés aux étiquettes, après les fins de bloc, aux trous et aux changements de mode
    let mut blocks: Vec<Vec<Insn>> = Vec::new();
    let mut prev: Option<Insn> = None;
    for &i in &insns {
        let new = match prev {
            None => true,
            Some(p) => {
                let size = if p.arm { 4 } else { 2 };
                p.arm != i.arm
                    || p.addr + size != i.addr
                    || (if p.arm { arm_ends_block(p.op) } else { ends_block(p.op) })
                    || starts.contains(&i.addr)
            }
        };
        if new {
            blocks.push(Vec::new());
        }
        blocks.last_mut().unwrap().push(i);
        prev = Some(i);
    }

    let dir = format!("{out_dir}/code");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    const FILES: usize = 16;
    let per = blocks.len().div_ceil(FILES);
    let mut table = String::new();
    let (mut n_arm, mut n_thumb) = (0, 0);
    for (f, chunk) in blocks.chunks(per).enumerate() {
        let mut s = String::from(
            "// Code généré par `ladysia-recomp <rom> recomp` : ne pas modifier.\n\
             #![allow(clippy::all, unused)]\nuse crate::gba::bus::Bus;\nuse crate::gba::cpu::Cpu;\n\n",
        );
        for b in chunk {
            let start = b[0].addr;
            let arm = b[0].arm;
            writeln!(s, "pub fn b{start:08x}(c: &mut Cpu, b: &mut Bus) {{").unwrap();
            for (k, i) in b.iter().enumerate() {
                if !arm && pure(i.op) && k + 1 < b.len() {
                    n_thumb += 1;
                    writeln!(s, "    b.cyc += 2;
    c.thumb_reg(b, 0x{:04x}, 0x{:08x});", i.op, i.addr).unwrap();
                    continue;
                }
                if arm {
                    n_arm += 1;
                    // lecture de l'instruction : 4 cycles en ROM, 1 en IWRAM et dans le BIOS
                    let fetch = if i.addr >> 24 == 8 { 4 } else { 1 };
                    writeln!(
                        s,
                        "    b.cyc += {fetch};\n    if c.arm_insn(b, 0x{:08x}, 0x{:08x}, 0x{:08x}) || b.halt_req {{ return; }}",
                        i.op,
                        i.addr,
                        ob.get(&i.addr).copied().unwrap_or(0)
                    )
                    .unwrap();
                    continue;
                }
                n_thumb += 1;
                let call = format!("c.thumb_insn(b, 0x{:04x}, 0x{:08x}, 0x{:08x})", i.op, i.addr, ob.get(&i.addr).copied().unwrap_or(0));
                if ends_block(i.op) {
                    writeln!(s, "    b.cyc += 2;\n    {call};").unwrap();
                } else if may_branch(i.op) {
                    writeln!(s, "    b.cyc += 2;\n    if {call} {{ return; }}").unwrap();
                } else if stores(i.op) {
                    writeln!(s, "    b.cyc += 2;\n    {call};\n    if b.halt_req {{ return; }}").unwrap();
                } else {
                    writeln!(s, "    b.cyc += 2;\n    {call};").unwrap();
                }
            }
            writeln!(s, "}}\n").unwrap();
            writeln!(table, "    (0x{start:08x}, {arm}, f{f}::b{start:08x}),").unwrap();
        }
        std::fs::write(format!("{dir}/f{f}.rs"), s).map_err(|e| e.to_string())?;
    }
    let n_files = blocks.chunks(per).count();
    let mut m = String::from("// Code généré par `ladysia-recomp <rom> recomp` : ne pas modifier.\n#![allow(clippy::all)]\n");
    for f in 0..n_files {
        writeln!(m, "mod f{f};").unwrap();
    }
    writeln!(m, "\n/// Empreinte de la zone de code de la ROM recompilée.\npub const CODE_HASH: u64 = 0x{:016X};\n", code_hash(rom)).unwrap();
    writeln!(m, "/// Empreinte de la même zone dans les données sans code machine (`donnees`).\npub const DATA_HASH: u64 = 0x{:016X};\n", code_hash(&blob)).unwrap();
    writeln!(m, "/// Copies de code en IWRAM : destination, source, longueur, empreinte du code d'origine.").unwrap();
    writeln!(m, "pub static COPIES: [(u32, u32, u32, u64); {}] = [", STATE_COPIES.len()).unwrap();
    for (d, sr, n) in STATE_COPIES {
        let o = (sr - 0x0800_0000) as usize;
        writeln!(m, "    (0x{d:08x}, 0x{sr:08x}, 0x{n:x}, 0x{:016X}),", fnv(&rom[o..o + n as usize])).unwrap();
    }
    writeln!(m, "];\n").unwrap();
    writeln!(m, "/// Blocs recompilés (adresse d'exécution, ARM, fonction), triés.").unwrap();
    writeln!(m, "pub static BLOCKS: [(u32, bool, super::Block); {}] = [\n{table}];", blocks.len()).unwrap();
    std::fs::write(format!("{dir}/mod.rs"), m).map_err(|e| e.to_string())?;
    Ok(format!(
        "{n_thumb} instructions Thumb (dont {found} trouvées par les pointeurs), {n_arm} ARM, {} blocs, {n_files} fichiers dans {dir}\n",
        blocks.len()
    ))
}

fn parse(rom: &[u8], txt: &str) -> Result<Parsed, String> {
    let mut insns: Vec<Insn> = Vec::new();
    let mut starts: BTreeSet<u32> = BTreeSet::new();
    let mut data: BTreeSet<u32> = BTreeSet::new();
    let mut mode: Option<bool> = None; // Some(arm)
    let mut label_pending = false;
    for line in txt.lines() {
        if let Some(h) = line.strip_prefix("@ ---- ") {
            mode = if h.contains("(Thumb") {
                Some(false)
            } else if h.contains("(ARM") {
                Some(true)
            } else {
                None
            };
            continue;
        }
        let t = line.trim_end();
        if !t.starts_with(' ') && t.ends_with(':') && !t.starts_with('@') && !t.starts_with('.') {
            label_pending = true;
            continue;
        }
        let Some(rest) = t.strip_prefix("    ") else { continue };
        let Some((a, rest)) = rest.split_once(": ") else { continue };
        let Ok(addr) = u32::from_str_radix(a, 16) else { continue };
        if rest.starts_with(".word") {
            // constante mêlée au code : jamais une instruction
            data.extend(addr..addr + 4);
        }
        let (hex, text) = rest.split_once("  ").unwrap_or((rest, ""));
        let (text, hex) = (text.trim(), hex.trim());
        let Some(arm) = mode else { continue };
        if text.starts_with('.') || !(0x0800_0000..0x0804_0000).contains(&addr) {
            label_pending = false;
            continue;
        }
        let ops: Vec<u32> = match (arm, hex.len()) {
            (true, 8) => vec![u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?],
            (false, 4) => vec![u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?],
            (false, 8) => vec![
                u32::from_str_radix(&hex[..4], 16).map_err(|e| e.to_string())?,
                u32::from_str_radix(&hex[4..], 16).map_err(|e| e.to_string())?,
            ],
            _ => continue,
        };
        if label_pending {
            starts.insert(addr);
            label_pending = false;
        }
        let size = if arm { 4 } else { 2 };
        for (k, op) in ops.into_iter().enumerate() {
            let a = addr + size * k as u32;
            // l'instruction doit être celle de la ROM
            let real = if arm { rom32(rom, a) } else { rom16(rom, a) };
            if real != op {
                return Err(format!("{a:08X} : {op:X} dans le listage, {real:X} dans la ROM"));
            }
            insns.push(Insn { addr: a, op, arm });
        }
    }
    // l'en-tête de la cartouche n'est pas du code (sauf son saut d'entrée, ajouté plus bas)
    insns.retain(|i| i.addr >= 0x0800_00C0);
    insns.sort_by_key(|i| i.addr);
    insns.dedup_by_key(|i| i.addr);
    let listed = insns.len();

    // code Thumb absent du désassemblage mais pointé par la ROM (fonctions appelées
    // seulement par une table) : décodé depuis le pointeur jusqu'au premier retour ou saut
    let known: BTreeSet<u32> = insns.iter().map(|i| i.addr).collect();
    let mut extra: BTreeSet<u32> = BTreeSet::new();
    for w in rom.chunks_exact(4) {
        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        if v & 1 == 1 && (0x0800_00C0..0x0804_0000).contains(&(v - 1)) && !known.contains(&(v - 1)) && !data.contains(&(v - 1)) {
            extra.insert(v - 1);
        }
    }
    // Un pointeur impair peut aussi viser des données : une suite n'est gardée que si elle
    // ressemble à du code (dans la zone de code du désassemblage, pas de mot nul ni
    // d'instruction indéfinie, appels `bl` complets, fin par un retour, un saut ou du
    // code connu). Sinon ses octets resteraient effacés à tort dans les données sans code.
    let code_end = known.iter().next_back().copied().unwrap_or(0x0800_00C0);
    // les pointeurs ne sont suivis que dans la zone de code du désassemblage ; au-delà (fin
    // de la zone : fonctions de bibliothèque puis tables), seulement les cibles d'appels
    extra.retain(|a| *a <= code_end);
    let mut todo: Vec<u32> = extra.iter().copied().collect();
    // fonctions mortes (jamais appelées, absentes du désassemblage) : tout `push {.., lr}`
    // hors du code connu est essayé aussi, après les pointeurs
    let mut dead: Vec<u32> = (0x0800_00C0..=code_end)
        .step_by(2)
        .filter(|a| !known.contains(a) && !data.contains(a) && rom16(rom, *a) & 0xFF00 == 0xB500)
        .collect();
    dead.reverse();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    // instructions déjà retenues par cette recherche
    let mut kept: BTreeSet<u32> = BTreeSet::new();
    while let Some(entry) = todo.pop().or_else(|| dead.pop()) {
        if kept.contains(&entry) || known.contains(&entry) || !seen.insert(entry) {
            continue;
        }
        let mut run: Vec<Insn> = Vec::new();
        let mut targets: Vec<u32> = Vec::new();
        let mut ok = false;
        let mut a = entry;
        for _ in 0..256 {
            if known.contains(&a) || kept.contains(&a) {
                // tombe dans du code connu ou déjà retenu
                ok = !run.is_empty();
                break;
            }
            if !(0x0800_00C0..0x0804_0000).contains(&a) || data.contains(&a) {
                break;
            }
            let op = rom16(rom, a);
            let top = op >> 11;
            let prev_top = run.last().map(|i: &Insn| i.op >> 11);
            let undefined = op == 0
                || top == 29
                || matches!(op >> 8, 0xB1 | 0xB2 | 0xB3 | 0xB6 | 0xB7 | 0xB8 | 0xB9 | 0xBA | 0xBB | 0xBE | 0xBF | 0xDE)
                || (top == 31) != (prev_top == Some(30));
            if undefined {
                break;
            }
            if top == 31 {
                // appel complet : la cible doit être dans la zone de code, hors constantes
                let hi = run.last().map(|i| i.op).unwrap_or(0);
                let t = (a - 2).wrapping_add(4).wrapping_add((((hi & 0x7FF) << 21) as i32 >> 9) as u32).wrapping_add((op & 0x7FF) << 1);
                if !(0x0800_00C0..0x0804_0000).contains(&t) || t & 1 != 0 || data.contains(&t) {
                    run.clear();
                    break;
                }
                targets.push(t);
            }
            run.push(Insn { addr: a, op, arm: false });
            if (26..=27).contains(&top) && (op >> 8) & 15 < 14 {
                targets.push(a.wrapping_add(4).wrapping_add(((op & 0xFF) as i8 as i32 * 2) as u32));
            }
            if top == 28 {
                targets.push(a.wrapping_add(4).wrapping_add((((op & 0x7FF) << 21) as i32 >> 20) as u32));
            }
            if ends_block(op) && top != 31 {
                ok = true;
                break;
            }
            a += 2;
        }
        if ok {
            starts.insert(entry);
            kept.extend(run.iter().map(|i| i.addr));
            insns.extend(run);
            todo.extend(targets);
        }
    }
    insns.sort_by_key(|i| i.addr);
    insns.dedup_by_key(|i| i.addr);
    let found = insns.len() - listed;
    // saut d'entrée de la cartouche (première instruction exécutée, en ARM)
    if !insns.iter().any(|i| i.addr == 0x0800_0000) {
        insns.push(Insn { addr: 0x0800_0000, op: rom32(rom, 0x0800_0000), arm: true });
        insns.sort_by_key(|i| i.addr);
    }
    starts.insert(0x0800_0000);
    Ok(Parsed { insns, starts, found, data })
}
