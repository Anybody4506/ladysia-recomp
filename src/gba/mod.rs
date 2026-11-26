//! Machine GBA réécrite en Rust : le code d'origine du jeu (ARM/Thumb) s'exécute
//! sur ce matériel recréé. Une image = 228 lignes de 1232 cycles.

pub mod apu;
pub mod bus;
pub mod cpu;
pub mod eeprom;
pub mod hle;
pub mod ppu;

use bus::Bus;
use cpu::Cpu;

/// Bits de KEYINPUT.
pub mod key {
    pub const A: u16 = 1;
    pub const B: u16 = 2;
    pub const SELECT: u16 = 4;
    pub const START: u16 = 8;
    pub const RIGHT: u16 = 16;
    pub const LEFT: u16 = 32;
    pub const UP: u16 = 64;
    pub const DOWN: u16 = 128;
    pub const R: u16 = 256;
    pub const L: u16 = 512;
}

pub struct Gba {
    pub cpu: Cpu,
    pub bus: Bus,
    pub frame: Vec<u32>,
    /// Fonctions du jeu réécrites en Rust, appelées à la place du code d'origine.
    pub natives: crate::jeu::Natives,
    /// Tests (`LADYSIA_NIVEAU`) : état du jeu (`0x03000B50`) imposé au prochain chargement
    /// de scène (0x0800043E), pour aller directement à un niveau.
    pub force_state: Option<u32>,
    /// Tests (`LADYSIA_COMPARE`) : adresse et cycles à chaque bloc ou fonction exécuté.
    pub trace: Option<Vec<(u32, u64)>>,
}

/// Lecture d'une sauvegarde instantanée.
pub struct State<'a> {
    d: &'a [u8],
    p: usize,
}

impl State<'_> {
    pub fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let b = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(b)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
}

const STATE_MAGIC: &[u8; 8] = b"LADYSIA1";

impl Gba {
    pub fn new(rom: Vec<u8>, save: Option<Vec<u8>>) -> Self {
        let natives = crate::jeu::Natives::new(&rom);
        crate::recomp::init(&rom);
        let mut natives = natives;
        if crate::recomp::data_only() {
            // pas de code d'origine : ni interprétation, ni comparaison avec l'original
            natives.enabled = true;
            natives.check = false;
        }
        Gba { cpu: Cpu::new(), bus: Bus::new(rom, eeprom::Eeprom::new(save)), frame: vec![0; ppu::W * ppu::H], natives, force_state: None, trace: None }
    }

    pub fn save_state(&self) -> Vec<u8> {
        let mut w = STATE_MAGIC.to_vec();
        self.cpu.state(&mut w);
        self.bus.state(&mut w);
        w
    }

    pub fn load_state(&mut self, d: &[u8]) -> bool {
        if !d.starts_with(STATE_MAGIC) {
            return false;
        }
        let backup = (self.cpu.clone(), self.bus.clone());
        let mut r = State { d, p: 8 };
        if self.cpu.load(&mut r).and_then(|_| self.bus.load(&mut r)).is_none() {
            (self.cpu, self.bus) = backup;
            return false;
        }
        if crate::recomp::data_only() {
            self.bus.strip_copies(crate::recomp::copies());
        }
        true
    }

    pub fn set_keys(&mut self, keys: u16) {
        // une direction et son opposée ne peuvent pas être enfoncées ensemble
        let mut k = keys;
        if k & key::LEFT != 0 && k & key::RIGHT != 0 {
            k &= !(key::LEFT | key::RIGHT);
        }
        if k & key::UP != 0 && k & key::DOWN != 0 {
            k &= !(key::UP | key::DOWN);
        }
        self.bus.keys = k;
        let cnt = self.bus.io16(0x132);
        if cnt & 0x4000 != 0 {
            let sel = (cnt & 0x3FF) as u16;
            let hit = if cnt & 0x8000 != 0 { k & sel == sel } else { k & sel != 0 };
            if hit {
                self.bus.irq(12);
            }
        }
    }

    fn run(&mut self, cycles: u64) {
        let target = self.bus.cyc + cycles;
        while self.bus.cyc < target {
            let pending = self.bus.irq_pending();
            if self.cpu.halted {
                if pending {
                    self.cpu.halted = false;
                } else {
                    let d = (target - self.bus.cyc).min(64);
                    self.bus.cyc += d;
                    self.bus.tick(d as u32);
                    continue;
                }
            }
            if pending && self.bus.ime() && self.cpu.irq_enabled() {
                self.cpu.irq();
            }
            if let Some(t) = &mut self.trace {
                // cycles mêlés aux registres : repère aussi le premier écart de valeur
                let h = self.cpu.r[..15].iter().fold(0u64, |h, &r| (h ^ r as u64).wrapping_mul(0x100_0000_01B3));
                t.push((self.cpu.r[15], self.bus.cyc ^ (h << 24)));
            }
            if let Some(n) = self.force_state {
                if self.cpu.r[15] == 0x0800_043E {
                    self.bus.write32(0x0300_0B50, n);
                    self.force_state = None;
                }
            }
            if self.natives.enabled {
                let pc = self.cpu.r[15];
                if pc.wrapping_sub(0x0800_0000) < crate::jeu::CODE_SIZE || pc >> 24 == 3 {
                    if let Some(id) = self.natives.lookup(pc, &self.bus) {
                        // le code natif est quasi instantané : ses accès mémoire ne
                        // comptent pas (sinon les interruptions du son seraient retardées)
                        let before = self.bus.cyc;
                        self.natives.call(id, &mut self.cpu, &mut self.bus);
                        self.bus.cyc = before + 20;
                        self.bus.tick(20);
                        continue;
                    }
                }
            }
            if self.natives.enabled {
                if let Some(b) = crate::recomp::lookup(self.cpu.r[15], self.cpu.thumb(), &self.bus) {
                    // bloc de code recompilé : comme une suite d'instructions interprétées
                    let before = self.bus.cyc;
                    crate::recomp::run(b, &mut self.cpu, &mut self.bus);
                    if self.bus.halt_req {
                        self.bus.halt_req = false;
                        self.cpu.halted = true;
                    }
                    let d = self.bus.cyc - before;
                    self.bus.tick(d as u32);
                    continue;
                }
            }
            crate::recomp::interpreting(self.cpu.r[15]);
            let before = self.bus.cyc;
            self.bus.count(self.cpu.r[15]);
            self.cpu.step(&mut self.bus);
            if self.bus.halt_req {
                self.bus.halt_req = false;
                self.cpu.halted = true;
            }
            let d = self.bus.cyc - before;
            self.bus.tick(d as u32);
        }
    }

    fn set_stat(&mut self, flag: u32, on: bool) {
        let v = self.bus.io16(4);
        self.bus.set_io16(4, if on { v | flag } else { v & !flag });
    }

    /// Exécute une image complète (environ 1/59,73 s de jeu).
    pub fn run_frame(&mut self) {
        for line in 0..228u32 {
            self.bus.set_io16(6, line);
            let stat = self.bus.io16(4);
            let hit = (stat >> 8) == line;
            self.set_stat(4, hit);
            if hit && stat & 0x20 != 0 {
                self.bus.irq(2);
            }
            if line == 160 {
                self.set_stat(1, true);
                if stat & 8 != 0 {
                    self.bus.irq(0);
                }
                self.bus.dma_event(1);
            }
            if line == 227 {
                self.set_stat(1, false);
            }
            if line == 0 {
                // les points de référence affines sont rechargés à chaque image
                for k in 0..2 {
                    let b = 0x28 + 0x10 * k;
                    self.bus.bg_ref[k][0] = ((self.bus.io32(b) << 4) as i32) >> 4;
                    self.bus.bg_ref[k][1] = ((self.bus.io32(b + 4) << 4) as i32) >> 4;
                }
            }
            self.run(960);
            self.set_stat(2, true);
            if self.bus.io16(4) & 0x10 != 0 {
                self.bus.irq(1);
            }
            if line < 160 {
                ppu::render_line(&mut self.bus, line as usize, &mut self.frame);
                self.bus.dma_event(2);
            }
            self.run(272);
            self.set_stat(2, false);
            for d in self.bus.bg_delay.iter_mut() {
                *d = d.saturating_sub(1);
            }
        }
    }
}
