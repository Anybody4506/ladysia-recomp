//! Synthèse des 4 canaux « Game Boy » de la GBA (deux carrés, onde, bruit) à partir des
//! écritures horodatées dans les registres 0x04000060-0x0400009F, et mixage avec le
//! DirectSound comme le fait le matériel (SOUNDCNT_L / SOUNDCNT_H).

#[derive(Clone)]
pub struct Psg {
    reg: [u8; 0x40],
    wave: [u8; 32],
    sq: [Square; 2],
    wv: Wave,
    nz: Noise,
}

#[derive(Default, Clone, Copy)]
struct Env {
    vol: i32,
    step: u32,
    up: bool,
    t: f64,
}

impl Env {
    fn load(&mut self, v: u16) {
        self.vol = (v >> 12) as i32;
        self.up = v & 0x800 != 0;
        self.step = ((v >> 8) & 7) as u32;
        self.t = 0.0;
    }
    fn tick(&mut self, dt: f64) {
        if self.step == 0 {
            return;
        }
        self.t += dt;
        let period = self.step as f64 / 64.0;
        while self.t >= period {
            self.t -= period;
            self.vol = (self.vol + if self.up { 1 } else { -1 }).clamp(0, 15);
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Square {
    on: bool,
    freq: u32,
    duty: u32,
    phase: f64,
    env: Env,
    len_left: f64,
    len_on: bool,
}

#[derive(Default, Clone, Copy)]
struct Wave {
    on: bool,
    freq: u32,
    phase: f64,
    len_left: f64,
    len_on: bool,
}

#[derive(Default, Clone, Copy)]
struct Noise {
    on: bool,
    lfsr: u16,
    narrow: bool,
    rate: f64,
    acc: f64,
    env: Env,
    len_left: f64,
    len_on: bool,
}

impl Default for Psg {
    fn default() -> Self {
        Psg { reg: [0; 0x40], wave: [0; 32], sq: [Square::default(); 2], wv: Wave::default(), nz: Noise::default() }
    }
}

impl Psg {
    fn r16(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.reg[o], self.reg[o + 1]])
    }

    /// Écriture d'un octet dans un registre (adresse IO relative à 0x04000060).
    pub fn write(&mut self, off: usize, v: u8) {
        if off >= 0x40 {
            return;
        }
        self.reg[off] = v;
        match off {
            // déclenchement : octet haut des registres de fréquence (bit 15 = restart)
            0x05 | 0x0D => {
                let c = if off == 0x05 { 0 } else { 1 };
                let (cnt, x) = if c == 0 { (self.r16(0x02), self.r16(0x04)) } else { (self.r16(0x08), self.r16(0x0C)) };
                let s = &mut self.sq[c];
                s.freq = (x & 0x7FF) as u32;
                s.len_on = x & 0x4000 != 0;
                s.duty = ((cnt >> 6) & 3) as u32;
                if v & 0x80 != 0 {
                    s.on = true;
                    s.env.load(cnt);
                    s.len_left = (64 - (cnt & 63) as u32) as f64 / 256.0;
                }
            }
            0x04 | 0x0C => {
                let c = if off == 0x04 { 0 } else { 1 };
                let x = if c == 0 { self.r16(0x04) } else { self.r16(0x0C) };
                self.sq[c].freq = (x & 0x7FF) as u32;
            }
            0x03 | 0x09 => {
                let c = if off == 0x03 { 0 } else { 1 };
                let cnt = if c == 0 { self.r16(0x02) } else { self.r16(0x08) };
                self.sq[c].duty = ((cnt >> 6) & 3) as u32;
                if cnt >> 12 == 0 && cnt & 0x800 == 0 {
                    self.sq[c].on = false; // DAC éteint
                }
            }
            0x10 => {
                if v & 0x80 == 0 {
                    self.wv.on = false;
                }
            }
            0x14 => self.wv.freq = (self.r16(0x14) & 0x7FF) as u32,
            0x15 => {
                let x = self.r16(0x14);
                self.wv.freq = (x & 0x7FF) as u32;
                self.wv.len_on = x & 0x4000 != 0;
                if v & 0x80 != 0 && self.reg[0x10] & 0x80 != 0 {
                    self.wv.on = true;
                    self.wv.phase = 0.0;
                    self.wv.len_left = (256 - self.reg[0x12] as u32) as f64 / 256.0;
                }
            }
            0x1D => {
                let x = self.r16(0x1C);
                let cnt = self.r16(0x18);
                let r = (x & 7) as f64;
                let shift = ((x >> 4) & 15) as i32;
                self.nz.rate = 524_288.0 / if r == 0.0 { 0.5 } else { r } / 2f64.powi(shift + 1);
                self.nz.narrow = x & 8 != 0;
                self.nz.len_on = x & 0x4000 != 0;
                if v & 0x80 != 0 {
                    self.nz.on = true;
                    self.nz.lfsr = 0x7FFF;
                    self.nz.env.load(cnt);
                    self.nz.len_left = (64 - (cnt & 63) as u32) as f64 / 256.0;
                }
            }
            0x30..=0x3F => {
                let i = (off - 0x30) * 2;
                self.wave[i] = v >> 4;
                self.wave[i + 1] = v & 15;
            }
            _ => {}
        }
    }

    /// Un échantillon de sortie (gauche + droite moyennés), domaine ~10 bits signé,
    /// `dma` = échantillon 8 bits signé du FIFO.
    pub fn sample(&mut self, dt: f64, dma: i8) -> f64 {
        let cnt_l = self.r16(0x20);
        let cnt_h = self.r16(0x22);
        let mut ch = [0f64; 4];

        for (i, s) in self.sq.iter_mut().enumerate() {
            if s.len_on {
                s.len_left -= dt;
                if s.len_left <= 0.0 {
                    s.on = false;
                }
            }
            s.env.tick(dt);
            if s.on && s.freq < 2048 {
                let f = 131_072.0 / (2048 - s.freq) as f64;
                s.phase = (s.phase + f * dt).fract();
                let duty = [0.125, 0.25, 0.5, 0.75][s.duty as usize];
                ch[i] = if s.phase < duty { s.env.vol as f64 } else { 0.0 };
            }
        }
        {
            let w = &mut self.wv;
            if w.len_on {
                w.len_left -= dt;
                if w.len_left <= 0.0 {
                    w.on = false;
                }
            }
            if w.on && w.freq < 2048 {
                let f = 65_536.0 / (2048 - w.freq) as f64;
                w.phase = (w.phase + f * dt).fract();
                let smp = self.wave[(w.phase * 32.0) as usize & 31] as f64;
                let vh = self.r16(0x12);
                let vol = if vh & 0x8000 != 0 { 0.75 } else { [0.0, 1.0, 0.5, 0.25][((vh >> 13) & 3) as usize] };
                ch[2] = smp * vol;
            }
        }
        {
            let n = &mut self.nz;
            if n.len_on {
                n.len_left -= dt;
                if n.len_left <= 0.0 {
                    n.on = false;
                }
            }
            n.env.tick(dt);
            if n.on {
                n.acc += n.rate * dt;
                while n.acc >= 1.0 {
                    n.acc -= 1.0;
                    let bit = (n.lfsr ^ (n.lfsr >> 1)) & 1;
                    n.lfsr = (n.lfsr >> 1) | (bit << 14);
                    if n.narrow {
                        n.lfsr = (n.lfsr & !0x40) | (bit << 6);
                    }
                }
                ch[3] = if n.lfsr & 1 == 0 { n.env.vol as f64 } else { 0.0 };
            }
        }

        // SOUNDCNT_L : volumes 0-7 et routage gauche/droite ; SOUNDCNT_H : rapport PSG et DMA
        let side = |vol: u16, en: u16| {
            let s: f64 = (0..4).filter(|i| en & (1 << i) != 0).map(|i| ch[i]).sum();
            s * (vol + 1) as f64
        };
        let ratio = [0.25, 0.5, 1.0, 1.0][(cnt_h & 3) as usize];
        let right = side(cnt_l & 7, (cnt_l >> 8) & 15) * ratio;
        let left = side((cnt_l >> 4) & 7, (cnt_l >> 12) & 15) * ratio;
        let dma_a = dma as f64 * if cnt_h & 4 != 0 { 4.0 } else { 2.0 };
        let dma_b = dma as f64 * if cnt_h & 8 != 0 { 4.0 } else { 2.0 };
        let r = right + if cnt_h & 0x100 != 0 { dma_a } else { 0.0 } + if cnt_h & 0x1000 != 0 { dma_b } else { 0.0 };
        let l = left + if cnt_h & 0x200 != 0 { dma_a } else { 0.0 } + if cnt_h & 0x2000 != 0 { dma_b } else { 0.0 };
        (l + r) / 2.0
    }
}
