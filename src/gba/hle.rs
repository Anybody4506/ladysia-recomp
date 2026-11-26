//! Réécriture en Rust des appels BIOS (SWI) : le BIOS d'origine n'est pas nécessaire.

use super::bus::Bus;
use super::cpu::Cpu;
use std::f64::consts::PI;

pub fn swi(cpu: &mut Cpu, bus: &mut Bus, n: u32, addr: u32) {
    let r = &mut cpu.r;
    match n {
        0x00 => {
            // SoftReset
            cpu.reset();
            cpu.jump(0x0800_0000);
        }
        0x01 => bus.clear(r[0]),
        0x02 => cpu.halted = true,
        0x03 => cpu.halted = true,
        0x04 | 0x05 => {
            let (discard, mask) = if n == 5 { (true, 1) } else { (r[0] != 0, r[1]) };
            if n == 5 {
                r[0] = 1;
                r[1] = 1;
            }
            bus.io[0x208] = 1; // le BIOS active IME
            if !cpu.waiting && discard {
                let f = bus.read16(0x0300_7FF8);
                bus.write16(0x0300_7FF8, (f & !mask) as u16);
            }
            let f = bus.read16(0x0300_7FF8);
            if f & mask != 0 {
                bus.write16(0x0300_7FF8, (f & !mask) as u16);
                cpu.waiting = false;
            } else {
                cpu.waiting = true;
                cpu.halted = true;
                cpu.jump(addr);
            }
        }
        0x06 | 0x07 => {
            if n == 7 {
                r.swap(0, 1);
            }
            let (a, b) = (r[0] as i32, r[1] as i32);
            if b != 0 {
                let q = a.wrapping_div(b);
                r[0] = q as u32;
                r[1] = a.wrapping_rem(b) as u32;
                r[3] = q.unsigned_abs();
            }
        }
        0x08 => r[0] = (r[0] as f64).sqrt() as u32,
        0x09 => {
            let t = r[0] as i16 as f64 / 16384.0;
            r[0] = ((t.atan() / (2.0 * PI) * 65536.0) as i32) as u32 & 0xFFFF;
        }
        0x0A => {
            let (x, y) = (r[0] as i16 as f64, r[1] as i16 as f64);
            let a = y.atan2(x);
            let a = if a < 0.0 { a + 2.0 * PI } else { a };
            r[0] = ((a / (2.0 * PI) * 65536.0) as u32) & 0xFFFF;
        }
        0x0B | 0x0C => {
            let (mut src, mut dst, cnt) = (r[0], r[1], r[2]);
            let fill = cnt & (1 << 24) != 0;
            let word = n == 0x0C || cnt & (1 << 26) != 0;
            let mut count = cnt & 0x1F_FFFF;
            if n == 0x0C {
                count = (count + 7) & !7;
            }
            let step = if word { 4 } else { 2 };
            let fixed = if word { bus.read32(src) } else { bus.read16(src) };
            for _ in 0..count {
                if word {
                    let v = if fill { fixed } else { bus.read32(src) };
                    bus.write32(dst, v);
                } else {
                    let v = if fill { fixed } else { bus.read16(src) };
                    bus.write16(dst, v as u16);
                }
                if !fill {
                    src = src.wrapping_add(step);
                }
                dst = dst.wrapping_add(step);
            }
        }
        0x0D => r[0] = 0xBAAE_187F,
        0x0E => bg_affine(bus, r[0], r[1], r[2]),
        0x0F => obj_affine(bus, r[0], r[1], r[2], r[3]),
        0x10 => bit_unpack(bus, r[0], r[1], r[2]),
        0x11 | 0x12 => {
            let out = lz77(bus, r[0]);
            store(bus, r[1], &out, n == 0x12);
        }
        0x13 => {
            let out = huff(bus, r[0]);
            store(bus, r[1], &out, true);
        }
        0x14 | 0x15 => {
            let out = rl(bus, r[0]);
            store(bus, r[1], &out, n == 0x15);
        }
        0x16..=0x18 => {
            let out = diff(bus, r[0], n == 0x18);
            store(bus, r[1], &out, n != 0x16);
        }
        0x19 => {}
        0x1F => {
            let base = bus.read32(r[0].wrapping_add(4)) as f64;
            let key = r[1] as f64 + r[2] as f64 / 256.0;
            r[0] = (base * 2f64.powf((key - 180.0) / 12.0)) as u32;
        }
        _ => eprintln!("SWI non géré : 0x{n:02X} en {addr:08X}"),
    }
}

fn store(bus: &mut Bus, dst: u32, data: &[u8], halfwords: bool) {
    if halfwords {
        for (k, c) in data.chunks(2).enumerate() {
            let v = c[0] as u16 | (*c.get(1).unwrap_or(&0) as u16) << 8;
            bus.write16(dst + 2 * k as u32, v);
        }
    } else {
        for (k, &b) in data.iter().enumerate() {
            bus.write8(dst + k as u32, b);
        }
    }
}

fn lz77(bus: &mut Bus, src: u32) -> Vec<u8> {
    let size = (bus.read32(src) >> 8) as usize;
    let mut s = src + 4;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let flags = bus.read8(s);
        s += 1;
        for bit in 0..8 {
            if out.len() >= size {
                break;
            }
            if flags & (0x80 >> bit) != 0 {
                let (b1, b2) = (bus.read8(s) as usize, bus.read8(s + 1) as usize);
                s += 2;
                let len = (b1 >> 4) + 3;
                let disp = (((b1 & 15) << 8) | b2) + 1;
                for _ in 0..len {
                    let v = if disp <= out.len() { out[out.len() - disp] } else { 0 };
                    out.push(v);
                }
            } else {
                out.push(bus.read8(s) as u8);
                s += 1;
            }
        }
    }
    out.truncate(size);
    out
}

fn rl(bus: &mut Bus, src: u32) -> Vec<u8> {
    let size = (bus.read32(src) >> 8) as usize;
    let mut s = src + 4;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let f = bus.read8(s);
        s += 1;
        if f & 0x80 != 0 {
            let v = bus.read8(s) as u8;
            s += 1;
            for _ in 0..(f & 0x7F) + 3 {
                out.push(v);
            }
        } else {
            for _ in 0..f + 1 {
                out.push(bus.read8(s) as u8);
                s += 1;
            }
        }
    }
    out.truncate(size);
    out
}

fn huff(bus: &mut Bus, src: u32) -> Vec<u8> {
    let hdr = bus.read32(src);
    let bits = (hdr & 15).max(1);
    let size = (hdr >> 8) as usize;
    let tree = src + 5;
    let mut data = src + 4 + (bus.read8(src + 4) + 1) * 2;
    let mut out = Vec::with_capacity(size);
    let (mut acc, mut nacc) = (0u32, 0u32);
    let mut node = tree;
    while out.len() < size {
        let word = bus.read32(data);
        data += 4;
        for bit in (0..32).rev() {
            let nv = bus.read8(node);
            let dir = (word >> bit) & 1;
            let next = (node & !1) + (nv & 0x3F) * 2 + 2 + dir;
            let leaf = if dir == 0 { nv & 0x80 != 0 } else { nv & 0x40 != 0 };
            if leaf {
                acc |= (bus.read8(next) & ((1 << bits) - 1)) << nacc;
                nacc += bits;
                node = tree;
                if nacc == 32 {
                    out.extend_from_slice(&acc.to_le_bytes());
                    acc = 0;
                    nacc = 0;
                    if out.len() >= size {
                        break;
                    }
                }
            } else {
                node = next;
            }
        }
    }
    out.truncate(size);
    out
}

fn diff(bus: &mut Bus, src: u32, sixteen: bool) -> Vec<u8> {
    let size = (bus.read32(src) >> 8) as usize;
    let mut out = Vec::with_capacity(size);
    if sixteen {
        let mut acc = 0u16;
        for k in 0..size / 2 {
            acc = acc.wrapping_add(bus.read16(src + 4 + 2 * k as u32) as u16);
            out.extend_from_slice(&acc.to_le_bytes());
        }
    } else {
        let mut acc = 0u8;
        for k in 0..size {
            acc = acc.wrapping_add(bus.read8(src + 4 + k as u32) as u8);
            out.push(acc);
        }
    }
    out
}

fn bit_unpack(bus: &mut Bus, src: u32, dst: u32, info: u32) {
    let len = bus.read16(info);
    let sw = bus.read8(info + 2);
    let dw = bus.read8(info + 3);
    let off = bus.read32(info + 4);
    let zero = off & 0x8000_0000 != 0;
    let off = off & 0x7FFF_FFFF;
    let (mut acc, mut nacc, mut d) = (0u32, 0u32, dst);
    for k in 0..len {
        let b = bus.read8(src + k);
        let mut sh = 0;
        while sh < 8 {
            let v = (b >> sh) & ((1 << sw) - 1);
            let v = if v != 0 || zero { v + off } else { 0 };
            acc |= (v & ((1u64 << dw) - 1) as u32) << nacc;
            nacc += dw;
            if nacc >= 32 {
                bus.write32(d, acc);
                d += 4;
                acc = 0;
                nacc = 0;
            }
            sh += sw.max(1);
        }
    }
}

fn sincos(angle: u32) -> (f64, f64) {
    let a = (angle & 0xFFFF) as f64 / 65536.0 * 2.0 * PI;
    (a.sin(), a.cos())
}

fn bg_affine(bus: &mut Bus, mut src: u32, mut dst: u32, count: u32) {
    for _ in 0..count {
        let ox = bus.read32(src) as i32 as f64;
        let oy = bus.read32(src + 4) as i32 as f64;
        let dx = bus.read16(src + 8) as i16 as f64;
        let dy = bus.read16(src + 10) as i16 as f64;
        let sx = bus.read16(src + 12) as i16 as f64;
        let sy = bus.read16(src + 14) as i16 as f64;
        let (s, c) = sincos(bus.read16(src + 16) & 0xFF00);
        let pa = (sx * c) as i32;
        let pb = -(sx * s) as i32;
        let pc = (sy * s) as i32;
        let pd = (sy * c) as i32;
        let x = ox as i32 - (pa * dx as i32 + pb * dy as i32);
        let y = oy as i32 - (pc * dx as i32 + pd * dy as i32);
        for (k, v) in [pa, pb, pc, pd].iter().enumerate() {
            bus.write16(dst + 2 * k as u32, *v as u16);
        }
        bus.write32(dst + 8, x as u32);
        bus.write32(dst + 12, y as u32);
        src += 20;
        dst += 16;
    }
}

fn obj_affine(bus: &mut Bus, mut src: u32, mut dst: u32, count: u32, step: u32) {
    for _ in 0..count {
        let sx = bus.read16(src) as i16 as f64;
        let sy = bus.read16(src + 2) as i16 as f64;
        let (s, c) = sincos(bus.read16(src + 4) & 0xFF00);
        let v = [(sx * c) as i32, -(sx * s) as i32, (sy * s) as i32, (sy * c) as i32];
        for x in v {
            bus.write16(dst, x as u16);
            dst += step;
        }
        src += 8;
    }
}
