//! Objets placés dans les niveaux et leurs sprites.
//!
//! Chaque entrée de couche d'une scène (0x20 octets) porte, en plus du décor :
//! `+0x04` pointeur vers la liste d'objets, `+0x0C` nombre d'objets.
//! Objet (0x14 octets) : `[définition][x u16 | y u16 << 16][param][param][param]`,
//! position en pixels dans la couche (le jeu la stocke en 24.8 dans l'entité, +0x48/+0x4C).
//!
//! Définition d'objet (lue par `Entity_Alloc` 0x0802A6EC, `Sprite_SetAnimation` 0x0802C5EC) :
//! `+0x00` table des animations (`[liste d'images][nombre]` × n), `+0x04` banque graphique
//! compressée (format « packed », adresse parfois impaire) utilisée si `+0x0E` & 2,
//! `+0x08` (u16) octets de VRAM, `+0x0A` (u16) n° de type (sert à choisir la palette),
//! `+0x0D` nombre d'animations.
//!
//! Image d'animation (0x14 octets) : `+0x00` image, `+0x0D`/`+0x0E` décalage x/y (s8),
//! `+0x10` bit 0 = miroir H, bit 1 = miroir V.
//! Image : `+0x00` graphismes (pointeur ROM, ou décalage dans la banque décompressée),
//! `+0x04` pièces, `+0x12` nombre de pièces. Graphismes = flux RLE (`rle.rs`) contenant
//! les tuiles 4 bits de toutes les pièces à la suite (mode OBJ 1D).
//! Pièce (12 octets) : x s16, y s16, largeur u8, hauteur u8, octets u16, ?
//!
//! Palette (`sub_0802C824`) : quartet n° `type` de la table pointée par scène `+0x0C`
//! -> banque de 16 couleurs dans la moitié OBJ de la palette de la scène.

use crate::packed::{self, Tables};
use crate::rle;
use crate::rom::{Rom, ROM_BASE};
use std::collections::BTreeMap;

fn rd(rom: &Rom, a: u32) -> u32 {
    a.checked_sub(ROM_BASE).and_then(|o| rom.u32(o as usize)).unwrap_or(0)
}
fn byte(rom: &Rom, a: u32) -> u8 {
    a.checked_sub(ROM_BASE).and_then(|o| rom.data.get(o as usize).copied()).unwrap_or(0)
}
fn is_rom(a: u32) -> bool {
    (0x0800_0000..0x0840_0000).contains(&a)
}

pub struct Object {
    pub def: u32,
    pub x: i32,
    pub y: i32,
    pub params: [u32; 3],
}

/// Liste d'objets d'une entrée de couche.
pub fn list(rom: &Rom, layer_entry: u32) -> Vec<Object> {
    let (p, n) = (rd(rom, layer_entry + 4), rd(rom, layer_entry + 0xC));
    if !is_rom(p) || n == 0 || n > 1000 {
        return Vec::new();
    }
    (0..n)
        .map(|i| p + i * 0x14)
        .map(|a| {
            let pos = rd(rom, a + 4);
            Object {
                def: rd(rom, a),
                x: (pos & 0xFFFF) as i16 as i32,
                y: (pos >> 16) as i16 as i32,
                params: [rd(rom, a + 8), rd(rom, a + 12), rd(rom, a + 16)],
            }
        })
        .collect()
}

/// Première image de la première animation d'une définition, en indices 0..15
/// (0 = transparent) relatifs au point d'ancrage (`ox`, `oy` = position du coin haut gauche).
pub struct Sprite {
    pub ox: i32,
    pub oy: i32,
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
    pub type_id: u16,
    /// Durée d'affichage (en images du jeu).
    pub duration: u8,
}

pub struct Sprites<'a> {
    rom: &'a Rom,
    t: &'a Tables,
    banks: BTreeMap<u32, Option<Vec<u8>>>,
    cache: BTreeMap<u32, Option<Sprite>>,
}

impl<'a> Sprites<'a> {
    pub fn new(rom: &'a Rom, t: &'a Tables) -> Self {
        Sprites { rom, t, banks: BTreeMap::new(), cache: BTreeMap::new() }
    }

    pub fn get(&mut self, def: u32) -> Option<&Sprite> {
        if !self.cache.contains_key(&def) {
            let s = self.build(def);
            self.cache.insert(def, s);
        }
        self.cache[&def].as_ref()
    }

    fn bank(&mut self, a: u32) -> Option<&Vec<u8>> {
        if !self.banks.contains_key(&a) {
            let (rom, t) = (self.rom, self.t);
            let d = packed::decompress(t, &rom.data[(a - ROM_BASE) as usize..]).map(|u| u.data);
            self.banks.insert(a, d);
        }
        self.banks[&a].as_ref()
    }

    /// Nombre d'animations d'une définition, et nombre d'images de chacune.
    pub fn anims(&self, def: u32) -> Vec<u32> {
        let rom = self.rom;
        let table = rd(rom, def);
        if !is_rom(def) || !is_rom(table) {
            return Vec::new();
        }
        (0..byte(rom, def + 0xD) as u32)
            .map(|a| {
                let (list, n) = (rd(rom, table + 8 * a), rd(rom, table + 8 * a + 4) & 0xFFFF);
                if is_rom(list) && n <= 128 { n } else { 0 }
            })
            .collect()
    }

    fn build(&mut self, def: u32) -> Option<Sprite> {
        if self.anims(def).first().copied().unwrap_or(0) == 0 {
            return None;
        }
        self.frame(def, 0, 0)
    }

    /// Image `idx` de l'animation `anim` (non mise en cache).
    pub fn frame(&mut self, def: u32, anim: u32, idx: u32) -> Option<Sprite> {
        let rom = self.rom;
        if !is_rom(def) {
            return None;
        }
        let type_id = (rd(rom, def + 8) >> 16) as u16;
        let list = rd(rom, rd(rom, def) + 8 * anim);
        if !is_rom(list) {
            return None;
        }
        let frames = list + 0x14 * idx;
        let image = rd(rom, frames);
        let duration = byte(rom, frames + 0xC);
        let dx = byte(rom, frames + 0xD) as i8 as i32;
        let dy = byte(rom, frames + 0xE) as i8 as i32;
        let flags = byte(rom, frames + 0x10);
        let (hflip, vflip) = (flags & 1 != 0, flags & 2 != 0);
        if !is_rom(image) {
            return None;
        }
        let gfx = rd(rom, image);
        let src: Vec<u8> = if byte(rom, def + 0xE) & 2 != 0 {
            let bank = self.bank(rd(rom, def + 4))?;
            bank.get(gfx as usize..)?.to_vec()
        } else if is_rom(gfx) {
            rom.data[(gfx - ROM_BASE) as usize..].iter().take(0x1_0000).copied().collect()
        } else {
            return None;
        };
        let tiles = rle::decompress(&src)?.data;

        // pièces
        let pieces_at = rd(rom, image + 4);
        let n = byte(rom, image + 0x12) as u32;
        if !is_rom(pieces_at) || n == 0 || n > 64 {
            return None;
        }
        let mut pieces = Vec::new();
        let mut off = 0usize;
        for i in 0..n {
            let a = pieces_at + 12 * i;
            let xy = rd(rom, a);
            let (x, y) = ((xy & 0xFFFF) as i16 as i32, (xy >> 16) as i16 as i32);
            let (w, h) = (byte(rom, a + 4) as usize, byte(rom, a + 5) as usize);
            let size = (rd(rom, a + 4) >> 16) as usize;
            if w == 0 || h == 0 || w % 8 != 0 || h % 8 != 0 || w > 64 || h > 64 {
                return None;
            }
            pieces.push((x, y, w, h, off));
            off += size;
        }
        if off > tiles.len() {
            return None;
        }
        // boîte englobante (avec miroirs : x -> -x - w)
        let place = |x: i32, y: i32, w: usize, h: usize| {
            let px = if hflip { -x - w as i32 } else { x };
            let py = if vflip { -y - h as i32 } else { y };
            (px, py)
        };
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        for &(x, y, w, h, _) in &pieces {
            let (px, py) = place(x, y, w, h);
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px + w as i32);
            y1 = y1.max(py + h as i32);
        }
        let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut out = vec![0u8; bw * bh];
        // les premières pièces sont devant (ordre OAM) : on dessine à l'envers
        for &(x, y, w, h, o) in pieces.iter().rev() {
            let (px, py) = place(x, y, w, h);
            for yy in 0..h {
                for xx in 0..w {
                    let t = (yy / 8) * (w / 8) + xx / 8;
                    let b = tiles[o + t * 32 + (yy % 8) * 4 + (xx % 8) / 2];
                    let c = if xx & 1 == 0 { b & 15 } else { b >> 4 };
                    if c == 0 {
                        continue;
                    }
                    let sx = if hflip { w - 1 - xx } else { xx };
                    let sy = if vflip { h - 1 - yy } else { yy };
                    let (ox, oy) = ((px - x0) as usize + sx, (py - y0) as usize + sy);
                    out[oy * bw + ox] = c;
                }
            }
        }
        let (dx, dy) = (if hflip { -dx } else { dx }, if vflip { -dy } else { dy });
        Some(Sprite { ox: x0 + dx, oy: y0 + dy, w: bw, h: bh, px: out, type_id, duration })
    }
}

/// N° de type d'une définition (`+0x0A`).
pub fn def_type(rom: &Rom, def: u32) -> u16 {
    (rd(rom, def + 8) >> 16) as u16
}

/// Définition du personnage joueur (créée en 0x08000C54).
pub const PLAYER: u32 = 0x0805_B304;

/// Banque de palette OBJ (0..15) d'un type d'objet, d'après la table de quartets de la scène.
pub fn palette_bank(rom: &Rom, scene: u32, type_id: u16) -> u8 {
    let table = rd(rom, scene + 0xC);
    if !is_rom(table) {
        return 0;
    }
    let b = byte(rom, table + (type_id as u32 >> 1));
    if type_id & 1 != 0 { b >> 4 } else { b & 15 }
}

/// Dessine les objets d'une scène sur une image RVB (`rgb`, `w` × `h`).
/// `objs` = (x, y, définition, drapeaux : 1 = miroir H, 2 = miroir V, cf. param 3 bits 8-9).
/// `obj_colors` = 256 couleurs OBJ. Renvoie (objets dessinés, objets sans sprite).
pub fn draw(
    rom: &Rom,
    sprites: &mut Sprites,
    scene: u32,
    objs: &[(i32, i32, u32, u32)],
    obj_colors: &[[u8; 3]],
    rgb: &mut [u8],
    w: usize,
    h: usize,
) -> (usize, usize) {
    let (mut ok, mut missing) = (0, 0);
    let put = |rgb: &mut [u8], x: i32, y: i32, c: [u8; 3]| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let i = (y as usize * w + x as usize) * 3;
            rgb[i..i + 3].copy_from_slice(&c);
        }
    };
    for &(x, y, def, flags) in objs {
        match sprites.get(def) {
            Some(s) => {
                let bank = palette_bank(rom, scene, s.type_id) as usize;
                for sy in 0..s.h {
                    for sx in 0..s.w {
                        let c = s.px[sy * s.w + sx];
                        if c != 0 {
                            // miroir de l'entité : x -> -1 - x autour du point d'ancrage
                            let (mut dx, mut dy) = (s.ox + sx as i32, s.oy + sy as i32);
                            if flags & 1 != 0 {
                                dx = -1 - dx;
                            }
                            if flags & 2 != 0 {
                                dy = -1 - dy;
                            }
                            put(rgb, x + dx, y + dy, obj_colors[bank * 16 + c as usize]);
                        }
                    }
                }
                ok += 1;
            }
            None => {
                // repère : petite croix magenta
                for d in -3..=3 {
                    put(rgb, x + d, y, [255, 0, 255]);
                    put(rgb, x, y + d, [255, 0, 255]);
                }
                missing += 1;
            }
        }
    }
    (ok, missing)
}

/// Planche de toutes les animations d'une définition : une ligne par animation, images
/// côte à côte ; dans une ligne, toutes les cases ont la même taille et le point
/// d'ancrage au même endroit (on voit donc le mouvement en parcourant la ligne).
/// Renvoie (largeur, hauteur, pixels 0..15, description texte, nombre d'images).
pub fn sheet(sprites: &mut Sprites, def: u32) -> Option<(usize, usize, Vec<u8>, String, usize)> {
    use std::fmt::Write as _;
    let counts = sprites.anims(def);
    let rows: Vec<Vec<Option<Sprite>>> =
        counts.iter().enumerate().map(|(a, &n)| (0..n).map(|i| sprites.frame(def, a as u32, i)).collect()).collect();
    const GAP: usize = 2;
    // boîte de chaque animation : (x0, y0, largeur de case, hauteur de case)
    let boxes: Vec<(i32, i32, usize, usize)> = rows
        .iter()
        .map(|r| {
            let f = || r.iter().flatten();
            match (f().map(|s| s.ox).min(), f().map(|s| s.oy).min()) {
                (Some(x0), Some(y0)) => {
                    let x1 = f().map(|s| s.ox + s.w as i32).max().unwrap();
                    let y1 = f().map(|s| s.oy + s.h as i32).max().unwrap();
                    (x0, y0, (x1 - x0) as usize + GAP, (y1 - y0) as usize + GAP)
                }
                _ => (0, 0, 0, 0),
            }
        })
        .collect();
    let w = rows.iter().zip(&boxes).map(|(r, b)| r.len() * b.2).max()?;
    let h: usize = boxes.iter().map(|b| b.3).sum();
    if w == 0 || h == 0 {
        return None;
    }
    let mut px = vec![0u8; w * h];
    let mut txt = String::new();
    let mut n = 0;
    let mut top = 0;
    for (a, (row, &(x0, y0, cw, ch))) in rows.iter().zip(&boxes).enumerate() {
        let durs: Vec<String> =
            row.iter().map(|s| s.as_ref().map_or("?".into(), |s| s.duration.to_string())).collect();
        writeln!(txt, "  animation {a:2} : {:2} images, durées {}", row.len(), durs.join(" ")).unwrap();
        for (i, s) in row.iter().enumerate() {
            let Some(s) = s else { continue };
            n += 1;
            let bx = i * cw + (s.ox - x0) as usize;
            let by = top + (s.oy - y0) as usize;
            for y in 0..s.h {
                for x in 0..s.w {
                    let c = s.px[y * s.w + x];
                    if c != 0 {
                        px[(by + y) * w + bx + x] = c;
                    }
                }
            }
        }
        top += ch;
    }
    Some((w, h, px, txt, n))
}

/// Écrit la planche de chaque définition dans `extract/sprites/`, avec la palette OBJ
/// (16 couleurs) d'une scène où elle apparaît.
pub fn export_sheets(sprites: &mut Sprites, defs: &BTreeMap<u32, Vec<[u8; 3]>>) -> std::io::Result<String> {
    use std::fmt::Write as _;
    std::fs::create_dir_all("extract/sprites")?;
    let mut idx = String::new();
    let (mut files, mut frames) = (0, 0);
    for (&def, pal) in defs {
        let rom = sprites.rom;
        let type_id = (rd(rom, def + 8) >> 16) as u16;
        let Some((w, h, px, txt, n)) = sheet(sprites, def) else {
            writeln!(idx, "{def:08X} type {type_id:3} : aucune image lisible").unwrap();
            continue;
        };
        std::fs::write(format!("extract/sprites/def_{def:08X}_type{type_id:03}.png"), crate::png::indexed_alpha0(w, h, &px, pal))?;
        writeln!(idx, "{def:08X} type {type_id:3} : {} animations, {n} images, planche {w}x{h}", sprites.anims(def).len()).unwrap();
        idx.push_str(&txt);
        files += 1;
        frames += n;
    }
    std::fs::write("extract/sprites/index.txt", idx)?;
    Ok(format!("Planches de sprites : {files} définitions, {frames} images -> extract/sprites/ (détail dans index.txt)
"))
}
