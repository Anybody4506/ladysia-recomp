//! Décompilation de Lady Sia : analyse de la ROM et extraction des ressources.
#![cfg_attr(feature = "embarque", windows_subsystem = "windows")]
#![cfg_attr(feature = "embarque", allow(dead_code, unused_imports))]
//! Usage : ladysia-decomp <chemin.gba> [play | extract | music | view]

mod analysis;
mod arm;
mod behaviour;
mod bios;
mod emu;
mod extract;
mod filtres;
mod game;
mod gba;
mod icones;
mod jeu;
mod levels;
mod listing;
mod lzr;
mod menu;
mod mods;
mod music;
mod objects;
mod packed;
mod psg;
mod recomp;
mod recomp_gen;
mod png;
mod rle;
mod rom;
mod scan;
mod sound;
mod thumb;
mod viewer;

use analysis::{Analysis, Mode, Origin};
use rom::{Header, Rom, ROM_BASE};
use scan::fmt_addr;
use std::fmt::Write as _;

fn main() {
    // version finale (`embarque`) : seulement le jeu, sans les outils de développement
    #[cfg(feature = "embarque")]
    game::run_embedded();
    #[cfg(not(feature = "embarque"))]
    outils();
}

/// Outils de développement : analyse de la ROM, extraction, recompilation, jeu sur fichier.
#[cfg(not(feature = "embarque"))]
fn outils() {
    let path = std::env::args().nth(1).expect("usage : ladysia-decomp <rom.gba>");
    let rom = Rom::load(&path).expect("impossible de lire la ROM");
    if std::env::args().nth(2).as_deref() == Some("extract") {
        print!("{}", extract::run(&rom).expect("extraction"));
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("armdis") {
        // armdis <fichier> <adresse de base> <début> <fin> : désassemble du code ARM
        let a: Vec<String> = std::env::args().collect();
        let data = std::fs::read(&a[3]).expect("fichier");
        let hex = |s: &str| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap();
        let (base, from, to) = (hex(&a[4]), hex(&a[5]), hex(&a[6]));
        let mut at = from;
        while at < to {
            let o = (at - base) as usize;
            let w = u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
            println!("{at:08X}: {w:08X}  {}", arm::decode(w, at).text);
            at += 4;
        }
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("recomp") {
        // recompilation statique : asm/ladysia.s -> src/recomp/code/
        match recomp_gen::run(&rom.data, "asm/ladysia.s", "src/recomp") {
            Ok(r) => print!("{r}"),
            Err(e) => {
                eprintln!("recomp : {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("donnees") {
        // données du jeu sans code machine (version finale) : <rom> donnees <sortie>
        let out = std::env::args().nth(3).expect("usage : ladysia-decomp <rom.gba> donnees <sortie>");
        match recomp_gen::data_blob(&rom.data, "asm/ladysia.s") {
            Ok(d) => {
                let zeros = rom.data.iter().zip(&d).filter(|(a, b)| a != b).count();
                // compressées (format `lzr`), relues pour vérification
                let c = lzr::compress(&d);
                assert!(lzr::decompress(&c).as_deref() == Some(&d[..]), "compression non réversible");
                std::fs::write(&out, &c).expect("écriture des données");
                println!("{out} : {} octets ({} avant compression), {zeros} octets de code et d'en-tête effacés", c.len(), d.len());
            }
            Err(e) => {
                eprintln!("donnees : {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("play") {
        if let Err(e) = game::run(&path) {
            eprintln!("jeu : {e}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("view") {
        let rom: &'static Rom = Box::leak(Box::new(rom));
        if let Err(e) = viewer::run(rom) {
            eprintln!("visionneuse : {e}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("music") {
        print!("{}", music::run(&rom).expect("musique"));
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("unpack") {
        // ladysia-decomp <rom> unpack <adresse hex> <sortie>
        let a = u32::from_str_radix(&std::env::args().nth(3).expect("adresse"), 16).expect("hex");
        let t = packed::Tables::from_rom(&rom);
        let u = packed::decompress(&t, &rom.data[(a - 0x0800_0000) as usize..]).expect("décodage");
        std::fs::write(std::env::args().nth(4).expect("sortie"), &u.data).unwrap();
        println!("{:08X} : {} octets", a, u.data.len());
        return;
    }
    let mut r = String::new();

    // --- En-tête ---
    let h = Header::parse(&rom);
    writeln!(r, "== En-tête ==").unwrap();
    writeln!(r, "Titre       : {}", h.title).unwrap();
    writeln!(r, "Code jeu    : {} (maker {})", h.game_code, h.maker).unwrap();
    writeln!(r, "Version     : {}", h.version).unwrap();
    writeln!(r, "Taille ROM  : {} Mo", rom.len() / (1024 * 1024)).unwrap();
    writeln!(r, "Octet fixe  : 0x{:02x} ({})", h.fixed_96, if h.fixed_96 == 0x96 { "ok" } else { "INVALIDE" }).unwrap();
    writeln!(
        r,
        "Checksum    : 0x{:02x} (calculé 0x{:02x}, {})",
        h.checksum,
        h.checksum_calc,
        if h.checksum == h.checksum_calc { "ok" } else { "DIFFÉRENT" }
    )
    .unwrap();
    writeln!(r, "Entrée      : 0x{:08x} -> {}", h.entry_branch, fmt_addr(h.entry_off)).unwrap();

    // --- crt0 : code ARM de démarrage ---
    writeln!(r, "\n== crt0 (ARM, depuis le point d'entrée) ==").unwrap();
    let mut regs = [None::<u32>; 16];
    let mut main_candidates = Vec::new();
    let mut off = h.entry_off;
    for _ in 0..64 {
        let Some(word) = rom.u32(off) else { break };
        let addr = ROM_BASE + off as u32;
        let ins = arm::decode(word, addr);
        let mut note = String::new();
        if let Some((rd, lit)) = ins.literal {
            let val = rom.addr_to_off(lit).and_then(|o| rom.u32(o));
            regs[rd as usize] = val;
            if let Some(v) = val {
                note = format!("  ; {} = 0x{v:08x}", arm::reg(rd));
            }
        }
        if let Some(rn) = ins.bx_reg {
            if let Some(v) = regs[rn as usize] {
                let mode = if v & 1 != 0 { "Thumb" } else { "ARM" };
                note = format!("  ; -> 0x{:08x} ({mode})", v & !1);
                if rom.is_rom_ptr(v) {
                    main_candidates.push(v);
                }
            }
        }
        writeln!(r, "{addr:08x}: {word:08x}  {}{note}", ins.text).unwrap();
        // Fin du crt0 : saut inconditionnel sans retour.
        if word >> 28 == 0xE && (ins.text.starts_with("b 0x") || (ins.bx_reg.is_some() && note.is_empty())) {
            break;
        }
        off += 4;
    }
    for v in &main_candidates {
        writeln!(r, "Candidat AgbMain : 0x{:08x}", v & !1).unwrap();
    }

    // --- Carte de la ROM ---
    writeln!(r, "\n== Carte de la ROM (blocs de 64 Ko) ==").unwrap();
    writeln!(r, "{:<12} {:>8} {:>8} {:>6}  {}", "adresse", "ptrs", "entropie", "rempl.", "nature probable").unwrap();
    for b in scan::blocks(&rom) {
        writeln!(
            r,
            "{:<12} {:>8} {:>8.2} {:>5.0}%  {}",
            fmt_addr(b.off),
            b.ptr_count,
            b.entropy,
            b.fill_ratio * 100.0,
            b.guess()
        )
        .unwrap();
    }

    // --- Données compressées ---
    let targets = scan::pointer_targets(&rom);
    let lz = scan::find_lz77(&rom, &targets);
    let packed: usize = lz.iter().map(|l| l.packed_len).sum();
    let unpacked: usize = lz.iter().map(|l| l.unpacked_len).sum();
    writeln!(r, "\n== Blocs LZ77 référencés : {} ==", lz.len()).unwrap();
    writeln!(r, "Total : {} Ko compressés -> {} Ko décompressés", packed / 1024, unpacked / 1024).unwrap();
    for l in lz.iter().take(20) {
        writeln!(r, "  {}  {:>6} -> {:>6} octets", fmt_addr(l.off), l.packed_len, l.unpacked_len).unwrap();
    }
    if lz.len() > 20 {
        writeln!(r, "  ... (liste complète dans lz77.txt)").unwrap();
    }

    // --- Moteur son ---
    writeln!(r, "\n== Moteur son ==").unwrap();
    match scan::find_m4a(&rom) {
        Some(m) => {
            writeln!(r, "Moteur Nintendo m4a (Sappy) détecté : SelectSong à {}", fmt_addr(m.select_song_off)).unwrap();
            match m.song_table {
                Some(t) => writeln!(r, "Table des morceaux : 0x{t:08x} ({} entrées)", m.song_count).unwrap(),
                None => writeln!(r, "Table des morceaux introuvable à l'offset attendu").unwrap(),
            }
        }
        None => writeln!(r, "Pas de moteur m4a standard : moteur son propriétaire (à identifier)").unwrap(),
    }

    // --- Étape 2 : recherche des fonctions ---
    let mut an = Analysis::default();
    an.code_limit = 0x0803_7B40;
    an.enqueue(&rom, ROM_BASE + h.entry_off as u32, Mode::Arm, Origin::Entry);
    an.run(&rom);
    if let Ok(sym) = std::fs::read_to_string("symbols.txt") {
        analysis::load_symbols(&mut an, &rom, &sym);
        an.run(&rom);
    }
    let from_entry = an.funcs.len();
    an.scan_callbacks(&rom);
    an.run(&rom);

    let code: u32 = an.funcs.values().map(|f| f.code_bytes()).sum();
    let count = |o: Origin| an.funcs.values().filter(|f| f.origin == o).count();
    writeln!(r, "\n== Fonctions ==").unwrap();
    writeln!(r, "Total            : {}", an.funcs.len()).unwrap();
    writeln!(r, "  depuis l'entrée : {from_entry} (appels directs et indirects)").unwrap();
    writeln!(r, "  via bl          : {}", count(Origin::Call)).unwrap();
    writeln!(r, "  appels indirects: {}", count(Origin::Indirect)).unwrap();
    writeln!(r, "  callbacks       : {}", count(Origin::Callback)).unwrap();
    writeln!(r, "  symbols.txt     : {}", count(Origin::Symbol)).unwrap();
    writeln!(r, "Fonctions nommées : {}", an.funcs.keys().filter(|a| an.names.contains_key(a)).count()).unwrap();
    writeln!(r, "  en ARM          : {}", an.funcs.values().filter(|f| f.mode == Mode::Arm).count()).unwrap();
    writeln!(
        r,
        "Code désassemblé : {} Ko ({:.1} % de la ROM)",
        code / 1024,
        code as f64 * 100.0 / rom.len() as f64
    )
    .unwrap();
    writeln!(r, "Tables de switch : {}", an.jump_tables).unwrap();
    let invalid = an.funcs.values().filter(|f| f.invalid > 0).count();
    writeln!(r, "Fonctions avec instruction invalide (à vérifier) : {invalid}").unwrap();

    let callers = an.callers();
    let mut top: Vec<_> = callers.iter().filter(|(a, _)| an.funcs.contains_key(a)).collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    writeln!(r, "\nFonctions les plus appelées (outils de base : copie mémoire, maths...) :").unwrap();
    for (a, n) in top.iter().take(15) {
        writeln!(r, "  {}  {n} appelants  ({} octets)", an.label(**a), an.funcs[a].code_bytes()).unwrap();
    }

    writeln!(r, "\nAppels BIOS (SWI) :").unwrap();
    for (n, c) in &an.swi {
        writeln!(r, "  0x{n:02x} {:<22} x{c}", bios::swi_name(*n)).unwrap();
    }
    if !an.ram_targets.is_empty() {
        writeln!(r, "\nAppels vers du code en RAM (copié au démarrage, à extraire) :").unwrap();
        for a in &an.ram_targets {
            writeln!(r, "  0x{a:08x} ({})", bios::region(*a).unwrap_or("?")).unwrap();
        }
    }

    // Fonctions appelées avec un pointeur vers les données de la ROM (après la fin du code) :
    // ce sont les chargeurs de ressources, dont la décompression maison.
    let code_end = an
        .funcs
        .iter()
        .filter(|(_, f)| f.origin != Origin::Callback)
        .map(|(a, f)| a + f.code_bytes())
        .max()
        .unwrap_or(ROM_BASE);
    let mut loaders: std::collections::BTreeMap<u32, (usize, Vec<u32>)> = Default::default();
    for (_, callee, args) in &an.call_args {
        for v in args.iter().flatten() {
            if rom.is_rom_ptr(*v) && *v > code_end + 0x1000 {
                let e = loaders.entry(*callee).or_default();
                e.0 += 1;
                if e.1.len() < 3 {
                    e.1.push(*v);
                }
            }
        }
    }
    let mut loaders: Vec<_> = loaders.into_iter().collect();
    loaders.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    writeln!(r, "\nFin estimée du code : 0x{code_end:08x}").unwrap();
    writeln!(r, "Fonctions recevant des pointeurs vers les données (chargeurs de ressources) :").unwrap();
    for (callee, (n, ex)) in loaders.iter().take(15) {
        let tags = an.funcs.get(callee).map(|f| listing::tags(&rom, f).join(",")).unwrap_or_default();
        let ex: Vec<String> = ex.iter().map(|v| format!("0x{v:08x}")).collect();
        writeln!(r, "  {:<22} x{n:<4} [{tags}] ex. {}", an.label(*callee), ex.join(" ")).unwrap();
    }

    std::fs::create_dir_all("asm").expect("création du dossier asm");
    std::fs::write("asm/ladysia.s", listing::write_asm(&an, &rom)).expect("écriture asm/ladysia.s");
    std::fs::write("fonctions.txt", listing::write_function_list(&an, &rom)).expect("écriture fonctions.txt");

    print!("{r}");
    std::fs::write("rapport.txt", &r).expect("écriture rapport.txt");
    let list: String = lz
        .iter()
        .map(|l| format!("{} {} {}\n", fmt_addr(l.off), l.packed_len, l.unpacked_len))
        .collect();
    std::fs::write("lz77.txt", list).expect("écriture lz77.txt");
}
