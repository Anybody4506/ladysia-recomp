//! Banque de sons (pilote maison, `sub_08031A34` et voisines ; état en IWRAM `0x03006ED8`).
//!
//! Base `0x083546DC`, offsets relatifs à la base :
//! - `+0x18` : table des musiques `[n][offset × n]` (12 morceaux, lus par `sub_08033DB4`) ;
//! - `+0x1C` : table des échantillons `[?][offset …]`, chaque échantillon =
//!   `[u32 longueur][u32 début de boucle, -1 = sans][u16 fréquence][u16 ?][u32 0]`
//!   puis `longueur` octets PCM 8 bits signés.

use crate::rom::{Rom, ROM_BASE};
use std::fmt::Write as _;

pub const BANK: u32 = 0x0835_46DC;

pub struct Sample {
    pub addr: u32,
    pub rate: u32,
    pub loop_start: Option<u32>,
    pub pcm: Vec<i8>,
}

fn u32_at(rom: &Rom, a: u32) -> u32 {
    rom.u32((a - ROM_BASE) as usize).unwrap_or(0)
}

pub fn samples(rom: &Rom) -> Vec<Sample> {
    let table = BANK + u32_at(rom, BANK + 0x1C);
    let mut out = Vec::new();
    let mut prev = 0;
    for i in 0.. {
        let off = u32_at(rom, table + 4 + 4 * i);
        let a = BANK + off;
        let len = u32_at(rom, a);
        if off <= prev || a >= 0x083F_5000 || len > 0x10_0000 {
            break;
        }
        prev = off;
        let lp = u32_at(rom, a + 4);
        let start = (a + 16 - ROM_BASE) as usize;
        out.push(Sample {
            addr: a,
            rate: (u32_at(rom, a + 8) & 0xFFFF).max(1),
            loop_start: (lp != u32::MAX).then_some(lp),
            pcm: rom.data[start..start + len as usize].iter().map(|&b| b as i8).collect(),
        });
    }
    out
}

/// WAV mono 8 bits (non signé, comme l'exige le format).
pub fn wav(rate: u32, pcm: &[i8]) -> Vec<u8> {
    let mut w = Vec::with_capacity(44 + pcm.len());
    let n = pcm.len() as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + n).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes()); // octets / s
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&8u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&n.to_le_bytes());
    w.extend(pcm.iter().map(|&s| (s as i16 + 128) as u8));
    w
}

pub fn run(rom: &Rom) -> std::io::Result<String> {
    std::fs::create_dir_all("extract/son")?;
    let list = samples(rom);
    let mut idx = String::from("n°  adresse   octets  fréquence  boucle\n");
    let mut total = 0;
    for (i, s) in list.iter().enumerate() {
        std::fs::write(format!("extract/son/ech_{i:02}_{:08X}.wav", s.addr), wav(s.rate, &s.pcm))?;
        total += s.pcm.len();
        let lp = s.loop_start.map_or("-".to_string(), |l| l.to_string());
        writeln!(idx, "{i:2}  {:08X}  {:6}  {:9}  {lp}", s.addr, s.pcm.len(), s.rate).unwrap();
    }
    std::fs::write("extract/son/index.txt", idx)?;
    let songs = u32_at(rom, BANK + u32_at(rom, BANK + 0x18));
    Ok(format!(
        "Son : banque {BANK:08X}, {} échantillons ({} Ko) -> extract/son/, {songs} musiques (rendu : commande `music`)\n",
        list.len(),
        total / 1024
    ))
}
