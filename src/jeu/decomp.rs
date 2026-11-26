//! Décompresseurs du jeu (étapes 4 et 5 de la décompilation) branchés à la place du
//! code d'origine.

use super::{bytes, read_u32, Env};
use crate::gba::bus::Bus;

/// Écrit `data` à `dst` par mots de 32 bits (le reste octet par octet).
fn store(bus: &mut Bus, dst: u32, data: &[u8]) {
    let words = data.len() / 4;
    for k in 0..words {
        let v = u32::from_le_bytes(data[4 * k..4 * k + 4].try_into().unwrap());
        bus.write32(dst + 4 * k as u32, v);
    }
    for k in words * 4..data.len() {
        bus.write8(dst + k as u32, data[k]);
    }
}

/// `Decompress(dest, src)` (0x0802F0B0) : RLE maison, renvoie la taille décompressée.
/// Original : Thumb -> `Decomp_Entry` (ARM 0x08000248) -> `Decomp_Core` (ARM en IWRAM).
pub fn decompress(env: &mut Env) {
    let Env { cpu, bus, nat } = env;
    let _ = &nat;
    let (dest, src) = (cpu.r[0], cpu.r[1]);
    bus.write32(0x0300_6CC4, src);
    bus.write32(0x0300_6CC8, dest);
    let size = read_u32(bus, src);
    bus.write32(0x0300_6CCC, size);
    let input = bytes(bus, src, size as usize * 2 + 16);
    if input.get(4).is_some_and(|m| m & 0xF == 0) {
        // copie brute : l'original lit des mots de 32 bits à partir de `src + 5`, adresse
        // non alignée, et l'ARM7 fait alors tourner les octets du mot lu
        let mut data = Vec::with_capacity(size as usize + 3);
        let mut a = src.wrapping_add(5);
        while data.len() < size as usize {
            let w = bus.read32(a & !3).rotate_right((a & 3) * 8);
            data.extend_from_slice(&w.to_le_bytes());
            a = a.wrapping_add(4);
        }
        data.truncate(size as usize);
        store(bus, dest, &data);
    } else if let Some(d) = crate::rle::decompress(&input) {
        store(bus, dest, &d.data);
    }
    cpu.r[0] = bus.read32(0x0300_6CCC);
}

/// `LoadPacked(dest, src)` (0x0802EE7C) : ressource LZ + Huffman du bloc 0x08260000.
/// Met la destination à zéro par DMA3, décode (dans un tampon en fin d'EWRAM si la
/// destination n'est pas en EWRAM, puis copie par DMA3) ; renvoie 1.
pub fn load_packed(env: &mut Env) {
    let Env { cpu, bus, nat } = env;
    let _ = &nat;
    let (dest, src) = (cpu.r[0], cpu.r[1]);
    let zero_word = cpu.r[13] - 28; // [sp] après les 7 empilements de l'original
    bus.write32(0x0200_1B64, 0);
    let size = read_u32(bus, src);
    bus.write32(0x0200_1C5C, size);
    bus.write32(0x0200_1224, size);
    let packed = read_u32(bus, src + 4);
    bus.write32(0x0200_1C48, packed);
    bus.write32(0x0200_1C50, src + 8);

    let input = bytes(bus, src, packed as usize + 8 + 64);
    let out = crate::packed::decompress(&nat.ctx.packed, &input).map(|u| u.data).unwrap_or_default();
    let dma = |bus: &mut Bus, s: u32, d: u32, cnt: u32| {
        bus.write32(0x0400_00D4, s);
        bus.write32(0x0400_00D8, d);
        bus.write32(0x0400_00DC, cnt);
    };
    bus.write32(zero_word, 0);
    let in_ewram = dest & 0xFF00_0000 == 0x0200_0000;
    let target = if in_ewram { dest } else { 0x0204_0000 - size };
    dma(bus, zero_word, target, (size >> 2) | 0x8500_0000);
    // l'original écrit octet par octet : la destination peut être non alignée
    let n = out.len().min(size as usize);
    for (k, &v) in out[..n].iter().enumerate() {
        bus.write8(target + k as u32, v);
    }
    if !in_ewram {
        dma(bus, target, dest, (size >> 2) | 0x8400_0000);
    }
    cpu.r[0] = 1;
}
