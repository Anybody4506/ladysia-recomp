//! Code du jeu recompilé en Rust (recompilation statique).
//!
//! Tout le code Thumb de la ROM est traduit, bloc par bloc (suite d'instructions sans
//! saut), en fonctions Rust compilées (`code/`, écrit par la commande `recomp`, voir
//! `recomp_gen.rs`). Quand le processeur arrive au début d'un bloc, sa fonction s'exécute
//! à la place de l'interpréteur ; les interruptions, le temps et l'arrêt du processeur
//! sont traités entre deux blocs, comme entre deux instructions. Les fonctions réécrites
//! à la main (`jeu/`) restent prioritaires. Le code ARM est recompilé aussi, y compris le
//! répartiteur d'interruptions recopié en IWRAM (utilisé seulement si l'IWRAM contient
//! exactement la copie attendue).
//!
//! Le code généré ne vaut que pour la ROM d'origine (empreinte de la zone de code) ; avec
//! une autre ROM, ou `LADYSIA_RECOMP=0`, tout est interprété.
//! Vérification `LADYSIA_RECOMP_CHECK=1` : les premières exécutions de chaque bloc sont
//! refaites par l'interpréteur sur une copie de la machine et comparées (registres,
//! drapeaux, cycles, mémoire).

mod code;

use crate::gba::bus::Bus;
use crate::gba::cpu::Cpu;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub type Block = fn(&mut Cpu, &mut Bus);

/// Index des blocs par demi-mot : zone de code de la ROM, puis IWRAM (0 = pas de bloc,
/// sinon rang + 1).
static TABLE: OnceLock<(Vec<u32>, Vec<u32>, Vec<u32>)> = OnceLock::new();
static ON: AtomicBool = AtomicBool::new(false);
static CHECK: AtomicBool = AtomicBool::new(false);
/// Données sans code machine (version finale) : rien ne peut être interprété.
static DATA: AtomicBool = AtomicBool::new(false);
/// Première adresse où il aurait fallu interpréter du code absent (0 : aucune).
pub static MISSING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static RUNS: AtomicU64 = AtomicU64::new(0);
/// Instructions encore interprétées par la boucle principale (hors appels des fonctions réécrites).
pub static INTERPRETED: AtomicU64 = AtomicU64::new(0);
pub static CHECKED: AtomicU64 = AtomicU64::new(0);
pub static MISMATCHES: AtomicU64 = AtomicU64::new(0);

/// Prépare la table ; renvoie vrai si le code recompilé correspond à la ROM.
pub fn init(rom: &[u8]) -> bool {
    let h = crate::recomp_gen::code_hash(rom);
    let data = h == code::DATA_HASH;
    DATA.store(data, Ordering::Relaxed);
    let ok = !code::BLOCKS.is_empty()
        && (data || h == code::CODE_HASH && std::env::var("LADYSIA_RECOMP").map(|v| v != "0").unwrap_or(true));
    if ok {
        TABLE.get_or_init(|| {
            let mut t = vec![0u32; (crate::jeu::CODE_SIZE / 2) as usize];
            let mut w = vec![0u32; 0x8000 / 2];
            let mut bios = vec![0u32; 0x4000 / 2];
            for (k, (a, _, _)) in code::BLOCKS.iter().enumerate() {
                if *a < 0x4000 {
                    bios[(a / 2) as usize] = k as u32 + 1;
                } else if a >> 24 == 3 {
                    w[((a - 0x0300_0000) / 2) as usize] = k as u32 + 1;
                } else {
                    t[((a - 0x0800_0000) / 2) as usize] = k as u32 + 1;
                }
            }
            (t, w, bios)
        });
        CHECK.store(!data && std::env::var("LADYSIA_RECOMP_CHECK").is_ok_and(|v| v == "1"), Ordering::Relaxed);
    }
    ON.store(ok, Ordering::Relaxed);
    ok
}

/// Vrai avec les données sans code machine : tout le code vient du Rust.
pub fn data_only() -> bool {
    DATA.load(Ordering::Relaxed)
}

/// À appeler quand une instruction en `pc` va être interprétée.
#[inline]
pub fn interpreting(pc: u32) {
    INTERPRETED.fetch_add(1, Ordering::Relaxed);
    if DATA.load(Ordering::Relaxed) {
        let _ = MISSING.compare_exchange(0, pc | 1 << 31, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// Copies de code en IWRAM et empreinte de leur code d'origine.
pub fn copies() -> &'static [(u32, u32, u32, u64)] {
    &code::COPIES
}

pub fn active() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn block_count() -> usize {
    code::BLOCKS.len()
}

/// Bloc recompilé qui commence en `pc` dans le mode courant (Thumb ou ARM). En IWRAM,
/// seulement si la mémoire contient la copie de la ROM dont le bloc a été tiré.
#[inline]
pub fn lookup(pc: u32, thumb: bool, bus: &Bus) -> Option<usize> {
    if !ON.load(Ordering::Relaxed) {
        return None;
    }
    let (rom, iw, bios) = TABLE.get()?;
    let off = pc.wrapping_sub(0x0800_0000);
    let i = if off < crate::jeu::CODE_SIZE {
        rom[(off >> 1) as usize]
    } else if pc < 0x4000 {
        bios[(pc >> 1) as usize]
    } else {
        let o = pc.wrapping_sub(0x0300_0000);
        if o >= 0x8000 {
            return None;
        }
        let i = iw[(o >> 1) as usize];
        if i == 0 {
            return None;
        }
        let ok = crate::recomp_gen::IWRAM_COPIES
            .iter()
            .any(|&(d, s, n)| (d..d + n).contains(&pc) && bus.iwram_is_copy(d, s, n));
        if !ok {
            return None;
        }
        i
    };
    if i == 0 || code::BLOCKS[i as usize - 1].1 == thumb {
        return None;
    }
    Some(i as usize - 1)
}

/// Nombre d'exécutions vérifiées par bloc.
const CHECKS_PER_BLOCK: u8 = 3;

/// Exécute le bloc `i` (vérifié contre l'interpréteur ses premières fois si demandé).
#[inline]
pub fn run(i: usize, cpu: &mut Cpu, bus: &mut Bus) {
    RUNS.fetch_add(1, Ordering::Relaxed);
    if !CHECK.load(Ordering::Relaxed) {
        (code::BLOCKS[i].2)(cpu, bus);
        return;
    }
    static SEEN: OnceLock<std::sync::Mutex<Vec<u8>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| std::sync::Mutex::new(vec![0; code::BLOCKS.len()]));
    let first = {
        let mut s = seen.lock().unwrap();
        let f = s[i] < CHECKS_PER_BLOCK;
        if f {
            s[i] += 1;
        }
        f
    };
    if !first {
        (code::BLOCKS[i].2)(cpu, bus);
        return;
    }
    let (mut c2, mut b2) = (cpu.clone(), bus.clone());
    let start = cpu.r[15];
    (code::BLOCKS[i].2)(cpu, bus);
    let end = cpu.r[15];
    let mut steps = 0;
    loop {
        c2.step(&mut b2);
        steps += 1;
        if c2.r[15] == end || steps > 10_000 {
            break;
        }
    }
    CHECKED.fetch_add(1, Ordering::Relaxed);
    let mut d = Vec::new();
    for r in 0..16 {
        if c2.r[r] != cpu.r[r] {
            d.push(format!("r{r} : interprété {:08X}, recompilé {:08X}", c2.r[r], cpu.r[r]));
        }
    }
    if c2.cpsr != cpu.cpsr {
        d.push(format!("cpsr : {:08X} / {:08X}", c2.cpsr, cpu.cpsr));
    }
    if b2.cyc != bus.cyc {
        d.push(format!("cycles : {} / {}", b2.cyc, bus.cyc));
    }
    if b2.halt_req != bus.halt_req {
        d.push("arrêt du processeur".into());
    }
    d.extend(b2.diff(bus, 0).into_iter().map(|(_, _, t)| t));
    if !d.is_empty() {
        let n = MISMATCHES.fetch_add(1, Ordering::Relaxed);
        if n < 30 {
            eprintln!("[recomp] bloc {start:08X} ({steps} instructions) : {}", d.iter().take(6).cloned().collect::<Vec<_>>().join(" ; "));
        }
    }
}
