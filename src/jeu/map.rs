//! Défilement des décors : écriture des colonnes / lignes de tuiles qui entrent
//! dans l'écran, à partir des lignes compressées du niveau.
//!
//! Couche (`L`) : `+0` en-tête de carte (`+0x12` hauteur en lignes, `+0x14` tuile vide),
//! `+8` table des lignes (un pointeur par ligne, 0 = ligne vide), `+0x38` drapeaux
//! (bit 1 = carte qui boucle verticalement), `+0x3A` première tuile en VRAM.
//! Une ligne est une suite de blocs : en-tête `n | 0x8000` suivi de `n` tuiles, ou
//! `n` seul pour `n` tuiles vides.

use super::Env;

/// Halfword d'un emplacement de la carte matérielle 32×32 (`dst` = tampon de la carte).
fn map_slot(env: &mut Env, dst_ptr: u32, row32: u32, col: u32) -> u32 {
    let base = env.ldr(dst_ptr);
    base.wrapping_add((row32 & 0x3FF) * 2).wrapping_add(col * 2)
}

/// `Map_DrawColumn(fond, couche, x, y, x_carte, y_carte)` (0x08027498) : écrit dans la
/// carte matérielle (colonne `x`, à partir de la ligne `y`, en 1/2048 de tuile) la
/// colonne `x_carte` du niveau, sur 22 lignes à partir de la ligne `y_carte - 1`.
pub fn draw_column(env: &mut Env) {
    let dst = env.cpu.r[0];
    let layer = env.cpu.r[1];
    let sp = env.cpu.r[13];
    let src_x = env.ldr(sp) >> 11;
    let src_y = env.ldr(sp + 4) >> 11;
    let map = env.ldr(layer);
    let empty = (0xFFFF + env.ldrh(map + 0x14) + env.ldrh(layer + 0x3A)) & 0xFFFF;
    let used = env.ldrh(0x0300_2FB4);
    env.strh(0x0300_2FB4, used + 0x100);
    let col = (env.cpu.r[2] >> 11) & 0x1F;
    let row0 = (env.cpu.r[3] >> 11) & 0x1F;

    let (mut row, mut line) = if src_y != 0 {
        ((row0.wrapping_sub(1)) & 0x1F, src_y - 1)
    } else if env.ldrh(layer + 0x38) & 2 != 0 {
        ((row0.wrapping_sub(1)) & 0x1F, env.ldrh(map + 0x12).wrapping_sub(1))
    } else {
        (row0, 0)
    };
    let end_wanted = line.wrapping_add(0x16);
    let map = env.ldr(layer);
    let height = env.ldrh(map + 0x12);
    let end = if end_wanted < height || env.ldrh(layer + 0x38) & 2 != 0 { end_wanted } else { height };
    row <<= 5;
    let rows = env.ldr(layer + 8);
    let mut p = rows.wrapping_add(line.wrapping_mul(4));

    while line < end {
        let mut s = env.ldr(p);
        p = p.wrapping_add(4);
        line = line.wrapping_add(1);
        if s == 0 {
            let a = map_slot(env, dst, row, col);
            env.strh(a, empty);
        } else {
            // cherche le bloc qui contient la colonne `src_x`
            let mut x = 0u32;
            let v = env.ldrh(s);
            let mut n = v & 0x7FFF;
            let mut literal = v & 0x8000 != 0;
            s = s.wrapping_add(2);
            if src_x >= n {
                loop {
                    if literal {
                        s = s.wrapping_add(n * 2);
                    }
                    x = (x + n) & 0xFFFF;
                    let v = env.ldrh(s);
                    n = v & 0x7FFF;
                    literal = v & 0x8000 != 0;
                    s = s.wrapping_add(2);
                    if src_x < x || src_x >= x + n {
                        continue;
                    }
                    break;
                }
            }
            let a = map_slot(env, dst, row, col);
            if literal {
                let t = env.ldrh(s.wrapping_add(src_x.wrapping_sub(x).wrapping_mul(2)));
                let first = env.ldrh(layer + 0x3A);
                env.strh(a, first + t);
            } else {
                env.strh(a, empty);
            }
        }
        let limit = rows.wrapping_add(env.ldrh(map + 0x12) * 4);
        if p >= limit {
            p = rows;
        }
        row = row.wrapping_add(0x20);
    }
}

const DRAW_COLUMN: u32 = 0x0802_7499;
const DRAW_ROW: u32 = 0x0802_786D;
const MODULO: u32 = 0x0803_5D41;
const SIN: u32 = 0x0802_D939;
const COS: u32 = 0x0802_DE01;
const DIV64: u32 = 0x0803_5841;
const MUL64: u32 = 0x0803_5E11;
const NEG64: u32 = 0x0803_5E81;
const UPDATE_OBJECTS: u32 = 0x0802_9565;
const SCROLLING: u32 = 0x0300_1E18;

/// Appel d'une fonction du jeu sur des entiers de 64 bits (r0:r1, r2:r3).
fn call64(env: &mut Env, f: u32, a: i64, b: i64) -> i64 {
    let lo = env.call(f, &[a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32]);
    ((env.cpu.r[1] as u64) << 32 | lo as u64) as i64
}
fn call64_1(env: &mut Env, f: u32, a: i64) -> i64 {
    let lo = env.call(f, &[a as u32, (a >> 32) as u32]);
    ((env.cpu.r[1] as u64) << 32 | lo as u64) as i64
}

/// Ramène une position dans les limites de la carte (ou la fait boucler).
/// `size` = taille de la carte, `view` = taille de l'écran, en tuiles.
fn clamp_axis(pos: i32, size: u32, view: u32, wrap: bool) -> i32 {
    let full = (size << 11) as i32;
    let max = (size.wrapping_sub(view) << 11) as i32;
    if pos < 0 {
        if wrap { pos.wrapping_add(full) } else { 0 }
    } else if pos > max {
        if !wrap {
            max
        } else if pos > full {
            pos.wrapping_sub(full)
        } else {
            pos
        }
    } else {
        pos
    }
}

/// `Layer_Scroll(fond, couche)` (0x08027A50) : applique le déplacement demandé à une
/// couche de décor (`+0x18/+0x1C` position, `+0x20/+0x22` déplacement), dessine la
/// colonne ou la ligne qui apparaît, met à jour la rotation / le zoom éventuels et
/// les registres de défilement du fond (`+0x18/+0x1C`).
pub fn layer_scroll(env: &mut Env) {
    let bg = env.cpu.r[0];
    let layer = env.cpu.r[1];
    env.strh(SCROLLING, 1);

    let map = env.ldr(layer);
    let w = env.ldrh(map + 0x10);
    let flags = env.ldrh(layer + 0x38);
    let sx = (env.ldr(layer + 0x18) as i32).wrapping_add(env.ldsh(layer + 0x20));
    let sx = clamp_axis(sx, w, 0x1E, flags & 1 != 0);
    let dx = if flags & 1 != 0 {
        env.ldrh(layer + 0x20)
    } else {
        (sx as u32).wrapping_sub(env.ldr(layer + 0x18)) & 0xFFFF
    } as u16 as i16 as i32;
    let old_x = env.ldr(bg + 0x18);
    let mut hx = old_x.wrapping_add(dx as u32) & 0xFFFF;
    if sx as u32 & 0x7FF != hx & 0x7FF {
        hx = (hx & 0xFFFF_F800) | (sx as u32 & 0x7FF);
    }

    let old_ly = env.ldr(layer + 0x1C);
    let sy = env.ldsh(layer + 0x22).wrapping_add(old_ly as i32);
    let map = env.ldr(layer);
    let h = env.ldrh(map + 0x12);
    let sy = clamp_axis(sy, h, 0x14, flags & 2 != 0);
    let dy = if env.ldrh(layer + 0x38) & 2 != 0 {
        env.ldrh(layer + 0x22)
    } else {
        (sy as u32).wrapping_sub(old_ly) & 0xFFFF
    } as u16 as i16 as i32;
    let mut hy = env.ldr(bg + 0x1C).wrapping_add(dy as u32) & 0xFFFF;
    if sy as u32 & 0x7FF != hy & 0x7FF {
        hy = (hy & 0xFFFF_F800) | (sy as u32 & 0x7FF);
    }

    // nouvelle colonne
    if hx >> 11 != old_x >> 11 {
        if dx > 0 {
            let mut x = (sx as u32).wrapping_add(0xF000);
            let full = env.ldrh(map + 0x10) << 11;
            if x as i32 > full as i32 {
                x = env.call(MODULO, &[x, full]);
            }
            env.call(DRAW_COLUMN, &[bg, layer, (hx + 0xF000) & 0xFFFF, hy, x, sy as u32]);
        } else if dx < 0 {
            env.call(DRAW_COLUMN, &[bg, layer, hx, hy, sx as u32, sy as u32]);
        }
    }
    // nouvelle ligne
    if env.ldr(bg + 0x1C) >> 11 != hy >> 11 {
        if dy > 0 {
            let map = env.ldr(layer);
            let full = env.ldrh(map + 0x12) << 11;
            let y = env.call(MODULO, &[(sy as u32).wrapping_add(0xA000), full]);
            env.call(DRAW_ROW, &[bg, layer, hx, hy + 0xA000, sx as u32, y]);
        } else if dy < 0 {
            env.call(DRAW_ROW, &[bg, layer, hx, hy, sx as u32, sy as u32]);
        }
    }

    // rotation (bit 2) et zoom (bit 3)
    let flags = env.ldrh(layer + 0x38);
    if flags & 0xC != 0 && env.ldr(bg + 0x14) != 0 {
        let (mut sin, mut cos, mut cx, mut cy) = (0x100i64, 0i64, 0u32, 0u32);
        let (mut zx, mut zy) = (0x100i64, 0x100i64);
        if flags & 4 != 0 {
            let a = env.ldsh(layer + 0x48);
            sin = env.call(SIN, &[a as u32]) as u16 as i16 as i64;
            let a = env.ldsh(layer + 0x48);
            cos = env.call(COS, &[a as u32]) as u16 as i16 as i64;
            cx = env.ldr(layer + 0x40);
            cy = env.ldr(layer + 0x44);
        }
        if env.ldrh(layer + 0x38) & 8 != 0 {
            zx = env.ldsh(layer + 0x4C) as i64;
            zy = env.ldsh(layer + 0x4E) as i64;
        }
        if zx == 0 {
            zx = 1;
        }
        if zy == 0 {
            zy = 1;
        }
        if env.ldr(bg + 0x14) != 0 {
            let ix = call64(env, DIV64, 0x10000, zx);
            let v = call64(env, MUL64, ix, sin);
            env.strh(bg + 0x22, (v >> 8) as u32);
            let v = call64(env, MUL64, ix, cos);
            env.strh(bg + 0x24, (v >> 8) as u32);
            let iy = call64(env, DIV64, 0x10000, zy);
            let v = call64(env, MUL64, iy, cos);
            let v = call64_1(env, NEG64, v);
            env.strh(bg + 0x26, (v >> 8) as u32);
            let v = call64(env, MUL64, iy, sin);
            env.strh(bg + 0x28, (v >> 8) as u32);
            env.str(bg + 0x2C, cx);
            env.str(bg + 0x30, cy);
        }
    }

    env.str(bg + 0x18, hx);
    env.str(bg + 0x1C, hy);
    env.call(UPDATE_OBJECTS, &[layer, sx as u32, sy as u32]);
    env.strh(SCROLLING, 0);
}

const TILE_FROM_SCREEN: u32 = 0x0802_85C9;
const MAIN_LAYER: u32 = 0x0300_6B10;
const CACHE_KEY: u32 = 0x0300_03E0;
const CACHE_TILE: u32 = 0x0300_03DC;
const LAST_TILE: u32 = 0x0300_03E4;

/// `Map_TileAt(couche, x, y)` (0x080293E8) : attribut de collision de la tuile en
/// (`x`, `y`) (en 1/2048 de tuile), 0 hors de la carte.
///
/// Tuile : bits 0-9 = numéro, bit 11 = miroir horizontal, bit 10 = vertical ; l'attribut
/// est l'octet `[couche+0xC][numéro]`, combiné (xor) aux bits de miroir (1 = horizontal,
/// 2 = vertical). Dernière tuile lue en cache (`0x030003E0` clé, `0x030003DC` tuile).
/// Pour la couche principale qui boucle, la tuile est lue dans la carte matérielle si elle
/// est à l'écran ; sinon la ligne compressée est parcourue.
pub fn tile_at(env: &mut Env) {
    let layer = env.cpu.r[0];
    let col = env.cpu.r[1] >> 11;
    let row = env.cpu.r[2] >> 11;
    let map = env.ldr(layer);
    if col >= env.ldrh(map + 0x10) || row >= env.ldrh(map + 0x12) {
        env.cpu.r[0] = 0;
        return;
    }
    let mut c = col & 0xFFFF;
    let r = row & 0xFFFF;
    let rows = env.ldr(layer + 8);
    let mut p = env.ldr(rows.wrapping_add(r * 4));
    let empty = env.ldrh(map + 0x14).wrapping_sub(1) & 0xFFFF;

    let tile = 'found: {
        if p == 0 {
            break 'found empty;
        }
        let key = (c << 16) | r;
        if env.ldr(CACHE_KEY) == key {
            break 'found env.ldrh(CACHE_TILE);
        }
        env.str(CACHE_KEY, key);
        if env.ldrh(SCROLLING) == 0 && layer == MAIN_LAYER {
            let sx = env.ldr(layer + 0x18) >> 11;
            let (x0, x1) = (sx & 0xFFFF, (sx + 0x1E) & 0xFFFF);
            let sy = env.ldr(layer + 0x1C) >> 11;
            let (y0, y1) = (sy & 0xFFFF, (sy + 0x14) & 0xFFFF);
            if env.ldrh(layer + 0x38) & 3 != 0 && c > x0 && c < x1 && r > y0 && r < y1 {
                let t = env.call(TILE_FROM_SCREEN, &[layer, (c - x0) & 0xFFFF, (r - y0) & 0xFFFF]);
                env.strh(CACHE_TILE, t);
                break 'found t;
            }
        }
        // parcours de la ligne compressée
        loop {
            let h = env.ldrh(p);
            let n = h & 0x7FFF;
            if h & 0x8000 != 0 {
                if n > c {
                    let t = env.ldrh(p + c * 2 + 2);
                    env.strh(CACHE_TILE, t);
                    break 'found t;
                }
                c = (c - n) & 0xFFFF;
                p = p.wrapping_add(n * 2 + 2);
            } else {
                if n > c {
                    env.strh(CACHE_TILE, empty);
                    break 'found empty;
                }
                c = (c - n) & 0xFFFF;
                p = p.wrapping_add(2);
            }
        }
    };

    env.strh(LAST_TILE, tile);
    let flip = ((tile & 0x800 != 0) as u32) | (((tile & 0x400 != 0) as u32) << 1);
    env.strh(LAST_TILE, tile & 0x3FF);
    let n = env.ldrh(LAST_TILE);
    let attrs = env.ldr(layer + 0xC);
    env.cpu.r[0] = (env.ldrb(attrs.wrapping_add(n)) ^ flip) & 0xFF;
}

const TILE_AT: u32 = 0x0802_93E9;
const PIXEL_MASKS: u32 = 0x0300_1E24;

/// `Map_Collide(couche, x, y)` (0x0802956C) : collision au pixel près en (`x`, `y`)
/// (8 bits de fraction). Les attributs 0-3 sont vides ; sinon le masque 8×8 de la
/// tuile (`[0x03001E24] + 8 × attribut`) dit si le pixel est plein, et la valeur rendue
/// est la nature du sol (4 bits dans `[couche+0xC]`, après les attributs) + 1.
pub fn collide(env: &mut Env) {
    let layer = env.cpu.r[0];
    let px = (env.cpu.r[1] >> 8) & 7;
    let py = (env.cpu.r[2] >> 8) & 7;
    let (x, y) = (env.cpu.r[1], env.cpu.r[2]);
    let a = env.call(TILE_AT, &[layer, x, y]) & 0xFF;
    let row = a << 3;
    env.cpu.r[0] = 'r: {
        if row <= 0x1F {
            break 'r 0;
        }
        let masks = env.ldr(PIXEL_MASKS);
        if env.ldrb(masks.wrapping_add(row + py)) & (0x80 >> px) == 0 {
            break 'r 0;
        }
        let t = env.ldrh(LAST_TILE);
        let map = env.ldr(layer);
        let attrs = env.ldr(layer + 0xC);
        let base = env.ldrh(map + 0x14);
        let b = env.ldrb(attrs.wrapping_add((t >> 1) + base));
        let kind = if t & 1 != 0 { b >> 4 } else { b & 0xF };
        (kind + 1) & 0xFF
    };
}

const LAYERS: u32 = 0x0300_6B10; // couches de décor, 0x54 octets chacune (0 = principale)
const BGS: u32 = 0x0300_69B0; // fonds matériels associés, 0x38 octets chacun
const LAYER_COUNT: u32 = 0x0300_6C94;
const LAYER_FIXED: u32 = 0x0300_6C80; // décalage permanent (x, y) par couche
const LAYER_PUSH: u32 = 0x0300_6AE0; // décalage de l'image (x, y), remis à 0
const SCROLL_DX: u32 = 0x0300_03E8;
const SCROLL_DY: u32 = 0x0300_03EC;
const LAYER_SCROLL_FN: u32 = 0x0802_7A51;
const LAYER_PLAIN_FN: u32 = 0x0802_80B9;

/// `Layers_Parallax()` (0x0802D074) : déplace les couches secondaires (une image sur
/// deux). Déplacement = celui de la couche principale × (taille de la couche / taille
/// de la principale), plus les décalages ; borné à ±1 tuile. La couche `0x03006C70`
/// suit la verticale imposée `0x03006C74`, la couche `0x03006C9C` ne bouge pas
/// verticalement ; les autres passent par leur fonction propre puis `Layer_Scroll`
/// (ou la version simple si le bit 0 de `fond+0x34` est mis).
pub fn layers_parallax(env: &mut Env) {
    use super::math::div;
    if env.ldr(0x0300_6CA8) & 1 != 0 {
        return;
    }
    let mut k = 1u32;
    while k < env.ldrh(LAYER_COUNT) {
        let l = LAYERS + 0x54 * k;
        let bg = BGS + 0x38 * k;
        let w0 = env.ldsh(LAYERS + 0x24);
        let mut dx = if w0 != 0 {
            let m = (env.ldr(SCROLL_DX) as i32).wrapping_mul(env.ldsh(l + 0x24));
            div(m, w0)
        } else {
            0
        };
        dx = dx.wrapping_add(env.ldsh(LAYER_FIXED + 4 * k).wrapping_add(env.ldsh(LAYER_PUSH + 4 * k)));
        let h0 = env.ldsh(LAYERS + 0x26);
        let mut dy = if h0 != 0 {
            let m = (env.ldr(SCROLL_DY) as i32).wrapping_mul(env.ldsh(l + 0x26));
            div(m, h0)
        } else {
            0
        };
        dy = dy.wrapping_add(env.ldsh(LAYER_FIXED + 2 + 4 * k).wrapping_add(env.ldsh(LAYER_PUSH + 2 + 4 * k)));

        let follow = env.ldrh(0x0300_6C70) == k;
        let fixed_y = env.ldrh(0x0300_6C9C) == k;
        if follow || fixed_y {
            env.strh(l + 0x20, dx as u32);
            if fixed_y {
                let y = env.ldrh(0x0300_6C74);
                env.strh(l + 0x22, y);
            } else {
                env.strh(l + 0x22, 0);
            }
            env.call(LAYER_PLAIN_FN, &[bg, l]);
        } else {
            let dx = dx.clamp(-0x800, 0x800);
            let dy = dy.clamp(-0x800, 0x800);
            env.strh(l + 0x20, dx as u32);
            env.strh(l + 0x22, dy as u32);
            let f = env.ldr(l + 0x14);
            if f != 0 {
                env.call(f, &[l]);
            }
            let fun = if env.ldrh(bg + 0x34) & 1 != 0 { LAYER_PLAIN_FN } else { LAYER_SCROLL_FN };
            env.call(fun, &[bg, l]);
        }
        env.strh(LAYER_PUSH + 4 * k, 0);
        env.strh(LAYER_PUSH + 2 + 4 * k, 0);
        k = (k + 1) & 0xFFFF;
    }
}
