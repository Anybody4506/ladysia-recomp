//! Son : deux canaux DirectSound (FIFO A / B, alimentés par DMA au rythme des
//! minuteurs 0 / 1) et les quatre canaux PSG (repris de `psg.rs`). Sortie stéréo
//! 16 bits à 32 768 Hz.

use crate::psg::Psg;
use std::collections::VecDeque;

pub const RATE: u32 = 32_768;

#[derive(Clone)]
pub struct Apu {
    pub psg: Psg,
    fifo: [VecDeque<i8>; 2],
    out: [i8; 2],
    /// Échantillons produits (gauche, droite entrelacés), vidés par la fenêtre.
    pub buf: Vec<i16>,
    /// FIFO vide au moment de jouer un échantillon (diagnostic).
    pub underruns: [u64; 2],
    pub pops: [u64; 2],
    /// Son adouci : interpolation linéaire entre deux échantillons du mixeur au lieu de
    /// les tenir en marches d'escalier (réglage du portage PC, n'agit que sur la sortie).
    pub soft: bool,
    lisse: [Lisseur; 2],
}

/// Reconstruction « son adouci » d'un canal DirectSound : chaque échantillon du mixeur
/// est daté au cycle près ; la sortie est interpolée dans le temps (avec un petit
/// retard fixe) puis passe dans un passe-bas réglé sur la fréquence du mixeur, comme le
/// filtre analogique de la console. Ne sert qu'à la sortie PC.
#[derive(Clone, Default)]
struct Lisseur {
    hist: VecDeque<(u64, f32)>,
    /// passe-bas d'ordre 2 (deux cellules en cascade = ordre 4)
    coef: [f32; 5],
    z: [[f32; 2]; 2],
    /// période d'entrée (cycles) pour laquelle `coef` a été calculé
    periode: u64,
}

/// Retard de reconstruction (cycles) : un peu plus de deux échantillons du mixeur.
const RETARD: u64 = 4096;

impl Lisseur {
    fn push(&mut self, at: u64, v: f32) {
        if let Some(&(t, _)) = self.hist.back() {
            if at + 1_000_000 < t {
                self.hist.clear(); // retour en arrière (chargement d'un état)
            }
        }
        self.hist.push_back((at, v));
        while self.hist.len() > 8 {
            self.hist.pop_front();
        }
    }

    fn sample(&mut self, at: u64) -> f32 {
        let tau = at.saturating_sub(RETARD);
        let h = &self.hist;
        let Some(&(t0, v0)) = h.front() else { return 0.0 };
        let mut v = v0;
        if tau >= t0 {
            v = h.back().unwrap().1;
            for i in 1..h.len() {
                let (ta, va) = h[i - 1];
                let (tb, vb) = h[i];
                if tau < tb {
                    v = if tb > ta { va + (vb - va) * (tau - ta) as f32 / (tb - ta) as f32 } else { vb };
                    break;
                }
            }
        }
        // fréquence du mixeur d'après les deux derniers échantillons
        if h.len() >= 2 {
            let d = h[h.len() - 1].0 - h[h.len() - 2].0;
            if d > 64 && d < 16_384 && (d as i64 - self.periode as i64).abs() > 8 {
                self.periode = d;
                self.regler(16_777_216.0 / d as f64);
            }
        }
        if self.periode == 0 {
            return v;
        }
        let [b0, b1, b2, a1, a2] = self.coef;
        let mut x = v;
        for z in self.z.iter_mut() {
            // forme transposée directe II
            let y = b0 * x + z[0];
            z[0] = b1 * x - a1 * y + z[1];
            z[1] = b2 * x - a2 * y;
            x = y;
        }
        x
    }

    /// Butterworth passe-bas à 45 % de la fréquence du mixeur (au plus 9 kHz).
    fn regler(&mut self, fe: f64) {
        let fc = (0.45 * fe).min(9000.0);
        let w = 2.0 * std::f64::consts::PI * fc / RATE as f64;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - cs) / a0;
        self.coef = [(b1 / 2.0) as f32, b1 as f32, (b1 / 2.0) as f32, (-2.0 * cs / a0) as f32, ((1.0 - alpha) / a0) as f32];
    }
}

impl Apu {
    pub fn new() -> Self {
        Apu { psg: Psg::default(), fifo: [VecDeque::new(), VecDeque::new()], out: [0; 2], buf: Vec::new(), underruns: [0; 2], pops: [0; 2], soft: false, lisse: [Lisseur::default(), Lisseur::default()] }
    }

    pub fn push(&mut self, ch: usize, v: i8) {
        if self.fifo[ch].len() < 32 {
            self.fifo[ch].push_back(v);
        }
    }

    pub fn state(&self, w: &mut Vec<u8>) {
        for ch in 0..2 {
            w.push(self.fifo[ch].len() as u8);
            w.extend(self.fifo[ch].iter().map(|v| *v as u8));
            w.push(self.out[ch] as u8);
        }
    }

    pub fn load(&mut self, r: &mut super::State) -> Option<()> {
        for ch in 0..2 {
            let n = r.u8()? as usize;
            self.fifo[ch] = r.bytes(n)?.iter().map(|v| *v as i8).collect();
            self.out[ch] = r.u8()? as i8;
        }
        Some(())
    }

    pub fn reset(&mut self, ch: usize) {
        self.fifo[ch].clear();
    }

    /// Le minuteur du canal a débordé : échantillon suivant. Renvoie le remplissage.
    pub fn pop(&mut self, ch: usize, at: u64) -> usize {
        self.pops[ch] += 1;
        match self.fifo[ch].pop_front() {
            Some(v) => self.out[ch] = v,
            None => self.underruns[ch] += 1,
        }
        if self.soft {
            self.lisse[ch].push(at, self.out[ch] as f32);
        }
        self.fifo[ch].len()
    }

    pub fn sample(&mut self, cnt_h: u32, cnt_x: u8, at: u64) {
        if cnt_x & 0x80 == 0 {
            self.buf.extend_from_slice(&[0, 0]);
            return;
        }
        let p = self.psg.sample(1.0 / RATE as f64, 0);
        let mut v = [self.out[0] as f64, self.out[1] as f64];
        if self.soft {
            for ch in 0..2 {
                v[ch] = self.lisse[ch].sample(at) as f64;
            }
        }
        let a = v[0] * if cnt_h & 4 != 0 { 4.0 } else { 2.0 };
        let b = v[1] * if cnt_h & 8 != 0 { 4.0 } else { 2.0 };
        let mut l = p;
        let mut r = p;
        if cnt_h & 0x100 != 0 {
            r += a;
        }
        if cnt_h & 0x200 != 0 {
            l += a;
        }
        if cnt_h & 0x1000 != 0 {
            r += b;
        }
        if cnt_h & 0x2000 != 0 {
            l += b;
        }
        let k = 24.0;
        self.buf.push((l * k).clamp(-32768.0, 32767.0) as i16);
        self.buf.push((r * k).clamp(-32768.0, 32767.0) as i16);
    }
}
