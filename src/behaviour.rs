//! Paramètres des objets : quels champs le code de comportement de chaque type lit.
//!
//! À la création (`sub_080299E8`, 0x08029B64), l'enregistrement de 20 octets de l'objet est
//! recopié tel quel dans l'entité en `+0x08..+0x1B` : définition `+0x08`, position `+0x0C`,
//! paramètres `+0x10`, `+0x14`, `+0x18`. La fonction de comportement vient de la table
//! [`TABLE`] indexée par le n° de type (définition `+0x0A`) et est rangée en entité `+0x28`
//! (0 = objet sans comportement propre).
//!
//! Le code ne lit ensuite que des morceaux de ces mots. Disposition commune, déduite des
//! lectures (toutes les fonctions suivent la même convention) :
//!
//! | entité | paramètre        | usage                                                     |
//! |--------|------------------|-----------------------------------------------------------|
//! | +0x10  | p1 bits 0..15    | non lu par les comportements analysés                     |
//! | +0x12  | p1 bits 16..31   | mode / variante / numéro (comparé à des constantes)       |
//! | +0x14  | p2 bits 0..7     | délai en images (0 => valeur par défaut, souvent 60)      |
//! | +0x15  | p2 bits 8..15    | distance vers le haut, en tuiles de 8 px                  |
//! | +0x16  | p2 bits 16..23   | distance vers la gauche, en tuiles                        |
//! | +0x17  | p2 bits 24..31   | distance vers le bas, en tuiles                           |
//! | +0x18  | p3 bits 0..7     | distance vers la droite, en tuiles                        |
//! | +0x19  | p3 bits 8..15    | drapeaux : 1 = miroir H, 2 = miroir V (code générique,    |
//! |        |                  | `sub_08029CD8`), 0x20/0x40/0x80 = options du type         |
//!
//! Les distances sont lues puis décalées de 11 bits (× 8 px en virgule fixe 24.8) et
//! ajoutées ou retranchées à la position de départ (`+0x40` x, `+0x44` y) : ce sont les
//! bornes d'un trajet ou d'une zone de détection.

use crate::rom::{Rom, ROM_BASE};
use crate::thumb::{self, Flow};
use std::collections::{BTreeMap, BTreeSet};

/// Table des fonctions de comportement, indexée par n° de type.
pub const TABLE: u32 = 0x083D_B2D0;

#[derive(Default, Clone, Debug)]
pub struct Use {
    /// Largeur de lecture en octets (la plus petite vue : un `ldr` sur +0x14 sert parfois
    /// à tester plusieurs octets à la fois, le champ lui-même reste un octet).
    pub width: u8,
    /// Valeur décalée de 11 bits (distance en tuiles).
    pub distance: bool,
    /// Valeur par défaut quand le champ vaut 0 (`cmp rX, #0 ; bne ; mov rX, #d`).
    pub default: Option<u32>,
    /// Bits testés (`mov rY, #m ; and rY, rX`).
    pub masks: u32,
    /// Constantes auxquelles le champ est comparé.
    pub compared: BTreeSet<u32>,
    /// Le code écrit ce champ avant de le lire : ce n'est pas un paramètre mais une
    /// variable de l'objet (la valeur du niveau est ignorée).
    pub overwritten: bool,
}

pub fn function(rom: &Rom, type_id: u16) -> u32 {
    word(rom, TABLE + 4 * type_id as u32)
}

fn word(rom: &Rom, a: u32) -> u32 {
    a.checked_sub(ROM_BASE).and_then(|o| rom.u32(o as usize)).unwrap_or(0)
}
fn half(rom: &Rom, a: u32) -> Option<u16> {
    a.checked_sub(ROM_BASE).and_then(|o| rom.u16(o as usize))
}
fn is_code(a: u32) -> bool {
    (0x0800_0000..0x0804_0000).contains(&a)
}

/// État suivi : décalage par rapport à l'entité, ou valeur constante, pour r0..r15.
#[derive(Clone, Default)]
struct Regs {
    ent: [Option<u32>; 16],
    val: [Option<u32>; 16],
    /// Dernier littéral chargé (base d'une éventuelle table de `switch`).
    last_lit: Option<u32>,
}

impl Regs {
    fn kill(&mut self, r: usize) {
        self.ent[r] = None;
        self.val[r] = None;
    }
}

/// Champs de paramètres (+0x10..+0x1B) lus par le comportement `f` et ses suites.
pub fn analyse(rom: &Rom, f: u32) -> BTreeMap<u32, Use> {
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut deep_writes = BTreeMap::new();
    if f != 0 {
        walk(rom, f & !1, 0, &mut seen, &mut out, &mut deep_writes);
    }
    // champs seulement écrits plus loin (jamais lus) : variables, pas des paramètres
    for (off, width) in deep_writes {
        store(&mut out, off, width);
    }
    out
}

fn walk(
    rom: &Rom,
    f: u32,
    depth: u32,
    seen: &mut BTreeSet<u32>,
    out: &mut BTreeMap<u32, Use>,
    deep_writes: &mut BTreeMap<u32, u8>,
) {
    if depth > 6 || !is_code(f) || !seen.insert(f) {
        return;
    }
    let mut start = Regs::default();
    start.ent[0] = Some(0);
    let mut todo = vec![(f, start)];
    let mut visited = BTreeSet::new();
    let mut follow: Vec<u32> = Vec::new();
    while let Some((mut a, mut r)) = todo.pop() {
        for _ in 0..4000 {
            if !is_code(a) || !visited.insert(a) {
                break;
            }
            let Some(h) = half(rom, a) else { break };
            let ins = thumb::decode(h, half(rom, a + 2), a);
            // appel avec r0 = entité : à vérifier avant que `step` n'efface r0
            if let (Flow::Call(t), Some(0)) = (ins.flow, r.ent[0]) {
                follow.push(t);
            }
            // écritures : seule la fonction d'initialisation (profondeur 0) écrase un paramètre
            let mut writes = Vec::new();
            step(rom, a, h, &ins, &mut r, out, &mut follow, &mut writes);
            for (off, w) in writes {
                if depth == 0 {
                    store(out, off, w);
                } else {
                    deep_writes.entry(off).or_insert(w);
                }
            }
            match ins.flow {
                Flow::Next | Flow::Swi(_) => a += ins.size,
                Flow::Call(_) => {
                    for k in 0..4 {
                        r.kill(k);
                    }
                    a += ins.size;
                }
                Flow::CondJump(t) => {
                    todo.push((t, r.clone()));
                    a += ins.size;
                }
                Flow::Jump(t) => a = t,
                // `mov pc, rX` : table de sauts d'un `switch` (même heuristique que analysis.rs)
                Flow::IndirectJump(_) => {
                    if let Some(tab) = r.last_lit {
                        for i in 0..256 {
                            let v = word(rom, tab + 4 * i);
                            if !is_code(v) || v.abs_diff(f) > 0x2_0000 {
                                break;
                            }
                            todo.push((v & !1, r.clone()));
                        }
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    for t in follow {
        walk(rom, t & !1, depth + 1, seen, out, deep_writes);
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    rom: &Rom,
    a: u32,
    h: u16,
    ins: &thumb::TInsn,
    r: &mut Regs,
    out: &mut BTreeMap<u32, Use>,
    follow: &mut Vec<u32>,
    writes: &mut Vec<(u32, u8)>,
) {
    let h = h as u32;
    let (rd, rs) = ((h & 7) as usize, ((h >> 3) & 7) as usize);
    let mut record = |off: u32, width: u8, dst: usize| {
        if !(0x10..0x1C).contains(&off) {
            return;
        }
        let u = out.entry(off).or_insert_with(Use::default);
        u.width = if u.width == 0 { width } else { u.width.min(width) };
        classify(rom, a + 2, dst, u);
    };
    match h >> 11 {
        // add/sub rd, rs, #imm3 | rn
        0b00011 => {
            let (imm, sub, n) = (h & 0x400 != 0, h & 0x200 != 0, ((h >> 6) & 7) as u32);
            let e = r.ent[rs];
            r.kill(rd);
            if imm && !sub {
                r.ent[rd] = e.map(|e| e + n);
            }
        }
        // mov/cmp/add/sub rd, #imm8
        0b00100..=0b00111 => {
            let (op, d, v) = ((h >> 11) & 3, ((h >> 8) & 7) as usize, h & 0xFF);
            match op {
                0 => {
                    r.kill(d);
                    r.val[d] = Some(v);
                }
                2 => {
                    r.ent[d] = r.ent[d].map(|e| e + v);
                    r.val[d] = r.val[d].map(|x| x + v);
                }
                3 => r.kill(d),
                _ => {}
            }
        }
        // ldr rd, [pc, #imm]
        0b01001 => {
            let d = ((h >> 8) & 7) as usize;
            r.kill(d);
            if let Some((_, lit)) = ins.literal {
                r.val[d] = Some(word(rom, lit));
                r.last_lit = Some(word(rom, lit));
            }
        }
        // ldr/str rd, [rb, #imm5] (mot ou octet)
        0b01100..=0b01111 => {
            let (byte, load) = (h & 0x1000 != 0, h & 0x800 != 0);
            let off = ((h >> 6) & 31) * if byte { 1 } else { 4 };
            if let Some(e) = r.ent[rs] {
                if load {
                    record(e + off, if byte { 1 } else { 4 }, rd);
                } else if (0x10..0x1C).contains(&(e + off)) {
                    writes.push((e + off, if byte { 1 } else { 4 }));
                } else if !byte && (e + off == 0x28 || e + off == 0x2C) {
                    if let Some(v) = r.val[rd] {
                        follow.push(v);
                    }
                }
            }
            if load {
                r.kill(rd);
            }
        }
        // ldrh/strh rd, [rb, #imm5]
        0b10000 | 0b10001 => {
            let load = h & 0x800 != 0;
            match (load, r.ent[rs]) {
                (true, Some(e)) => record(e + ((h >> 6) & 31) * 2, 2, rd),
                (false, Some(e)) if (0x10..0x1C).contains(&(e + ((h >> 6) & 31) * 2)) => {
                    writes.push((e + ((h >> 6) & 31) * 2, 2))
                }
                _ => {}
            }
            if load {
                r.kill(rd);
            }
        }
        // ldr*/str* rd, [rb, ro]
        0b01010 | 0b01011 => {
            let ro = ((h >> 6) & 7) as usize;
            let op = (h >> 9) & 7;
            if op >= 3 {
                if let (Some(e), Some(v)) = (r.ent[rs], r.val[ro]) {
                    let w = match op {
                        4 => 4,
                        5 | 7 => 2,
                        _ => 1,
                    };
                    record(e + v, w, rd);
                }
                r.kill(rd);
            }
        }
        // opérations sur registres hauts : add / mov
        0b01000 if (h >> 10) & 1 == 1 => {
            let op = (h >> 8) & 3;
            let d = ((h & 7) | ((h >> 4) & 8)) as usize;
            let s = ((h >> 3) & 15) as usize;
            match op {
                2 => {
                    r.ent[d] = r.ent[s];
                    r.val[d] = r.val[s];
                }
                0 => {
                    let (e, v) = (r.ent[d], r.val[s]);
                    r.kill(d);
                    if let (Some(e), Some(v)) = (e, v) {
                        r.ent[d] = Some(e + v);
                    }
                }
                _ => {}
            }
        }
        _ => {
            for k in 0..16 {
                if ins.clobber & (1 << k) != 0 {
                    r.kill(k);
                }
            }
        }
    }
}

/// Écriture d'un champ : s'il n'a encore jamais été lu, c'est une variable, pas un paramètre.
fn store(out: &mut BTreeMap<u32, Use>, off: u32, width: u8) {
    out.entry(off).or_insert_with(|| Use { width, overwritten: true, ..Use::default() });
}

/// Regarde les quelques instructions qui suivent la lecture d'un champ dans `reg`.
fn classify(rom: &Rom, mut a: u32, reg: usize, u: &mut Use) {
    let mut mask_reg: Option<(usize, u32)> = None;
    for k in 0..4 {
        let Some(h) = half(rom, a) else { return };
        let h = h as u32;
        let (rd, rs) = ((h & 7) as usize, ((h >> 3) & 7) as usize);
        // lsl rd, reg, #11
        if h >> 11 == 0 && (h >> 6) & 31 == 11 && rs == reg {
            u.distance = true;
        }
        // mov rX, #m (masque)
        if h >> 11 == 0b00100 {
            mask_reg = Some((((h >> 8) & 7) as usize, h & 0xFF));
        }
        // and rX, reg
        if h >> 6 == 0b0100000000 && rs == reg {
            if let Some((m, v)) = mask_reg {
                if m == rd {
                    u.masks |= v;
                }
            }
        }
        // cmp reg, #imm
        if h >> 11 == 0b00101 && ((h >> 8) & 7) as usize == reg {
            let v = h & 0xFF;
            u.compared.insert(v);
            // cmp #0 ; bne/beq ; mov reg, #d  => valeur par défaut
            if v == 0 && k < 3 {
                if let Some(m) = half(rom, a + 4) {
                    let m = m as u32;
                    if m >> 11 == 0b00100 && ((m >> 8) & 7) as usize == reg {
                        u.default = Some(m & 0xFF);
                    }
                }
            }
        }
        a += 2;
    }
}

/// Nom lisible d'un champ selon son décalage et son usage.
pub fn field_name(off: u32, u: &Use) -> String {
    match (off, u.distance) {
        (0x15, true) => "haut".into(),
        (0x16, true) => "gauche".into(),
        (0x17, true) => "bas".into(),
        (0x18, true) => "droite".into(),
        (_, true) => format!("distance+{off:02X}"),
        (0x12, _) => "mode".into(),
        (0x14, _) if u.default.is_some() => "délai".into(),
        (0x19, _) => "options".into(),
        _ => format!("+{off:02X}"),
    }
}

/// Valeur d'un champ dans les trois mots de paramètres (décalage entité 0x10..0x1B).
pub fn field_value(params: &[u32; 3], off: u32, width: u8) -> u32 {
    let bytes: Vec<u8> = params.iter().flat_map(|p| p.to_le_bytes()).collect();
    let i = (off - 0x10) as usize;
    (0..width as usize).filter_map(|k| bytes.get(i + k)).rev().fold(0, |acc, &b| acc << 8 | b as u32)
}

/// Description d'un objet : champs lus par son comportement, avec leur valeur.
pub fn describe(fields: &BTreeMap<u32, Use>, params: &[u32; 3], f: u32) -> String {
    let mut parts = Vec::new();
    let mode_desc = mode_meaning(f, params[0] >> 16);
    let flags = (params[2] >> 8) & 3;
    if flags & 1 != 0 {
        parts.push("miroir H".to_string());
    }
    if flags & 2 != 0 {
        parts.push("miroir V".to_string());
    }
    for (&off, u) in fields.iter().filter(|(_, u)| !u.overwritten) {
        let v = field_value(params, off, u.width);
        let name = field_name(off, u);
        let s = match (off, u) {
            (0x12, _) if mode_desc.is_some() => format!("{name}={v} [{}]", mode_desc.as_ref().unwrap()),
            (0x19, u) => {
                let m = v & u.masks;
                if m == 0 {
                    continue;
                }
                format!("{name}={m:#04x}")
            }
            (_, u) if u.distance => format!("{name}={v}t"),
            (_, u) if u.default.is_some() && v == 0 => format!("{name}={} (défaut)", u.default.unwrap()),
            _ => format!("{name}={v}"),
        };
        parts.push(s);
    }
    // octets non nuls que le comportement ne lit pas (hors miroirs, gérés à part)
    let bytes: Vec<u8> = params.iter().flat_map(|p| p.to_le_bytes()).collect();
    let covered = |i: u32| fields.iter().any(|(&o, u)| (o..o + u.width as u32).contains(&(0x10 + i)));
    let mut unread = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        let b = if i == 9 { b & !3 & !fields.get(&0x19).map_or(0, |u| u.masks as u8) } else { b };
        if b != 0 && !covered(i as u32) {
            unread.push(format!("+{:02X}={b:#04x}", 0x10 + i));
        }
    }
    if !unread.is_empty() {
        parts.push(format!("(non lus : {})", unread.join(" ")));
    }
    parts.join(" ")
}

/// Résumé d'un type : quels champs sont lus et comment.
pub fn summary(fields: &BTreeMap<u32, Use>) -> String {
    let mut parts = Vec::new();
    for (&off, u) in fields {
        if u.overwritten {
            parts.push(format!("+{off:02X} écrasé par le code ({} o)", u.width));
            continue;
        }
        let mut s = format!("{} (+{off:02X}, {} o", field_name(off, u), u.width);
        if u.distance {
            s.push_str(", tuiles");
        }
        if let Some(d) = u.default {
            s.push_str(&format!(", défaut {d}"));
        }
        if u.masks != 0 {
            s.push_str(&format!(", bits {:#04x}", u.masks));
        }
        let cmp: Vec<String> = u.compared.iter().filter(|&&c| c != 0).map(|c| c.to_string()).collect();
        if !cmp.is_empty() {
            s.push_str(&format!(", comparé à {}", cmp.join("/")));
        }
        s.push(')');
        parts.push(s);
    }
    if parts.is_empty() { "aucun paramètre lu".into() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valeurs_de_champs() {
        // archer : p1 = 0x10000 (mode 1), p2 = 0x5A (délai 90), p3 = 0x100 (miroir H)
        let p = [0x0001_0000, 0x0000_005A, 0x0000_0100];
        assert_eq!(field_value(&p, 0x12, 2), 1);
        assert_eq!(field_value(&p, 0x14, 1), 0x5A);
        assert_eq!(field_value(&p, 0x19, 1), 1);
        // plateforme : p2 = 0x000A003C -> délai 60, gauche 10 ; p3 = 0x10A -> droite 10
        let p = [0, 0x000A_003C, 0x0000_010A];
        assert_eq!(field_value(&p, 0x16, 1), 10);
        assert_eq!(field_value(&p, 0x18, 1), 10);
    }

    #[test]
    fn sens_des_modes() {
        assert!(mode_meaning(0x0802_0D15, 2).unwrap().contains("rebondissant"));
        assert!(mode_meaning(0x0800_C129, 12).unwrap().starts_with("garde"));
        assert!(mode_meaning(0x0800_C129, 0).unwrap().starts_with("patrouille"));
        assert_eq!(mode_meaning(0x0800_7CC5, 3).unwrap(), "lieu n°3 de la carte (curseur enregistré en 0x03000080+8×3)");
        assert!(mode_meaning(0x0800_0001, 0).is_none());
    }
}

/// Sens du mode (`+0x12`) pour les comportements lus à la main.
/// Clé : fonction de comportement (sans le bit Thumb). Preuves dans NOTES.md.
pub fn mode_meaning(f: u32, m: u32) -> Option<String> {
    Some(match f & !1 {
        // carte du monde : lieux (villes, château…)
        0x0800_7CC4 => match m {
            0 => "décor de la carte (aucun lieu enregistré)".into(),
            _ => format!("lieu n°{m} de la carte (curseur enregistré en 0x03000080+8×{m})"),
        },
        0x0800_7C74 => format!("flèche du lieu n°{m}, clignote si le lieu est ouvert (bit {} de 0x030000BC)", m.wrapping_sub(1)),
        // soldat-rat (421) et soldat à bouclier (422)
        0x0800_C128 => match m {
            _ if m & 1 == 1 => "variante spéciale (sub_0800D528)".into(),
            22 => "variante sub_0800D57C".into(),
            10..=19 => "garde : immobile, se retourne toutes les `délai` images, attaque si le joueur approche".into(),
            20..=29 => "patrouille entre gauche et droite, palette 9".into(),
            _ => "patrouille entre gauche et droite".into(),
        },
        // canon
        0x0802_0D14 => match m {
            2 => "boulet rebondissant (2,5 px/image)".into(),
            3 => "boulet rapide (3,5 px/image)".into(),
            _ => "boulet normal (2,5 px/image)".into(),
        },
        // archer
        0x0802_0D3C => match m {
            0 => "tir presque horizontal (3 px/image)".into(),
            1 => "tir à 45° vers le haut".into(),
            _ => "autre comportement (sub_0800B928)".into(),
        },
        // tonneau et objet 189 : n° d'objet unique (drapeau persistant)
        0x0802_0C78 => format!("objet unique n°{m} : si le drapeau 0x03000314+{} est mis, remplacé par 0x08071600", 2 * m.wrapping_sub(1)),
        0x0802_0548 => format!("objet unique n°{m} : disparaît si le drapeau 0x03000314+{} est mis", 2 * m.wrapping_sub(1)),
        // portes
        0x0802_0D00 => match m {
            12 => "porte vers la destination 12 (sortie spéciale sub_08001DE4)".into(),
            _ => format!("porte vers la destination {m}"),
        },
        // oiseau
        0x0800_ED88 => match m {
            1 => "caché et inactif".into(),
            _ => "actif (sub_0800EB6C)".into(),
        },
        // taupe
        0x0802_1D94 => match m {
            0 => "comportement A (animation 4)".into(),
            _ => "comportement B (animation 6)".into(),
        },
        _ => return None,
    })
}
