//! Fonctions d'affichage réécrites.

use super::Env;
use crate::gba::bus::Bus;

/// `Palette_Fade(niveau)` (0x080016C8) : recopie les 256 couleurs de fond (source
/// pointée par `0x03000004`) et les 256 couleurs de sprites (source pointée par
/// `0x03000000`) dans la palette matérielle, chaque composante multipliée par
/// `niveau / 32`. Comme l'original, aucune limite : au-delà de 32, une composante
/// déborde sur la voisine.
pub fn palette_fade(env: &mut Env) {
    let Env { cpu, bus, nat } = env;
    let _ = &nat;
    let level = cpu.r[0] & 0xFF;
    for (src_ptr, dst) in [(0x0300_0004u32, 0x0500_0000u32), (0x0300_0000, 0x0500_0200)] {
        let src = bus.read32(src_ptr);
        for k in 0..256u32 {
            let c = bus.read16(src + 2 * k);
            let b = (((c << 16) >> 26) & 0x1F) * level >> 5;
            let g = (((c << 16) >> 21) & 0x1F) * level >> 5;
            let r = (c & 0x1F) * level >> 5;
            bus.write16(dst + 2 * k, ((b << 10) + (g << 5) + r) as u16);
        }
    }
}

/// `Entity_OnScreen(entité)` (0x0802A2E0) : l'objet est-il dans l'écran ?
///
/// Position `+0x48/+0x4C` (8 bits de fraction), relative à la caméra `+0x1C` si elle
/// existe (`+0x18/+0x1C` = origine, `+0x28..+0x34` = bornes). Boîte `+0x5E/+0x60/+0x62/
/// +0x64`, doublée si l'objet est agrandi (bit 0x80 de `+0x58`) ou mise à l'échelle par
/// `+0x66/+0x68` (bit 0x100) quand l'échelle dépasse 2. Met le bit 0 de `+0x58` et
/// renvoie 1 si visible, sinon efface ce bit et renvoie 0.
pub fn entity_on_screen(env: &mut Env) {
    let Env { cpu, bus, nat } = env;
    let _ = &nat;
    let e = cpu.r[0];
    let ldsh = |bus: &mut Bus, a: u32| bus.read16(a) as u16 as i16 as i32;
    let cam = bus.read32(e + 0x1C);
    let mut x = bus.read32(e + 0x48) as i32;
    let mut y = bus.read32(e + 0x4C) as i32;
    if cam != 0 {
        x = x.wrapping_sub(bus.read32(cam + 0x18) as i32);
        y = y.wrapping_sub(bus.read32(cam + 0x1C) as i32);
    }
    let flags = bus.read16(e + 0x58) & 0xFFFE;
    bus.write16(e + 0x58, flags as u16);
    let mut right = ldsh(bus, e + 0x62);
    let mut left = ldsh(bus, e + 0x5E);
    let mut bottom = ldsh(bus, e + 0x64);
    let mut top = ldsh(bus, e + 0x60);
    if flags & 0x80 != 0 {
        right <<= 1;
        left <<= 1;
        bottom <<= 1;
        top <<= 1;
    } else if flags & 0x100 != 0 {
        let sx = ldsh(bus, e + 0x66);
        if sx > 0x200 {
            right = right.wrapping_mul(sx) >> 8;
            // l'original multiplie la valeur déjà mise à l'échelle (et non `left`)
            left = sx.wrapping_mul(right) >> 8;
        }
        let sy = ldsh(bus, e + 0x68);
        if sy > 0x200 {
            bottom = bottom.wrapping_mul(sy) >> 8;
            top = top.wrapping_mul(sy) >> 8;
        }
    }
    let (min_x, max_x, min_y, max_y) = if cam != 0 {
        (
            bus.read32(cam + 0x28) as i32,
            (bus.read32(cam + 0x2C) as i32).wrapping_add(0xF000),
            bus.read32(cam + 0x30) as i32,
            (bus.read32(cam + 0x34) as i32).wrapping_add(0xA000),
        )
    } else {
        (0, 0xF000, 0, 0xA000)
    };
    let visible = x.wrapping_add(right) >= min_x
        && x.wrapping_add(left) <= max_x
        && bottom.wrapping_add(y) >= min_y
        && y.wrapping_add(top) <= max_y;
    if visible {
        let v = bus.read16(e + 0x58) | 1;
        bus.write16(e + 0x58, v as u16);
    }
    cpu.r[0] = visible as u32;
}

/// Fonds matériels (0x38 octets chacun) et couches de décor (0x54 octets chacune).
const BGS: u32 = 0x0300_69B0;
const LAYERS: u32 = 0x0300_6B10;
const LAYER_COUNT: u32 = 0x0300_6C94;
/// Bit 0 : jeu figé (pause, chargement).
const FROZEN: u32 = 0x0300_6CA8;
const DISPSTAT: u32 = 0x0400_0004;

/// `Bg_Regs(fond)` (0x080283B0) : écrit le défilement (`+0x18/+0x1C`, 8 bits de
/// fraction) dans les registres pointés par `+0xC/+0x10`, ajoute ce défilement au point
/// de référence (`+0x2C/+0x30`) et, si `+0x14` pointe sur des registres affines, y écrit
/// la matrice (`+0x22..+0x28`) puis le point de référence (28 bits signés, en deux
/// demi-mots).
fn bg_regs(env: &mut Env, b: u32) {
    let sx = env.ldr(b + 0x18);
    let px = env.ldr(b + 0xC);
    env.strh(px, sx >> 8);
    let sy = env.ldr(b + 0x1C);
    let py = env.ldr(b + 0x10);
    env.strh(py, sy >> 8);
    let aff = env.ldr(b + 0x14);
    let rx = env.ldr(b + 0x2C).wrapping_add(sx);
    env.str(b + 0x2C, rx);
    let ry = env.ldr(b + 0x30).wrapping_add(sy);
    env.str(b + 0x30, ry);
    if aff != 0 {
        for k in 0..4 {
            let v = env.ldrh(b + 0x22 + 2 * k);
            env.strh(aff + 2 * k, v);
        }
        env.strh(aff + 8, rx);
        env.strh(aff + 10, ((rx as i32) >> 16) as u32 & 0xFFF);
        env.strh(aff + 12, ry);
        env.strh(aff + 14, ((ry as i32) >> 16) as u32 & 0xFFF);
    }
}

pub fn bg_regs_native(env: &mut Env) {
    let b = env.cpu.r[0];
    bg_regs(env, b);
}

/// `Bgs_Regs()` (0x0802D858) : `Bg_Regs` pour chaque couche.
pub fn bgs_regs(env: &mut Env) {
    let mut i = 0u32;
    while i < env.ldrh(LAYER_COUNT) {
        bg_regs(env, BGS + 0x38 * i);
        i = (i + 1) & 0xFFFF;
    }
}

/// `Camera_Move(dx, dy)` (0x0802CFA8) : déplace la couche principale (sauf jeu figé),
/// avec la poussée de l'image (`0x03006AE0`), bornée à ±8 pixels ; rappel de la couche
/// (`+0x14`), puis `Layer_Plain` ou `Layer_Scroll` selon le fond ; garde le
/// déplacement effectif (`0x030003E8/EC`) et remet la poussée à zéro.
pub fn camera_move(env: &mut Env) {
    let (dx, dy) = (env.cpu.r[0], env.cpu.r[1]);
    if env.ldr(FROZEN) & 1 != 0 || env.ldrh(LAYER_COUNT) == 0 {
        return;
    }
    let x = (dx.wrapping_add(env.ldsh(0x0300_6AE0) as u32) as i32).clamp(-0x800, 0x800) as u32;
    let y = (dy.wrapping_add(env.ldsh(0x0300_6AE2) as u32) as i32).clamp(-0x800, 0x800) as u32;
    let (ox, oy) = (env.ldr(LAYERS + 0x18), env.ldr(LAYERS + 0x1C));
    env.strh(LAYERS + 0x20, x);
    env.strh(LAYERS + 0x22, y);
    let cb = env.ldr(LAYERS + 0x14);
    if cb != 0 {
        env.call(cb, &[LAYERS, cb, x, y]);
    } else {
        env.cpu.r[2] = x;
        env.cpu.r[3] = y;
    }
    let fun = if env.ldrh(BGS + 0x34) & 1 != 0 { 0x0802_80B9 } else { 0x0802_7A51 };
    env.call(fun, &[BGS, LAYERS]);
    let v = env.ldr(LAYERS + 0x18).wrapping_sub(ox);
    env.str(0x0300_03E8, v);
    let v = env.ldr(LAYERS + 0x1C).wrapping_sub(oy);
    env.str(0x0300_03EC, v);
    env.strh(0x0300_6AE0, 0);
    env.strh(0x0300_6AE2, 0);
}

/// `Frame_Camera()` (0x080019E8) : si une cinématique tient la caméra (`0x030009A0`),
/// elle avance (`0x0802CEEC(cinématique)`) quand le bit 4 de `0x03001C88` est mis ; sinon `Camera_Move` avec la
/// vitesse demandée (`+0x20/+0x22` de la couche principale).
pub fn frame_camera(env: &mut Env) {
    let script = env.ldr(0x0300_09A0);
    if script != 0 {
        let flags = env.ldrh(0x0300_1C88);
        if flags & 4 != 0 {
            // l'original laisse la cinématique en r0 et les drapeaux en r1
            env.call(0x0802_CEED, &[script, flags]);
        }
        return;
    }
    let x = env.ldsh(LAYERS + 0x20) as u32;
    let y = env.ldsh(LAYERS + 0x22) as u32;
    env.call(0x0802_CFA9, &[x, y]);
}

/// `Sprites_Upload()` (0x0802C284, VBlank) : copie par DMA 0 les graphismes des sprites
/// en file d'attente (tampon `0x03001F54`, destination `+0x20`, taille `+0x74`), puis
/// l'OAM préparée (`0x03005650`, `0x03001E40` entrées). Les groupes de paramètres affines
/// au-delà des sprites utilisés sont cachés en gardant leur paramètre, et les entrées
/// utilisées à l'image précédente (`0x03004E40`) mais plus maintenant sont cachées.
pub fn sprites_upload(env: &mut Env) {
    const OAM: u32 = 0x0700_0000;
    const SHADOW: u32 = 0x0300_5650;
    const USED: u32 = 0x0300_1E40;
    const AFFINE: u32 = 0x0300_1F50;
    const PREV: u32 = 0x0300_4E40;
    let mut src = env.ldr(0x0300_1F54);
    let mut i = 0u32;
    while i < env.ldrh(0x0300_5B50) {
        let e = env.ldr(0x0300_67A0 + 4 * i);
        let dst = env.ldr(e + 0x20);
        let len = env.ldrh(e + 0x74);
        env.str(0x0400_00B0, src);
        env.str(0x0400_00B4, dst);
        env.str(0x0400_00B8, (len >> 2) | 0x8400_0000);
        env.ldr(0x0400_00B8);
        src = src.wrapping_add(len);
        i = (i + 1) & 0xFFFF;
    }
    if env.ldrh(USED) != 0 || env.ldrh(AFFINE) != 0 {
        env.str(0x0400_00B0, SHADOW);
        env.str(0x0400_00B4, OAM);
        let n = env.ldrh(USED);
        env.str(0x0400_00B8, (n << 2) | 0x8000_0000);
        env.ldr(0x0400_00B8);
        let groups = env.ldrh(AFFINE);
        let n = env.ldrh(USED);
        if (groups << 2) as i32 > n as i32 {
            let mut h = (n << 2) & 0xFFFF;
            while (h as i32) < (env.ldrh(AFFINE) << 4) as i32 {
                for k in (0..16).step_by(4) {
                    let a = OAM + 2 * (h + k);
                    env.strh(a, 0xA0);
                    env.strh(a + 2, 0xF0);
                    let p = env.ldrh(SHADOW + 2 * (h + k + 3));
                    env.strh(a + 6, p);
                }
                h = (h + 16) & 0xFFFF;
            }
        }
    }
    let a = env.ldrh(USED) << 2;
    let b = env.ldrh(AFFINE) << 4;
    let mut h = (if (b as i32) < (a as i32) { a } else { b }) & 0xFFFF;
    while (h as i32) < (env.ldrh(PREV) << 2) as i32 {
        let a = OAM + 2 * h;
        env.strh(a, 0xA0);
        env.strh(a + 2, 0xF0);
        env.strh(a + 6, 0);
        h = (h + 4) & 0xFFFF;
    }
    let n = env.ldrh(USED);
    env.strh(PREV, n);
    env.strh(0x0300_2FB4, 0);
}

/// Incrémente le compteur 16 bits `c` modulo `[m]` ; vrai quand il revient à 0.
fn tick(env: &mut Env, c: u32, m: u32) -> bool {
    let v = env.ldrh(c) + 1;
    env.strh(c, v);
    let (v, m) = (env.ldrh(c), env.ldrh(m));
    let r = if m == 0 { 0 } else { v % m };
    env.strh(c, r);
    r & 0xFFFF == 0
}

/// `Layers_VBlank()` (0x0802D2A4, interruption VBlank) : animations de tuiles des
/// couches (compteurs `0x03006AFC` et `+6` de chaque couche, `0x0802D240`), puis, si une
/// couche est un plan d'eau (`0x03006C70` différent de 0x100) : interruption de ligne
/// (VCOUNT) à la surface et gestionnaire de ligne choisi, couches affichées dans DISPCNT
/// (sauf celle de l'eau), ondulation verticale (`0x03006A90`) et position de l'eau.
pub fn layers_vblank(env: &mut Env) {
    if env.ldr(FROZEN) & 1 == 0 {
        if env.ldrh(0x0300_6C9C) != 0x100 {
            if tick(env, 0x0300_6AFC, 0x0300_6C90) {
                let v = env.ldrh(0x0300_6B00) << 8;
                env.strh(0x0300_6C74, v);
            } else {
                env.strh(0x0300_6C74, 0);
            }
        }
        let mut i = 0u32;
        while i < env.ldrh(LAYER_COUNT) {
            let a = 0x0300_6AA0 + 12 * i;
            if env.ldrh(a) != 0 {
                let l = LAYERS + 0x54 * i;
                let t = env.ldr(l + 4);
                if env.ldrh(t + 0x14) & 0x8000 == 0 && tick(env, a + 6, a + 2) {
                    env.call(0x0802_D241, &[l]);
                }
            }
            i = (i + 1) & 0xFFFF;
        }
    }
    if env.ldrh(0x0300_6C70) == 0x100 {
        return;
    }
    env.strh(0x0300_6AF0, 0x1040);
    let r0 = env.ldrh(0x0300_6AF8).wrapping_sub(env.ldr(LAYERS + 0x1C) >> 8);
    let line = r0.wrapping_add(0x67);
    env.strh(0x0300_6C78, line);
    let ds = env.ldrh(DISPSTAT);
    if r0.wrapping_add(0x66) & 0xFFFF <= 0x9E {
        env.str(0x0300_0918, 0x0802_D727);
        env.strh(DISPSTAT, (ds & 0xDF) | (line << 8) | 0x20);
    } else {
        env.str(0x0300_0918, 0x0802_DEF1);
        env.strh(DISPSTAT, ds & 0xDF);
    }
    let n = env.ldrh(LAYER_COUNT);
    let w = env.ldrh(0x0300_6C70);
    for k in 0..n {
        if k != w {
            let v = env.ldrh(0x0300_6AF0) | env.ldrh(0x0835_22E4 + 2 * k);
            env.strh(0x0300_6AF0, v);
        }
    }
    if env.ldr(FROZEN) & 1 == 0 && tick(env, 0x0300_6AD0, 0x0300_6AD4) {
        let v = (env.ldrh(0x0300_6C7C) << 8).wrapping_add(env.ldrh(0x0300_6A90));
        env.strh(0x0300_6A90, v);
    }
    let v = env.ldrh(0x0300_6AF0);
    env.strh(0x0400_0000, v);
    let b = BGS + 0x38 * env.ldrh(0x0300_6C70);
    let x = env.ldr(b + 0x18);
    let y = env
        .ldrh(0x0300_6A90)
        .wrapping_sub(env.ldrh(0x0300_6C78) << 8)
        .wrapping_add(0xA600);
    env.str(b + 0x18, x);
    env.str(b + 0x1C, y);
}
