//! Étape 2 : parcours récursif du code pour retrouver toutes les fonctions.
//!
//! Point de départ : le point d'entrée (crt0 ARM). On suit ensuite les sauts,
//! les appels `bl`, les appels indirects via `bx rN` dont la valeur est connue,
//! les tables de `switch`, puis on complète avec les pointeurs de fonctions
//! trouvés dans les données (callbacks).

use crate::arm;
use crate::rom::Rom;
use crate::thumb::{self, Flow};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Arm,
    Thumb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Point d'entrée de la cartouche.
    Entry,
    /// Cible d'un `bl`.
    Call,
    /// Adresse chargée puis appelée via `bx`.
    Indirect,
    /// Pointeur trouvé dans les données qui pointe sur un `push {.., lr}`.
    Callback,
    /// Déclarée dans symbols.txt.
    Symbol,
}

pub struct Line {
    pub size: u32,
    pub raw: u32,
    pub text: String,
    /// Cible de saut/appel, pour remplacer l'adresse par un label.
    pub target: Option<u32>,
}

pub struct Function {
    pub mode: Mode,
    pub origin: Origin,
    pub lines: BTreeMap<u32, Line>,
    pub calls: BTreeSet<u32>,
    /// Adresses des littéraux (et tables de saut) utilisés.
    pub literals: BTreeSet<u32>,
    pub invalid: usize,
}

impl Function {
    pub fn code_bytes(&self) -> u32 {
        self.lines.values().map(|l| l.size).sum()
    }
}

#[derive(Default)]
pub struct Analysis {
    pub funcs: BTreeMap<u32, Function>,
    /// Adresse d'instruction -> fonction propriétaire.
    owner: HashMap<u32, u32>,
    queue: Vec<(u32, Mode, Origin)>,
    pub labels: BTreeSet<u32>,
    pub swi: BTreeMap<u32, usize>,
    pub jump_tables: usize,
    /// Appels vers du code en RAM (copié depuis la ROM au démarrage).
    pub ram_targets: BTreeSet<u32>,
    /// Noms donnés à la main (symbols.txt).
    pub names: HashMap<u32, String>,
    /// Appels avec la valeur connue de r0-r2 : (appelant, appelé, arguments).
    pub call_args: Vec<(u32, u32, [Option<u32>; 3])>,
    /// Fin de la zone de code : un littéral Thumb (impair) en dessous est une fonction.
    pub code_limit: u32,
}

/// Instruction décodée, quel que soit le mode.
struct Step {
    size: u32,
    raw: u32,
    text: String,
    flow: Flow,
    literal: Option<(u32, u32)>,
    clobber: u32,
    pc_rel: Option<u32>,
}

fn step(rom: &Rom, pc: u32, mode: Mode) -> Option<Step> {
    let off = rom.addr_to_off(pc)?;
    match mode {
        Mode::Thumb => {
            let h = rom.u16(off)?;
            let t = thumb::decode(h, rom.u16(off + 2), pc);
            let raw = if t.size == 4 { (h as u32) << 16 | rom.u16(off + 2)? as u32 } else { h as u32 };
            Some(Step {
                size: t.size,
                raw,
                text: t.text,
                flow: t.flow,
                literal: t.literal,
                clobber: t.clobber,
                pc_rel: t.pc_rel,
            })
        }
        Mode::Arm => {
            let w = rom.u32(off)?;
            let a = arm::decode(w, pc);
            let flow = if let Some(r) = a.bx_reg {
                match (a.always, r) {
                    (false, _) => Flow::Next,
                    (true, 14) => Flow::Return,
                    (true, r) => Flow::BxReg(r),
                }
            } else if let Some(t) = a.branch {
                if a.link {
                    Flow::Call(t)
                } else if a.always {
                    Flow::Jump(t)
                } else {
                    Flow::CondJump(t)
                }
            } else if a.writes_pc && a.always {
                Flow::Return
            } else if let Some(n) = a.swi {
                Flow::Swi(n)
            } else if a.text.starts_with(".word") {
                Flow::Invalid
            } else {
                Flow::Next
            };
            Some(Step {
                size: 4,
                raw: w,
                text: a.text,
                flow,
                literal: a.literal,
                clobber: a.clobber,
                pc_rel: a.pc_rel,
            })
        }
    }
}

impl Analysis {
    pub fn enqueue(&mut self, rom: &Rom, target: u32, mode: Mode, origin: Origin) {
        let a = target & !1;
        if rom.addr_to_off(a).is_none() {
            if (0x0200_0000..0x0400_0000).contains(&a) {
                self.ram_targets.insert(a);
            }
            return;
        }
        self.queue.push((a, mode, origin));
    }

    pub fn run(&mut self, rom: &Rom) {
        while let Some((a, m, o)) = self.queue.pop() {
            self.explore(rom, a, m, o);
        }
    }

    /// Ajoute les pointeurs Thumb (bit 0 à 1) qui visent un prologue `push {.., lr}`.
    pub fn scan_callbacks(&mut self, rom: &Rom) {
        for off in (0..rom.len().saturating_sub(3)).step_by(4) {
            let v = rom.u32(off).unwrap();
            if v & 1 == 0 {
                continue;
            }
            if let Some(t) = rom.addr_to_off(v) {
                if rom.u16(t).is_some_and(|h| h & 0xFF00 == 0xB500) {
                    self.enqueue(rom, v, Mode::Thumb, Origin::Callback);
                }
            }
        }
    }

    fn explore(&mut self, rom: &Rom, start: u32, mode: Mode, origin: Origin) {
        if self.funcs.contains_key(&start) || self.owner.contains_key(&start) {
            return;
        }
        let mut f = Function {
            mode,
            origin,
            lines: BTreeMap::new(),
            calls: BTreeSet::new(),
            literals: BTreeSet::new(),
            invalid: 0,
        };
        let mut blocks = vec![start];
        let mut new_funcs = Vec::new();

        while let Some(b) = blocks.pop() {
            let mut pc = b;
            let mut regs = [None::<u32>; 16];
            let mut last_table = None;
            let mut lr_set = false;
            loop {
                if f.lines.contains_key(&pc) || self.owner.contains_key(&pc) {
                    break;
                }
                let Some(s) = step(rom, pc, mode) else { break };
                let mut target = None;
                let mut stop = false;
                match s.flow {
                    Flow::Next => {}
                    Flow::Jump(t) => {
                        target = Some(t);
                        self.labels.insert(t);
                        blocks.push(t);
                        stop = true;
                    }
                    Flow::CondJump(t) => {
                        target = Some(t);
                        self.labels.insert(t);
                        blocks.push(t);
                    }
                    Flow::Call(t) => {
                        target = Some(t);
                        f.calls.insert(t);
                        self.call_args.push((start, t, [regs[0], regs[1], regs[2]]));
                        new_funcs.push((t, mode, Origin::Call));
                        // Appel via un « trampoline » `bx rN` (__call_via_rN de GCC).
                        if mode == Mode::Thumb {
                            if let Some(h) = rom.addr_to_off(t).and_then(|o| rom.u16(o)) {
                                if h & 0xFF87 == 0x4700 {
                                    if let Some(v) = regs[((h >> 3) & 15) as usize] {
                                        f.calls.insert(v & !1);
                                        let m = if v & 1 != 0 { Mode::Thumb } else { Mode::Arm };
                                        new_funcs.push((v, m, Origin::Indirect));
                                    }
                                }
                            }
                        }
                    }
                    Flow::Return => stop = true,
                    Flow::BxReg(r) => {
                        // `bx pc` en Thumb : passage en ARM à l'adresse alignée suivante (veneer GCC).
                        let known = if r == 15 && mode == Mode::Thumb { Some((pc + 4) & !3) } else { regs[r as usize] };
                        if let Some(v) = known {
                            f.calls.insert(v & !1);
                            let m = if v & 1 != 0 { Mode::Thumb } else { Mode::Arm };
                            new_funcs.push((v, m, Origin::Indirect));
                        }
                        // `mov lr, pc ; bx rN` est un appel : l'exécution revient ensuite.
                        stop = !lr_set;
                    }
                    Flow::IndirectJump(_) => {
                        if let Some(tab) = last_table {
                            self.read_jump_table(rom, start, tab, &mut f, &mut blocks);
                        }
                        stop = true;
                    }
                    Flow::Swi(n) => *self.swi.entry(n).or_default() += 1,
                    Flow::Invalid => {
                        f.invalid += 1;
                        stop = true;
                    }
                }
                if origin == Origin::Entry {
                    if let Some(a) = s.pc_rel {
                        // Le crt0 calcule l'adresse du gestionnaire d'interruptions ainsi.
                        new_funcs.push((a, Mode::Arm, Origin::Indirect));
                    }
                }
                for (i, slot) in regs.iter_mut().enumerate() {
                    if s.clobber & (1 << i) != 0 {
                        *slot = None;
                    }
                }
                if let Some((rd, la)) = s.literal {
                    f.literals.insert(la);
                    let v = rom.addr_to_off(la).and_then(|o| rom.u32(o));
                    regs[rd as usize] = v;
                    if v.is_some_and(|v| rom.is_rom_ptr(v)) {
                        last_table = v;
                    }
                    // Pointeur de fonction Thumb dans la zone de code (gestionnaire d'état, callback).
                    if let Some(v) = v.filter(|v| v & 1 != 0 && rom.is_rom_ptr(*v) && *v < self.code_limit) {
                        new_funcs.push((v, Mode::Thumb, Origin::Callback));
                    }
                }
                lr_set = s.text == "mov lr, pc";
                f.lines.insert(pc, Line { size: s.size, raw: s.raw, text: s.text, target });
                if stop {
                    break;
                }
                pc += s.size;
            }
        }

        for &a in f.lines.keys() {
            self.owner.insert(a, start);
        }
        self.funcs.insert(start, f);
        for (t, m, o) in new_funcs {
            self.enqueue(rom, t, m, o);
        }
    }

    /// Table de `switch` GCC : `ldr rX, =table ; ... ; mov pc, rY`.
    fn read_jump_table(&mut self, rom: &Rom, start: u32, tab: u32, f: &mut Function, blocks: &mut Vec<u32>) {
        let Some(toff) = rom.addr_to_off(tab) else { return };
        let mut n = 0;
        for i in 0..256 {
            let Some(v) = rom.u32(toff + i * 4) else { break };
            let t = v & !1;
            if !rom.is_rom_ptr(v) || t.abs_diff(start) > 0x2_0000 {
                break;
            }
            f.literals.insert(tab + i as u32 * 4);
            self.labels.insert(t);
            blocks.push(t);
            n += 1;
        }
        if n > 0 {
            self.jump_tables += 1;
        }
    }

    pub fn callers(&self) -> HashMap<u32, usize> {
        let mut m = HashMap::new();
        for f in self.funcs.values() {
            for &c in &f.calls {
                *m.entry(c).or_default() += 1;
            }
        }
        m
    }

    pub fn label(&self, a: u32) -> String {
        if let Some(n) = self.names.get(&(a & !1)) {
            n.clone()
        } else if self.funcs.contains_key(&(a & !1)) {
            format!("sub_{:08X}", a & !1)
        } else {
            format!("loc_{:08X}", a & !1)
        }
    }
}

/// Charge symbols.txt : noms de fonctions et tables de pointeurs de fonctions.
///
/// ```text
/// # commentaire
/// 08000308 AgbMain            (ajouter 1 à l'adresse pour une fonction Thumb)
/// table 0835182C 15 IntrHandler
/// ```
pub fn load_symbols(an: &mut Analysis, rom: &Rom, text: &str) {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["table", addr, count, prefix] => {
                let (Ok(addr), Ok(count)) = (u32::from_str_radix(addr, 16), count.parse::<u32>()) else {
                    continue;
                };
                for i in 0..count {
                    let Some(v) = rom.addr_to_off(addr + i * 4).and_then(|o| rom.u32(o)) else { break };
                    an.names.entry(v & !1).or_insert_with(|| format!("{prefix}_{i:02}"));
                    let m = if v & 1 != 0 { Mode::Thumb } else { Mode::Arm };
                    an.enqueue(rom, v, m, Origin::Symbol);
                }
            }
            [addr, name, ..] => {
                let Ok(v) = u32::from_str_radix(addr, 16) else { continue };
                an.names.insert(v & !1, name.to_string());
                let m = if v & 1 != 0 { Mode::Thumb } else { Mode::Arm };
                an.enqueue(rom, v, m, Origin::Symbol);
            }
            _ => {}
        }
    }
}
