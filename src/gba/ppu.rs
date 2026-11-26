//! Rendu ligne par ligne : modes 0 à 5, fonds texte et affines, sprites normaux et
//! affines, fenêtres (WIN0, WIN1, OBJ) et effets de couleur (transparence, éclaircir,
//! assombrir).

use super::bus::Bus;

pub const W: usize = 240;
pub const H: usize = 160;
const NONE: u16 = 0x8000;

const SIZES: [[(i32, i32); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

fn r16(m: &[u8], a: usize) -> u32 {
    u16::from_le_bytes([m[a], m[a + 1]]) as u32
}

fn color(pal: &[u8], idx: usize) -> u16 {
    (r16(pal, idx * 2) & 0x7FFF) as u16
}

pub fn render_line(bus: &mut Bus, y: usize, frame: &mut [u32]) {
    let io = &bus.io;
    let dispcnt = r16(io, 0);
    let row = &mut frame[y * W..(y + 1) * W];
    if dispcnt & 0x80 != 0 {
        row.fill(0xFF_FFFF);
        return;
    }
    let mode = dispcnt & 7;
    let mut bg = [[NONE; W]; 4];
    let mut on = [false; 4];
    for i in 0..4 {
        if dispcnt & (0x100 << i) == 0 || bus.bg_delay[i] != 0 {
            continue;
        }
        let ok = match (mode, i) {
            (0, _) | (1, 0 | 1) => {
                text(bus, i, y, &mut bg[i]);
                true
            }
            (1, 2) | (2, 2 | 3) => {
                affine(bus, i, &mut bg[i]);
                true
            }
            (3..=5, 2) => {
                bitmap(bus, mode, dispcnt, y, &mut bg[i]);
                true
            }
            _ => false,
        };
        on[i] = ok;
    }

    let mut obj = [NONE; W];
    let mut oprio = [4u8; W];
    let mut semi = [false; W];
    let mut objwin = [false; W];
    if dispcnt & 0x1000 != 0 {
        sprites(bus, dispcnt, y as i32, &mut obj, &mut oprio, &mut semi, &mut objwin);
    }

    let io = &bus.io;
    // fenêtres
    let mut mask = [0x3Fu8; W];
    if dispcnt & 0xE000 != 0 {
        let winin = r16(io, 0x48);
        let winout = r16(io, 0x4A);
        let inside = |n: usize, x: usize| {
            let h = r16(io, 0x40 + 2 * n);
            let v = r16(io, 0x44 + 2 * n);
            let (x1, x2) = ((h >> 8) as usize, ((h & 0xFF) as usize).min(W));
            let (y1, y2) = ((v >> 8) as usize, ((v & 0xFF) as usize).min(H));
            let ix = if x1 <= x2 { x >= x1 && x < x2 } else { x >= x1 || x < x2 };
            let iy = if y1 <= y2 { y >= y1 && y < y2 } else { y >= y1 || y < y2 };
            ix && iy
        };
        for x in 0..W {
            let mut m = (winout & 0x3F) as u8;
            if dispcnt & 0x8000 != 0 && objwin[x] {
                m = ((winout >> 8) & 0x3F) as u8;
            }
            if dispcnt & 0x4000 != 0 && inside(1, x) {
                m = ((winin >> 8) & 0x3F) as u8;
            }
            if dispcnt & 0x2000 != 0 && inside(0, x) {
                m = (winin & 0x3F) as u8;
            }
            mask[x] = m;
        }
    }

    // ordre des fonds : priorité puis numéro
    let mut order: Vec<usize> = (0..4).filter(|&i| on[i]).collect();
    order.sort_by_key(|&i| (r16(io, 8 + 2 * i) & 3, i));
    let prio: [u32; 4] = std::array::from_fn(|i| r16(io, 8 + 2 * i) & 3);

    let bldcnt = r16(io, 0x50);
    let bmode = (bldcnt >> 6) & 3;
    let alpha = r16(io, 0x52);
    let eva = (alpha & 31).min(16);
    let evb = ((alpha >> 8) & 31).min(16);
    let ey = (r16(io, 0x54) & 31).min(16);
    let backdrop = color(&bus.pal, 0);

    for x in 0..W {
        let m = mask[x];
        let mut layers = [(5u32, backdrop); 2];
        let mut found = 0;
        'p: for p in 0..4u32 {
            if obj[x] != NONE && oprio[x] as u32 == p && m & 0x10 != 0 {
                layers[found] = (4, obj[x]);
                found += 1;
                if found == 2 {
                    break 'p;
                }
            }
            for &i in &order {
                if prio[i] == p && m & (1 << i) != 0 && bg[i][x] != NONE {
                    layers[found] = (i as u32, bg[i][x]);
                    found += 1;
                    if found == 2 {
                        break 'p;
                    }
                }
            }
        }
        let (top, second) = (layers[0], layers[1]);
        let first_ok = bldcnt & (1 << top.0) != 0;
        let second_ok = (bldcnt >> 8) & (1 << second.0) != 0;
        let mut c = top.1;
        if top.0 == 4 && semi[x] && second_ok {
            c = blend(top.1, second.1, eva, evb);
        } else if m & 0x20 != 0 && first_ok {
            c = match bmode {
                1 if second_ok => blend(top.1, second.1, eva, evb),
                2 => fade(top.1, ey, true),
                3 => fade(top.1, ey, false),
                _ => top.1,
            };
        }
        row[x] = rgb(c);
    }

    // avance des points de référence affines
    for k in 0..2 {
        let pb = r16(io, 0x22 + 0x10 * k) as i16 as i32;
        let pd = r16(io, 0x26 + 0x10 * k) as i16 as i32;
        bus.bg_ref[k][0] += pb;
        bus.bg_ref[k][1] += pd;
    }
}

fn rgb(c: u16) -> u32 {
    let ch = |v: u16| {
        let v = (v & 31) as u32;
        (v << 3) | (v >> 2)
    };
    (ch(c) << 16) | (ch(c >> 5) << 8) | ch(c >> 10)
}

fn blend(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let mut out = 0;
    for s in [0, 5, 10] {
        let ca = ((a >> s) & 31) as u32;
        let cb = ((b >> s) & 31) as u32;
        out |= (((ca * eva + cb * evb) >> 4).min(31) as u16) << s;
    }
    out
}

fn fade(a: u16, ey: u32, up: bool) -> u16 {
    let mut out = 0;
    for s in [0, 5, 10] {
        let c = ((a >> s) & 31) as u32;
        let v = if up { c + (((31 - c) * ey) >> 4) } else { c - ((c * ey) >> 4) };
        out |= (v as u16) << s;
    }
    out
}

fn text(bus: &Bus, i: usize, y: usize, out: &mut [u16; W]) {
    let io = &bus.io;
    let cnt = r16(io, 8 + 2 * i);
    let hofs = (r16(io, 0x10 + 4 * i) & 0x1FF) as usize;
    let vofs = (r16(io, 0x12 + 4 * i) & 0x1FF) as usize;
    let cb = ((cnt >> 2) & 3) as usize * 0x4000;
    let sb = ((cnt >> 8) & 31) as usize * 0x800;
    let bpp8 = cnt & 0x80 != 0;
    let size = cnt >> 14;
    let w = if size & 1 != 0 { 512 } else { 256 };
    let h = if size & 2 != 0 { 512 } else { 256 };
    let yy = (y + vofs) % h;
    let vram = &bus.vram;
    let pal = &bus.pal;
    for (x, px_out) in out.iter_mut().enumerate() {
        let xx = (x + hofs) % w;
        let block = xx / 256 + (yy / 256) * (w / 256);
        let se_addr = sb + block * 0x800 + ((yy % 256) / 8) * 64 + ((xx % 256) / 8) * 2;
        if se_addr + 1 >= vram.len() {
            continue;
        }
        let se = r16(vram, se_addr);
        let tile = (se & 0x3FF) as usize;
        let mut px = xx % 8;
        let mut py = yy % 8;
        if se & 0x400 != 0 {
            px = 7 - px;
        }
        if se & 0x800 != 0 {
            py = 7 - py;
        }
        if bpp8 {
            let a = cb + tile * 64 + py * 8 + px;
            if a >= 0x1_0000 {
                continue;
            }
            let idx = vram[a] as usize;
            if idx != 0 {
                *px_out = color(pal, idx);
            }
        } else {
            let a = cb + tile * 32 + py * 4 + px / 2;
            if a >= 0x1_0000 {
                continue;
            }
            let b = vram[a];
            let idx = if px & 1 != 0 { b >> 4 } else { b & 15 } as usize;
            if idx != 0 {
                *px_out = color(pal, ((se >> 12) as usize) * 16 + idx);
            }
        }
    }
}

fn affine(bus: &Bus, i: usize, out: &mut [u16; W]) {
    let io = &bus.io;
    let k = i - 2;
    let cnt = r16(io, 8 + 2 * i);
    let pa = r16(io, 0x20 + 0x10 * k) as i16 as i32;
    let pc = r16(io, 0x24 + 0x10 * k) as i16 as i32;
    let size = 128i32 << (cnt >> 14);
    let wrap = cnt & 0x2000 != 0;
    let cb = ((cnt >> 2) & 3) as usize * 0x4000;
    let sb = ((cnt >> 8) & 31) as usize * 0x800;
    let [mut fx, mut fy] = bus.bg_ref[k];
    for px_out in out.iter_mut() {
        let (mut tx, mut ty) = (fx >> 8, fy >> 8);
        fx += pa;
        fy += pc;
        if tx < 0 || ty < 0 || tx >= size || ty >= size {
            if !wrap {
                continue;
            }
            tx = tx.rem_euclid(size);
            ty = ty.rem_euclid(size);
        }
        let ma = sb + (ty as usize / 8) * (size as usize / 8) + tx as usize / 8;
        let Some(&tile) = bus.vram.get(ma) else { continue };
        let a = cb + tile as usize * 64 + (ty as usize % 8) * 8 + tx as usize % 8;
        let idx = *bus.vram.get(a).unwrap_or(&0) as usize;
        if idx != 0 {
            *px_out = color(&bus.pal, idx);
        }
    }
}

fn bitmap(bus: &Bus, mode: u32, dispcnt: u32, y: usize, out: &mut [u16; W]) {
    let page = if dispcnt & 0x10 != 0 { 0xA000 } else { 0 };
    for (x, p) in out.iter_mut().enumerate() {
        *p = match mode {
            3 => (r16(&bus.vram, (y * W + x) * 2) & 0x7FFF) as u16,
            4 => {
                let idx = bus.vram[page + y * W + x] as usize;
                if idx == 0 {
                    continue;
                }
                color(&bus.pal, idx)
            }
            _ => {
                if x >= 160 || y >= 128 {
                    continue;
                }
                (r16(&bus.vram, page + (y * 160 + x) * 2) & 0x7FFF) as u16
            }
        };
    }
}

#[allow(clippy::too_many_arguments)]
fn sprites(
    bus: &Bus,
    dispcnt: u32,
    y: i32,
    obj: &mut [u16; W],
    oprio: &mut [u8; W],
    semi: &mut [bool; W],
    objwin: &mut [bool; W],
) {
    let oam = &bus.oam;
    let vram = &bus.vram;
    let one_d = dispcnt & 0x40 != 0;
    let bitmap_mode = dispcnt & 7 >= 3;
    for n in 0..128 {
        let a0 = r16(oam, n * 8);
        let a1 = r16(oam, n * 8 + 2);
        let a2 = r16(oam, n * 8 + 4);
        let aff = a0 & 0x100 != 0;
        if !aff && a0 & 0x200 != 0 {
            continue;
        }
        let shape = (a0 >> 14) as usize;
        let omode = (a0 >> 10) & 3;
        if shape == 3 || omode == 3 {
            continue;
        }
        let (w, h) = SIZES[shape][(a1 >> 14) as usize];
        let (bw, bh) = if aff && a0 & 0x200 != 0 { (w * 2, h * 2) } else { (w, h) };
        let dy = (y - (a0 & 0xFF) as i32).rem_euclid(256);
        if dy >= bh {
            continue;
        }
        let mut sx = (a1 & 0x1FF) as i32;
        if sx >= 256 {
            sx -= 512;
        }
        let tile = (a2 & 0x3FF) as i32;
        if bitmap_mode && tile < 512 {
            continue;
        }
        let prio = ((a2 >> 10) & 3) as u8;
        let palb = (a2 >> 12) as usize;
        let bpp8 = a0 & 0x2000 != 0;
        let mat = if aff {
            let p = ((a1 >> 9) & 31) as usize * 32;
            [6, 14, 22, 30].map(|o| r16(oam, p + o) as i16 as i32)
        } else {
            [0; 4]
        };
        for px in 0..bw {
            let x = sx + px;
            if !(0..W as i32).contains(&x) {
                continue;
            }
            let (tx, ty) = if aff {
                let (rx, ry) = (px - bw / 2, dy - bh / 2);
                let tx = ((mat[0] * rx + mat[1] * ry) >> 8) + w / 2;
                let ty = ((mat[2] * rx + mat[3] * ry) >> 8) + h / 2;
                if tx < 0 || ty < 0 || tx >= w || ty >= h {
                    continue;
                }
                (tx, ty)
            } else {
                let tx = if a1 & 0x1000 != 0 { w - 1 - px } else { px };
                let ty = if a1 & 0x2000 != 0 { h - 1 - dy } else { dy };
                (tx, ty)
            };
            let (cx, cy) = (tx / 8, ty / 8);
            let idx;
            let c;
            if bpp8 {
                let t = if one_d { tile + (cy * (w / 8) + cx) * 2 } else { (tile & !1) + cy * 32 + cx * 2 };
                let a = 0x1_0000 + (t as usize & 0x3FF) * 32 + (ty % 8) as usize * 8 + (tx % 8) as usize;
                idx = vram[a.min(vram.len() - 1)] as usize;
                c = 256 + idx;
            } else {
                let t = if one_d { tile + cy * (w / 8) + cx } else { tile + cy * 32 + cx };
                let a = 0x1_0000 + (t as usize & 0x3FF) * 32 + (ty % 8) as usize * 4 + (tx % 8) as usize / 2;
                let b = vram[a.min(vram.len() - 1)];
                idx = if tx & 1 != 0 { b >> 4 } else { b & 15 } as usize;
                c = 256 + palb * 16 + idx;
            }
            if idx == 0 {
                continue;
            }
            let x = x as usize;
            if omode == 2 {
                objwin[x] = true;
                continue;
            }
            if obj[x] == NONE || prio < oprio[x] {
                obj[x] = color(&bus.pal, c);
                oprio[x] = prio;
                semi[x] = omode == 1;
            }
        }
    }
}
