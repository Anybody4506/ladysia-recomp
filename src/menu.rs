//! Menu d'options du portage PC (Échap) : affichage, son, vitesse, sauvegardes
//! instantanées, fonctions réécrites. Réglages gardés dans un fichier `.ini`.

use crate::mods::{AMBIANCES, GRAVITES, NIVEAUX, SAUTS, TAILLES, VITESSES};
use std::path::{Path, PathBuf};

pub const TURBO: [u32; 4] = [2, 3, 4, 8];

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub fullscreen: bool,
    pub scale: u32,
    /// 0 pixels nets, 1 lissée, 2 nets adoucis, 3 HD (xBR).
    pub filter: u32,
    /// 0 aucun, 1 lignes de balayage, 2 grille LCD, 3 écran cathodique.
    pub effect: u32,
    pub stretch: bool,
    /// 0 originales, 1 écran GBA, 2 vives.
    pub colors: u32,
    pub ghost: bool,
    pub volume: u32,
    /// Son adouci (interpolation des échantillons du mixeur).
    pub soft_sound: bool,
    pub turbo: usize,
    pub slot: u32,
    pub natives: bool,
    pub fps: bool,
    /// Mods : santé toujours pleine, vies infinies, rembobinage (R / gâchette gauche),
    /// pause quand la fenêtre n'est pas au premier plan.
    pub mod_health: bool,
    pub mod_lives: bool,
    pub mod_rewind: bool,
    pub pause_unfocused: bool,
    /// Sauvegarde instantanée automatique en quittant, rechargée au lancement.
    pub resume: bool,
    /// Touches du clavier (codes VK Windows) de chaque bouton GBA, dans l'ordre de
    /// `BUTTONS` : haut, bas, gauche, droite, A, B, L, R, Start, Select.
    pub keys: [Vec<u8>; 10],
    /// Boutons de la manette (masques XInput, voir `PAD_BUTTONS`) de chaque bouton GBA.
    pub pad: [u16; 10],
    /// Mods de la page MODS (voir `mods.rs`) : mort subite, saut (`SAUTS`), gravité
    /// (`GRAVITES`), course rapide, vol libre, taille (`TAILLES`), toupie, transparence,
    /// palette imposée (0 = d'origine), monde miroir, ambiance (`AMBIANCES`), vitesse du
    /// jeu (rang dans `VITESSES`).
    pub mod_sudden: bool,
    pub mod_jump: u32,
    pub mod_gravity: u32,
    pub mod_run: bool,
    pub mod_fly: bool,
    pub mod_size: u32,
    pub mod_spin: bool,
    pub mod_ghost: bool,
    pub mod_palette: u32,
    pub mod_mirror: bool,
    pub mod_look: u32,
    pub mod_speed: u32,
}

/// Boutons de la manette Xbox : bit (`wButtons` d'XInput ; 0x0400 / 0x0800 : gâchettes
/// gauche / droite), nom affiché, nom dans le `.ini`. Dans l'ordre d'affichage.
pub const PAD_BUTTONS: [(u16, &str, &str); 16] = [
    (0x1000, "A", "A"),
    (0x2000, "B", "B"),
    (0x4000, "X", "X"),
    (0x8000, "Y", "Y"),
    (0x0100, "LB", "LB"),
    (0x0200, "RB", "RB"),
    (0x0400, "LT", "LT"),
    (0x0800, "RT", "RT"),
    (0x0010, "START", "START"),
    (0x0020, "BACK", "BACK"),
    (0x0040, "L3", "L3"),
    (0x0080, "R3", "R3"),
    (0x0001, "CROIX ↑", "CROIX_HAUT"),
    (0x0002, "CROIX ↓", "CROIX_BAS"),
    (0x0004, "CROIX ←", "CROIX_GAUCHE"),
    (0x0008, "CROIX →", "CROIX_DROITE"),
];

/// Manette d'origine : croix (et stick gauche), A = A / Y, B = B / X, L = LB / LT,
/// R = RB / RT, Start, Select = Back.
pub const DEFAULT_PAD: [u16; 10] = [0x0001, 0x0002, 0x0004, 0x0008, 0x9000, 0x6000, 0x0500, 0x0A00, 0x0010, 0x0020];

/// Noms des boutons de la manette d'un masque, dans l'ordre de `PAD_BUTTONS`.
pub fn pad_names(m: u16) -> Vec<&'static str> {
    PAD_BUTTONS.iter().filter(|b| m & b.0 != 0).map(|b| b.1).collect()
}

/// Boutons GBA configurables : nom affiché, clé du `.ini`.
pub const BUTTONS: [(&str, &str); 10] = [
    ("HAUT", "touche_haut"),
    ("BAS", "touche_bas"),
    ("GAUCHE", "touche_gauche"),
    ("DROITE", "touche_droite"),
    ("A", "touche_a"),
    ("B", "touche_b"),
    ("L", "touche_l"),
    ("R", "touche_r"),
    ("START", "touche_start"),
    ("SELECT", "touche_select"),
];

/// Touches d'origine : flèches, A = X / Espace / K / A, B = W / Z / J, L = Q, R = S,
/// Start = Entrée, Select = Retour arrière / Maj droite.
pub const DEFAULT_KEYS: [&[u8]; 10] = [
    &[0x26],
    &[0x28],
    &[0x25],
    &[0x27],
    &[b'X', 0x20, b'K', b'A'],
    &[b'W', b'Z', b'J'],
    &[b'Q'],
    &[b'S'],
    &[0x0D],
    &[0x08, 0xA1],
];

/// Au plus 4 touches par bouton.
const MAX_KEYS: usize = 4;

pub fn default_keys() -> [Vec<u8>; 10] {
    std::array::from_fn(|i| DEFAULT_KEYS[i].to_vec())
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            fullscreen: false,
            scale: 3,
            filter: 0,
            effect: 0,
            stretch: false,
            colors: 0,
            ghost: false,
            volume: 10,
            soft_sound: true,
            turbo: 1,
            slot: 1,
            natives: true,
            fps: true,
            mod_health: false,
            mod_lives: false,
            mod_rewind: true,
            pause_unfocused: false,
            resume: false,
            keys: default_keys(),
            pad: DEFAULT_PAD,
            mod_sudden: false,
            mod_jump: 0,
            mod_gravity: 0,
            mod_run: false,
            mod_fly: false,
            mod_size: 0,
            mod_spin: false,
            mod_ghost: false,
            mod_palette: 0,
            mod_mirror: false,
            mod_look: 0,
            mod_speed: 2,
        }
    }
}

impl Settings {
    /// Nombre de mods actifs (page MODS).
    pub fn mods_actifs(&self) -> usize {
        [
            self.mod_health,
            self.mod_lives,
            self.mod_sudden,
            self.mod_rewind,
            self.mod_speed != 2,
            self.mod_jump != 0,
            self.mod_gravity != 0,
            self.mod_run,
            self.mod_fly,
            self.mod_size != 0,
            self.mod_spin,
            self.mod_ghost,
            self.mod_palette != 0,
            self.mod_mirror,
            self.mod_look != 0,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }

    /// Coupe tous les mods (le rembobinage reste comme il est).
    pub fn sans_mods(&mut self) {
        let d = Settings::default();
        self.mod_health = false;
        self.mod_lives = false;
        self.mod_sudden = false;
        self.mod_speed = d.mod_speed;
        self.mod_jump = 0;
        self.mod_gravity = 0;
        self.mod_run = false;
        self.mod_fly = false;
        self.mod_size = 0;
        self.mod_spin = false;
        self.mod_ghost = false;
        self.mod_palette = 0;
        self.mod_mirror = false;
        self.mod_look = 0;
    }

    pub fn load(path: &Path) -> Settings {
        let mut s = Settings::default();
        let Ok(txt) = std::fs::read_to_string(path) else { return s };
        for line in txt.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let b = v == "1" || v.eq_ignore_ascii_case("oui");
            let n: u32 = v.parse().unwrap_or(0);
            match k {
                "plein_ecran" => s.fullscreen = b,
                "taille" => s.scale = n.clamp(1, 8),
                "lissage" => s.filter = if b { 1 } else { 0 },
                "filtre" => s.filter = n.min(FILTERS.len() as u32 - 1),
                "effet" => s.effect = n.min(EFFECTS.len() as u32 - 1),
                "couleurs" => s.colors = n.min(COLORS.len() as u32 - 1),
                "remanence" => s.ghost = b,
                "son_adouci" => s.soft_sound = b,
                "etirer" => s.stretch = b,
                "couleurs_ecran_gba" => {
                    if b {
                        s.colors = 1
                    }
                }
                "volume" => s.volume = n.min(10),
                "acceleration" => s.turbo = (n as usize).min(TURBO.len() - 1),
                "emplacement" => s.slot = n.clamp(1, 9),
                "fonctions_reecrites" => s.natives = b,
                "images_par_seconde" => s.fps = b,
                "mod_sante_infinie" => s.mod_health = b,
                "mod_vies_infinies" => s.mod_lives = b,
                "mod_rembobinage" => s.mod_rewind = b,
                "pause_hors_fenetre" => s.pause_unfocused = b,
                "reprise_auto" => s.resume = b,
                "mod_mort_subite" => s.mod_sudden = b,
                "mod_saut" => s.mod_jump = n.min(SAUTS.len() as u32 - 1),
                "mod_gravite" => s.mod_gravity = n.min(GRAVITES.len() as u32 - 1),
                "mod_course_rapide" => s.mod_run = b,
                "mod_vol_libre" => s.mod_fly = b,
                "mod_taille" => s.mod_size = n.min(TAILLES.len() as u32 - 1),
                "mod_toupie" => s.mod_spin = b,
                "mod_transparente" => s.mod_ghost = b,
                "mod_palette" => s.mod_palette = n.min(15),
                "mod_miroir" => s.mod_mirror = b,
                "mod_ambiance" => s.mod_look = n.min(AMBIANCES.len() as u32 - 1),
                "mod_vitesse" => s.mod_speed = n.min(VITESSES.len() as u32 - 1),
                _ if k.starts_with("manette_") => {
                    if let Some(i) = BUTTONS.iter().position(|b| b.1[7..] == k[8..]) {
                        let mut m = 0u16;
                        for t in v.split(',') {
                            let t = t.trim();
                            if let Some(b) = PAD_BUTTONS.iter().find(|b| b.2.eq_ignore_ascii_case(t)) {
                                if (m | b.0).count_ones() as usize <= MAX_KEYS {
                                    m |= b.0;
                                }
                            }
                        }
                        s.pad[i] = m;
                    }
                }
                _ => {
                    if let Some(i) = BUTTONS.iter().position(|b| b.1 == k) {
                        let mut l: Vec<u8> = Vec::new();
                        for t in v.split(',') {
                            if let Ok(vk) = u8::from_str_radix(t.trim(), 16) {
                                if vk != 0 && !reserved(vk, false) && !l.contains(&vk) && l.len() < MAX_KEYS {
                                    l.push(vk);
                                }
                            }
                        }
                        s.keys[i] = l;
                    }
                }
            }
        }
        s
    }

    pub fn save(&self, path: &Path) {
        let b = |v: bool| if v { 1 } else { 0 };
        let txt = format!(
            "; Réglages du portage PC de Lady Sia (modifiables aussi dans le menu Échap)\n\
             ; filtre : 0 pixels nets, 1 lissée, 2 nets adoucis, 3 HD (xBR)\n\
             ; effet : 0 aucun, 1 lignes de balayage, 2 grille LCD, 3 écran cathodique\n\
             ; couleurs : 0 originales, 1 écran GBA, 2 vives\n\
             plein_ecran={}\ntaille={}\nfiltre={}\neffet={}\netirer={}\ncouleurs={}\nremanence={}\nvolume={}\n\
             son_adouci={}\nacceleration={}\nemplacement={}\nfonctions_reecrites={}\nimages_par_seconde={}\n\
             ; mods (activables aussi dans le menu Échap)\n\
             mod_sante_infinie={}\nmod_vies_infinies={}\nmod_rembobinage={}\npause_hors_fenetre={}\nreprise_auto={}\n",
            b(self.fullscreen),
            self.scale,
            self.filter,
            self.effect,
            b(self.stretch),
            self.colors,
            b(self.ghost),
            self.volume,
            b(self.soft_sound),
            self.turbo,
            self.slot,
            b(self.natives),
            b(self.fps),
            b(self.mod_health),
            b(self.mod_lives),
            b(self.mod_rewind),
            b(self.pause_unfocused),
            b(self.resume)
        );
        let mut txt = txt;
        txt.push_str(&format!(
            "; page MODS : saut 0-2, gravité 0-2, taille 0-3, palette 0-15, ambiance 0-5, vitesse 0-4 (50 à 200 %)\n\
             mod_mort_subite={}\nmod_saut={}\nmod_gravite={}\nmod_course_rapide={}\nmod_vol_libre={}\nmod_taille={}\n\
             mod_toupie={}\nmod_transparente={}\nmod_palette={}\nmod_miroir={}\nmod_ambiance={}\nmod_vitesse={}\n",
            b(self.mod_sudden),
            self.mod_jump,
            self.mod_gravity,
            b(self.mod_run),
            b(self.mod_fly),
            self.mod_size,
            b(self.mod_spin),
            b(self.mod_ghost),
            self.mod_palette,
            b(self.mod_mirror),
            self.mod_look,
            self.mod_speed
        ));
        txt.push_str("; commandes du clavier : codes de touche Windows (VK) en hexadécimal, séparés par des virgules\n");
        for (i, (_, k)) in BUTTONS.iter().enumerate() {
            let l: Vec<String> = self.keys[i].iter().map(|v| format!("{v:02X}")).collect();
            txt.push_str(&format!("{k}={}\n", l.join(",")));
        }
        txt.push_str("; commandes de la manette : A, B, X, Y, LB, RB, LT, RT, START, BACK, L3, R3, CROIX_HAUT/BAS/GAUCHE/DROITE\n");
        for (i, (_, k)) in BUTTONS.iter().enumerate() {
            let l: Vec<&str> = PAD_BUTTONS.iter().filter(|b| self.pad[i] & b.0 != 0).map(|b| b.2).collect();
            txt.push_str(&format!("manette_{}={}\n", &k[7..], l.join(",")));
        }
        let _ = std::fs::write(path, txt);
    }
}

pub fn state_path(sav: &Path, slot: u32) -> PathBuf {
    sav.with_extension(format!("etat{slot}"))
}

/// Action demandée par le menu.
pub enum Action {
    None,
    Close,
    SaveState,
    LoadState,
    Reset,
    /// Recommencer directement au niveau donné (état du jeu, voir `mods::NIVEAUX`).
    Level(u32),
    Quit,
}

pub const FILTERS: [&str; 4] = ["PIXELS NETS", "LISSÉE", "NETS ADOUCIS", "HD (XBR)"];
pub const EFFECTS: [&str; 4] = ["AUCUN", "LIGNES DE BALAYAGE", "GRILLE LCD", "ÉCRAN CATHODIQUE"];
pub const COLORS: [&str; 3] = ["ORIGINALES", "ÉCRAN GBA", "VIVES"];

const ITEMS: usize = 24;
/// Lignes de la page MODS.
const MOD_ITEMS: usize = 18;
/// Lignes de l'écran des commandes : 10 boutons, touches d'origine, retour.
const KEY_ITEMS: usize = 12;
/// Lignes visibles à la fois (le menu défile).
const ROWS: usize = 14;

pub struct Menu {
    pub open: bool,
    sel: usize,
    top: usize,
    /// Écran des commandes ouvert (du clavier, ou de la manette si `pad_page`), et ligne choisie.
    keys_page: bool,
    pad_page: bool,
    /// Page MODS ouverte, ligne choisie, première ligne affichée, niveau à lancer (1 à 36).
    mods_page: bool,
    msel: usize,
    mtop: usize,
    niveau: u32,
    ksel: usize,
    /// Attente d'une touche pour le bouton `.0` (`.1` : ajouter plutôt que remplacer).
    capture: Option<(usize, bool)>,
    /// Message de l'écran des commandes.
    kmsg: Option<String>,
}

impl Menu {
    pub fn new() -> Self {
        Menu { open: false, sel: 0, top: 0, keys_page: false, pad_page: false, mods_page: false, msel: 0, mtop: 0, niveau: 1, ksel: 0, capture: None, kmsg: None }
    }

    fn label(&self, s: &Settings, i: usize) -> String {
        let yn = |b: bool| if b { "OUI" } else { "NON" };
        match i {
            0 => "REPRENDRE LE JEU".into(),
            1 => format!("PLEIN ÉCRAN : {}", yn(s.fullscreen)),
            2 => format!("TAILLE DE LA FENÊTRE : X{}", s.scale),
            3 => format!("IMAGE : {}", FILTERS[s.filter as usize]),
            4 => format!("EFFET : {}", EFFECTS[s.effect as usize]),
            5 => format!("PROPORTIONS : {}", if s.stretch { "ÉTIRÉES" } else { "GBA (3:2)" }),
            6 => format!("COULEURS : {}", COLORS[s.colors as usize]),
            7 => format!("RÉMANENCE LCD : {}", yn(s.ghost)),
            8 => format!("VOLUME : {}%", s.volume * 10),
            9 => format!("SON ADOUCI : {}", yn(s.soft_sound)),
            10 => format!("ACCÉLÉRÉ (TAB) : X{}", TURBO[s.turbo]),
            11 => format!("MODS... ({} ACTIFS)", s.mods_actifs()),
            12 => format!("PAUSE HORS FENÊTRE : {}", yn(s.pause_unfocused)),
            13 => format!("REPRISE AUTO DE LA PARTIE : {}", yn(s.resume)),
            14 => format!("IMAGES/S DANS LE TITRE : {}", yn(s.fps)),
            15 => format!("EMPLACEMENT : {}", s.slot),
            16 => "SAUVEGARDE INSTANTANÉE (F5)".into(),
            17 => "CHARGER L'EMPLACEMENT (F7)".into(),
            18 if crate::recomp::data_only() => "CODE DU JEU : 100 % RUST".into(),
            18 => format!("FONCTIONS RÉÉCRITES : {}", yn(s.natives)),
            19 => "COMMANDES DU CLAVIER...".into(),
            20 => "COMMANDES DE LA MANETTE...".into(),
            21 => "RÉGLAGES D'ORIGINE".into(),
            22 => "RECOMMENCER LE JEU".into(),
            _ => "QUITTER".into(),
        }
    }

    fn mod_label(&self, s: &Settings, i: usize) -> String {
        let yn = |b: bool| if b { "OUI" } else { "NON" };
        match i {
            0 => format!("SANTÉ INFINIE : {}", yn(s.mod_health)),
            1 => format!("VIES INFINIES : {}", yn(s.mod_lives)),
            2 => format!("MORT SUBITE (1 COUP) : {}", yn(s.mod_sudden)),
            3 => format!("REMBOBINAGE (R) : {}", yn(s.mod_rewind)),
            4 => format!("VITESSE DU JEU : {}%", VITESSES[s.mod_speed as usize]),
            5 => format!("SAUT : {}", SAUTS[s.mod_jump as usize]),
            6 => format!("GRAVITÉ : {}", GRAVITES[s.mod_gravity as usize]),
            7 => format!("COURSE RAPIDE : {}", yn(s.mod_run)),
            8 => format!("VOL LIBRE (HAUT/BAS) : {}", yn(s.mod_fly)),
            9 => format!("TAILLE DE SIA : {}", TAILLES[s.mod_size as usize]),
            10 => format!("SIA TOUPIE : {}", yn(s.mod_spin)),
            11 => format!("SIA TRANSPARENTE : {}", yn(s.mod_ghost)),
            12 if s.mod_palette == 0 => "COULEURS DE SIA : D'ORIGINE".into(),
            12 => format!("COULEURS DE SIA : PALETTE {}", s.mod_palette),
            13 => format!("MONDE MIROIR : {}", yn(s.mod_mirror)),
            14 => format!("AMBIANCE : {}", AMBIANCES[s.mod_look as usize]),
            15 => format!("ALLER AU NIVEAU {} (ENTRÉE)", self.niveau),
            16 => "TOUT DÉSACTIVER".into(),
            _ => "RETOUR".into(),
        }
    }

    /// Touches : `up`, `down`, `left`, `right`, `ok`, `back` (fronts montants).
    pub fn input(&mut self, s: &mut Settings, k: [bool; 6]) -> Action {
        let [up, down, left, right, ok, back] = k;
        if self.keys_page {
            self.keys_input(s, k);
            return Action::None;
        }
        if self.mods_page {
            return self.mods_input(s, k);
        }
        if back {
            self.open = false;
            return Action::Close;
        }
        if up {
            self.sel = (self.sel + ITEMS - 1) % ITEMS;
        }
        if down {
            self.sel = (self.sel + 1) % ITEMS;
        }
        let d: i32 = if right || ok { 1 } else if left { -1 } else { 0 };
        if d == 0 {
            return Action::None;
        }
        let step = |v: u32, lo: u32, hi: u32| ((v as i32 + d).clamp(lo as i32, hi as i32)) as u32;
        let cycle = |v: u32, n: usize| ((v as i32 + d).rem_euclid(n as i32)) as u32;
        match self.sel {
            0 if ok => {
                self.open = false;
                return Action::Close;
            }
            1 => s.fullscreen = !s.fullscreen,
            2 => s.scale = step(s.scale, 1, 8),
            3 => s.filter = cycle(s.filter, FILTERS.len()),
            4 => s.effect = cycle(s.effect, EFFECTS.len()),
            5 => s.stretch = !s.stretch,
            6 => s.colors = cycle(s.colors, COLORS.len()),
            7 => s.ghost = !s.ghost,
            8 => s.volume = step(s.volume, 0, 10),
            9 => s.soft_sound = !s.soft_sound,
            10 => s.turbo = step(s.turbo as u32, 0, TURBO.len() as u32 - 1) as usize,
            11 if ok || right => {
                self.mods_page = true;
                self.msel = 0;
                self.mtop = 0;
            }
            12 => s.pause_unfocused = !s.pause_unfocused,
            13 => s.resume = !s.resume,
            14 => s.fps = !s.fps,
            15 => s.slot = step(s.slot, 1, 9),
            16 if ok => {
                self.open = false;
                return Action::SaveState;
            }
            17 if ok => {
                self.open = false;
                return Action::LoadState;
            }
            18 if crate::recomp::data_only() => {}
            18 => s.natives = !s.natives,
            19 | 20 if ok || right => {
                self.keys_page = true;
                self.pad_page = self.sel == 20;
                self.ksel = 0;
                self.kmsg = None;
            }
            21 if ok => {
                // affichage, son et jeu d'origine ; garde la fenêtre, le volume et l'emplacement
                s.filter = 0;
                s.effect = 0;
                s.stretch = false;
                s.colors = 0;
                s.ghost = false;
                s.soft_sound = false;
                s.sans_mods();
            }
            22 if ok => {
                self.open = false;
                return Action::Reset;
            }
            23 if ok => return Action::Quit,
            _ => {}
        }
        Action::None
    }

    fn mods_input(&mut self, s: &mut Settings, k: [bool; 6]) -> Action {
        let [up, down, left, right, ok, back] = k;
        if back {
            self.mods_page = false;
            return Action::None;
        }
        if up {
            self.msel = (self.msel + MOD_ITEMS - 1) % MOD_ITEMS;
        }
        if down {
            self.msel = (self.msel + 1) % MOD_ITEMS;
        }
        let d: i32 = if right || ok { 1 } else if left { -1 } else { 0 };
        if d == 0 {
            return Action::None;
        }
        let cycle = |v: u32, n: usize| ((v as i32 + d).rem_euclid(n as i32)) as u32;
        match self.msel {
            0 => s.mod_health = !s.mod_health,
            1 => s.mod_lives = !s.mod_lives,
            2 => s.mod_sudden = !s.mod_sudden,
            3 => s.mod_rewind = !s.mod_rewind,
            4 => s.mod_speed = (s.mod_speed as i32 + d).clamp(0, VITESSES.len() as i32 - 1) as u32,
            5 => s.mod_jump = cycle(s.mod_jump, SAUTS.len()),
            6 => s.mod_gravity = cycle(s.mod_gravity, GRAVITES.len()),
            7 => s.mod_run = !s.mod_run,
            8 => s.mod_fly = !s.mod_fly,
            9 => s.mod_size = cycle(s.mod_size, TAILLES.len()),
            10 => s.mod_spin = !s.mod_spin,
            11 => s.mod_ghost = !s.mod_ghost,
            12 => s.mod_palette = cycle(s.mod_palette, 16),
            13 => s.mod_mirror = !s.mod_mirror,
            14 => s.mod_look = cycle(s.mod_look, AMBIANCES.len()),
            15 if ok => {
                self.open = false;
                self.mods_page = false;
                return Action::Level(NIVEAUX.start() + self.niveau - 1);
            }
            15 => {
                let n = (NIVEAUX.end() - NIVEAUX.start() + 1) as i32;
                self.niveau = (self.niveau as i32 - 1 + d).rem_euclid(n) as u32 + 1;
            }
            16 if ok => s.sans_mods(),
            17 if ok => self.mods_page = false,
            _ => {}
        }
        Action::None
    }

    /// Ouvre la page MODS sur une ligne (captures de test).
    pub fn open_mods(&mut self, sel: usize) {
        self.open = true;
        self.mods_page = true;
        self.msel = sel;
    }

    fn keys_input(&mut self, s: &mut Settings, k: [bool; 6]) {
        let [up, down, left, right, ok, back] = k;
        if back {
            self.keys_page = false;
            return;
        }
        if up || down {
            self.kmsg = None;
        }
        if up {
            self.ksel = (self.ksel + KEY_ITEMS - 1) % KEY_ITEMS;
        }
        if down {
            self.ksel = (self.ksel + 1) % KEY_ITEMS;
        }
        match self.ksel {
            i @ 0..=9 if self.pad_page => {
                if ok {
                    self.capture = Some((i, false));
                    self.kmsg = None;
                } else if right {
                    if (s.pad[i].count_ones() as usize) < MAX_KEYS {
                        self.capture = Some((i, true));
                        self.kmsg = None;
                    } else {
                        self.kmsg = Some(format!("{MAX_KEYS} BOUTONS AU PLUS PAR BOUTON GBA"));
                    }
                } else if left && s.pad[i] != 0 {
                    // retire le dernier dans l'ordre d'affichage
                    if let Some(b) = PAD_BUTTONS.iter().rev().find(|b| s.pad[i] & b.0 != 0) {
                        s.pad[i] &= !b.0;
                    }
                    self.kmsg = (s.pad[i] == 0).then(|| format!("{} N'A PLUS DE BOUTON", BUTTONS[i].0));
                }
            }
            i @ 0..=9 => {
                if ok {
                    self.capture = Some((i, false));
                    self.kmsg = None;
                } else if right {
                    if s.keys[i].len() < MAX_KEYS {
                        self.capture = Some((i, true));
                        self.kmsg = None;
                    } else {
                        self.kmsg = Some(format!("{MAX_KEYS} TOUCHES AU PLUS PAR BOUTON"));
                    }
                } else if left && !s.keys[i].is_empty() {
                    s.keys[i].pop();
                    self.kmsg = s.keys[i].is_empty().then(|| format!("{} N'A PLUS DE TOUCHE", BUTTONS[i].0));
                }
            }
            10 if ok && self.pad_page => {
                s.pad = DEFAULT_PAD;
                self.kmsg = Some("BOUTONS D'ORIGINE REMIS".into());
            }
            10 if ok => {
                s.keys = default_keys();
                self.kmsg = Some("TOUCHES D'ORIGINE REMISES".into());
            }
            11 if ok => self.keys_page = false,
            _ => {}
        }
    }

    /// Ouvre l'écran des commandes (captures de test), en attente d'une touche si `capture`.
    pub fn open_keys(&mut self, sel: usize, capture: bool) {
        self.open = true;
        self.keys_page = true;
        self.pad_page = false;
        self.ksel = sel;
        self.capture = capture.then_some((sel, false));
    }

    /// Ouvre l'écran de la manette (captures de test), en attente d'un bouton si `capture`.
    pub fn open_pad(&mut self, sel: usize, capture: bool) {
        self.open_keys(sel, capture);
        self.pad_page = true;
    }

    /// Vrai pendant l'attente d'une touche à attribuer.
    pub fn capturing(&self) -> bool {
        self.open && self.capture.is_some()
    }

    /// Vrai pendant l'attente d'un bouton de la manette (le clavier ne sert qu'à annuler).
    pub fn capturing_pad(&self) -> bool {
        self.capturing() && self.pad_page
    }

    /// Bouton de la manette appuyé pendant l'attente (`bit` de `PAD_BUTTONS`) : il est
    /// retiré des autres boutons GBA.
    pub fn capture_pad(&mut self, s: &mut Settings, bit: u16) {
        let Some((i, add)) = self.capture.take() else { return };
        let Some(b) = PAD_BUTTONS.iter().find(|b| b.0 == bit) else { return };
        let mut vides = Vec::new();
        for (j, m) in s.pad.iter_mut().enumerate() {
            if j != i && *m & bit != 0 {
                *m &= !bit;
                if *m == 0 {
                    vides.push(BUTTONS[j].0);
                }
            }
        }
        s.pad[i] = if add { s.pad[i] | bit } else { bit };
        self.kmsg = Some(match vides.first() {
            Some(v) => format!("ATTENTION : {v} N'A PLUS DE BOUTON"),
            None => format!("{} : {}", BUTTONS[i].0, b.1),
        });
    }

    /// Touche appuyée pendant l'attente (code VK) : Échap annule, les touches
    /// réservées au portage sont refusées, la touche est retirée des autres boutons.
    pub fn capture_key(&mut self, s: &mut Settings, vk: u8) {
        let Some((i, add)) = self.capture.take() else { return };
        if vk == 0x1B {
            return;
        }
        if reserved(vk, s.mod_rewind) {
            self.kmsg = Some(format!("{} EST RÉSERVÉE AU PORTAGE", key_name(vk)));
            return;
        }
        let mut vides = Vec::new();
        for (j, l) in s.keys.iter_mut().enumerate() {
            if j != i && l.contains(&vk) {
                l.retain(|&v| v != vk);
                if l.is_empty() {
                    vides.push(BUTTONS[j].0);
                }
            }
        }
        if add {
            if !s.keys[i].contains(&vk) {
                s.keys[i].push(vk);
            }
        } else {
            s.keys[i] = vec![vk];
        }
        self.kmsg = Some(match vides.first() {
            Some(b) => format!("ATTENTION : {b} N'A PLUS DE TOUCHE"),
            None => format!("{} : {}", BUTTONS[i].0, key_name(vk)),
        });
    }

    fn draw_keys(&self, s: &Settings, frame: &mut [u32]) {
        let pad = self.pad_page;
        text(frame, 8, 4, if pad { "COMMANDES DE LA MANETTE" } else { "COMMANDES DU CLAVIER" }, 0xFF_D040);
        for i in 0..KEY_ITEMS {
            let y = 16 + i as i32 * 9;
            let sel = i == self.ksel;
            if sel {
                fill(frame, 4, y - 1, 236, y + 8, 0x30_3070);
                text(frame, 6, y, ">", 0xFF_FFFF);
            }
            let col = if sel { 0xFF_FFFF } else { 0xC0_C0C0 };
            match i {
                0..=9 => {
                    text(frame, 14, y, BUTTONS[i].0, if sel { 0xFF_D040 } else { 0xA0_A0E0 });
                    let l: Vec<String> = if pad {
                        pad_names(s.pad[i]).into_iter().map(String::from).collect()
                    } else {
                        s.keys[i].iter().map(|&v| key_name(v)).collect()
                    };
                    let t = if l.is_empty() { "(AUCUNE)".to_string() } else { l.join(", ") };
                    text(frame, 62, y, &t, col);
                }
                10 => text(frame, 14, y, if pad { "BOUTONS D'ORIGINE" } else { "TOUCHES D'ORIGINE" }, col),
                _ => text(frame, 14, y, "RETOUR", col),
            }
        }
        if let Some(m) = &self.kmsg {
            text(frame, 8, 126, m, 0xFF_A060);
        }
        if pad {
            text(frame, 4, 138, "A : REMPLACER   → : AJOUTER", 0x90_90C0);
            text(frame, 4, 148, "← : RETIRER   B : RETOUR", 0x90_90C0);
        } else {
            text(frame, 4, 138, "ENTRÉE : REMPLACER    → : AJOUTER", 0x90_90C0);
            text(frame, 4, 148, "← : RETIRER   ÉCHAP : RETOUR", 0x90_90C0);
        }
        if let Some((i, add)) = self.capture {
            fill(frame, 20, 54, 220, 98, 0x10_1030);
            fill(frame, 20, 54, 220, 55, 0xFF_D040);
            fill(frame, 20, 97, 220, 98, 0xFF_D040);
            fill(frame, 20, 54, 21, 98, 0xFF_D040);
            fill(frame, 219, 54, 220, 98, 0xFF_D040);
            let (quoi, appuie) = if pad {
                (if add { "AJOUTER UN BOUTON À" } else { "NOUVEAU BOUTON POUR" }, "APPUIE SUR UN BOUTON")
            } else {
                (if add { "AJOUTER UNE TOUCHE À" } else { "NOUVELLE TOUCHE POUR" }, "APPUIE SUR UNE TOUCHE")
            };
            let t = format!("{quoi} : {}", BUTTONS[i].0);
            let centre = |t: &str| 120 - 3 * t.chars().count() as i32;
            text(frame, centre(&t), 62, &t, 0xFF_FFFF);
            text(frame, centre(appuie), 74, appuie, 0xFF_FFFF);
            text(frame, centre("(ÉCHAP : ANNULER)"), 86, "(ÉCHAP : ANNULER)", 0x90_90C0);
        }
    }

    /// Dessine le menu par-dessus l'image du jeu (240×160).
    pub fn draw(&mut self, s: &Settings, frame: &mut [u32]) {
        for p in frame.iter_mut() {
            let c = *p;
            *p = ((c >> 2) & 0x3F_3F3F) + 0x08_0810;
        }
        if self.keys_page {
            self.draw_keys(s, frame);
            return;
        }
        let mods = self.mods_page;
        let items = if mods { MOD_ITEMS } else { ITEMS };
        text(frame, 8, 4, if mods { "MODS" } else { "OPTIONS" }, 0xFF_D040);
        let (sel, mut top) = if mods { (self.msel, self.mtop) } else { (self.sel, self.top) };
        if sel < top {
            top = sel;
        }
        if sel >= top + ROWS {
            top = sel + 1 - ROWS;
        }
        if mods {
            self.mtop = top;
        } else {
            self.top = top;
        }
        if top > 0 {
            text(frame, 226, 6, "▲", 0x90_90C0);
        }
        if top + ROWS < items {
            text(frame, 226, 16 + ROWS as i32 * 9, "▼", 0x90_90C0);
        }
        for i in top..(top + ROWS).min(items) {
            let y = 16 + (i - top) as i32 * 9;
            let sel = i == sel;
            if sel {
                for yy in y - 1..y + 8 {
                    for xx in 4..236 {
                        frame[yy as usize * 240 + xx] = 0x30_3070;
                    }
                }
                text(frame, 6, y, ">", 0xFF_FFFF);
            }
            let l = if mods { self.mod_label(s, i) } else { self.label(s, i) };
            text(frame, 14, y, &l, if sel { 0xFF_FFFF } else { 0xC0_C0C0 });
        }
    }
}

/// Bandeau de message en bas de l'image.
pub fn banner(frame: &mut [u32], msg: &str) {
    for y in 148..160 {
        for x in 0..240 {
            frame[y * 240 + x] = 0x10_1020;
        }
    }
    text(frame, 4, 150, msg, 0xFF_FFFF);
}

fn fill(frame: &mut [u32], x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
    for y in y0.max(0)..y1.min(160) {
        for x in x0.max(0)..x1.min(240) {
            frame[y as usize * 240 + x as usize] = c;
        }
    }
}

/// Touches du portage qu'on ne peut pas attribuer : Échap (menu), Tab (accéléré),
/// F5 / F6 / F7 / F11, P (pause), R (rembobinage, si le mod est actif).
pub fn reserved(vk: u8, rewind: bool) -> bool {
    matches!(vk, 0x1B | 0x09 | 0x74 | 0x75 | 0x76 | 0x7A | b'P') || (rewind && vk == b'R')
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
}

/// Nom affichable d'une touche (code VK Windows).
pub fn key_name(vk: u8) -> String {
    let s = match vk {
        b'A'..=b'Z' | b'0'..=b'9' => return (vk as char).to_string(),
        0x60..=0x69 => return format!("PAVÉ {}", vk - 0x60),
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        0x08 => "RET. ARRIÈRE",
        0x0D => "ENTRÉE",
        0x13 => "PAUSE",
        0x14 => "VERR. MAJ",
        0x20 => "ESPACE",
        0x21 => "PAGE PRÉC.",
        0x22 => "PAGE SUIV.",
        0x23 => "FIN",
        0x24 => "DÉBUT",
        0x25 => "←",
        0x26 => "↑",
        0x27 => "→",
        0x28 => "↓",
        0x2C => "IMPR. ÉCRAN",
        0x2D => "INSER",
        0x2E => "SUPPR",
        0x5D => "MENU",
        0x6A => "PAVÉ *",
        0x6B => "PAVÉ +",
        0x6D => "PAVÉ -",
        0x6E => "PAVÉ .",
        0x6F => "PAVÉ /",
        0x90 => "VERR. NUM",
        0x91 => "ARRÊT DÉFIL",
        0xA0 => "MAJ G",
        0xA1 => "MAJ D",
        0xA2 => "CTRL G",
        0xA3 => "CTRL D",
        0xA4 => "ALT G",
        0xA5 => "ALT GR",
        _ => {
            // ponctuation : le caractère de la disposition du clavier
            #[cfg(windows)]
            {
                let c = unsafe { MapVirtualKeyW(vk as u32, 2) } & 0xFFFF;
                if let Some(ch) = char::from_u32(c).and_then(|c| c.to_uppercase().next()) {
                    if !ch.is_control() && glyph(ch) != [0; 7] {
                        return ch.to_string();
                    }
                }
            }
            return format!("TOUCHE {vk:02X}");
        }
    };
    s.into()
}

pub fn text(frame: &mut [u32], x: i32, y: i32, s: &str, color: u32) {
    let mut cx = x;
    for ch in s.chars() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    let (px, py) = (cx + col, y + row as i32);
                    if (0..240).contains(&px) && (0..160).contains(&py) {
                        frame[py as usize * 240 + px as usize] = color;
                    }
                }
            }
        }
        cx += 6;
    }
}

pub fn glyph(c: char) -> [u8; 7] {
    let c = c.to_uppercase().next().unwrap_or(' ');
    match c {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'É' => [0x02, 0x04, 0x1F, 0x10, 0x1E, 0x10, 0x1F],
        'È' => [0x08, 0x04, 0x1F, 0x10, 0x1E, 0x10, 0x1F],
        'Ê' => [0x04, 0x0A, 0x1F, 0x10, 0x1E, 0x10, 0x1F],
        'À' => [0x08, 0x04, 0x0E, 0x11, 0x1F, 0x11, 0x11],
        'Ç' => [0x0E, 0x11, 0x10, 0x10, 0x11, 0x0E, 0x04],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '*' => [0x00, 0x15, 0x0E, 0x1F, 0x0E, 0x15, 0x00],
        '↑' => [0x04, 0x0E, 0x15, 0x04, 0x04, 0x04, 0x04],
        '↓' => [0x04, 0x04, 0x04, 0x04, 0x15, 0x0E, 0x04],
        '←' => [0x00, 0x04, 0x08, 0x1F, 0x08, 0x04, 0x00],
        '→' => [0x00, 0x04, 0x02, 0x1F, 0x02, 0x04, 0x00],
        '▲' => [0x00, 0x04, 0x0E, 0x1F, 0x00, 0x00, 0x00],
        '▼' => [0x00, 0x00, 0x1F, 0x0E, 0x04, 0x00, 0x00],
        _ => [0; 7],
    }
}

/// Table de correction « écran GBA » (couleurs moins saturées et plus sombres, comme
/// l'écran LCD non rétroéclairé), indexée par couleur 15 bits.
pub fn lcd_table() -> Vec<u32> {
    let mut t = vec![0u32; 32768];
    for (i, out) in t.iter_mut().enumerate() {
        let lin = |v: usize| ((v & 31) as f64 / 31.0).powf(4.0);
        let (lr, lg, lb) = (lin(i), lin(i >> 5), lin(i >> 10));
        let f = |v: f64| ((v / 255.0).powf(1.0 / 2.2) * 255.0 * 255.0 / 280.0).clamp(0.0, 255.0) as u32;
        let r = f(50.0 * lg + 255.0 * lr);
        let g = f(30.0 * lb + 230.0 * lg + 10.0 * lr);
        let b = f(220.0 * lb + 10.0 * lg + 50.0 * lr);
        *out = (r << 16) | (g << 8) | b;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_ini() {
        let dir = std::env::temp_dir().join(format!("ladysia_ini_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.ini");
        let mut s = Settings::default();
        let mut m = Menu::new();
        // A remplacé par la touche L, puis B reçoit aussi Ctrl droit
        m.open_keys(4, true);
        m.capture_key(&mut s, b'L');
        m.open_keys(5, false);
        m.keys_input(&mut s, [false, false, false, true, false, false]);
        m.capture_key(&mut s, 0xA3);
        // touche réservée refusée
        m.open_keys(6, true);
        m.capture_key(&mut s, 0x74);
        assert_eq!(s.keys[4], vec![b'L']);
        assert_eq!(s.keys[5], vec![b'W', b'Z', b'J', 0xA3]);
        assert_eq!(s.keys[6], vec![b'Q']);
        // manette : A reçoit X (pris à B), Start devient RB (pris à R)
        m.open_pad(4, false);
        m.keys_input(&mut s, [false, false, false, true, false, false]);
        m.capture_pad(&mut s, 0x4000);
        m.open_pad(8, true);
        m.capture_pad(&mut s, 0x0200);
        assert_eq!(s.pad[4], 0xD000);
        assert_eq!(s.pad[5], 0x2000);
        assert_eq!(s.pad[8], 0x0200);
        assert_eq!(s.pad[7], 0x0800);
        assert_eq!(pad_names(s.pad[4]), vec!["A", "X", "Y"]);
        s.save(&p);
        assert!(Settings::load(&p) == s);
        // un ancien .ini sans commandes garde les touches d'origine
        std::fs::write(&p, "volume=5\n").unwrap();
        assert_eq!(Settings::load(&p).keys, default_keys());
        assert_eq!(Settings::load(&p).pad, DEFAULT_PAD);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(key_name(0x26), "↑");
        assert_eq!(key_name(0x61), "PAVÉ 1");
    }
}
