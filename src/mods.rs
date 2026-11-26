//! Mods activables du portage PC (menu Échap › MODS). Ils agissent sur la mémoire du jeu
//! entre deux images, sur les touches, sur l'image affichée ou sur l'émulateur
//! (rembobinage, vitesse), sans changer le code du jeu.
//!
//! Variables trouvées par recherche mémoire (voir NOTES.md) : santé en `0x03001E08`
//! (5 = cœur plein, 0 = mort), vies en `0x03001DE0`, vitesses du joueur en `0x03001E00`
//! (horizontale) et `0x03001E10` (verticale), entité du joueur à pied en
//! `[0x030009A0]` (type `0x0805B304`). Entité : `+0x48/+0x4C` position (24.8),
//! `+0x5A/+0x5C` vitesse, `+0x58` drapeaux (0x20 transparence, 0x80 rotation d'angle
//! `+0x3C`, 0x100 échelle `+0x66/+0x68`, 0x200 palette imposée `+0x72`), `+0x6C` octet
//! non nul quand le joueur touche le sol.

use crate::gba::{key, Gba};
use crate::menu::Settings;
use std::collections::VecDeque;

pub const SANTE: u32 = 0x0300_1E08;
pub const SANTE_MAX: u32 = 5;
pub const VIES: u32 = 0x0300_1DE0;
pub const VIES_MAX: u32 = 9;
const JOUEUR: u32 = 0x0300_09A0;
const TYPE_JOUEUR: u32 = 0x0805_B304;
/// Vitesse verticale du joueur tenue par le jeu (recopiée dans l'entité à chaque pas).
const VY_JOUEUR: u32 = 0x0300_1E10;
/// Vitesse horizontale du joueur tenue par le jeu.
const VX_JOUEUR: u32 = 0x0300_1E00;

/// Vitesses du jeu proposées (en %).
pub const VITESSES: [u32; 5] = [50, 75, 100, 150, 200];
pub const SAUTS: [&str; 3] = ["NORMAL", "HAUT", "TRÈS HAUT"];
pub const GRAVITES: [&str; 3] = ["NORMALE", "LUNE", "PLUME"];
pub const TAILLES: [&str; 4] = ["NORMALE", "MINI", "GRANDE", "GÉANTE"];
pub const AMBIANCES: [&str; 6] = ["NORMALE", "GAME BOY", "NOIR ET BLANC", "NUIT", "NÉGATIF", "SÉPIA"];
/// Premier et dernier état du jeu qui sont des niveaux jouables.
pub const NIVEAUX: std::ops::RangeInclusive<u32> = 16..=51;

/// Ce que les mods retiennent d'une image à l'autre.
#[derive(Default)]
pub struct Suivi {
    /// Vitesse verticale du joueur à l'image précédente (détection du départ d'un saut).
    vy: i32,
    angle: u32,
    /// Drapeaux d'entité mis par les mods (à retirer quand le mod est coupé).
    drapeaux: u32,
}

/// Entité du joueur à pied, si elle existe.
fn joueur(gba: &mut Gba) -> Option<u32> {
    let e = gba.bus.read32(JOUEUR);
    ((e >> 24 == 2 || e >> 24 == 3) && e & 3 == 0 && gba.bus.read32(e) == TYPE_JOUEUR).then_some(e)
}

/// À appeler avant chaque image émulée ; renvoie les touches à donner au jeu.
pub fn appliquer(gba: &mut Gba, cfg: &Settings, suivi: &mut Suivi, mut keys: u16) -> u16 {
    if cfg.mod_health {
        let h = gba.bus.read8(SANTE);
        // 0 = mort en cours (chute dans le vide…) : on n'y touche pas
        if h > 0 && h < SANTE_MAX {
            gba.bus.write8(SANTE, SANTE_MAX as u8);
        }
    } else if cfg.mod_sudden && gba.bus.read8(SANTE) > 1 {
        // mort subite : un seul point de santé
        gba.bus.write8(SANTE, 1);
    }
    if cfg.mod_lives {
        let v = gba.bus.read8(VIES);
        if v < VIES_MAX {
            gba.bus.write8(VIES, VIES_MAX as u8);
        }
    }
    if cfg.mod_mirror {
        // monde miroir : l'image est retournée, gauche et droite aussi
        let (l, r) = (keys & key::LEFT != 0, keys & key::RIGHT != 0);
        keys &= !(key::LEFT | key::RIGHT);
        if l {
            keys |= key::RIGHT;
        }
        if r {
            keys |= key::LEFT;
        }
    }
    if !cfg.mod_run {
        crate::jeu::entity::COURSE_PLUS.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    let Some(e) = joueur(gba) else {
        suivi.vy = 0;
        suivi.drapeaux = 0;
        return keys;
    };
    let rd16 = |g: &mut Gba, a: u32| g.bus.read16(a) as u16 as i16 as i32;
    let vy = rd16(gba, VY_JOUEUR);
    let sol = gba.bus.read8(e + 0x6C) != 0;

    // saut plus haut : la vitesse de départ est augmentée à l'image où le saut commence
    if cfg.mod_jump != 0 && suivi.vy >= 0 && vy <= -600 {
        let k = if cfg.mod_jump == 1 { 135 } else { 170 };
        gba.bus.write16(VY_JOUEUR, (vy * k / 100) as u16);
    }
    // gravité plus faible : une partie de ce que le jeu ajoute (64 par pas, un pas sur
    // deux images) est retirée tant que le joueur est en l'air
    if cfg.mod_gravity != 0 && !sol {
        let v = rd16(gba, VY_JOUEUR);
        let moins = if cfg.mod_gravity == 1 { 16 } else { 24 };
        gba.bus.write16(VY_JOUEUR, (v - moins) as u16);
    }
    suivi.vy = rd16(gba, VY_JOUEUR);

    // course rapide : le déplacement horizontal que le jeu applique au joueur est amplifié
    // de moitié (dans `jeu::entity::entity_frame`)
    crate::jeu::entity::COURSE_PLUS.store(if cfg.mod_run { 50 } else { 0 }, std::sync::atomic::Ordering::Relaxed);
    // vol libre : haut et bas pilotent la vitesse verticale (plus de chute), le jeu
    // garde ses collisions avec le décor
    if cfg.mod_fly {
        let v = if keys & key::UP != 0 {
            -640
        } else if keys & key::DOWN != 0 {
            640
        } else {
            0
        };
        if v != 0 || !sol {
            gba.bus.write16(VY_JOUEUR, v as u16);
        }
        suivi.vy = v;
    }

    // apparence : drapeaux d'affichage de l'entité
    let mut voulus = 0u32;
    if cfg.mod_size != 0 {
        voulus |= 0x100;
        // facteur d'agrandissement en 8.8
        let k: u16 = match cfg.mod_size {
            1 => 0xA0,
            2 => 0x180,
            _ => 0x200,
        };
        gba.bus.write16(e + 0x66, k);
        gba.bus.write16(e + 0x68, k);
    }
    if cfg.mod_spin {
        voulus |= 0x80;
        suivi.angle = (suivi.angle + 0x30) & 0xFFF;
        gba.bus.write16(e + 0x3C, suivi.angle as u16);
    }
    if cfg.mod_ghost {
        voulus |= 0x20;
    }
    if cfg.mod_palette != 0 {
        voulus |= 0x200;
        gba.bus.write8(e + 0x72, (cfg.mod_palette & 15) as u8);
    }
    let f = gba.bus.read16(e + 0x58);
    let retires = suivi.drapeaux & !voulus;
    let nf = (f & !retires) | voulus;
    if nf != f {
        gba.bus.write16(e + 0x58, nf as u16);
    }
    suivi.drapeaux = voulus;
    keys
}

/// Image affichée : monde miroir puis ambiance de couleurs (avant le menu et les messages).
pub fn image(cfg: &Settings, frame: &mut [u32]) {
    if cfg.mod_mirror {
        for row in frame.chunks_exact_mut(240) {
            row.reverse();
        }
    }
    if cfg.mod_look == 0 {
        return;
    }
    for p in frame.iter_mut() {
        let (r, g, b) = ((*p >> 16) & 255, (*p >> 8) & 255, *p & 255);
        let l = (r * 77 + g * 150 + b * 29) >> 8;
        let (r, g, b) = match cfg.mod_look {
            // quatre verts de la première Game Boy
            1 => [(15, 56, 15), (48, 98, 48), (139, 172, 15), (155, 188, 15)][(l >> 6) as usize],
            2 => (l, l, l),
            3 => (r * 2 / 5, g / 2, (b * 3 / 4 + 40).min(255)),
            4 => (255 - r, 255 - g, 255 - b),
            _ => ((l * 112 / 100 + 20).min(255), (l * 95 / 100 + 8).min(255), l * 70 / 100),
        };
        *p = (r << 16) | (g << 8) | b;
    }
}

/// Son d'une ou plusieurs images de jeu ramené à la durée d'une image affichée (549
/// échantillons stéréo) quand la vitesse du jeu est modifiée : `stock` garde le son
/// produit, on en consomme `549 × vitesse / 100` échantillons par image affichée.
pub fn son_vitesse(stock: &mut Vec<i16>, produit: &[i16], vitesse: u32) -> Vec<i16> {
    stock.extend_from_slice(produit);
    const N: usize = 549;
    let dispo = stock.len() / 2;
    let pris = (N * vitesse as usize / 100).min(dispo);
    let mut out = Vec::with_capacity(2 * N);
    if pris == 0 {
        out.resize(2 * N, 0);
        return out;
    }
    for i in 0..N {
        let p = i * pris / N;
        out.push(stock[2 * p]);
        out.push(stock[2 * p + 1]);
    }
    stock.drain(..2 * pris);
    // pas d'accumulation : au plus une image d'avance
    if stock.len() > 4 * N {
        let extra = stock.len() - 4 * N;
        stock.drain(..extra & !1);
    }
    out
}

/// Rembobinage : un état toutes les `PAS` images, sur `DUREE` secondes au plus.
pub struct Rembobinage {
    etats: VecDeque<Vec<u8>>,
    n: u32,
}

const PAS: u32 = 4;
const DUREE: usize = 10;

impl Rembobinage {
    pub fn new() -> Self {
        Rembobinage { etats: VecDeque::new(), n: 0 }
    }

    /// Après chaque image jouée normalement.
    pub fn noter(&mut self, gba: &Gba) {
        self.n = self.n.wrapping_add(1);
        if self.n % PAS == 0 {
            if self.etats.len() >= DUREE * 60 / PAS as usize {
                // réutilise le plus vieux tampon
                let mut v = self.etats.pop_front().unwrap();
                v.clear();
                v.extend_from_slice(&gba.save_state());
                self.etats.push_back(v);
            } else {
                self.etats.push_back(gba.save_state());
            }
        }
    }

    /// Une image de rembobinage (vitesse ×2) ; `false` quand l'historique est épuisé.
    pub fn reculer(&mut self, gba: &mut Gba) -> bool {
        let Some(st) = self.etats.pop_back() else { return false };
        if !gba.load_state(&st) {
            return false;
        }
        // une image sans touche pour redessiner l'écran à cet instant
        gba.set_keys(0);
        gba.run_frame();
        gba.bus.apu.buf.clear();
        self.n = 0;
        true
    }

    pub fn secondes(&self) -> f32 {
        self.etats.len() as f32 * PAS as f32 / 60.0
    }

    pub fn vider(&mut self) {
        self.etats.clear();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn son_a_vitesse_variable() {
        // une image de jeu = 549 échantillons stéréo ; la sortie fait toujours cette durée
        let image: Vec<i16> = (0..2 * 549).map(|i| i as i16).collect();
        for (vitesse, images_jeu) in [(50u32, [1usize, 0]), (200, [2, 2]), (150, [1, 2]), (75, [1, 1])] {
            let mut stock = Vec::new();
            for n in images_jeu {
                let produit: Vec<i16> = image.iter().cycle().take(n * image.len()).copied().collect();
                assert_eq!(super::son_vitesse(&mut stock, &produit, vitesse).len(), 2 * 549, "{vitesse}%");
            }
            assert!(stock.len() <= 4 * 549);
        }
    }
}
