//! Processeur ARM7TDMI complet : jeux d'instructions ARM et Thumb, modes et
//! registres en banque, interruptions. Les appels BIOS (SWI) sont traités en Rust
//! (voir `hle.rs`) : il n'y a pas besoin du BIOS d'origine.

use super::bus::Bus;

pub const USR: u32 = 0x10;
pub const FIQ: u32 = 0x11;
pub const IRQ: u32 = 0x12;
pub const SVC: u32 = 0x13;
pub const SYS: u32 = 0x1F;

const T: u32 = 1 << 5;
const I: u32 = 1 << 7;
const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;

#[derive(Clone)]
pub struct Cpu {
    /// r15 vaut, pendant l'exécution, l'adresse de l'instruction + 8 (ARM) ou + 4 (Thumb).
    pub r: [u32; 16],
    pub cpsr: u32,
    pub spsr: u32,
    bank_sp: [u32; 6],
    bank_lr: [u32; 6],
    bank_spsr: [u32; 6],
    fiq_hi: [u32; 5],
    usr_hi: [u32; 5],
    branched: bool,
    /// Processeur arrêté jusqu'à la prochaine interruption (Halt, IntrWait).
    pub halted: bool,
    /// Un IntrWait est en cours (on ré-exécute le SWI après chaque interruption).
    pub waiting: bool,
}

fn bank(mode: u32) -> usize {
    match mode {
        FIQ => 1,
        IRQ => 2,
        SVC => 3,
        0x17 => 4,
        0x1B => 5,
        _ => 0,
    }
}

impl Cpu {
    /// État laissé par le BIOS au lancement de la cartouche.
    pub fn new() -> Self {
        let mut c = Cpu {
            r: [0; 16],
            cpsr: SYS,
            spsr: 0,
            bank_sp: [0; 6],
            bank_lr: [0; 6],
            bank_spsr: [0; 6],
            fiq_hi: [0; 5],
            usr_hi: [0; 5],
            branched: false,
            halted: false,
            waiting: false,
        };
        c.reset();
        c
    }

    pub fn reset(&mut self) {
        self.cpsr = SYS;
        self.bank_sp[bank(IRQ)] = 0x0300_7FA0;
        self.bank_sp[bank(SVC)] = 0x0300_7FE0;
        self.r = [0; 16];
        self.r[13] = 0x0300_7F00;
        self.r[15] = 0x0800_0000;
        self.halted = false;
        self.waiting = false;
    }

    /// Retour d'une fonction (comme `bx lr`).
    pub fn ret(&mut self, lr: u32) {
        self.bx(lr);
    }

    pub fn state(&self, w: &mut Vec<u8>) {
        let words = self.r.iter().copied().chain([self.cpsr, self.spsr]).chain(self.bank_sp.iter().copied())
            .chain(self.bank_lr.iter().copied()).chain(self.bank_spsr.iter().copied()).chain(self.fiq_hi.iter().copied()).chain(self.usr_hi.iter().copied());
        for v in words {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w.push(self.halted as u8);
        w.push(self.waiting as u8);
    }

    pub fn load(&mut self, r: &mut super::State) -> Option<()> {
        for i in 0..16 {
            self.r[i] = r.u32()?;
        }
        self.cpsr = r.u32()?;
        self.spsr = r.u32()?;
        for a in [&mut self.bank_sp, &mut self.bank_lr, &mut self.bank_spsr] {
            for v in a.iter_mut() {
                *v = r.u32()?;
            }
        }
        for a in [&mut self.fiq_hi, &mut self.usr_hi] {
            for v in a.iter_mut() {
                *v = r.u32()?;
            }
        }
        self.halted = r.u8()? != 0;
        self.waiting = r.u8()? != 0;
        Some(())
    }

    pub fn thumb(&self) -> bool {
        self.cpsr & T != 0
    }

    pub fn irq_enabled(&self) -> bool {
        self.cpsr & I == 0
    }

    fn set_mode(&mut self, new: u32) {
        let old = self.cpsr & 0x1F;
        if old == new {
            return;
        }
        let (o, n) = (bank(old), bank(new));
        self.bank_sp[o] = self.r[13];
        self.bank_lr[o] = self.r[14];
        self.bank_spsr[o] = self.spsr;
        if old == FIQ {
            self.fiq_hi.copy_from_slice(&self.r[8..13]);
            self.r[8..13].copy_from_slice(&self.usr_hi);
        }
        if new == FIQ {
            self.usr_hi.copy_from_slice(&self.r[8..13]);
            self.r[8..13].copy_from_slice(&self.fiq_hi);
        }
        self.r[13] = self.bank_sp[n];
        self.r[14] = self.bank_lr[n];
        self.spsr = self.bank_spsr[n];
        self.cpsr = (self.cpsr & !0x1F) | new;
    }

    fn set_cpsr(&mut self, v: u32) {
        self.set_mode(v & 0x1F);
        self.cpsr = v;
    }

    /// Entrée dans une interruption (vecteur 0x18 du BIOS).
    pub fn irq(&mut self) {
        let ret = self.r[15] + 4; // adresse de la prochaine instruction + 4
        let old = self.cpsr;
        self.set_mode(IRQ);
        self.spsr = old;
        self.r[14] = ret;
        self.cpsr = (self.cpsr | I) & !T;
        self.r[15] = 0x18;
        self.halted = false;
    }

    /// Saut (aligné selon le mode courant).
    #[inline(always)]
    pub fn set_pc(&mut self, v: u32) {
        self.r[15] = if self.thumb() { v & !1 } else { v & !3 };
        self.branched = true;
    }

    #[inline(always)]
    fn bx(&mut self, v: u32) {
        if v & 1 != 0 {
            self.cpsr |= T;
        } else {
            self.cpsr &= !T;
        }
        self.set_pc(v);
    }

    // ------------------------------------------------------------ drapeaux --
    #[inline(always)]
    fn c(&self) -> bool {
        self.cpsr & C != 0
    }
    #[inline(always)]
    fn flag(&mut self, f: u32, on: bool) {
        if on {
            self.cpsr |= f;
        } else {
            self.cpsr &= !f;
        }
    }
    #[inline(always)]
    fn nz(&mut self, v: u32) {
        self.cpsr = (self.cpsr & !(N | Z)) | (v & N) | if v == 0 { Z } else { 0 };
    }
    #[inline(always)]
    fn add(&mut self, a: u32, b: u32, carry: bool, s: bool) -> u32 {
        let r = a as u64 + b as u64 + carry as u64;
        let res = r as u32;
        if s {
            self.nz(res);
            self.flag(C, r >> 32 != 0);
            self.flag(V, (!(a ^ b) & (a ^ res)) >> 31 != 0);
        }
        res
    }
    #[inline(always)]
    fn sub(&mut self, a: u32, b: u32, carry: bool, s: bool) -> u32 {
        self.add(a, !b, carry, s)
    }

    #[inline(always)]
    fn cond(&self, c: u32) -> bool {
        let (n, z, cc, v) = (self.cpsr & N != 0, self.cpsr & Z != 0, self.c(), self.cpsr & V != 0);
        match c {
            0 => z,
            1 => !z,
            2 => cc,
            3 => !cc,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => cc && !z,
            9 => !cc || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            13 => z || n != v,
            14 => true,
            _ => false,
        }
    }

    #[inline(always)]
    fn shift_imm(&self, typ: u32, v: u32, amt: u32) -> (u32, bool) {
        let c = self.c();
        match typ {
            0 => {
                if amt == 0 {
                    (v, c)
                } else {
                    (v << amt, (v >> (32 - amt)) & 1 != 0)
                }
            }
            1 => {
                if amt == 0 {
                    (0, v >> 31 != 0)
                } else {
                    (v >> amt, (v >> (amt - 1)) & 1 != 0)
                }
            }
            2 => {
                if amt == 0 {
                    (((v as i32) >> 31) as u32, v >> 31 != 0)
                } else {
                    (((v as i32) >> amt) as u32, (v >> (amt - 1)) & 1 != 0)
                }
            }
            _ => {
                if amt == 0 {
                    (((c as u32) << 31) | (v >> 1), v & 1 != 0)
                } else {
                    (v.rotate_right(amt), (v >> (amt - 1)) & 1 != 0)
                }
            }
        }
    }

    #[inline(always)]
    fn shift_reg(&self, typ: u32, v: u32, amt: u32) -> (u32, bool) {
        if amt == 0 {
            return (v, self.c());
        }
        match typ {
            0 => match amt {
                1..=31 => (v << amt, (v >> (32 - amt)) & 1 != 0),
                32 => (0, v & 1 != 0),
                _ => (0, false),
            },
            1 => match amt {
                1..=31 => (v >> amt, (v >> (amt - 1)) & 1 != 0),
                32 => (0, v >> 31 != 0),
                _ => (0, false),
            },
            2 => {
                if amt < 32 {
                    (((v as i32) >> amt) as u32, (v >> (amt - 1)) & 1 != 0)
                } else {
                    let s = ((v as i32) >> 31) as u32;
                    (s, s & 1 != 0)
                }
            }
            _ => {
                let a = amt & 31;
                if a == 0 { (v, v >> 31 != 0) } else { (v.rotate_right(a), (v >> (a - 1)) & 1 != 0) }
            }
        }
    }

    /// Exécute une instruction. Les cycles sont comptés par le bus (`bus.cyc`).
    pub fn step(&mut self, bus: &mut Bus) {
        let pc = self.r[15];
        self.branched = false;
        bus.pc_now = pc | self.thumb() as u32;
        bus.ob = None;
        if self.thumb() {
            let op = bus.read16(pc);
            self.r[15] = pc.wrapping_add(4);
            self.exec_thumb(bus, op, pc);
            if !self.branched {
                self.r[15] = pc.wrapping_add(2);
            } else {
                bus.cyc += 2;
            }
        } else {
            let op = bus.read32(pc);
            self.r[15] = pc.wrapping_add(8);
            if self.cond(op >> 28) {
                self.exec_arm(bus, op, pc);
            }
            if !self.branched {
                self.r[15] = pc.wrapping_add(4);
            } else {
                bus.cyc += 3;
            }
        }
    }

    /// Exécute l'instruction Thumb `op` située en `pc`, comme `step` sans la lecture
    /// de l'instruction (dont le coût est compté par l'appelant). Utilisée par le code
    /// recompilé (`recomp`) avec `op` et `pc` constants : le compilateur n'en garde que
    /// la branche utile. Renvoie vrai si l'instruction a sauté. `ob` : valeur de « bus
    /// ouvert » après cette instruction (constante elle aussi, voir `Bus::ob`).
    #[inline(always)]
    pub fn thumb_insn(&mut self, bus: &mut Bus, op: u32, pc: u32, ob: u32) -> bool {
        bus.pc_now = pc | 1;
        bus.ob = Some(ob);
        self.branched = false;
        self.r[15] = pc.wrapping_add(4);
        self.exec_thumb(bus, op, pc);
        if !self.branched {
            self.r[15] = pc.wrapping_add(2);
            false
        } else {
            bus.cyc += 2;
            true
        }
    }

    /// Comme `thumb_insn` pour une instruction de calcul pur (ni mémoire, ni saut) qui
    /// n'est pas la dernière de son bloc : elle ne peut pas lire le bus, donc ni
    /// l'instruction en cours ni la valeur de bus ouvert n'ont besoin d'être notées.
    #[inline(always)]
    pub fn thumb_reg(&mut self, bus: &mut Bus, op: u32, pc: u32) {
        self.r[15] = pc.wrapping_add(4);
        self.exec_thumb(bus, op, pc);
        self.r[15] = pc.wrapping_add(2);
    }

    /// Comme `thumb_insn` pour une instruction ARM (condition comprise).
    #[inline(always)]
    pub fn arm_insn(&mut self, bus: &mut Bus, op: u32, pc: u32, ob: u32) -> bool {
        bus.pc_now = pc;
        bus.ob = Some(ob);
        self.branched = false;
        self.r[15] = pc.wrapping_add(8);
        if self.cond(op >> 28) {
            self.exec_arm(bus, op, pc);
        }
        if !self.branched {
            self.r[15] = pc.wrapping_add(4);
            false
        } else {
            bus.cyc += 3;
            true
        }
    }

    /// Valeur de r15 « actuelle » pour un SWI (adresse de l'instruction).
    fn swi(&mut self, bus: &mut Bus, n: u32, addr: u32) {
        super::hle::swi(self, bus, n, addr);
    }

    /// Recommence l'instruction en `addr` (IntrWait) ou saute ailleurs.
    pub fn jump(&mut self, addr: u32) {
        self.r[15] = addr;
        self.branched = true;
    }

    // ------------------------------------------------------------------ ARM --
    #[inline(always)]
    fn exec_arm(&mut self, bus: &mut Bus, i: u32, pc: u32) {
        if i & 0x0FFF_FFF0 == 0x012F_FF10 {
            let v = self.r[(i & 15) as usize];
            self.bx(v);
        } else if i & 0x0FC0_00F0 == 0x0000_0090 {
            let (rd, rn, rs, rm) = ((i >> 16) & 15, (i >> 12) & 15, (i >> 8) & 15, i & 15);
            let mut v = self.r[rm as usize].wrapping_mul(self.r[rs as usize]);
            if i & (1 << 21) != 0 {
                v = v.wrapping_add(self.r[rn as usize]);
            }
            self.r[rd as usize] = v;
            if i & (1 << 20) != 0 {
                self.nz(v);
            }
            bus.cyc += 2;
        } else if i & 0x0F80_00F0 == 0x0080_0090 {
            let (hi, lo, rs, rm) = ((i >> 16) & 15, (i >> 12) & 15, (i >> 8) & 15, i & 15);
            let (a, b) = (self.r[rm as usize], self.r[rs as usize]);
            let mut v = if i & (1 << 22) != 0 {
                (a as i32 as i64).wrapping_mul(b as i32 as i64) as u64
            } else {
                a as u64 * b as u64
            };
            if i & (1 << 21) != 0 {
                v = v.wrapping_add(((self.r[hi as usize] as u64) << 32) | self.r[lo as usize] as u64);
            }
            self.r[lo as usize] = v as u32;
            self.r[hi as usize] = (v >> 32) as u32;
            if i & (1 << 20) != 0 {
                self.flag(N, v >> 63 != 0);
                self.flag(Z, v == 0);
            }
            bus.cyc += 3;
        } else if i & 0x0FB0_0FF0 == 0x0100_0090 {
            let (rn, rd, rm) = ((i >> 16) & 15, (i >> 12) & 15, i & 15);
            let a = self.r[rn as usize];
            let src = self.r[rm as usize];
            if i & (1 << 22) != 0 {
                let t = bus.read8(a);
                bus.write8(a, src as u8);
                self.r[rd as usize] = t;
            } else {
                let t = bus.read32(a & !3).rotate_right((a & 3) * 8);
                bus.write32(a & !3, src);
                self.r[rd as usize] = t;
            }
        } else if i & 0x0E00_0090 == 0x0000_0090 && i & 0x60 != 0 {
            self.arm_half(bus, i);
        } else if i & 0x0FBF_0FFF == 0x010F_0000 {
            self.r[((i >> 12) & 15) as usize] = if i & (1 << 22) != 0 { self.spsr } else { self.cpsr };
        } else if i & 0x0DB0_F000 == 0x0120_F000 {
            let v = if i & (1 << 25) != 0 { (i & 0xFF).rotate_right(((i >> 8) & 15) * 2) } else { self.r[(i & 15) as usize] };
            let mut mask = 0;
            for k in 0..4 {
                if i & (1 << (16 + k)) != 0 {
                    mask |= 0xFF << (8 * k);
                }
            }
            if i & (1 << 22) != 0 {
                self.spsr = (self.spsr & !mask) | (v & mask);
            } else {
                if self.cpsr & 0x1F == USR {
                    mask &= 0xFF00_0000;
                }
                let new = (self.cpsr & !mask) | (v & mask);
                self.set_cpsr(new);
            }
        } else if i & 0x0C00_0000 == 0 {
            self.arm_dp(i);
        } else if i & 0x0C00_0000 == 0x0400_0000 {
            self.arm_ldr(bus, i);
        } else if i & 0x0E00_0000 == 0x0800_0000 {
            self.arm_ldm(bus, i);
        } else if i & 0x0E00_0000 == 0x0A00_0000 {
            let off = (((i & 0xFF_FFFF) << 8) as i32 >> 6) as u32;
            if i & (1 << 24) != 0 {
                self.r[14] = pc.wrapping_add(4);
            }
            let t = self.r[15].wrapping_add(off);
            self.set_pc(t);
        } else if i & 0x0F00_0000 == 0x0F00_0000 {
            self.swi(bus, (i >> 16) & 0xFF, pc);
        }
        // coprocesseur / non défini : ignoré
    }

    #[inline(always)]
    fn arm_dp(&mut self, i: u32) {
        let opc = (i >> 21) & 15;
        let s = i & (1 << 20) != 0;
        let rn = ((i >> 16) & 15) as usize;
        let rd = ((i >> 12) & 15) as usize;
        let mut a = self.r[rn];
        let (b, carry) = if i & (1 << 25) != 0 {
            let rot = ((i >> 8) & 15) * 2;
            let v = (i & 0xFF).rotate_right(rot);
            (v, if rot == 0 { self.c() } else { v >> 31 != 0 })
        } else {
            let rm = (i & 15) as usize;
            let typ = (i >> 5) & 3;
            if i & 0x10 != 0 {
                let amt = self.r[((i >> 8) & 15) as usize] & 0xFF;
                let mut v = self.r[rm];
                if rm == 15 {
                    v = v.wrapping_add(4);
                }
                if rn == 15 {
                    a = a.wrapping_add(4);
                }
                self.shift_reg(typ, v, amt)
            } else {
                self.shift_imm(typ, self.r[rm], (i >> 7) & 31)
            }
        };
        let logic = |cpu: &mut Cpu, v: u32| {
            if s {
                cpu.nz(v);
                cpu.flag(C, carry);
            }
            v
        };
        let c = self.c();
        let res = match opc {
            0 | 8 => logic(self, a & b),
            1 | 9 => logic(self, a ^ b),
            2 | 10 => self.sub(a, b, true, s),
            3 => self.sub(b, a, true, s),
            4 | 11 => self.add(a, b, false, s),
            5 => self.add(a, b, c, s),
            6 => self.sub(a, b, c, s),
            7 => self.sub(b, a, c, s),
            12 => logic(self, a | b),
            13 => logic(self, b),
            14 => logic(self, a & !b),
            _ => logic(self, !b),
        };
        if (8..=11).contains(&opc) {
            return;
        }
        self.r[rd] = res;
        if rd == 15 {
            if s {
                let sp = self.spsr;
                self.set_cpsr(sp);
            }
            self.set_pc(res);
        }
    }

    #[inline(always)]
    fn arm_half(&mut self, bus: &mut Bus, i: u32) {
        let (p, u, w, l) = (i & (1 << 24) != 0, i & (1 << 23) != 0, i & (1 << 21) != 0, i & (1 << 20) != 0);
        let rn = ((i >> 16) & 15) as usize;
        let rd = ((i >> 12) & 15) as usize;
        let off = if i & (1 << 22) != 0 { ((i >> 4) & 0xF0) | (i & 15) } else { self.r[(i & 15) as usize] };
        let base = self.r[rn];
        let moved = if u { base.wrapping_add(off) } else { base.wrapping_sub(off) };
        let ea = if p { moved } else { base };
        if l {
            let v = match (i >> 5) & 3 {
                1 => (bus.read16(ea & !1)).rotate_right((ea & 1) * 8),
                2 => bus.read8(ea) as u8 as i8 as i32 as u32,
                _ => {
                    if ea & 1 != 0 {
                        bus.read8(ea) as u8 as i8 as i32 as u32
                    } else {
                        bus.read16(ea) as u16 as i16 as i32 as u32
                    }
                }
            };
            if !p || w {
                self.r[rn] = moved;
            }
            self.r[rd] = v;
            if rd == 15 {
                self.set_pc(v);
            }
        } else {
            let v = if rd == 15 { self.r[15].wrapping_add(4) } else { self.r[rd] };
            bus.write16(ea & !1, v as u16);
            if !p || w {
                self.r[rn] = moved;
            }
        }
    }

    #[inline(always)]
    fn arm_ldr(&mut self, bus: &mut Bus, i: u32) {
        let (p, u, b, w, l) =
            (i & (1 << 24) != 0, i & (1 << 23) != 0, i & (1 << 22) != 0, i & (1 << 21) != 0, i & (1 << 20) != 0);
        let rn = ((i >> 16) & 15) as usize;
        let rd = ((i >> 12) & 15) as usize;
        let off = if i & (1 << 25) != 0 {
            self.shift_imm((i >> 5) & 3, self.r[(i & 15) as usize], (i >> 7) & 31).0
        } else {
            i & 0xFFF
        };
        let base = self.r[rn];
        let moved = if u { base.wrapping_add(off) } else { base.wrapping_sub(off) };
        let ea = if p { moved } else { base };
        if l {
            let v = if b { bus.read8(ea) } else { bus.read32(ea & !3).rotate_right((ea & 3) * 8) };
            if !p || w {
                self.r[rn] = moved;
            }
            self.r[rd] = v;
            if rd == 15 {
                self.set_pc(v);
            }
        } else {
            let v = if rd == 15 { self.r[15].wrapping_add(4) } else { self.r[rd] };
            if b {
                bus.write8(ea, v as u8);
            } else {
                bus.write32(ea & !3, v);
            }
            if !p || w {
                self.r[rn] = moved;
            }
        }
    }

    #[inline(always)]
    fn arm_ldm(&mut self, bus: &mut Bus, i: u32) {
        let (p, u, s, w, l) =
            (i & (1 << 24) != 0, i & (1 << 23) != 0, i & (1 << 22) != 0, i & (1 << 21) != 0, i & (1 << 20) != 0);
        let rn = ((i >> 16) & 15) as usize;
        let mut list = i & 0xFFFF;
        let empty = list == 0;
        if empty {
            list = 1 << 15;
        }
        let n = if empty { 16 } else { list.count_ones() };
        let base = self.r[rn];
        let mut addr = match (u, p) {
            (true, true) => base.wrapping_add(4),
            (true, false) => base,
            (false, true) => base.wrapping_sub(4 * n),
            (false, false) => base.wrapping_sub(4 * n).wrapping_add(4),
        };
        let new_base = if u { base.wrapping_add(4 * n) } else { base.wrapping_sub(4 * n) };
        let user = s && !(l && list & (1 << 15) != 0);
        let old_mode = self.cpsr & 0x1F;
        if user {
            self.set_mode(SYS);
        }
        if l {
            if w && list & (1 << rn) == 0 {
                self.r[rn] = new_base;
            }
            for r in 0..16 {
                if list & (1 << r) == 0 {
                    continue;
                }
                let v = bus.read32(addr & !3);
                addr = addr.wrapping_add(4);
                if r == 15 {
                    if s {
                        let sp = self.spsr;
                        self.set_cpsr(sp);
                    }
                    self.set_pc(v);
                } else {
                    self.r[r] = v;
                }
            }
        } else {
            for r in 0..16 {
                if list & (1 << r) == 0 {
                    continue;
                }
                let v = if r == 15 { self.r[15].wrapping_add(4) } else { self.r[r] };
                bus.write32(addr & !3, v);
                addr = addr.wrapping_add(4);
            }
            if w {
                self.r[rn] = new_base;
            }
        }
        if user {
            self.set_mode(old_mode);
        }
    }

    // ---------------------------------------------------------------- Thumb --
    #[inline(always)]
    fn exec_thumb(&mut self, bus: &mut Bus, op: u32, pc: u32) {
        let i = op;
        let rd = (i & 7) as usize;
        let rs = ((i >> 3) & 7) as usize;
        match i >> 11 {
            0..=2 => {
                let (v, c) = self.shift_imm((i >> 11) & 3, self.r[rs], (i >> 6) & 31);
                self.r[rd] = v;
                self.nz(v);
                self.flag(C, c);
            }
            3 => {
                let x = (i >> 6) & 7;
                let b = if i & 0x400 != 0 { x } else { self.r[x as usize] };
                let a = self.r[rs];
                self.r[rd] = if i & 0x200 != 0 { self.sub(a, b, true, true) } else { self.add(a, b, false, true) };
            }
            4..=7 => {
                let d = ((i >> 8) & 7) as usize;
                let imm = i & 0xFF;
                let a = self.r[d];
                match (i >> 11) & 3 {
                    0 => {
                        self.r[d] = imm;
                        self.nz(imm);
                    }
                    1 => {
                        self.sub(a, imm, true, true);
                    }
                    2 => self.r[d] = self.add(a, imm, false, true),
                    _ => self.r[d] = self.sub(a, imm, true, true),
                }
            }
            8 if i & 0x400 == 0 => self.thumb_alu(bus, i, rd, rs),
            8 => {
                let d = ((i & 7) | ((i >> 4) & 8)) as usize;
                let s = ((i >> 3) & 15) as usize;
                let v = self.r[s];
                match (i >> 8) & 3 {
                    0 => {
                        let r = self.r[d].wrapping_add(v);
                        self.r[d] = r;
                        if d == 15 {
                            self.set_pc(r);
                        }
                    }
                    1 => {
                        let a = self.r[d];
                        self.sub(a, v, true, true);
                    }
                    2 => {
                        self.r[d] = v;
                        if d == 15 {
                            self.set_pc(v);
                        }
                    }
                    _ => self.bx(v),
                }
            }
            9 => {
                let d = ((i >> 8) & 7) as usize;
                let a = (self.r[15] & !3).wrapping_add((i & 0xFF) * 4);
                self.r[d] = bus.read32(a);
            }
            10 | 11 => {
                let a = self.r[rs].wrapping_add(self.r[((i >> 6) & 7) as usize]);
                if i & 0x200 == 0 {
                    match (i >> 10) & 3 {
                        0 => bus.write32(a & !3, self.r[rd]),
                        1 => bus.write8(a, self.r[rd] as u8),
                        2 => self.r[rd] = bus.read32(a & !3).rotate_right((a & 3) * 8),
                        _ => self.r[rd] = bus.read8(a),
                    }
                } else {
                    match (i >> 10) & 3 {
                        0 => bus.write16(a & !1, self.r[rd] as u16),
                        1 => self.r[rd] = bus.read8(a) as u8 as i8 as i32 as u32,
                        2 => self.r[rd] = bus.read16(a & !1).rotate_right((a & 1) * 8),
                        _ => {
                            self.r[rd] = if a & 1 != 0 {
                                bus.read8(a) as u8 as i8 as i32 as u32
                            } else {
                                bus.read16(a) as u16 as i16 as i32 as u32
                            }
                        }
                    }
                }
            }
            12..=15 => {
                let byte = i & 0x1000 != 0;
                let off = (i >> 6) & 31;
                let a = self.r[rs].wrapping_add(if byte { off } else { off * 4 });
                match (i & 0x800 != 0, byte) {
                    (false, false) => bus.write32(a & !3, self.r[rd]),
                    (false, true) => bus.write8(a, self.r[rd] as u8),
                    (true, false) => self.r[rd] = bus.read32(a & !3).rotate_right((a & 3) * 8),
                    (true, true) => self.r[rd] = bus.read8(a),
                }
            }
            16 | 17 => {
                let a = self.r[rs].wrapping_add(((i >> 6) & 31) * 2);
                if i & 0x800 != 0 {
                    self.r[rd] = bus.read16(a & !1).rotate_right((a & 1) * 8);
                } else {
                    bus.write16(a & !1, self.r[rd] as u16);
                }
            }
            18 | 19 => {
                let d = ((i >> 8) & 7) as usize;
                let a = self.r[13].wrapping_add((i & 0xFF) * 4);
                if i & 0x800 != 0 {
                    self.r[d] = bus.read32(a & !3).rotate_right((a & 3) * 8);
                } else {
                    bus.write32(a & !3, self.r[d]);
                }
            }
            20 | 21 => {
                let d = ((i >> 8) & 7) as usize;
                let base = if i & 0x800 != 0 { self.r[13] } else { self.r[15] & !3 };
                self.r[d] = base.wrapping_add((i & 0xFF) * 4);
            }
            22 | 23 => {
                if i & 0xFF00 == 0xB000 {
                    let off = (i & 0x7F) * 4;
                    self.r[13] = if i & 0x80 != 0 { self.r[13].wrapping_sub(off) } else { self.r[13].wrapping_add(off) };
                } else if i & 0x600 == 0x400 {
                    let list = i & 0xFF;
                    let extra = i & 0x100 != 0;
                    if i & 0x800 != 0 {
                        let mut a = self.r[13];
                        for r in 0..8 {
                            if list & (1 << r) != 0 {
                                self.r[r] = bus.read32(a & !3);
                                a = a.wrapping_add(4);
                            }
                        }
                        if extra {
                            let v = bus.read32(a & !3);
                            a = a.wrapping_add(4);
                            self.set_pc(v);
                        }
                        self.r[13] = a;
                    } else {
                        let n = list.count_ones() + extra as u32;
                        let mut a = self.r[13].wrapping_sub(4 * n);
                        self.r[13] = a;
                        for r in 0..8 {
                            if list & (1 << r) != 0 {
                                bus.write32(a & !3, self.r[r]);
                                a = a.wrapping_add(4);
                            }
                        }
                        if extra {
                            bus.write32(a & !3, self.r[14]);
                        }
                    }
                }
            }
            24 | 25 => {
                let b = ((i >> 8) & 7) as usize;
                let list = i & 0xFF;
                let mut a = self.r[b];
                if i & 0x800 != 0 {
                    for r in 0..8 {
                        if list & (1 << r) != 0 {
                            self.r[r] = bus.read32(a & !3);
                            a = a.wrapping_add(4);
                        }
                    }
                    if list & (1 << b) == 0 {
                        self.r[b] = a;
                    }
                } else {
                    for r in 0..8 {
                        if list & (1 << r) != 0 {
                            bus.write32(a & !3, self.r[r]);
                            a = a.wrapping_add(4);
                        }
                    }
                    self.r[b] = a;
                }
            }
            26 | 27 => {
                let c = (i >> 8) & 15;
                if c == 15 {
                    self.swi(bus, i & 0xFF, pc);
                } else if c != 14 && self.cond(c) {
                    let t = self.r[15].wrapping_add(((i & 0xFF) as i8 as i32 * 2) as u32);
                    self.set_pc(t);
                }
            }
            28 => {
                let off = (((i & 0x7FF) << 21) as i32 >> 20) as u32;
                let t = self.r[15].wrapping_add(off);
                self.set_pc(t);
            }
            30 => {
                let off = (((i & 0x7FF) << 21) as i32 >> 9) as u32;
                self.r[14] = self.r[15].wrapping_add(off);
            }
            31 => {
                let t = self.r[14].wrapping_add((i & 0x7FF) << 1);
                self.r[14] = pc.wrapping_add(2) | 1;
                self.set_pc(t);
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn thumb_alu(&mut self, bus: &mut Bus, i: u32, rd: usize, rs: usize) {
        let a = self.r[rd];
        let b = self.r[rs];
        let c = self.c();
        match (i >> 6) & 15 {
            0 => {
                self.r[rd] = a & b;
                self.nz(a & b);
            }
            1 => {
                self.r[rd] = a ^ b;
                self.nz(a ^ b);
            }
            k @ (2 | 3 | 4 | 7) => {
                let typ = match k {
                    2 => 0,
                    3 => 1,
                    4 => 2,
                    _ => 3,
                };
                let (v, cc) = self.shift_reg(typ, a, b & 0xFF);
                self.r[rd] = v;
                self.nz(v);
                self.flag(C, cc);
                bus.cyc += 1;
            }
            5 => self.r[rd] = self.add(a, b, c, true),
            6 => self.r[rd] = self.sub(a, b, c, true),
            8 => self.nz(a & b),
            9 => self.r[rd] = self.sub(0, b, true, true),
            10 => {
                self.sub(a, b, true, true);
            }
            11 => {
                self.add(a, b, false, true);
            }
            12 => {
                self.r[rd] = a | b;
                self.nz(a | b);
            }
            13 => {
                let v = a.wrapping_mul(b);
                self.r[rd] = v;
                self.nz(v);
                bus.cyc += 2;
            }
            14 => {
                self.r[rd] = a & !b;
                self.nz(a & !b);
            }
            _ => {
                self.r[rd] = !b;
                self.nz(!b);
            }
        }
    }
}
