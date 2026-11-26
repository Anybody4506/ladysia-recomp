//! Objets du jeu (entités) réécrits.
//!
//! Structure d'une entité (offsets utiles ici) :
//! - `+0x00` type (pointeur ; `+8` taille des graphismes, `+0xA` identifiant)
//! - `+0x04` animation (pointeur : `+0` tableau d'images de 20 octets, `+4` nombre d'images)
//! - `+0x1C` caméra (ou 0), `+0x20` emplacement en VRAM, `+0x28` comportement (fonction)
//! - `+0x48/+0x4C` position (8 bits de fraction), `+0x50` décalage dans les graphismes
//! - `+0x54` priorité, `+0x56` profondeur, `+0x58` drapeaux, `+0x5A/+0x5C` vitesse
//! - `+0x5E..+0x64` boîte à l'écran, `+0x6A` image courante, `+0x6B` durée restante
//! - `+0x72` palette imposée, `+0x74` taille des graphismes décompressés

use super::Env;

/// Mod « course rapide » : supplément de déplacement horizontal du joueur, en % (0 = jeu
/// d'origine). Réglé par `mods::appliquer`.
pub static COURSE_PLUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

const ENTITY_ON_SCREEN: u32 = 0x0802_A2E1;
const ENTITY_HIDE: u32 = 0x0802_A50D;
const VRAM_FREE: u32 = 0x0802_A695;
const VRAM_ALLOC: u32 = 0x0802_A581;
const MODULO: u32 = 0x0803_62E9;
const PALETTE_OF: u32 = 0x0802_C825;
const DRAW_AFFINE: u32 = 0x0802_BD69;
const DECOMPRESS: u32 = 0x0802_F0B1;

const FRAME_COUNTER: u32 = 0x0300_6CA8; // bit 0 : image impaire (mise à jour une image sur deux)
const TILES_USED: u32 = 0x0300_2FB4; // tuiles de sprites envoyées cette image
const OAM_COUNT: u32 = 0x0300_1E40; // nombre de sprites dans la copie de l'OAM
const OAM_BUFFER: u32 = 0x0300_5650;
const UPLOAD_COUNT: u32 = 0x0300_5B50;
const UPLOAD_LIST: u32 = 0x0300_67A0;
const VRAM_CURSOR: u32 = 0x0300_5A5C;

/// Forme et taille d'un sprite matériel pour une pièce de `w`×`h` pixels
/// (bits 14-15 de l'attribut 0 et de l'attribut 1). Les dimensions inconnues tombent
/// sur la même valeur que dans l'original.
fn obj_shape(w: u32, h: u32) -> (u32, u32) {
    match (w, h) {
        (8, 16) => (0x8000, 0),
        (8, 32) => (0x8000, 0x4000),
        (8, _) => (0, 0),
        (16, 16) => (0, 0x4000),
        (16, 32) => (0x8000, 0x8000),
        (16, _) => (0x4000, 0),
        (32, 16) => (0x4000, 0x8000),
        (32, 32) => (0, 0x8000),
        (32, 64) => (0x8000, 0xC000),
        (32, _) => (0x4000, 0x4000),
        (64, 64) => (0, 0xC000),
        (64, _) => (0x4000, 0xC000),
        _ => (0, 0),
    }
}

/// Adresse de l'image courante de l'animation.
fn frame_ptr(env: &mut Env, e: u32) -> u32 {
    let anim = env.ldr(e + 4);
    let n = env.ldrb(e + 0x6A);
    env.ldr(anim).wrapping_add(n * 20)
}

fn set_flags(env: &mut Env, e: u32, or: u32) {
    let h = env.ldrh(e + 0x58);
    env.strh(e + 0x58, h | or);
}

/// Libère l'emplacement VRAM de l'entité s'il y en a un.
fn release_vram(env: &mut Env, e: u32) {
    let slot = env.ldr(e + 0x20);
    if slot != 0 {
        env.call(VRAM_FREE, &[slot]);
        env.str(e + 0x20, 0);
        set_flags(env, e, 0x40);
    }
}

/// `Entity_Frame(entité)` (0x08029CD8) : mise à jour d'une entité pour l'image en
/// cours — comportement, déplacement, animation, envoi des graphismes en VRAM et
/// ajout de ses sprites dans la copie de l'OAM.
pub fn entity_frame(env: &mut Env) {
    let e = env.cpu.r[0];
    let cam = env.ldr(e + 0x1C);
    let fl = e + 0x58;

    if env.call(ENTITY_ON_SCREEN, &[e]) & 0xFF == 0 && env.ldrh(fl) & 2 == 0 {
        env.call(ENTITY_HIDE, &[e, 1]);
        return;
    }

    // mise à jour logique : une image sur deux sauf si le drapeau 0x1000 est mis
    let odd = env.ldr(FRAME_COUNTER) & 1 != 0;
    if !odd || env.ldrh(fl) & 0x1000 != 0 {
        let behaviour = env.ldr(e + 0x28);
        if behaviour != 0 {
            env.call(behaviour, &[e]);
        }
        let mut vx = env.ldsh(e + 0x5A);
        // mod « course rapide » : déplacement horizontal du joueur amplifié (en %)
        let plus = COURSE_PLUS.load(std::sync::atomic::Ordering::Relaxed);
        if plus != 0 && e == env.bus.read32(0x0300_09A0) {
            vx += vx * plus / 100;
        }
        let x = env.ldr(e + 0x48).wrapping_add(vx as u32);
        env.str(e + 0x48, x);
        let vy = env.ldsh(e + 0x5C);
        let y = env.ldr(e + 0x4C).wrapping_add(vy as u32);
        env.str(e + 0x4C, y);
        if env.ldrh(fl) & 1 == 0 {
            // hors de l'écran : on rend la VRAM
            release_vram(env, e);
            return;
        }
    }

    // animation
    let mut frame = frame_ptr(env, e);
    let h = env.ldrh(fl);
    if h & 0x800 == 0
        && (env.ldr(FRAME_COUNTER) & 1 == 0 || h & 0x1000 != 0)
        && env.ldrb(frame + 0xC) != 0
        && env.ldrh(TILES_USED) <= 0x1FF
    {
        let t = env.ldrb(e + 0x6B).wrapping_sub(1);
        env.strb(e + 0x6B, t);
        if t & 0xFF == 0 {
            let n = env.ldrb(e + 0x6A) + 1;
            env.strb(e + 0x6A, n);
            let anim = env.ldr(e + 4);
            let n = env.ldrb(e + 0x6A);
            let count = env.ldrb(anim + 4);
            let n = env.call(MODULO, &[n, count]);
            env.strb(e + 0x6A, n);
            frame = frame_ptr(env, e);
            let d = env.ldrb(frame + 0xC);
            env.strb(e + 0x6B, d);
            set_flags(env, e, 0x40);
        }
    }

    // invisible ou sans graphismes : on rend la VRAM
    let h = env.ldrh(fl);
    let hidden = h & 0x10 != 0 || env.ldr(e) == 0 || {
        let group = env.ldr(frame);
        env.ldrb(group + 0x12) == 0
    };
    if hidden {
        release_vram(env, e);
        return;
    }
    if env.ldr(e + 0x20) == 0 {
        env.strh(fl, h | 0x40);
        let slot = env.call(VRAM_ALLOC, &[e]);
        env.str(e + 0x20, slot);
    }

    // retournements : ceux de l'image combinés à ceux de l'entité
    let f2 = frame_ptr(env, e);
    let img = env.ldrb(f2 + 0x10);
    let hflip = (img & 1) ^ ((env.ldrh(fl) & 4 != 0) as u32) != 0;
    let img = env.ldrb(f2 + 0x10);
    let vflip = ((env.ldrh(fl) >> 3) & 1) ^ ((img & 2 != 0) as u32) != 0;

    // position à l'écran
    let mut x = (env.ldr(e + 0x48) as i32) >> 8;
    let mut y = (env.ldr(e + 0x4C) as i32) >> 8;
    if cam != 0 {
        x = x.wrapping_sub((env.ldr(cam + 0x18) >> 8) as i32);
        y = y.wrapping_sub((env.ldr(cam + 0x1C) >> 8) as i32);
    }

    // boîte de l'image (origine +0xD/+0xE, étendues +4/+6/+8/+0xA)
    let ox = env.ldsb(frame + 0xD);
    let (sx, left) = if hflip {
        (x.wrapping_sub(ox), (-ox).wrapping_sub(env.ldrh(frame + 8) as i32))
    } else {
        (x.wrapping_add(ox), ox.wrapping_add(env.ldrh(frame + 4) as i32))
    };
    env.strh(e + 0x5E, left as u32);
    let oy = env.ldsb(frame + 0xE);
    let (sy, top) = if vflip {
        (y.wrapping_sub(oy), (-oy).wrapping_sub(env.ldrh(frame + 0xA) as i32))
    } else {
        (y.wrapping_add(oy), oy.wrapping_add(env.ldrh(frame + 6) as i32))
    };
    env.strh(e + 0x60, top as u32);
    double_box(env, e, 0x5E, 0x60);
    if x.wrapping_add(env.ldsh(e + 0x5E)) > 0xF0 || y.wrapping_add(env.ldsh(e + 0x60)) > 0xA0 {
        return;
    }
    let ox = env.ldsb(frame + 0xD);
    let right = if hflip {
        (-ox).wrapping_sub(env.ldrh(frame + 4) as i32)
    } else {
        ox.wrapping_add(env.ldrh(frame + 8) as i32)
    };
    env.strh(e + 0x62, right as u32);
    let oy = env.ldsb(frame + 0xE);
    let bottom = if vflip {
        (-oy).wrapping_sub(env.ldrh(frame + 6) as i32)
    } else {
        oy.wrapping_add(env.ldrh(frame + 0xA) as i32)
    };
    env.strh(e + 0x64, bottom as u32);
    double_box(env, e, 0x62, 0x64);
    if x.wrapping_add(env.ldsh(e + 0x62)) < 0 || y.wrapping_add(env.ldsh(e + 0x64)) < 0 {
        return;
    }

    // attributs communs des sprites
    let h = env.ldrh(fl);
    let mosaic_mode = if h & 0x20 != 0 { 0x400 } else { 0 }; // semi-transparence
    let flips = if hflip { 0x1000 } else { 0 } | if vflip { 0x2000 } else { 0 };
    if h & 0x180 != 0 {
        // agrandi ou mis à l'échelle : sprites affines
        env.call(DRAW_AFFINE, &[e, sx as u32, sy as u32, mosaic_mode, flips]);
        return;
    }
    if env.ldrh(OAM_COUNT) > 0x7F {
        return;
    }
    let mut tile = (env.ldr(e + 0x20) << 17) >> 22;
    let group = env.ldr(frame);
    let src = env.ldr(group).wrapping_add(env.ldr(e + 0x50));
    let pal = if h & 0x400 != 0 {
        0xF
    } else if h & 0x200 != 0 {
        env.ldrb(e + 0x72)
    } else {
        let t = env.ldr(e);
        env.call(PALETTE_OF, &[t]) & 0xFF
    };

    // envoi des graphismes de l'image en VRAM
    if env.ldrh(fl) & 0x40 != 0 {
        let k = env.ldrh(UPLOAD_COUNT);
        env.strh(UPLOAD_COUNT, k + 1);
        env.str(UPLOAD_LIST + 4 * (k & 0xFFFF), e);
        let t = env.ldr(e);
        if env.ldrh(t + 0xA) != 0x237 {
            let dst = env.ldr(VRAM_CURSOR);
            let n = env.call(DECOMPRESS, &[dst, src]);
            env.strh(e + 0x74, n);
        } else {
            // graphismes non compressés : copie DMA 3
            let n = env.ldrh(t + 8);
            env.strh(e + 0x74, n);
            env.str(0x0400_00D4, src);
            let dst = env.ldr(VRAM_CURSOR);
            env.str(0x0400_00D8, dst);
            let n = env.ldrh(e + 0x74);
            env.str(0x0400_00DC, (n >> 2) | 0x8400_0000);
            env.ldr(0x0400_00DC);
        }
        let n = env.ldrh(e + 0x74);
        let used = env.ldrh(TILES_USED);
        env.strh(TILES_USED, (n >> 5) + used);
        let n = env.ldrh(e + 0x74);
        let cur = env.ldr(VRAM_CURSOR);
        env.str(VRAM_CURSOR, cur.wrapping_add(n));
    }

    let mut prio = env.ldrh(e + 0x54);
    if env.ldsh(e + 0x56) < 0 && prio <= 2 {
        prio = (prio + 1) & 0xFFFF;
    }

    // une pièce = un sprite matériel
    let mut group = env.ldr(frame);
    let mut c = 0u32;
    while c < env.ldrb(group + 0x12) {
        let part = env.ldr(group + 4).wrapping_add(c * 12);
        let dx = env.ldsh(part);
        let dy = env.ldsh(part + 2);
        let mut px = sx.wrapping_add(dx);
        let mut py = sy.wrapping_add(dy);
        if hflip {
            px = sx.wrapping_sub(dx).wrapping_sub(env.ldrb(part + 4) as i32);
        }
        if vflip {
            py = sy.wrapping_sub(dy).wrapping_sub(env.ldrb(part + 5) as i32);
        }
        let visible = px <= 0xF0 && py <= 0xA0 && {
            let w = env.ldrb(part + 4);
            px.wrapping_add(w as i32) >= 0 && {
                let hh = env.ldrb(part + 5);
                py.wrapping_add(hh as i32) >= 0
            }
        };
        if visible {
            let (shape, size) = obj_shape(env.ldrb(part + 4), env.ldrb(part + 5));
            let k = env.ldrh(OAM_COUNT);
            if k <= 0x7F {
                env.strh(OAM_COUNT, k + 1);
                let p = OAM_BUFFER + k * 8;
                env.strh(p, (py as u32 & 0xFF) | shape | mosaic_mode);
                env.strh(p + 2, (px as u32 & 0x1FF) | size | flips);
                env.strh(p + 4, (pal << 12) | tile | (prio << 10));
            }
            tile = tile.wrapping_add(env.ldrh(part + 6) >> 5);
            group = env.ldr(frame);
        } else {
            tile = tile.wrapping_add(env.ldrh(part + 6) >> 5);
        }
        c = (c + 1) & 0xFFFF;
    }
    let h = env.ldrh(fl);
    env.strh(fl, h & 0xFFBF);
}

/// Double la boîte (`+a`, `+b`) si l'entité est agrandie ou mise à l'échelle.
fn double_box(env: &mut Env, e: u32, a: u32, b: u32) {
    if env.ldrh(e + 0x58) & 0x180 != 0 {
        let v = env.ldsh(e + a) << 1;
        env.strh(e + a, v as u32);
        let v = env.ldsh(e + b) << 1;
        env.strh(e + b, v as u32);
    }
}

const ENTITY_ALLOC: u32 = 0x0802_A6ED;
const ENTITY_FREE_LIST: u32 = 0x0802_A425; // libère (entité, mode)
const LAYER_REMOVE: u32 = 0x0802_9929; // retire l'entité de sa couche d'affichage
const LAYER_INSERT: u32 = 0x0802_97FD; // l'insère dans la couche de sa priorité
const BEHAVIOURS: u32 = 0x083D_B2D0; // comportement par identifiant de type
const ENTITY_FRAME: u32 = 0x0802_9CD9;

/// Parcourt une file d'attente `(compteur octet, tableau)` puis la vide.
fn drain(env: &mut Env, count: u32, mut each: impl FnMut(&mut Env, u32)) {
    let mut i = 0u32;
    while i < env.ldrb(count) {
        each(env, i);
        i = (i + 1) & 0xFFFF;
    }
    env.strb(count, 0);
}

/// `Entities_Update()` (0x080299E8) : une image du gestionnaire d'entités.
///
/// 1. traite les files d'attente (suppressions, retraits et changements de couche) ;
/// 2. fait apparaître les objets du niveau qui entrent dans la zone de la caméra ;
/// 3. remet à zéro les compteurs de sprites, puis met à jour chaque entité des
///    4 couches d'affichage (`Entity_Frame`).
pub fn entities_update(env: &mut Env) {
    let base = env.ldr(0x0300_1F54);
    env.str(VRAM_CURSOR, base);

    drain(env, 0x0300_5A54, |env, i| {
        let p = 0x0300_1E50 + 8 * i;
        let e = env.ldr(p);
        let mode = env.ldrb(p + 4);
        env.call(ENTITY_FREE_LIST, &[e, mode]);
    });
    drain(env, 0x0300_5A58, |env, i| {
        let e = env.ldr(0x0300_5AD0 + 4 * i);
        env.call(LAYER_REMOVE, &[e]);
    });
    drain(env, 0x0300_5B58, |env, i| {
        let e = env.ldr(0x0300_4DC0 + 4 * i);
        env.call(LAYER_INSERT, &[e]);
    });
    drain(env, 0x0300_5A50, |env, i| {
        let p = 0x0300_1F60 + 8 * i;
        let e = env.ldr(p);
        env.call(LAYER_REMOVE, &[e]);
        let prio = env.ldrh(p + 4);
        env.strh(e + 0x54, prio);
        let depth = env.ldrh(p + 6);
        env.strh(e + 0x56, depth);
        env.call(LAYER_INSERT, &[e]);
    });

    let busy = env.ldrh(0x0300_5AC0);
    env.strh(0x0300_5AC0, busy | 1);

    // objets du niveau : enregistrements de 20 octets
    // (+0 objet, +4 caméra, +8 entité créée, +0xC état : bit 0 = apparu)
    let mut i = 0u32;
    while i < env.ldrh(0x0300_5B54) {
        let rec = 0x0300_2060 + 20 * i;
        if env.ldr(rec + 0xC) & 3 == 0 && spawn_wanted(env, rec) {
            let cam = env.ldr(rec + 4);
            let obj = env.ldr(rec);
            let kind = env.ldr(obj);
            let arg = env.ldsh(obj + 8);
            let ent = env.call(ENTITY_ALLOC, &[cam, kind, arg as u32]);
            env.str(rec + 8, ent);
            if ent == 0 {
                break; // plus de place
            }
            let obj = env.ldr(rec);
            let x = (env.ldsh(obj + 4) << 8) as u32;
            env.str(ent + 0x48, x);
            env.str(ent + 0x40, x);
            let y = (env.ldsh(obj + 6) << 8) as u32;
            env.str(ent + 0x4C, y);
            env.str(ent + 0x44, y);
            let kind = env.ldr(obj);
            let id = env.ldrh(kind + 0xA);
            let behaviour = env.ldr(BEHAVIOURS + 4 * id);
            env.str(ent + 0x28, behaviour);
            env.strb(ent + 0x6D, i);
            let dst = env.ldr(rec + 8) + 8;
            let src = env.ldr(rec);
            for k in 0..5 {
                let v = env.ldr(src + 4 * k);
                env.str(dst + 4 * k, v);
            }
            let st = env.ldr(rec + 0xC);
            env.str(rec + 0xC, st | 1);
            for (bit, flag) in [(1, 4), (2, 8)] {
                let obj = env.ldr(rec);
                if env.ldrb(obj + 0x11) & bit != 0 {
                    let ent = env.ldr(rec + 8);
                    set_flags(env, ent, flag);
                }
            }
        }
        i = (i + 1) & 0xFFFF;
    }

    let busy = env.ldrh(0x0300_5AC0);
    env.strh(0x0300_5AC0, busy & 0xFFFE);
    env.strh(0x0300_1F50, 0);
    env.strh(OAM_COUNT, 0);
    env.strh(UPLOAD_COUNT, 0);

    // couches d'affichage : 4 listes de 196 entités
    for layer in 0..4u32 {
        let mut j = 0u32;
        while j < env.ldrh(0x0300_1E48 + 2 * layer) {
            let e = env.ldr(0x0300_5B60 + 0x310 * layer + 4 * j);
            if env.ldr(e) != 0 {
                env.call(ENTITY_FRAME, &[e]);
            }
            j = (j + 1) & 0xFFFF;
        }
    }
}

/// L'objet du niveau `rec` doit-il apparaître ? Il faut qu'il soit dans la zone de
/// la caméra, et hors de l'écran visible (sauf drapeaux 0x5 de `0x03006CA8`).
fn spawn_wanted(env: &mut Env, rec: u32) -> bool {
    let cam = env.ldr(rec + 4);
    let obj = env.ldr(rec);
    let x = (env.ldsh(obj + 4) << 8).wrapping_sub(env.ldr(cam + 0x18) as i32);
    if x < env.ldr(cam + 0x28) as i32 || x > (env.ldr(cam + 0x2C) as i32).wrapping_add(0xF000) {
        return false;
    }
    let y = (env.ldsh(obj + 6) << 8).wrapping_sub(env.ldr(cam + 0x1C) as i32);
    if y < env.ldr(cam + 0x30) as i32 || y > (env.ldr(cam + 0x34) as i32).wrapping_add(0xA000) {
        return false;
    }
    env.ldr(FRAME_COUNTER) & 5 != 0 || y as u32 > 0xA000 || x as u32 > 0xF000
}

const FALL: u32 = 0x0802_8651;
const RISE: u32 = 0x0802_87D1;
const MOVE_RIGHT: u32 = 0x0802_8941;
const MOVE_LEFT: u32 = 0x0802_8B41;
const MAP_COLLIDE: u32 = 0x0802_956D;
/// Taille de la boîte de collision en cours (indice des tables de décalage).
const PROBE_SIZE: u32 = 0x0300_1E20;
const PROBE_DX: u32 = 0x0835_204C;
const PROBE_DY: u32 = 0x0835_2088;
const PROBE_RESULT: u32 = 0x0300_1E30;
const GROUND_TIMER: u32 = 0x0300_03D8;

/// `Entity_Probe(entité, arg)` (0x08028D3C) : déplacement contre le décor puis sondage
/// de 3 × 3 points autour de l'entité (tables de décalages par taille, symétrisées si
/// l'entité est retournée) ; résultat de `Map_Collide` dans `0x03001E30 + 4i + j`.
/// L'état « au sol » (`+0x6C`) est gardé quelques images après l'avoir perdu.
pub fn entity_probe(env: &mut Env) {
    let e = env.cpu.r[0];
    let arg = env.cpu.r[1];
    let was = env.ldrb(e + 0x6C);
    env.strb(e + 0x6C, 0);
    let vy = env.ldsh(e + 0x5C);
    if vy > 0 {
        env.call(FALL, &[e]);
    } else if vy < 0 {
        env.call(RISE, &[e]);
    }
    let vx = env.ldsh(e + 0x5A);
    if vx > 0 {
        env.call(MOVE_RIGHT, &[e, arg]);
    } else if vx < 0 {
        env.call(MOVE_LEFT, &[e, arg]);
    }

    let mirror = |k: u32| (((k << 1) & 3) | k) ^ 2;
    for i in 0..3u32 {
        for j in 0..3u32 {
            let cam = env.ldr(e + 0x1C);
            let size = env.ldrh(PROBE_SIZE);
            let dx = if env.ldrh(e + 0x58) & 4 != 0 {
                env.ldr(PROBE_DX + 4 * (3 * size + mirror(i))).wrapping_neg()
            } else {
                env.ldr(PROBE_DX + 4 * (3 * size + i))
            };
            let x = env.ldr(e + 0x48).wrapping_add(dx);
            let size = env.ldrh(PROBE_SIZE);
            let dy = if env.ldrh(e + 0x58) & 8 != 0 {
                env.ldr(PROBE_DY + 4 * (3 * size + mirror(j))).wrapping_neg()
            } else {
                env.ldr(PROBE_DY + 4 * (3 * size + j))
            };
            let y = env.ldr(e + 0x4C).wrapping_add(dy);
            let v = env.call(MAP_COLLIDE, &[cam, x, y]);
            env.strb(PROBE_RESULT + 4 * i + j, v);
        }
    }

    if env.ldrb(e + 0x6C) == 0 && (was != 0 || env.ldrb(GROUND_TIMER) != 0) {
        let t = (env.ldrb(GROUND_TIMER) + 1) & 0xFF;
        env.strb(GROUND_TIMER, t);
        if t & 3 != 0 {
            env.strb(e + 0x6C, was);
        }
    } else {
        env.strb(GROUND_TIMER, 0);
    }
}

/// File des entités à cacher (8 octets chacune : entité, drapeau) et sa longueur (32 au plus).
const HIDE_LIST: u32 = 0x0300_1E50;
const HIDE_COUNT: u32 = 0x0300_5A54;

/// `Entity_Hide(entité, drapeau)` (0x0802A50C) : sauf si l'entité est marquée
/// « toujours active » (drapeau 2), l'ajoute une seule fois à la file des entités à
/// cacher (si elle n'est pas pleine) et défait son lien avec ses voisines (`+0x30/+0x34`).
pub fn entity_hide(env: &mut Env) {
    let e = env.cpu.r[0];
    let flag = env.cpu.r[1] & 0xFF;
    if env.ldrh(e + 0x58) & 2 != 0 {
        return;
    }
    let n = env.ldrb(HIDE_COUNT);
    for i in 0..n {
        if env.ldr(HIDE_LIST + 8 * i) == e {
            return;
        }
    }
    let c = env.ldrb(HIDE_COUNT);
    if c <= 0x1F {
        env.str(HIDE_LIST + 8 * c, e);
        env.strb(HIDE_COUNT, c + 1);
        env.strh(HIDE_LIST + 8 * c + 4, flag);
    }
    let next = env.ldr(e + 0x30);
    if next != 0 {
        env.str(next + 0x34, 0);
    }
    let prev = env.ldr(e + 0x34);
    if prev != 0 {
        env.str(prev + 0x30, 0);
    }
}

/// Table d'occupation de la VRAM des sprites : une entrée de 16 bits par tuile (1 024
/// tuiles), au début de chaque bloc : bit 15 = occupé, bits 0-14 = longueur du bloc.
const VRAM_TABLE: u32 = 0x0300_4E50;
const OBJ_TILES: u32 = 0x0601_0000;

fn vram_entry(env: &mut Env, i: u32) -> u32 {
    env.ldrh(VRAM_TABLE + 2 * i)
}

/// `VRAM_Alloc(entité)` (0x0802A580) : premier bloc libre assez grand pour les
/// graphismes de l'entité (taille du type `+8`, en tuiles de 32 octets), coupé à la
/// taille voulue ; si aucun ne convient, les blocs libres voisins sont fusionnés puis on
/// cherche une seconde fois. Renvoie l'adresse en VRAM, ou 0.
pub fn vram_alloc(env: &mut Env) {
    let t = env.ldr(env.cpu.r[0]);
    let need = env.ldrh(t + 8) >> 5;
    for pass in 0..2 {
        let mut i = 0u32;
        loop {
            let v = vram_entry(env, i);
            if v & 0x8000 == 0 && v >= need {
                let rest = (v - need) & 0xFFFF;
                env.strh(VRAM_TABLE + 2 * i, (need + 0x8000) & 0xFFFF);
                if rest != 0 {
                    env.strh(VRAM_TABLE + 2 * (i + need), rest);
                }
                env.cpu.r[0] = OBJ_TILES + i * 32;
                return;
            }
            i = (i + (v & 0x7FFF)) & 0xFFFF;
            if i > 0x3FF {
                break;
            }
        }
        if pass == 1 {
            break;
        }
        // fusion des blocs libres consécutifs
        let mut i = 0u32;
        loop {
            if vram_entry(env, i) & 0x8000 == 0 {
                let mut len = vram_entry(env, i);
                let mut j = i + len;
                if vram_entry(env, j) & 0x8000 == 0 && j as i32 <= 0x3FF {
                    loop {
                        len = (len + vram_entry(env, i + len)) & 0xFFFF;
                        j = i + len;
                        if vram_entry(env, j) & 0x8000 != 0 || j as i32 > 0x3FF {
                            break;
                        }
                    }
                }
                env.strh(VRAM_TABLE + 2 * i, len);
            }
            i = (i + (vram_entry(env, i) & 0x7FFF)) & 0xFFFF;
            if i > 0x3FF {
                break;
            }
        }
    }
    env.cpu.r[0] = 0;
}

/// `VRAM_Free(adresse)` (0x0802A694) : marque libre le bloc qui commence à `adresse` et
/// le fusionne avec le bloc suivant s'il est libre.
pub fn vram_free(env: &mut Env) {
    let a = env.cpu.r[0];
    if a == 0 {
        return;
    }
    let mut off = a.wrapping_sub(OBJ_TILES);
    if (off as i32) < 0 {
        off = off.wrapping_add(0x1F);
    }
    let i = (off << 11) >> 16;
    let v = vram_entry(env, i) & 0x7FFF;
    env.strh(VRAM_TABLE + 2 * i, v);
    let len = vram_entry(env, i);
    let j = i + len;
    let next = vram_entry(env, j);
    if (1..=0x7FFF).contains(&next) {
        env.strh(VRAM_TABLE + 2 * j, 0);
        env.strh(VRAM_TABLE + 2 * i, len + next);
    }
}
