//! Petit émulateur Thumb (ARM7TDMI) pour exécuter des fonctions isolées du jeu.
//!
//! Sert d'oracle : on lance la vraie routine de la ROM, puis on compare sa sortie
//! avec la réécriture en Rust. Pas de vidéo, pas d'interruptions, pas d'ARM :
//! seulement la RAM, la ROM et le DMA3 (utilisé par le décodeur pour remplir/copier).

use crate::rom::Rom;

/// Adresse de retour fictive : quand pc l'atteint, la fonction appelée est terminée.
const RETURN_MAGIC: u32 = 0x0F00_0000;

pub struct Cpu<'a> {
    pub r: [u32; 16],
    n: bool,
    z: bool,
    c: bool,
    v: bool,
    rom: &'a Rom,
    pub ewram: Vec<u8>,
    pub iwram: Vec<u8>,
    pub io: Vec<u8>,
    /// VRAM, palette et OAM (utiles quand on exécute du code d'initialisation).
    pub video: Vec<u8>,
    pub steps: u64,
    /// Mode tolérant : accès hors carte ignorés (lecture = 0), SWI du BIOS simulés.
    pub lenient: bool,
    /// Mode ARM en cours (sinon Thumb).
    pub arm: bool,
    /// Journal des écritures d'octets dans une plage (adresse, valeur).
    pub log_range: std::ops::Range<u32>,
    pub log: Vec<(u32, u8)>,
    /// Second journal (registres son PSG, par exemple).
    pub log2_range: std::ops::Range<u32>,
    pub log2: Vec<(u32, u8)>,
}

#[derive(Debug)]
pub enum EmuError {
    BadAccess(u32, u32),
    Unsupported(u32, u16),
    TooLong,
}

impl<'a> Cpu<'a> {
    pub fn new(rom: &'a Rom) -> Self {
        Cpu {
            r: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            rom,
            ewram: vec![0; 0x40000],
            iwram: vec![0; 0x8000],
            io: vec![0; 0x400],
            video: vec![0; 0x18000 + 0x800],
            steps: 0,
            lenient: false,
            arm: false,
            log_range: 0..0,
            log: Vec::new(),
            log2_range: 0..0,
            log2: Vec::new(),
        }
    }

    // --- Mémoire ---

    fn region(&mut self, addr: u32) -> Option<(&mut [u8], usize)> {
        let a = addr as usize;
        match addr >> 24 {
            0x02 => Some((&mut self.ewram[..], a & 0x3FFFF)),
            0x03 => Some((&mut self.iwram[..], a & 0x7FFF)),
            0x04 if a & 0xFFFFFF < 0x400 => Some((&mut self.io[..], a & 0x3FF)),
            0x05 => Some((&mut self.video[..], 0x18000 + (a & 0x3FF))),
            0x06 => Some((&mut self.video[..], (a & 0x1FFFF) % 0x18000)),
            0x07 => Some((&mut self.video[..], 0x18400 + (a & 0x3FF))),
            _ => None,
        }
    }

    pub fn read8(&mut self, addr: u32) -> Result<u32, EmuError> {
        if addr >> 24 == 0x08 {
            // Pas addr_to_off : il efface le bit 0 (adresses Thumb).
            let o = (addr - 0x0800_0000) as usize;
            return self.rom.data.get(o).map(|&b| b as u32).ok_or(EmuError::BadAccess(self.r[15], addr));
        }
        let pc = self.r[15];
        let lenient = self.lenient;
        match self.region(addr) {
            Some((m, o)) => Ok(m[o] as u32),
            // EEPROM de sauvegarde : toujours « prête ».
            None if lenient && addr >> 24 == 0x0D => Ok(1),
            None if lenient => Ok(0),
            None => Err(EmuError::BadAccess(pc, addr)),
        }
    }

    pub fn read16(&mut self, addr: u32) -> Result<u32, EmuError> {
        let a = addr & !1;
        Ok(self.read8(a)? | self.read8(a + 1)? << 8)
    }

    pub fn read32(&mut self, addr: u32) -> Result<u32, EmuError> {
        let a = addr & !3;
        let v = self.read16(a)? | self.read16(a + 2)? << 16;
        // Lecture non alignée : l'ARM7 fait une rotation du mot.
        Ok(v.rotate_right((addr & 3) * 8))
    }

    pub fn write8(&mut self, addr: u32, v: u32) -> Result<(), EmuError> {
        let pc = self.r[15];
        let lenient = self.lenient;
        if self.log_range.contains(&addr) {
            self.log.push((addr, v as u8));
        }
        if self.log2_range.contains(&addr) {
            self.log2.push((addr, v as u8));
        }
        match self.region(addr) {
            Some((m, o)) => m[o] = v as u8,
            None if lenient => return Ok(()),
            None => return Err(EmuError::BadAccess(pc, addr)),
        }
        if addr >> 24 == 0x04 && (addr & 0x3FF) == 0xDF && v & 0x80 != 0 {
            self.dma3()?;
        }
        Ok(())
    }

    pub fn write16(&mut self, addr: u32, v: u32) -> Result<(), EmuError> {
        let a = addr & !1;
        self.write8(a, v)?;
        self.write8(a + 1, v >> 8)
    }

    pub fn write32(&mut self, addr: u32, v: u32) -> Result<(), EmuError> {
        let a = addr & !3;
        self.write16(a, v)?;
        self.write16(a + 2, v >> 16)
    }

    /// DMA3 immédiat, déclenché par l'écriture du bit « enable » de DMA3CNT_H.
    fn dma3(&mut self) -> Result<(), EmuError> {
        let io32 = |io: &[u8], o: usize| u32::from_le_bytes(io[o..o + 4].try_into().unwrap());
        let mut src = io32(&self.io, 0xD4);
        let mut dst = io32(&self.io, 0xD8);
        let cnt = io32(&self.io, 0xDC);
        let ctrl = cnt >> 16;
        let mut count = cnt & 0xFFFF;
        if count == 0 {
            count = 0x4000;
        }
        let word = ctrl & (1 << 10) != 0;
        let step: u32 = if word { 4 } else { 2 };
        let step_for = |mode: u32| match mode {
            0 | 3 => step as i32,
            1 => -(step as i32),
            _ => 0,
        };
        let (ds, ss) = (step_for((ctrl >> 5) & 3), step_for((ctrl >> 7) & 3));
        self.io[0xDF] &= 0x7F; // le DMA se désactive tout seul
        for _ in 0..count {
            if word {
                let v = self.read32(src)?;
                self.write32(dst, v)?;
            } else {
                let v = self.read16(src)?;
                self.write16(dst, v)?;
            }
            src = src.wrapping_add(ss as u32);
            dst = dst.wrapping_add(ds as u32);
        }
        Ok(())
    }

    // --- Drapeaux ---

    fn nz(&mut self, v: u32) {
        self.n = v >> 31 != 0;
        self.z = v == 0;
    }

    fn add_flags(&mut self, a: u32, b: u32, carry: bool) -> u32 {
        let (r1, c1) = a.overflowing_add(b);
        let (r, c2) = r1.overflowing_add(carry as u32);
        self.c = c1 || c2;
        self.v = ((a ^ r) & (b ^ r)) >> 31 != 0;
        self.nz(r);
        r
    }

    fn sub_flags(&mut self, a: u32, b: u32, carry: bool) -> u32 {
        self.add_flags(a, !b, carry)
    }

    fn cond(&self, c: u32) -> bool {
        match c {
            0 => self.z,
            1 => !self.z,
            2 => self.c,
            3 => !self.c,
            4 => self.n,
            5 => !self.n,
            6 => self.v,
            7 => !self.v,
            8 => self.c && !self.z,
            9 => !self.c || self.z,
            10 => self.n == self.v,
            11 => self.n != self.v,
            12 => !self.z && self.n == self.v,
            13 => self.z || self.n != self.v,
            _ => true,
        }
    }

    /// Décalage par registre (format ALU) avec retenue, sémantique ARM7.
    fn shift_reg(&mut self, kind: u32, v: u32, amt: u32) -> u32 {
        if amt == 0 {
            return v;
        }
        match kind {
            0 => match amt {
                1..=31 => {
                    self.c = (v >> (32 - amt)) & 1 != 0;
                    v << amt
                }
                32 => {
                    self.c = v & 1 != 0;
                    0
                }
                _ => {
                    self.c = false;
                    0
                }
            },
            1 => match amt {
                1..=31 => {
                    self.c = (v >> (amt - 1)) & 1 != 0;
                    v >> amt
                }
                32 => {
                    self.c = v >> 31 != 0;
                    0
                }
                _ => {
                    self.c = false;
                    0
                }
            },
            2 => {
                if amt < 32 {
                    self.c = (v >> (amt - 1)) & 1 != 0;
                    ((v as i32) >> amt) as u32
                } else {
                    self.c = v >> 31 != 0;
                    ((v as i32) >> 31) as u32
                }
            }
            _ => {
                let r = v.rotate_right(amt & 31);
                self.c = r >> 31 != 0;
                r
            }
        }
    }

    // --- Exécution ---

    /// Appelle la fonction Thumb `addr` avec les arguments r0-r3, jusqu'à son retour.
    pub fn call(&mut self, addr: u32, args: &[u32], max_steps: u64) -> Result<u32, EmuError> {
        for (i, &a) in args.iter().enumerate() {
            self.r[i] = a;
        }
        self.r[13] = 0x0300_7F00;
        self.r[14] = RETURN_MAGIC | 1;
        self.run_until(addr, RETURN_MAGIC, max_steps)?;
        Ok(self.r[0])
    }

    /// Exécute à partir de `addr` (registres conservés) jusqu'à ce que pc vaille `stop`.
    pub fn run_until(&mut self, addr: u32, stop: u32, max_steps: u64) -> Result<(), EmuError> {
        self.arm = false;
        self.r[15] = addr & !1;
        let start = self.steps;
        while self.r[15] != stop {
            self.step()?;
            self.steps += 1;
            if self.steps - start > max_steps {
                return Err(EmuError::TooLong);
            }
        }
        Ok(())
    }

    /// Quelques fonctions du BIOS (mode tolérant).
    fn bios(&mut self, n: u32) -> Result<(), EmuError> {
        match n {
            6 => {
                // Div : r0 / r1
                let (a, b) = (self.r[0] as i32, self.r[1] as i32);
                if b != 0 {
                    let q = a.wrapping_div(b);
                    self.r[1] = a.wrapping_rem(b) as u32;
                    self.r[0] = q as u32;
                    self.r[3] = q.unsigned_abs();
                }
            }
            8 => self.r[0] = (self.r[0] as f64).sqrt() as u32,
            0xB | 0xC => {
                // CpuSet / CpuFastSet
                let (mut src, mut dst, cnt) = (self.r[0], self.r[1], self.r[2]);
                let fill = cnt & (1 << 24) != 0;
                let word = n == 0xC || cnt & (1 << 26) != 0;
                let mut count = cnt & 0x1F_FFFF;
                if n == 0xC {
                    count = (count + 7) & !7;
                }
                let step = if word { 4 } else { 2 };
                for _ in 0..count {
                    if word {
                        let v = self.read32(src)?;
                        self.write32(dst, v)?;
                    } else {
                        let v = self.read16(src)?;
                        self.write16(dst, v)?;
                    }
                    if !fill {
                        src = src.wrapping_add(step);
                    }
                    dst = dst.wrapping_add(step);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), EmuError> {
        if self.arm {
            return self.step_arm();
        }
        let pc = self.r[15];
        let h = self.read16(pc)?;
        let mut next = pc + 2;
        let rd = (h & 7) as usize;
        let rs = ((h >> 3) & 7) as usize;
        // Valeur de pc vue par l'instruction : adresse + 4.
        let pcv = pc + 4;
        let bad = || EmuError::Unsupported(pc, h as u16);
        if h == 0x4778 && self.lenient {
            // `bx pc` : passerelle vers du code ARM (activation des IRQ…) -> ignorée.
            self.r[15] = self.r[14] & !1;
            return Ok(());
        }

        match h >> 11 {
            0..=2 => {
                let amt = (h >> 6) & 31;
                let v = self.r[rs];
                let res = match h >> 11 {
                    0 => self.shift_reg(0, v, amt),
                    // lsr/asr #0 signifie #32
                    1 => self.shift_reg(1, v, if amt == 0 { 32 } else { amt }),
                    _ => self.shift_reg(2, v, if amt == 0 { 32 } else { amt }),
                };
                self.r[rd] = res;
                self.nz(res);
            }
            3 => {
                let v = (h >> 6) & 7;
                let op2 = if h & 0x400 != 0 { v } else { self.r[v as usize] };
                let a = self.r[rs];
                self.r[rd] = if h & 0x200 != 0 { self.sub_flags(a, op2, true) } else { self.add_flags(a, op2, false) };
            }
            4..=7 => {
                let d = ((h >> 8) & 7) as usize;
                let imm = h & 0xFF;
                match (h >> 11) & 3 {
                    0 => {
                        self.r[d] = imm;
                        self.nz(imm);
                    }
                    1 => {
                        self.sub_flags(self.r[d], imm, true);
                    }
                    2 => self.r[d] = self.add_flags(self.r[d], imm, false),
                    _ => self.r[d] = self.sub_flags(self.r[d], imm, true),
                }
            }
            8 if h & 0x400 == 0 => {
                let (a, b) = (self.r[rd], self.r[rs]);
                match (h >> 6) & 15 {
                    0 => {
                        self.r[rd] = a & b;
                        self.nz(a & b)
                    }
                    1 => {
                        self.r[rd] = a ^ b;
                        self.nz(a ^ b)
                    }
                    k @ (2 | 3 | 4 | 7) => {
                        let kind = match k {
                            2 => 0,
                            3 => 1,
                            4 => 2,
                            _ => 3,
                        };
                        let r = self.shift_reg(kind, a, b & 0xFF);
                        self.r[rd] = r;
                        self.nz(r);
                    }
                    5 => self.r[rd] = self.add_flags(a, b, self.c),
                    6 => self.r[rd] = self.sub_flags(a, b, self.c),
                    8 => self.nz(a & b),
                    9 => self.r[rd] = self.sub_flags(0, b, true),
                    10 => {
                        self.sub_flags(a, b, true);
                    }
                    11 => {
                        self.add_flags(a, b, false);
                    }
                    12 => {
                        self.r[rd] = a | b;
                        self.nz(a | b)
                    }
                    13 => {
                        let r = a.wrapping_mul(b);
                        self.r[rd] = r;
                        self.nz(r)
                    }
                    14 => {
                        self.r[rd] = a & !b;
                        self.nz(a & !b)
                    }
                    _ => {
                        self.r[rd] = !b;
                        self.nz(!b)
                    }
                }
            }
            8 => {
                let d = ((h & 7) | ((h >> 4) & 8)) as usize;
                let s = ((h >> 3) & 15) as usize;
                let get = |cpu: &Self, i: usize| if i == 15 { pcv } else { cpu.r[i] };
                match (h >> 8) & 3 {
                    0 => {
                        let v = get(self, d).wrapping_add(get(self, s));
                        if d == 15 {
                            next = v & !1;
                        } else {
                            self.r[d] = v;
                        }
                    }
                    1 => {
                        let (a, b) = (get(self, d), get(self, s));
                        self.sub_flags(a, b, true);
                    }
                    2 => {
                        let v = get(self, s);
                        if d == 15 {
                            next = v & !1;
                        } else {
                            self.r[d] = v;
                        }
                    }
                    _ => {
                        let v = get(self, s);
                        if v & 1 == 0 && v != RETURN_MAGIC {
                            if !self.lenient {
                                return Err(bad()); // passage en ARM
                            }
                            self.arm = true;
                            next = v & !3;
                        } else {
                            next = v & !1;
                        }
                    }
                }
            }
            9 => {
                let d = ((h >> 8) & 7) as usize;
                self.r[d] = self.read32((pcv & !3) + (h & 0xFF) * 4)?;
            }
            10 | 11 => {
                let addr = self.r[rs].wrapping_add(self.r[((h >> 6) & 7) as usize]);
                let op = (h >> 10) & 3;
                if h & 0x200 == 0 {
                    match op {
                        0 => self.write32(addr, self.r[rd])?,
                        1 => self.write8(addr, self.r[rd])?,
                        2 => self.r[rd] = self.read32(addr)?,
                        _ => self.r[rd] = self.read8(addr)?,
                    }
                } else {
                    match op {
                        0 => self.write16(addr, self.r[rd])?,
                        1 => self.r[rd] = self.read8(addr)? as u8 as i8 as i32 as u32,
                        2 => self.r[rd] = self.read16(addr)?,
                        _ => self.r[rd] = self.read16(addr)? as u16 as i16 as i32 as u32,
                    }
                }
            }
            12..=15 => {
                let byte = h & 0x1000 != 0;
                let imm = (h >> 6) & 31;
                let addr = self.r[rs].wrapping_add(if byte { imm } else { imm * 4 });
                match (h & 0x800 != 0, byte) {
                    (false, false) => self.write32(addr, self.r[rd])?,
                    (false, true) => self.write8(addr, self.r[rd])?,
                    (true, false) => self.r[rd] = self.read32(addr)?,
                    (true, true) => self.r[rd] = self.read8(addr)?,
                }
            }
            16 | 17 => {
                let addr = self.r[rs].wrapping_add(((h >> 6) & 31) * 2);
                if h & 0x800 != 0 {
                    self.r[rd] = self.read16(addr)?;
                } else {
                    self.write16(addr, self.r[rd])?;
                }
            }
            18 | 19 => {
                let d = ((h >> 8) & 7) as usize;
                let addr = self.r[13].wrapping_add((h & 0xFF) * 4);
                if h & 0x800 != 0 {
                    self.r[d] = self.read32(addr)?;
                } else {
                    self.write32(addr, self.r[d])?;
                }
            }
            20 | 21 => {
                let d = ((h >> 8) & 7) as usize;
                let base = if h & 0x800 != 0 { self.r[13] } else { pcv & !3 };
                self.r[d] = base.wrapping_add((h & 0xFF) * 4);
            }
            22 | 23 if h & 0xFF00 == 0xB000 => {
                let off = (h & 0x7F) * 4;
                self.r[13] = if h & 0x80 != 0 { self.r[13].wrapping_sub(off) } else { self.r[13].wrapping_add(off) };
            }
            22 | 23 if h & 0x600 == 0x400 => {
                let list = h & 0xFF;
                let extra = h & 0x100 != 0;
                if h & 0x800 == 0 {
                    let n = list.count_ones() + extra as u32;
                    let mut addr = self.r[13].wrapping_sub(n * 4);
                    self.r[13] = addr;
                    for i in 0..8 {
                        if list & (1 << i) != 0 {
                            self.write32(addr, self.r[i])?;
                            addr += 4;
                        }
                    }
                    if extra {
                        self.write32(addr, self.r[14])?;
                    }
                } else {
                    let mut addr = self.r[13];
                    for i in 0..8 {
                        if list & (1 << i) != 0 {
                            self.r[i] = self.read32(addr)?;
                            addr += 4;
                        }
                    }
                    if extra {
                        next = self.read32(addr)? & !1;
                        addr += 4;
                    }
                    self.r[13] = addr;
                }
            }
            24 | 25 => {
                let b = ((h >> 8) & 7) as usize;
                let list = h & 0xFF;
                let mut addr = self.r[b];
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        if h & 0x800 != 0 {
                            self.r[i] = self.read32(addr)?;
                        } else {
                            self.write32(addr, self.r[i])?;
                        }
                        addr += 4;
                    }
                }
                if h & 0x800 == 0 || list & (1 << b) == 0 {
                    self.r[b] = addr;
                }
            }
            26 | 27 => {
                let c = (h >> 8) & 15;
                if c == 15 && self.lenient {
                    self.bios(h as u32 & 0xFF)?;
                    self.r[15] = next;
                    return Ok(());
                }
                if c >= 14 {
                    return Err(bad()); // swi / indéfini
                }
                if self.cond(c) {
                    next = pcv.wrapping_add(((h & 0xFF) as i8 as i32 * 2) as u32);
                }
            }
            28 => next = pcv.wrapping_add((((h & 0x7FF) << 21) as i32 >> 20) as u32),
            30 => {
                let lo = self.read16(pc + 2)?;
                if lo >> 11 != 0b11111 {
                    return Err(bad());
                }
                let hi = ((h & 0x7FF) << 21) as i32 >> 9;
                next = pcv.wrapping_add(hi as u32).wrapping_add((lo & 0x7FF) << 1);
                self.r[14] = (pc + 4) | 1;
            }
            _ => return Err(bad()),
        }
        self.r[15] = next;
        Ok(())
    }

    // --- Mode ARM (mixeur du pilote son, copié en IWRAM) ---

    /// Décalage d'opérande ARM par immédiat (lsr/asr #0 = #32, ror #0 = rrx).
    fn shift_imm(&mut self, kind: u32, v: u32, amt: u32) -> u32 {
        match (kind, amt) {
            (0, 0) => v,
            (1, 0) | (2, 0) => self.shift_reg(kind, v, 32),
            (3, 0) => {
                let c = self.c as u32;
                self.c = v & 1 != 0;
                (v >> 1) | (c << 31)
            }
            _ => self.shift_reg(kind, v, amt),
        }
    }

    fn arm_branch_to(&mut self, v: u32) {
        if v & 1 != 0 || v == RETURN_MAGIC {
            self.arm = false;
            self.r[15] = v & !1;
        } else {
            self.r[15] = v & !3;
        }
    }

    fn step_arm(&mut self) -> Result<(), EmuError> {
        let pc = self.r[15];
        let w = self.read32(pc)?;
        let bad = || EmuError::Unsupported(pc, (w >> 16) as u16);
        self.r[15] = pc + 4;
        if !self.cond(w >> 28) {
            return Ok(());
        }
        let reg = |cpu: &Self, i: usize| if i == 15 { pc + 8 } else { cpu.r[i] };
        let rn = ((w >> 16) & 15) as usize;
        let rd = ((w >> 12) & 15) as usize;
        if w & 0x0FFF_FFF0 == 0x012F_FF10 {
            let v = reg(self, (w & 15) as usize);
            self.arm_branch_to(v);
            return Ok(());
        }
        match (w >> 25) & 7 {
            0 if w & 0x0FC0_00F0 == 0x0000_0090 => {
                // mul / mla
                let (d, n, s, m) = ((w >> 16) & 15, (w >> 12) & 15, (w >> 8) & 15, w & 15);
                let mut v = self.r[m as usize].wrapping_mul(self.r[s as usize]);
                if w & (1 << 21) != 0 {
                    v = v.wrapping_add(self.r[n as usize]);
                }
                self.r[d as usize] = v;
                if w & (1 << 20) != 0 {
                    self.nz(v);
                }
            }
            0 if w & 0x0F80_00F0 == 0x0080_0090 => {
                // umull / umlal / smull / smlal
                let (hi, lo, s, m) = ((w >> 16) & 15, (w >> 12) & 15, (w >> 8) & 15, w & 15);
                let (a, b) = (self.r[m as usize], self.r[s as usize]);
                let mut v = if w & (1 << 22) != 0 {
                    (a as i32 as i64).wrapping_mul(b as i32 as i64) as u64
                } else {
                    (a as u64) * (b as u64)
                };
                if w & (1 << 21) != 0 {
                    v = v.wrapping_add(((self.r[hi as usize] as u64) << 32) | self.r[lo as usize] as u64);
                }
                self.r[lo as usize] = v as u32;
                self.r[hi as usize] = (v >> 32) as u32;
                if w & (1 << 20) != 0 {
                    self.n = v >> 63 != 0;
                    self.z = v == 0;
                }
            }
            0 if w & 0x0FB0_0FF0 == 0x0100_0090 => {
                // swp / swpb
                let addr = self.r[rn];
                let m = (w & 15) as usize;
                if w & (1 << 22) != 0 {
                    let old = self.read8(addr)?;
                    self.write8(addr, self.r[m])?;
                    self.r[rd] = old;
                } else {
                    let old = self.read32(addr)?;
                    self.write32(addr, self.r[m])?;
                    self.r[rd] = old;
                }
            }
            0 if w & 0x90 == 0x90 => {
                // ldrh / strh / ldrsb / ldrsh
                let off = if w & (1 << 22) != 0 { ((w >> 4) & 0xF0) | (w & 15) } else { self.r[(w & 15) as usize] };
                let up = w & (1 << 23) != 0;
                let base = reg(self, rn);
                let moved = if up { base.wrapping_add(off) } else { base.wrapping_sub(off) };
                let pre = w & (1 << 24) != 0;
                let addr = if pre { moved } else { base };
                let wb = !pre || w & (1 << 21) != 0;
                if w & (1 << 20) != 0 {
                    let v = match (w >> 5) & 3 {
                        1 => self.read16(addr)?,
                        2 => self.read8(addr)? as u8 as i8 as i32 as u32,
                        _ => self.read16(addr)? as u16 as i16 as i32 as u32,
                    };
                    if wb {
                        self.r[rn] = moved;
                    }
                    self.r[rd] = v;
                } else {
                    let v = reg(self, rd);
                    self.write16(addr, v)?;
                    if wb {
                        self.r[rn] = moved;
                    }
                }
            }
            0 | 1 => {
                let op = (w >> 21) & 15;
                let s = w & (1 << 20) != 0;
                if !s && (8..=11).contains(&op) {
                    // mrs / msr : pas de modes ni d'interruptions ici
                    if w & 0x0FBF_0FFF == 0x010F_0000 {
                        self.r[rd] = (self.n as u32) << 31
                            | (self.z as u32) << 30
                            | (self.c as u32) << 29
                            | (self.v as u32) << 28
                            | 0x1F;
                    }
                    return Ok(());
                }
                let (c0, v0) = (self.c, self.v);
                let op2 = if w & (1 << 25) != 0 {
                    let rot = ((w >> 8) & 15) * 2;
                    let v = (w & 0xFF).rotate_right(rot);
                    if rot != 0 {
                        self.c = v >> 31 != 0;
                    }
                    v
                } else {
                    let m = (w & 15) as usize;
                    let kind = (w >> 5) & 3;
                    if w & 0x10 != 0 {
                        let amt = self.r[((w >> 8) & 15) as usize] & 0xFF;
                        let v = if m == 15 { pc + 12 } else { self.r[m] };
                        self.shift_reg(kind, v, amt)
                    } else {
                        let v = reg(self, m);
                        self.shift_imm(kind, v, (w >> 7) & 31)
                    }
                };
                let shc = self.c;
                let a = if rn == 15 && w & (1 << 25) == 0 && w & 0x10 != 0 { pc + 12 } else { reg(self, rn) };
                let res = match op {
                    0 | 8 => a & op2,
                    1 | 9 => a ^ op2,
                    2 | 10 => self.sub_flags(a, op2, true),
                    3 => self.sub_flags(op2, a, true),
                    4 | 11 => self.add_flags(a, op2, false),
                    5 => self.add_flags(a, op2, c0),
                    6 => self.sub_flags(a, op2, c0),
                    7 => self.sub_flags(op2, a, c0),
                    12 => a | op2,
                    13 => op2,
                    14 => a & !op2,
                    _ => !op2,
                };
                if s {
                    if !matches!(op, 2..=7 | 10 | 11) {
                        self.nz(res);
                        self.c = shc;
                        self.v = v0;
                    }
                } else {
                    // add/sub calculent les drapeaux : sans S, on les rétablit
                    let (n, z) = (self.n, self.z);
                    let _ = (n, z);
                    self.c = c0;
                    self.v = v0;
                }
                if !(8..=11).contains(&op) {
                    if rd == 15 {
                        self.r[15] = res & !3;
                    } else {
                        self.r[rd] = res;
                    }
                }
            }
            2 | 3 => {
                // ldr / str
                let off = if w & (1 << 25) == 0 {
                    w & 0xFFF
                } else {
                    let v = reg(self, (w & 15) as usize);
                    let c = self.c;
                    let o = self.shift_imm((w >> 5) & 3, v, (w >> 7) & 31);
                    self.c = c;
                    o
                };
                let up = w & (1 << 23) != 0;
                let base = reg(self, rn);
                let moved = if up { base.wrapping_add(off) } else { base.wrapping_sub(off) };
                let pre = w & (1 << 24) != 0;
                let addr = if pre { moved } else { base };
                let byte = w & (1 << 22) != 0;
                let wb = !pre || w & (1 << 21) != 0;
                if w & (1 << 20) != 0 {
                    let v = if byte { self.read8(addr)? } else { self.read32(addr)? };
                    if wb {
                        self.r[rn] = moved;
                    }
                    if rd == 15 {
                        self.arm_branch_to(v);
                    } else {
                        self.r[rd] = v;
                    }
                } else {
                    let v = if rd == 15 { pc + 12 } else { self.r[rd] };
                    if byte {
                        self.write8(addr, v)?;
                    } else {
                        self.write32(addr, v)?;
                    }
                    if wb {
                        self.r[rn] = moved;
                    }
                }
            }
            4 => {
                // ldm / stm (pile pleine descendante, etc.)
                let list = w & 0xFFFF;
                let n = list.count_ones();
                let up = w & (1 << 23) != 0;
                let pre = w & (1 << 24) != 0;
                let base = self.r[rn];
                let lowest = if up { base } else { base.wrapping_sub(n * 4) };
                let mut addr = match (up, pre) {
                    (true, false) => lowest,
                    (true, true) => lowest.wrapping_add(4),
                    (false, false) => lowest.wrapping_add(4),
                    (false, true) => lowest,
                };
                let end = if up { base.wrapping_add(n * 4) } else { base.wrapping_sub(n * 4) };
                let load = w & (1 << 20) != 0;
                if w & (1 << 21) != 0 && !(load && list & (1 << rn) != 0) {
                    self.r[rn] = end;
                }
                for i in 0..16 {
                    if list & (1 << i) == 0 {
                        continue;
                    }
                    if load {
                        let v = self.read32(addr)?;
                        if i == 15 {
                            self.arm_branch_to(v);
                        } else {
                            self.r[i] = v;
                        }
                    } else {
                        let v = if i == 15 { pc + 12 } else { self.r[i] };
                        self.write32(addr, v)?;
                    }
                    addr = addr.wrapping_add(4);
                }
            }
            5 => {
                if w & (1 << 24) != 0 {
                    self.r[14] = pc + 4;
                }
                let off = ((w & 0xFF_FFFF) << 8) as i32 >> 6;
                self.r[15] = (pc + 8).wrapping_add(off as u32);
            }
            7 if w & (1 << 24) != 0 && self.lenient => self.bios((w >> 16) & 0xFF)?,
            _ => return Err(bad()),
        }
        Ok(())
    }
}
