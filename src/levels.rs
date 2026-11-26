//! Rendu des décors : scène -> palette + couches (enregistrements du répertoire 0x0825xxxx).
//!
//! Scène (lue par `sub_0802C974`) : +0x00 liste des couches (pas de 0x20, la 1re valeur
//! de chaque entrée = enregistrement de décor), +0x08 palette compressée (format
//! « packed », 1 024 octets : 256 couleurs BG puis 256 OBJ), +0x10 (u16) nombre de couches.
//!
//! Enregistrement de décor (7 mots) : tuiles, carte, attributs, extra, w | h << 16,
//! nombre de tuiles (u16 en +0x14), octets en +0x18/+0x19.
//!
//! Carte (lue par `sub_08029310`) : h + 1 mots d'en-tête (nombre de mots de chaque ligne),
//! puis pour chaque ligne une suite de commandes : mot avec bit 15 = n entrées d'écran
//! qui suivent (tuile 10 bits, bit 10 = miroir H, bit 11 = miroir V, bits 12-15 = palette),
//! sinon = n cases vides à sauter.

use crate::packed::{self, Tables};
use crate::behaviour;
use crate::objects;
use crate::png;
use crate::rom::{Rom, ROM_BASE};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub struct Record {
    pub addr: u32,
    pub tiles: u32,
    pub map: u32,
    pub w: usize,
    pub h: usize,
}

fn rd(rom: &Rom, a: u32) -> u32 {
    a.checked_sub(ROM_BASE).and_then(|o| rom.u32(o as usize)).unwrap_or(0)
}

pub fn record(rom: &Rom, a: u32) -> Option<Record> {
    let res = |v: u32| (0x0826_0000..0x0836_0000).contains(&v);
    let w: Vec<u32> = (0..7).map(|i| rd(rom, a + i * 4)).collect();
    let ok = res(w[0]) && res(w[1]) && res(w[2]) && (w[3] == 0 || res(w[3])) && w[6] & 0xFF == 6;
    ok.then(|| Record { addr: a, tiles: w[0], map: w[1], w: (w[4] & 0xFFFF) as usize, h: (w[4] >> 16) as usize })
}

/// Entrées d'écran de la carte, en grille w × h (0xFFFF = case vide).
pub fn decode_map(data: &[u8], w: usize, h: usize) -> Option<Vec<u16>> {
    let v: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let mut grid = vec![0xFFFFu16; w * h];
    let mut p = h + 1;
    for y in 0..h {
        let end = p + *v.get(y)? as usize;
        let mut x = 0;
        while p < end {
            let c = *v.get(p)?;
            p += 1;
            if c & 0x8000 != 0 {
                for _ in 0..(c & 0x7FFF) {
                    let e = *v.get(p)?;
                    p += 1;
                    if x < w {
                        grid[y * w + x] = e;
                    }
                    x += 1;
                }
            } else {
                x += c as usize;
            }
        }
    }
    (p == v.len()).then_some(grid)
}

/// 256 couleurs BGR555 -> RGB8.
fn rgb(pal: &[u8]) -> Vec<[u8; 3]> {
    pal.chunks_exact(2)
        .take(512)
        .map(|c| {
            let v = u16::from_le_bytes([c[0], c[1]]);
            let f = |s: u16| (((v >> s) & 31) as u32 * 255 / 31) as u8;
            [f(0), f(5), f(10)]
        })
        .collect()
}

fn render(grid: &[u16], w: usize, h: usize, tiles: &[u8]) -> Vec<u8> {
    let n = tiles.len() / 32;
    let pw = w * 8;
    let mut px = vec![0u8; pw * h * 8];
    for ty in 0..h {
        for tx in 0..w {
            let e = grid[ty * w + tx];
            let t = (e & 0x3FF) as usize;
            if e == 0xFFFF || t >= n {
                continue;
            }
            let pal = ((e >> 12) as u8) << 4;
            for y in 0..8 {
                for x in 0..8 {
                    let sx = if e & 0x400 != 0 { 7 - x } else { x };
                    let sy = if e & 0x800 != 0 { 7 - y } else { y };
                    let b = tiles[t * 32 + sy * 4 + sx / 2];
                    let c = if sx & 1 == 0 { b & 15 } else { b >> 4 };
                    if c != 0 {
                        px[(ty * 8 + y) * pw + tx * 8 + x] = pal | c;
                    }
                }
            }
        }
    }
    px
}

struct Layer {
    index: usize,
    /// Adresse de l'entrée de couche dans la scène (liste d'objets en +4).
    entry: u32,
    w: usize,
    h: usize,
    /// Facteurs de défilement (8.8 : 0x100 = suit la caméra).
    fx: u32,
    fy: u32,
    /// Priorité GBA (0 = devant).
    prio: u32,
    px: Vec<u8>,
}

/// Superpose les couches d'une scène en un panorama.
///
/// Référence : la couche la plus grande (le terrain de jeu). Pour une colonne X du
/// panorama, une couche de facteur f est lue en X × f / f_ref (idem en Y), répétée si elle
/// est plus petite ; une couche fixe (facteur 0) est répétée telle quelle. Ordre du
/// matériel : priorité 3 -> 0, et à priorité égale, le BG de plus petit numéro devant.
fn reference(layers: &[Layer]) -> &Layer {
    layers.iter().max_by_key(|l| (l.w * l.h, l.fx)).unwrap()
}

fn compose(layers: &[Layer]) -> (usize, usize, Vec<u8>) {
    let r = reference(layers);
    let (w, h) = (r.w, r.h);
    let mut order: Vec<&Layer> = layers.iter().collect();
    order.sort_by_key(|l| (std::cmp::Reverse(l.prio), std::cmp::Reverse(l.index)));
    let mut out = vec![0u8; w * h];
    let map = |p: usize, f: u32, fref: u32, size: usize| {
        let v = if f == 0 || fref == 0 { p } else { p * f as usize / fref as usize };
        v % size
    };
    for l in order {
        let xs: Vec<usize> = (0..w).map(|x| map(x, l.fx, r.fx, l.w)).collect();
        for y in 0..h {
            let ly = map(y, l.fy, r.fy, l.h);
            let src = &l.px[ly * l.w..(ly + 1) * l.w];
            for (d, &sx) in out[y * w..(y + 1) * w].iter_mut().zip(&xs) {
                if src[sx] != 0 {
                    *d = src[sx];
                }
            }
        }
    }
    (w, h, out)
}

pub fn run(rom: &Rom, t: &Tables, decoded: &BTreeMap<u32, Vec<u8>>) -> std::io::Result<String> {
    // 1. Scènes : liste de couches valides + palette compressée de 1 024 octets.
    let mut scenes: Vec<(u32, u32, u32, Vec<u32>)> = Vec::new();
    let mut palettes: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for off in (0x37B40..0x25_0000usize).step_by(4) {
        let s = ROM_BASE + off as u32;
        let (list, pal) = (rd(rom, s), rd(rom, s + 8));
        let count = (rom.u32(off + 0x10).unwrap_or(0) & 0xFFFF) as u32;
        if !(1..=4).contains(&count) || !(0x0803_7B40..0x0840_0000).contains(&list) || rd(rom, pal) != 0x400 {
            continue;
        }
        let layers: Vec<u32> = (0..count).map(|i| rd(rom, list + i * 0x20)).collect();
        if layers.iter().any(|&r| record(rom, r).is_none()) {
            continue;
        }
        if !palettes.contains_key(&pal) {
            match packed::decompress(t, &rom.data[(pal - ROM_BASE) as usize..]) {
                Some(u) if u.data.len() == 1024 => {
                    palettes.insert(pal, u.data);
                }
                _ => continue,
            }
        }
        scenes.push((s, list, pal, layers));
    }

    // 2. Rendu de chaque couche avec la palette de sa scène.
    std::fs::create_dir_all("extract/niveaux")?;
    let mut rep = String::new();
    let mut done: BTreeSet<u32> = BTreeSet::new();
    let mut bad_maps = 0;
    let mut composed = 0;
    let mut sprites = objects::Sprites::new(rom, t);
    let mut obj_rep = String::new();
    let (mut n_objs, mut n_drawn, mut n_missing, mut with_objs) = (0, 0, 0, 0);
    // palette (16 couleurs) de chaque définition, prise dans la première scène où elle apparaît
    let mut def_pals: BTreeMap<u32, Vec<[u8; 3]>> = BTreeMap::new();
    let mut type_fields: BTreeMap<u16, BTreeMap<u32, behaviour::Use>> = BTreeMap::new();
    let mut type_count: BTreeMap<u16, usize> = BTreeMap::new();
    let mut bank0: BTreeMap<Vec<[u8; 3]>, usize> = BTreeMap::new();
    for (s, list, pal, layers) in &scenes {
        let colors = rgb(&palettes[pal]);
        let mut rendered: Vec<Layer> = Vec::new();
        for (i, &ra) in layers.iter().enumerate() {
            let r = record(rom, ra).unwrap();
            let (Some(tiles), Some(map)) = (decoded.get(&r.tiles), decoded.get(&r.map)) else { continue };
            let Some(grid) = decode_map(map, r.w, r.h) else {
                bad_maps += 1;
                continue;
            };
            let px = render(&grid, r.w, r.h, tiles);
            let name = format!("extract/niveaux/scene_{s:08X}_couche{i}_{ra:08X}.png");
            std::fs::write(&name, png::indexed(r.w * 8, r.h * 8, &px, &colors[..256]))?;
            done.insert(ra);
            let e = list + i as u32 * 0x20;
            rendered.push(Layer {
                index: i,
                entry: e,
                w: r.w * 8,
                h: r.h * 8,
                fx: rd(rom, e + 0x10) & 0xFFFF,
                fy: rd(rom, e + 0x10) >> 16,
                prio: rd(rom, e + 0x14) & 3,
                px,
            });
            writeln!(rep, "  scène {s:08X} couche {i} : décor {ra:08X} {}x{} tuiles, palette {pal:08X}", r.w, r.h).unwrap();
        }
        if rendered.is_empty() {
            continue;
        }
        let (w, h, px) = compose(&rendered);
        if rendered.len() > 1 {
            std::fs::write(format!("extract/niveaux/scene_{s:08X}_complete.png"), png::indexed(w, h, &px, &colors[..256]))?;
            composed += 1;
        }
        // 3. Objets placés : coordonnées de chaque couche ramenées à la couche de référence.
        let r = reference(&rendered);
        let mut objs = Vec::new();
        for l in &rendered {
            let scale = |v: i32, f: u32, fref: u32| if f == 0 || fref == 0 { v } else { v * fref as i32 / f as i32 };
            for o in objects::list(rom, l.entry) {
                let ty = objects::def_type(rom, o.def);
                let fields = type_fields.entry(ty).or_insert_with(|| behaviour::analyse(rom, behaviour::function(rom, ty)));
                writeln!(
                    obj_rep,
                    "{s:08X} couche {} : {:08X} type {ty:3} x={:5} y={:5} param {:08X} {:08X} {:08X}  {}",
                    l.index,
                    o.def,
                    o.x,
                    o.y,
                    o.params[0],
                    o.params[1],
                    o.params[2],
                    behaviour::describe(fields, &o.params, behaviour::function(rom, ty))
                )
                .unwrap();
                *type_count.entry(ty).or_insert(0usize) += 1;
                let flags = (o.params[2] >> 8) & 3;
                objs.push((scale(o.x, l.fx, r.fx), scale(o.y, l.fy, r.fy), o.def, flags));
            }
        }
        if !objs.is_empty() {
            let mut img: Vec<u8> = px.iter().flat_map(|&c| colors[c as usize]).collect();
            for &(_, _, d, _) in &objs {
                def_pals.entry(d).or_insert_with(|| {
                    let b = objects::palette_bank(rom, *s, objects::def_type(rom, d)) as usize;
                    colors[256 + b * 16..256 + b * 16 + 16].to_vec()
                });
            }
            *bank0.entry(colors[256..272].to_vec()).or_default() += 1;
            let (ok, miss) = objects::draw(rom, &mut sprites, *s, &objs, &colors[256..], &mut img, w, h);
            std::fs::write(format!("extract/niveaux/scene_{s:08X}_objets.png"), png::rgb(w, h, &img))?;
            n_objs += objs.len();
            n_drawn += ok;
            n_missing += miss;
            with_objs += 1;
        }
    }
    let mut head = format!(
        "Scènes trouvées : {}, palettes : {}, décors rendus en couleur : {} (cartes illisibles : {bad_maps}), scènes superposées : {composed}\n",
        scenes.len(),
        palettes.len(),
        done.len()
    );
    writeln!(
        head,
        "Objets placés : {n_objs} dans {with_objs} scènes, {n_drawn} dessinés avec leur sprite, {n_missing} marqués d'une croix -> scene_*_objets.png"
    )
    .unwrap();
    // Le joueur n'apparaît dans aucune liste d'objets et utilise la banque OBJ 0 (vérifié en
    // essayant les 16 banques). On prend la banque 0 la plus fréquente parmi les scènes à objets.
    if let Some((pal, _)) = bank0.iter().max_by_key(|e| *e.1) {
        def_pals.entry(objects::PLAYER).or_insert_with(|| pal.clone());
    }
    head.push_str(&objects::export_sheets(&mut sprites, &def_pals)?);
    std::fs::write("extract/niveaux/index.txt", &rep)?;
    std::fs::write("extract/niveaux/objets.txt", &obj_rep)?;
    // Types d'objets : fonction de comportement et paramètres qu'elle lit.
    let mut types = String::from("type  objets  comportement  paramètres lus
");
    let (mut with_fn, mut with_params) = (0, 0);
    for (ty, fields) in &type_fields {
        let f = behaviour::function(rom, *ty);
        with_fn += (f != 0) as usize;
        with_params += (!fields.is_empty()) as usize;
        let fs = if f == 0 { "-".to_string() } else { format!("{:08X}", f & !1) };
        writeln!(types, "{ty:4}  {:6}  {fs:>12}  {}", type_count[ty], behaviour::summary(fields)).unwrap();
    }
    std::fs::write("extract/niveaux/types.txt", types)?;
    writeln!(
        head,
        "Paramètres d'objets : {} types placés, {with_fn} avec un comportement, {with_params} dont le code lit des paramètres -> types.txt, objets.txt",
        type_fields.len()
    )
    .unwrap();
    head.push_str("Détail dans extract/niveaux/index.txt\n");
    Ok(head)
}

/// Table des niveaux (12 octets par état de jeu : scène, n° de musique, ?).
pub const LEVEL_TABLE: u32 = 0x0835_1868;
pub const LEVEL_COUNT: u32 = 60;

/// Couche prête à afficher (pour la visionneuse).
pub struct ViewLayer {
    pub index: usize,
    pub w: usize,
    pub h: usize,
    /// Facteurs de défilement 8.8 (0 = fixe à l'écran).
    pub fx: u32,
    pub fy: u32,
    pub prio: u32,
    /// Index de couleur 0..255 (0 = transparent).
    pub px: Vec<u8>,
    pub objects: Vec<objects::Object>,
}

/// Scène chargée : 512 couleurs (BG puis OBJ) et couches.
pub struct SceneView {
    pub addr: u32,
    pub colors: Vec<[u8; 3]>,
    pub layers: Vec<ViewLayer>,
}

/// Décode une ressource du bloc compressé (avec cache).
fn unpack<'c>(rom: &Rom, t: &Tables, cache: &'c mut BTreeMap<u32, Option<Vec<u8>>>, a: u32) -> Option<&'c Vec<u8>> {
    cache
        .entry(a)
        .or_insert_with(|| a.checked_sub(ROM_BASE).and_then(|o| packed::decompress(t, rom.data.get(o as usize..)?)).map(|u| u.data))
        .as_ref()
}

impl SceneView {
    /// Charge la scène `s` (structure de 0x18 octets) ; les couches illisibles sont ignorées.
    pub fn load(rom: &Rom, t: &Tables, cache: &mut BTreeMap<u32, Option<Vec<u8>>>, s: u32) -> Option<SceneView> {
        let (list, pal) = (rd(rom, s), rd(rom, s + 8));
        let count = rd(rom, s + 0x10) & 0xFFFF;
        if !(1..=8).contains(&count) || !(0x0800_0000..0x0840_0000).contains(&list) {
            return None;
        }
        let colors = rgb(unpack(rom, t, cache, pal).filter(|p| p.len() >= 1024)?);
        let mut layers = Vec::new();
        for i in 0..count {
            let e = list + i * 0x20;
            let Some(r) = record(rom, rd(rom, e)) else { continue };
            let Some(tiles) = unpack(rom, t, cache, r.tiles).cloned() else { continue };
            let Some(grid) = unpack(rom, t, cache, r.map).and_then(|m| decode_map(m, r.w, r.h)) else { continue };
            layers.push(ViewLayer {
                index: i as usize,
                w: r.w * 8,
                h: r.h * 8,
                fx: rd(rom, e + 0x10) & 0xFFFF,
                fy: rd(rom, e + 0x10) >> 16,
                prio: rd(rom, e + 0x14) & 3,
                px: render(&grid, r.w, r.h, &tiles),
                objects: objects::list(rom, e),
            });
        }
        (!layers.is_empty()).then_some(SceneView { addr: s, colors, layers })
    }

    /// Couche de référence (terrain de jeu) : la plus grande.
    pub fn reference(&self) -> &ViewLayer {
        self.layers.iter().max_by_key(|l| (l.w * l.h, l.fx)).unwrap()
    }
}
