//! Icônes du portage PC : l'indication « appuie vite sur A » de la course du Royaume de
//! l'air (flèche rouge au-dessus d'un bouton qui s'enfonce) est remplacée par une touche
//! de clavier qui s'enfonce au même rythme, marquée de la touche attribuée au bouton A
//! (« A » avec les touches d'origine, voir `touche_icone`).
//!
//! Le jeu envoie en VRAM, toutes les 4 images, l'une des 4 images de l'animation : un
//! sprite 16×32 (flèche + bouton) et un petit sprite (bout droit du bouton). Avant chaque
//! image, on reconnaît le sprite 16×32 à l'empreinte de ses tuiles et on le remplace ; le
//! petit sprite du bout du bouton (même palette, en bas à droite) est masqué dans l'OAM.
//! VRAM et OAM ne sont lues que par l'affichage.

use crate::gba::bus::Bus;

/// Empreintes (FNV-1a 64 bits) des 4 images du sprite 16×32 et du bout 8×8 ; vrai si
/// l'image montre le bouton enfoncé.
const GROS: [(u64, bool); 4] = [
    (0x42B7_D535_998B_18E5, true),
    (0xC2B7_28D2_65D2_1517, false),
    (0x8964_E1BC_679A_40C7, false),
    (0x6711_B3B1_DE93_D17B, true),
];

/// Touche relevée, sans inscription (16×32, couleurs de la palette du sprite :
/// 1 contour noir, 8 crème, 14 reflet, 7 côté gris-violet). L'inscription est écrite
/// en couleur 1 sur la face (lignes 3 à 11, colonnes 2 à 13).
const RELEVEE: [&str; 18] = [
    "..111111111111..",
    ".1EEEEEEEEEEEE1.",
    "1E888888888888E1",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1888888888888881",
    "1777777777777771",
    "1777777777777771",
    "1777777777777771",
    ".17777777777771.",
    "..111111111111..",
];

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01B3))
}

/// Petite police 3×5 (lettres et chiffres) pour les inscriptions de 2 ou 3 caractères.
fn petit(c: char) -> [u8; 5] {
    const L: [[u8; 5]; 26] = [
        [2, 5, 7, 5, 5],
        [6, 5, 6, 5, 6],
        [3, 4, 4, 4, 3],
        [6, 5, 5, 5, 6],
        [7, 4, 6, 4, 7],
        [7, 4, 6, 4, 4],
        [3, 4, 5, 5, 3],
        [5, 5, 7, 5, 5],
        [7, 2, 2, 2, 7],
        [1, 1, 1, 5, 2],
        [5, 5, 6, 5, 5],
        [4, 4, 4, 4, 7],
        [5, 7, 7, 5, 5],
        [6, 5, 5, 5, 5],
        [2, 5, 5, 5, 2],
        [6, 5, 6, 4, 4],
        [2, 5, 5, 6, 3],
        [6, 5, 6, 5, 5],
        [3, 4, 2, 1, 6],
        [7, 2, 2, 2, 2],
        [5, 5, 5, 5, 7],
        [5, 5, 5, 5, 2],
        [5, 5, 7, 7, 5],
        [5, 5, 2, 5, 5],
        [5, 5, 2, 2, 2],
        [7, 1, 2, 4, 7],
    ];
    const D: [[u8; 5]; 10] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [6, 1, 2, 4, 7],
        [6, 1, 2, 1, 6],
        [5, 5, 7, 1, 1],
        [7, 4, 6, 1, 6],
        [3, 4, 6, 5, 2],
        [7, 1, 2, 2, 2],
        [2, 5, 2, 5, 2],
        [2, 5, 3, 1, 6],
    ];
    match c {
        'A'..='Z' => L[c as usize - 'A' as usize],
        '0'..='9' => D[c as usize - '0' as usize],
        _ => [0; 5],
    }
}

/// Touche montrée sur l'icône parmi celles du bouton A : « A » si elle en fait partie
/// (touches d'origine), sinon la première ; « A » si le bouton n'a plus de touche.
pub fn touche_icone(touches_a: &[u8]) -> u8 {
    if touches_a.contains(&b'A') {
        b'A'
    } else {
        touches_a.first().copied().unwrap_or(b'A')
    }
}

/// Inscription de la touche : un caractère (police 5×7 du menu) ou une abréviation de
/// 2 ou 3 caractères (police 3×5).
fn inscription(vk: u8) -> String {
    match vk {
        0x60..=0x69 => return ((b'0' + vk - 0x60) as char).to_string(),
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        _ => {}
    }
    let court = match vk {
        0x08 => "RET",
        0x0D => "ENT",
        0x13 => "PSE",
        0x14 => "VMA",
        0x20 => "ESP",
        0x21 => "PGH",
        0x22 => "PGB",
        0x23 => "FIN",
        0x24 => "DEB",
        0x2C => "IMP",
        0x2D => "INS",
        0x2E => "SUP",
        0x5D => "MNU",
        0x90 => "NUM",
        0x91 => "DEF",
        0xA0 | 0xA1 => "MAJ",
        0xA2 | 0xA3 => "CTL",
        0xA4 => "ALT",
        0xA5 => "AGR",
        _ => "",
    };
    if !court.is_empty() {
        return court.into();
    }
    let nom = crate::menu::key_name(vk);
    if nom.chars().count() == 1 {
        nom
    } else {
        format!("{vk:02X}")
    }
}

/// Les 32 lignes de la touche (enfoncée : face descendue de 2 pixels, côté plus court).
fn dessin(enfoncee: bool, vk: u8) -> [[u8; 16]; 32] {
    let mut img = [[0u8; 16]; 32];
    let lignes: Vec<&str> = if enfoncee {
        // contour, face (13 lignes), un seul rang de côté, contour
        RELEVEE[..13].iter().chain(&RELEVEE[15..]).copied().collect()
    } else {
        RELEVEE.to_vec()
    };
    let haut = 26 - lignes.len();
    for (y, l) in lignes.iter().enumerate() {
        for (x, c) in l.chars().enumerate() {
            img[haut + y][x] = c.to_digit(16).unwrap_or(8) as u8;
        }
    }
    let t: Vec<char> = inscription(vk).chars().collect();
    if t.len() == 1 {
        // 5×7 centré sur la face (colonnes 5 à 9, lignes 4 à 10)
        for (r, bits) in crate::menu::glyph(t[0]).iter().enumerate() {
            for c in 0..5 {
                if bits & (0x10 >> c) != 0 {
                    img[haut + 4 + r][5 + c] = 1;
                }
            }
        }
    } else {
        let larg = 4 * t.len() - 1;
        let x0 = 2 + (12 - larg) / 2;
        for (i, &ch) in t.iter().enumerate() {
            for (r, bits) in petit(ch).iter().enumerate() {
                for c in 0..3 {
                    if bits & (4 >> c) != 0 {
                        img[haut + 5 + r][x0 + 4 * i + c] = 1;
                    }
                }
            }
        }
    }
    img
}

/// Tuiles 4 bits (mode 1D) d'un sprite 16×32.
fn tuiles(img: &[[u8; 16]; 32]) -> [u8; 256] {
    let mut out = [0u8; 256];
    for y in 0..32 {
        for x in 0..16 {
            let t = (y / 8) * 2 + x / 8;
            let o = t * 32 + (y % 8) * 4 + (x % 8) / 2;
            out[o] |= img[y][x] << (4 * (x & 1));
        }
    }
    out
}

/// Empreintes de nos deux touches (relevée, enfoncée) une fois écrites en VRAM : le jeu
/// ne renvoie l'image qu'une fois sur 4, la touche reste alors en place.
fn nos_empreintes(vk: u8) -> [u64; 2] {
    static E: std::sync::Mutex<Option<(u8, [u64; 2])>> = std::sync::Mutex::new(None);
    let mut e = E.lock().unwrap();
    match *e {
        Some((v, h)) if v == vk => h,
        _ => {
            let h = [fnv(&tuiles(&dessin(false, vk))), fnv(&tuiles(&dessin(true, vk)))];
            *e = Some((vk, h));
            h
        }
    }
}

/// À appeler avant chaque image émulée ; `vk` : touche à inscrire (`touche_icone`).
pub fn remplacer(bus: &mut Bus, vk: u8) {
    let mut gros = None;
    let mut petits = Vec::new();
    for i in 0..128 {
        let a0 = u16::from_le_bytes([bus.oam[8 * i], bus.oam[8 * i + 1]]);
        let a1 = u16::from_le_bytes([bus.oam[8 * i + 2], bus.oam[8 * i + 3]]);
        let a2 = u16::from_le_bytes([bus.oam[8 * i + 4], bus.oam[8 * i + 5]]);
        if (a0 >> 8) & 3 == 2 || a0 & 0x2000 != 0 {
            continue; // caché, ou 256 couleurs
        }
        let base = 0x1_0000 + 32 * (a2 & 0x3FF) as usize;
        let (x, y, pal) = ((a1 & 0x1FF) as i32, (a0 & 0xFF) as i32, a2 >> 12);
        match (a0 >> 14, a1 >> 14) {
            (2, 2) if base + 256 <= bus.vram.len() => {
                let h = fnv(&bus.vram[base..base + 256]);
                if let Some(&(_, enf)) = GROS.iter().find(|(g, _)| *g == h) {
                    gros = Some((Some((base, enf)), x, y, pal));
                } else if nos_empreintes(vk).contains(&h) {
                    gros = Some((None, x, y, pal));
                }
            }
            (_, 0) => petits.push((i, x, y, pal)),
            _ => {}
        }
    }
    let Some((nouveau, gx, gy, gpal)) = gros else { return };
    if let Some((base, enf)) = nouveau {
        bus.vram[base..base + 256].copy_from_slice(&tuiles(&dessin(enf, vk)));
    }
    // le bout droit du bouton d'origine : petit sprite (8×8 ou 8×16) de même palette, en
    // bas à droite de la touche (écart mesuré : +11..+15 en x, +15..+22 en y) ; masqué dans l'OAM,
    // que le jeu réécrit à chaque image
    for (i, x, y, pal) in petits {
        let (dx, dy) = ((x - gx).rem_euclid(512), (y - gy).rem_euclid(256));
        if pal == gpal && (6..=20).contains(&dx) && (10..=26).contains(&dy) {
            let a0 = u16::from_le_bytes([bus.oam[8 * i], bus.oam[8 * i + 1]]);
            let v = (a0 & !0x0300) | 0x0200;
            bus.oam[8 * i..8 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn touche_complete() {
        for enf in [false, true] {
            for vk in [b'A', 0x20, 0x26, 0x7B, 0xBA, b'7'] {
                let img = super::dessin(enf, vk);
                // contour du bas sur la même ligne, une inscription présente
                assert_eq!(img[25][2], 1);
                assert!(img.iter().flatten().filter(|&&p| p == 1).count() > 60, "{vk:02X}");
            }
        }
        assert_eq!(super::touche_icone(&[b'X', 0x20, b'K', b'A']), b'A');
        assert_eq!(super::touche_icone(&[0x20, b'K']), 0x20);
        assert_eq!(super::inscription(0x20), "ESP");
        assert_eq!(super::inscription(0x7B), "F12");
    }
}
