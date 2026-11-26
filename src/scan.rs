//! Cartographie de la ROM : pointeurs, entropie, blocs LZ77, moteur son m4a.

use crate::rom::{Rom, ROM_BASE};
use std::collections::HashSet;

pub struct Block {
    pub off: usize,
    pub ptr_count: usize,
    pub fill_ratio: f64,
    pub entropy: f64,
}

impl Block {
    /// Classement grossier, à affiner par le désassemblage.
    pub fn guess(&self) -> &'static str {
        if self.fill_ratio > 0.95 {
            "vide (remplissage)"
        } else if self.entropy > 7.5 {
            "données compressées ?"
        } else if self.ptr_count > 300 {
            "tables / pointeurs"
        } else if self.entropy > 5.0 && self.ptr_count > 20 {
            "code probable"
        } else {
            "données (graphismes ?)"
        }
    }
}

pub const BLOCK_SIZE: usize = 0x10000;

pub fn blocks(rom: &Rom) -> Vec<Block> {
    rom.data
        .chunks(BLOCK_SIZE)
        .enumerate()
        .map(|(i, chunk)| {
            let ptr_count = chunk
                .chunks_exact(4)
                .filter(|w| rom.is_rom_ptr(u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
                .count();
            let mut hist = [0usize; 256];
            for &b in chunk {
                hist[b as usize] += 1;
            }
            let n = chunk.len() as f64;
            let entropy = hist
                .iter()
                .filter(|&&h| h > 0)
                .map(|&h| {
                    let p = h as f64 / n;
                    -p * p.log2()
                })
                .sum();
            let fill_ratio = hist[0].max(hist[0xFF]) as f64 / n;
            Block { off: i * BLOCK_SIZE, ptr_count, fill_ratio, entropy }
        })
        .collect()
}

/// Ensemble des offsets de la ROM qui sont la cible d'au moins un pointeur aligné.
pub fn pointer_targets(rom: &Rom) -> HashSet<usize> {
    rom.data
        .chunks_exact(4)
        .filter_map(|w| rom.addr_to_off(u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
        .collect()
}

pub struct Lz77 {
    pub off: usize,
    pub packed_len: usize,
    pub unpacked_len: usize,
    pub referenced: bool,
}

/// Décompresse un bloc LZ77 du BIOS GBA (type 0x10). Renvoie (données, octets lus).
pub fn lz77_decompress(src: &[u8]) -> Option<(Vec<u8>, usize)> {
    if src.len() < 4 || src[0] != 0x10 {
        return None;
    }
    let size = u32::from_le_bytes([src[1], src[2], src[3], 0]) as usize;
    let mut out = Vec::with_capacity(size);
    let mut i = 4;
    while out.len() < size {
        let flags = *src.get(i)?;
        i += 1;
        for bit in (0..8).rev() {
            if out.len() >= size {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(*src.get(i)?);
                i += 1;
            } else {
                let b1 = *src.get(i)? as usize;
                let b2 = *src.get(i + 1)? as usize;
                i += 2;
                let len = (b1 >> 4) + 3;
                let disp = (((b1 & 0xF) << 8) | b2) + 1;
                if disp > out.len() {
                    return None; // référence avant le début : faux positif
                }
                for _ in 0..len {
                    out.push(out[out.len() - disp]);
                }
            }
        }
    }
    out.truncate(size);
    Some((out, i))
}

pub fn find_lz77(rom: &Rom, targets: &HashSet<usize>) -> Vec<Lz77> {
    let mut found = Vec::new();
    let mut off = 0;
    while off + 4 <= rom.len() {
        let size = rom.u32(off).unwrap() >> 8;
        // Seuls les blocs pointés depuis ailleurs sont retenus, sinon trop de faux positifs.
        if rom.data[off] == 0x10 && (0x40..=0x40000).contains(&size) && targets.contains(&off) {
            if let Some((_, used)) = lz77_decompress(&rom.data[off..]) {
                if used < size as usize {
                    found.push(Lz77 {
                        off,
                        packed_len: used,
                        unpacked_len: size as usize,
                        referenced: true,
                    });
                    off += (used + 3) & !3;
                    continue;
                }
            }
        }
        off += 4;
    }
    found
}

pub struct SoundEngine {
    pub select_song_off: usize,
    pub song_table: Option<u32>,
    pub song_count: usize,
}

/// Signature Thumb de `m4aSongNumStart` / SelectSong du moteur Nintendo "MusicPlayer2000" (Sappy).
const SELECT_SONG: [u8; 30] = [
    0x00, 0xB5, 0x00, 0x04, 0x07, 0x4A, 0x08, 0x49, 0x40, 0x0B, 0x40, 0x18, 0x83, 0x88, 0x59,
    0x00, 0xC9, 0x18, 0x89, 0x00, 0x89, 0x18, 0x0A, 0x68, 0x01, 0x68, 0x10, 0x1C, 0x00, 0xF0,
];

pub fn find_m4a(rom: &Rom) -> Option<SoundEngine> {
    let pos = rom.data.windows(SELECT_SONG.len()).position(|w| w == SELECT_SONG)?;
    // Le pointeur vers la table des morceaux se trouve dans le pool littéral, 40 octets plus loin.
    let song_table = rom.u32(pos + 40).filter(|&p| rom.is_rom_ptr(p));
    let mut song_count = 0;
    if let Some(off) = song_table.and_then(|p| rom.addr_to_off(p)) {
        // Chaque entrée fait 8 octets : pointeur vers l'en-tête du morceau + numéros de lecteur.
        while let Some(p) = rom.u32(off + song_count * 8) {
            if !rom.is_rom_ptr(p) {
                break;
            }
            song_count += 1;
        }
    }
    Some(SoundEngine { select_song_off: pos, song_table, song_count })
}

pub fn fmt_addr(off: usize) -> String {
    format!("0x{:08x}", ROM_BASE + off as u32)
}
