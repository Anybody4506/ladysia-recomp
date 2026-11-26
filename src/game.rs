//! Le jeu complet dans une fenêtre Windows : image, son, clavier et manette Xbox.
//!
//! Touches (modifiables dans Échap > Commandes du clavier) : flèches = croix,
//! A / X / Espace = A, W / Z = B, Q = L, S = R, Entrée = Start, Retour arrière = Select,
//! Tab (maintenu) = accéléré,
//! P = pause, Échap = menu d'options (`menu.rs`), F5 / F7 = sauver / charger l'état,
//! F6 = emplacement suivant, F11 = plein écran.
//! La sauvegarde (EEPROM) est écrite à côté de la ROM, en `.sav` (ou `LADYSIA_SAVE=chemin`).
//!
//! Tests sans fenêtre : `LADYSIA_FRAMES=n` (nombre d'images), `LADYSIA_SHOTS=dossier`
//! (une capture PNG toutes les `LADYSIA_EVERY` images), `LADYSIA_PRESS=image:touches,...`
//! (ex. `300:start,320:-,400:a`).

use crate::gba::{key, ppu, Gba};
use std::ffi::c_void;
use std::time::{Duration, Instant};

#[allow(non_snake_case, clippy::upper_case_acronyms, dead_code)]
mod win {
    use std::ffi::c_void;
    pub type HWND = *mut c_void;
    pub type HANDLE = *mut c_void;

    #[repr(C)]
    pub struct WNDCLASSW {
        pub style: u32,
        pub lpfnWndProc: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
        pub cbClsExtra: i32,
        pub cbWndExtra: i32,
        pub hInstance: HANDLE,
        pub hIcon: HANDLE,
        pub hCursor: HANDLE,
        pub hbrBackground: HANDLE,
        pub lpszMenuName: *const u16,
        pub lpszClassName: *const u16,
    }
    #[repr(C)]
    pub struct MSG {
        pub hwnd: HWND,
        pub message: u32,
        pub wParam: usize,
        pub lParam: isize,
        pub time: u32,
        pub pt: [i32; 2],
        pub private: u32,
    }
    #[repr(C)]
    pub struct RECT {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }
    #[repr(C)]
    pub struct BITMAPINFO {
        pub biSize: u32,
        pub biWidth: i32,
        pub biHeight: i32,
        pub biPlanes: u16,
        pub biBitCount: u16,
        pub biCompression: u32,
        pub biSizeImage: u32,
        pub biXPelsPerMeter: i32,
        pub biYPelsPerMeter: i32,
        pub biClrUsed: u32,
        pub biClrImportant: u32,
        pub colors: [u32; 1],
    }
    #[repr(C)]
    pub struct WAVEFORMATEX {
        pub wFormatTag: u16,
        pub nChannels: u16,
        pub nSamplesPerSec: u32,
        pub nAvgBytesPerSec: u32,
        pub nBlockAlign: u16,
        pub wBitsPerSample: u16,
        pub cbSize: u16,
    }
    #[repr(C)]
    pub struct WAVEHDR {
        pub lpData: *mut u8,
        pub dwBufferLength: u32,
        pub dwBytesRecorded: u32,
        pub dwUser: usize,
        pub dwFlags: u32,
        pub dwLoops: u32,
        pub lpNext: *mut WAVEHDR,
        pub reserved: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    pub struct XINPUT_STATE {
        pub dwPacketNumber: u32,
        pub wButtons: u16,
        pub bLeftTrigger: u8,
        pub bRightTrigger: u8,
        pub sThumbLX: i16,
        pub sThumbLY: i16,
        pub sThumbRX: i16,
        pub sThumbRY: i16,
    }

    pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    pub const WS_VISIBLE: u32 = 0x1000_0000;
    pub const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
    pub const WM_DESTROY: u32 = 0x0002;
    pub const WM_QUIT: u32 = 0x0012;
    pub const PM_REMOVE: u32 = 1;
    pub const SRCCOPY: u32 = 0x00CC_0020;
    pub const IDC_ARROW: usize = 32512;
    pub const WHDR_DONE: u32 = 1;
    pub const SWP_NOMOVE: u32 = 2;
    pub const SWP_NOZORDER: u32 = 4;
    pub const SWP_FRAMECHANGED: u32 = 0x20;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const GWL_STYLE: i32 = -16;
    #[repr(C)]
    pub struct MONITORINFO {
        pub cbSize: u32,
        pub rcMonitor: RECT,
        pub rcWork: RECT,
        pub dwFlags: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn RegisterClassW(c: *const WNDCLASSW) -> u16;
        pub fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: HWND,
            menu: HANDLE,
            inst: HANDLE,
            param: *mut c_void,
        ) -> HWND;
        pub fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
        pub fn PeekMessageW(m: *mut MSG, h: HWND, a: u32, b: u32, r: u32) -> i32;
        pub fn TranslateMessage(m: *const MSG) -> i32;
        pub fn DispatchMessageW(m: *const MSG) -> isize;
        pub fn PostQuitMessage(code: i32);
        pub fn GetDC(h: HWND) -> HANDLE;
        pub fn ReleaseDC(h: HWND, dc: HANDLE) -> i32;
        pub fn GetClientRect(h: HWND, r: *mut RECT) -> i32;
        pub fn AdjustWindowRect(r: *mut RECT, style: u32, menu: i32) -> i32;
        pub fn SetWindowTextW(h: HWND, s: *const u16) -> i32;
        pub fn GetAsyncKeyState(k: i32) -> i16;
        pub fn GetForegroundWindow() -> HWND;
        pub fn LoadCursorW(inst: HANDLE, name: usize) -> HANDLE;
        pub fn SetWindowLongPtrW(h: HWND, idx: i32, v: isize) -> isize;
        pub fn GetWindowRect(h: HWND, r: *mut RECT) -> i32;
        pub fn MonitorFromWindow(h: HWND, flags: u32) -> HANDLE;
        pub fn GetMonitorInfoW(m: HANDLE, mi: *mut MONITORINFO) -> i32;
        pub fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        pub fn StretchDIBits(
            dc: HANDLE,
            xd: i32,
            yd: i32,
            wd: i32,
            hd: i32,
            xs: i32,
            ys: i32,
            ws: i32,
            hs: i32,
            bits: *const c_void,
            bmi: *const BITMAPINFO,
            usage: u32,
            rop: u32,
        ) -> i32;
        pub fn SetStretchBltMode(dc: HANDLE, mode: i32) -> i32;
        pub fn SetBrushOrgEx(dc: HANDLE, x: i32, y: i32, old: *mut c_void) -> i32;
        pub fn PatBlt(dc: HANDLE, x: i32, y: i32, w: i32, h: i32, rop: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetModuleHandleW(name: *const u16) -> HANDLE;
        pub fn LoadLibraryW(name: *const u16) -> HANDLE;
        pub fn GetProcAddress(m: HANDLE, name: *const u8) -> *mut c_void;
    }
    #[link(name = "winmm")]
    unsafe extern "system" {
        pub fn waveOutOpen(h: *mut HANDLE, dev: u32, fmt: *const WAVEFORMATEX, cb: usize, inst: usize, flags: u32) -> u32;
        pub fn waveOutPrepareHeader(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutUnprepareHeader(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutWrite(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
        pub fn waveOutReset(h: HANDLE) -> u32;
        pub fn waveOutClose(h: HANDLE) -> u32;
        pub fn timeBeginPeriod(ms: u32) -> u32;
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn wndproc(h: win::HWND, m: u32, w: usize, l: isize) -> isize {
    unsafe {
        if m == win::WM_DESTROY {
            win::PostQuitMessage(0);
            return 0;
        }
        win::DefWindowProcW(h, m, w, l)
    }
}

// ----------------------------------------------------------------- son ----
const BUFFERS: usize = 6;

struct Audio {
    h: win::HANDLE,
    hdr: Vec<Box<win::WAVEHDR>>,
    data: Vec<Vec<i16>>,
    used: [bool; BUFFERS],
    /// Diagnostic : la carte son n'avait plus rien à jouer / tampon perdu faute de place.
    starved: u32,
    dropped: u32,
    /// Dernier envoi : une file vide ne compte comme coupure que si le son tournait
    /// déjà (pas au démarrage ni en sortie de pause ou de menu).
    last: Option<Instant>,
}

impl Audio {
    fn open() -> Option<Audio> {
        let rate = crate::gba::apu::RATE;
        let fmt = win::WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 2,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut h = std::ptr::null_mut();
        if unsafe { win::waveOutOpen(&mut h, u32::MAX, &fmt, 0, 0, 0) } != 0 {
            return None;
        }
        let hdr = (0..BUFFERS).map(|_| Box::new(unsafe { std::mem::zeroed::<win::WAVEHDR>() })).collect();
        Some(Audio { h, hdr, data: vec![Vec::new(); BUFFERS], used: [false; BUFFERS], starved: 0, dropped: 0, last: None })
    }

    /// Nombre de tampons en cours de lecture.
    fn queued(&mut self) -> usize {
        let size = std::mem::size_of::<win::WAVEHDR>() as u32;
        for k in 0..BUFFERS {
            if self.used[k] && self.hdr[k].dwFlags & win::WHDR_DONE != 0 {
                unsafe { win::waveOutUnprepareHeader(self.h, &mut *self.hdr[k], size) };
                self.used[k] = false;
            }
        }
        self.used.iter().filter(|u| **u).count()
    }

    fn play(&mut self, pcm: &[i16]) {
        if self.queued() == 0 && self.last.is_some_and(|t| t.elapsed() < Duration::from_millis(40)) {
            self.starved += 1;
        }
        self.last = Some(Instant::now());
        let Some(k) = (0..BUFFERS).find(|&k| !self.used[k]) else {
            self.dropped += 1;
            return;
        };
        let size = std::mem::size_of::<win::WAVEHDR>() as u32;
        self.data[k].clear();
        self.data[k].extend_from_slice(pcm);
        let h = &mut *self.hdr[k];
        *h = unsafe { std::mem::zeroed() };
        h.lpData = self.data[k].as_mut_ptr() as *mut u8;
        h.dwBufferLength = (self.data[k].len() * 2) as u32;
        unsafe {
            win::waveOutPrepareHeader(self.h, h, size);
            win::waveOutWrite(self.h, h, size);
        }
        self.used[k] = true;
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        unsafe {
            win::waveOutReset(self.h);
            self.queued();
            win::waveOutClose(self.h);
        }
    }
}

/// Attente précise. `sleep` dure au moins un « tic » de Windows, soit 15,6 ms quand la
/// précision d'1 ms demandée par `timeBeginPeriod` n'est pas honorée (fenêtre cachée,
/// économie d'énergie) : la boucle du jeu tombait alors à 43 images/s. Une minuterie
/// haute résolution reste précise dans tous les cas ; à défaut, `sleep`.
fn attendre(d: Duration) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateWaitableTimerExW(attr: *mut c_void, nom: *const u16, drapeaux: u32, acces: u32) -> *mut c_void;
        fn SetWaitableTimer(h: *mut c_void, echeance: *const i64, periode: i32, f: *mut c_void, arg: *mut c_void, reveil: i32) -> i32;
        fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    }
    thread_local! {
        // CREATE_WAITABLE_TIMER_HIGH_RESOLUTION (2), TIMER_ALL_ACCESS
        static MINUTERIE: *mut c_void = unsafe { CreateWaitableTimerExW(std::ptr::null_mut(), std::ptr::null(), 2, 0x1F_0003) };
    }
    let h = MINUTERIE.with(|h| *h);
    // échéance relative, en unités de 100 ns
    let echeance = -((d.as_nanos() / 100).max(1) as i64);
    if h.is_null() || unsafe { SetWaitableTimer(h, &echeance, 0, std::ptr::null_mut(), std::ptr::null_mut(), 0) } == 0 {
        std::thread::sleep(d);
        return;
    }
    unsafe { WaitForSingleObject(h, 1000) };
}

// ------------------------------------------------------------- manette ----
type XInputGetState = unsafe extern "system" fn(u32, *mut win::XINPUT_STATE) -> u32;

fn xinput() -> Option<XInputGetState> {
    for dll in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"] {
        let m = unsafe { win::LoadLibraryW(wide(dll).as_ptr()) };
        if m.is_null() {
            continue;
        }
        let f = unsafe { win::GetProcAddress(m, c"XInputGetState".as_ptr() as *const u8) };
        if !f.is_null() {
            return Some(unsafe { std::mem::transmute::<*mut c_void, XInputGetState>(f) });
        }
    }
    None
}

/// Boutons appuyés de la manette (bits de `menu::PAD_BUTTONS`) : le stick gauche compte
/// comme la croix, les gâchettes comme les bits 0x0400 / 0x0800.
fn pad_raw(get: XInputGetState) -> u16 {
    let mut st = win::XINPUT_STATE::default();
    if unsafe { get(0, &mut st) } != 0 {
        return 0;
    }
    let mut b = st.wButtons & !0x0C00;
    const DEAD: i16 = 12000;
    if st.sThumbLX < -DEAD {
        b |= 0x0004;
    }
    if st.sThumbLX > DEAD {
        b |= 0x0008;
    }
    if st.sThumbLY > DEAD {
        b |= 0x0001;
    }
    if st.sThumbLY < -DEAD {
        b |= 0x0002;
    }
    if st.bLeftTrigger > 100 {
        b |= 0x0400;
    }
    if st.bRightTrigger > 100 {
        b |= 0x0800;
    }
    b
}

/// Boutons GBA d'après les boutons de la manette et leur attribution.
fn pad_keys(raw: u16, pad: &[u16; 10]) -> u16 {
    let mut k = 0;
    for (i, m) in pad.iter().enumerate() {
        if raw & m != 0 {
            k |= GBA_BUTTONS[i];
        }
    }
    k
}

fn down(vk: i32) -> bool {
    unsafe { win::GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

/// Boutons GBA dans l'ordre de `menu::BUTTONS`.
const GBA_BUTTONS: [u16; 10] = [key::UP, key::DOWN, key::LEFT, key::RIGHT, key::A, key::B, key::L, key::R, key::START, key::SELECT];

fn keyboard(keys: &[Vec<u8>; 10]) -> u16 {
    let mut k = 0;
    for (vks, g) in keys.iter().zip(GBA_BUTTONS) {
        if vks.iter().any(|&v| down(v as i32)) {
            k |= g;
        }
    }
    k
}

/// Touche nouvellement appuyée (front montant) parmi tous les codes VK du clavier,
/// pour l'attribution d'une touche dans le menu ; `held` garde l'état précédent.
fn new_key(held: &mut [bool; 256]) -> Option<u8> {
    let mut hit = None;
    for vk in 0x08..=0xFEusize {
        // souris, Maj / Ctrl / Alt génériques (on garde gauche / droite), IME, Windows
        if matches!(vk, 0x10..=0x12 | 0x15..=0x1A | 0x1C..=0x1F | 0x5B | 0x5C | 0xA6..=0xB7 | 0xE5 | 0xE7) {
            continue;
        }
        let d = down(vk as i32);
        if d && !held[vk] && hit.is_none() {
            hit = Some(vk as u8);
        }
        held[vk] = d;
    }
    hit
}

fn save_path(rom_path: &str) -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LADYSIA_SAVE") {
        return p.into();
    }
    std::path::Path::new(rom_path).with_extension("sav")
}

fn write_save(gba: &mut Gba, path: &std::path::Path) {
    if gba.bus.eeprom.dirty {
        gba.bus.eeprom.dirty = false;
        if let Err(e) = std::fs::write(path, gba.bus.eeprom.image()) {
            eprintln!("sauvegarde impossible ({}) : {e}", path.display());
        }
    }
}

fn png(frame: &[u32]) -> Vec<u8> {
    png_wh(frame, ppu::W, ppu::H)
}

fn png_wh(frame: &[u32], w: usize, h: usize) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(frame.len() * 3);
    for p in frame {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    crate::png::rgb(w, h, &rgb)
}

fn parse_press(s: &str) -> Vec<(u64, u16)> {
    let names = [
        ("a", key::A),
        ("b", key::B),
        ("select", key::SELECT),
        ("start", key::START),
        ("right", key::RIGHT),
        ("left", key::LEFT),
        ("up", key::UP),
        ("down", key::DOWN),
        ("r", key::R),
        ("l", key::L),
    ];
    let mut out = Vec::new();
    for item in s.split(',').filter(|x| !x.is_empty()) {
        let Some((f, ks)) = item.split_once(':') else { continue };
        let mut k = 0;
        for n in ks.split('+') {
            if let Some((_, v)) = names.iter().find(|(x, _)| *x == n) {
                k |= v;
            }
        }
        if let Ok(f) = f.parse() {
            out.push((f, k));
        }
    }
    out
}

/// Mode test sans fenêtre.
fn headless(gba: &mut Gba, frames: u64, sav: &std::path::Path) -> Result<(), String> {
    // touche inscrite sur l'icône « appuie sur A » : réglages à côté de la sauvegarde,
    // ou LADYSIA_TOUCHE_A=code VK en hexadécimal
    let touche_a = std::env::var("LADYSIA_TOUCHE_A")
        .ok()
        .and_then(|v| u8::from_str_radix(v.trim(), 16).ok())
        .unwrap_or_else(|| crate::icones::touche_icone(&crate::menu::Settings::load(&sav.with_extension("ini")).keys[4]));
    let shots = std::env::var("LADYSIA_SHOTS").ok();
    gba.bus.apu.soft = std::env::var("LADYSIA_SON_ADOUCI").is_ok();
    let every: u64 = std::env::var("LADYSIA_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let press = parse_press(&std::env::var("LADYSIA_PRESS").unwrap_or_default());
    if let Some(d) = &shots {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let profile = std::env::var("LADYSIA_PROFILE").ok();
    if profile.is_some() {
        gba.bus.profile = Some(vec![0; ((crate::jeu::CODE_SIZE + 0x8000) / 2) as usize]);
    }
    let mut keys = 0;
    let mut audio = Vec::new();
    let trace_fifo = std::env::var("LADYSIA_FIFO").is_ok();
    let t0 = Instant::now();
    let trace_pops = std::env::var("LADYSIA_POPS").is_ok();
    // LADYSIA_MODS=sante,vies : mods actifs ; LADYSIA_REMBOBINE=image:pas : test du rembobinage
    // LADYSIA_MODS=sante,vies,subite,saut=2,gravite=1,course,vol,taille=3,toupie,
    // transparente,palette=5,miroir,ambiance=1
    let mods_cfg = std::env::var("LADYSIA_MODS").ok().map(|v| {
        let val = |k: &str| v.split(',').find_map(|t| t.strip_prefix(k)?.strip_prefix('=')?.parse::<u32>().ok()).unwrap_or(0);
        let a = |k: &str| v.split(',').any(|t| t == k);
        crate::menu::Settings {
            mod_health: a("sante"),
            mod_lives: a("vies"),
            mod_sudden: a("subite"),
            mod_jump: val("saut"),
            mod_gravity: val("gravite"),
            mod_run: a("course"),
            mod_fly: a("vol"),
            mod_size: val("taille"),
            mod_spin: a("toupie"),
            mod_ghost: a("transparente"),
            mod_palette: val("palette"),
            mod_mirror: a("miroir"),
            mod_look: val("ambiance"),
            ..Default::default()
        }
    });
    let mut suivi_mods = crate::mods::Suivi::default();
    let rembob_test: Option<(u64, u32)> = std::env::var("LADYSIA_REMBOBINE").ok().and_then(|v| {
        let (a, b) = v.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    let mut rembob = crate::mods::Rembobinage::new();
    // LADYSIA_ETAT=fichier : part d'un état instantané
    if std::env::var_os("LADYSIA_AUDIT").is_some() {
        let mask = crate::recomp_gen::data_mask(gba.bus.rom(), "asm/ladysia.s")?;
        gba.bus.audit = Some(Box::new((mask, Default::default())));
    }
    if let Some(n) = std::env::var("LADYSIA_NIVEAU").ok().and_then(|v| v.parse().ok()) {
        gba.force_state = Some(n);
    }
    // LADYSIA_COMPARE=<autre ROM ou données> : seconde machine menée en parallèle
    let mut jumeau: Option<Gba> = match std::env::var("LADYSIA_COMPARE") {
        Ok(p) => {
            let rom = crate::lzr::load(std::fs::read(&p).map_err(|e| format!("{p} : {e}"))?)?;
            let mut o = Gba::new(rom, std::fs::read(sav).ok());
            o.force_state = gba.force_state;
            o.trace = Some(Vec::new());
            gba.trace = Some(Vec::new());
            Some(o)
        }
        Err(_) => None,
    };
    if let Ok(p) = std::env::var("LADYSIA_ETAT") {
        let d = std::fs::read(&p).map_err(|e| e.to_string())?;
        if !gba.load_state(&d) {
            return Err(format!("état illisible : {p}"));
        }
        if let Some(o) = &mut jumeau {
            o.load_state(&d);
        }
    }
    // LADYSIA_PC=n : histogramme des adresses exécutées sur les n dernières images
    let pc_trace: Option<u64> = std::env::var("LADYSIA_PC").ok().and_then(|v| v.parse().ok());
    for f in 0..frames {
        if pc_trace.is_some_and(|n| f + n == frames) {
            gba.bus.profile = Some(vec![0; ((crate::jeu::CODE_SIZE + 0x8000) / 2) as usize]);
        }
        if trace_pops && f % 300 < 6 {
            eprintln!("image {f} : lectures FIFO A {} B {}", gba.bus.apu.pops[0], gba.bus.apu.pops[1]);
        }
        for &(at, k) in &press {
            if at == f {
                keys = k;
            }
        }
        if let Some(c) = &mods_cfg {
            keys = crate::mods::appliquer(gba, c, &mut suivi_mods, keys);
        }
        if let Some((at, n)) = rembob_test {
            if f == at {
                // rembobine `n` pas puis reprend normalement
                let avant = gba.bus.read8(crate::mods::SANTE);
                let mut ok = 0;
                for _ in 0..n {
                    if rembob.reculer(gba) {
                        ok += 1;
                    }
                }
                eprintln!(
                    "rembobinage à l'image {f} : {ok} pas ({:.1} s d'historique restant), santé {avant} -> {}",
                    rembob.secondes(),
                    gba.bus.read8(crate::mods::SANTE)
                );
                if let Some(d) = &shots {
                    std::fs::write(format!("{d}/rembob_{f}.png"), png(&gba.frame)).map_err(|e| e.to_string())?;
                }
            }
        }
        gba.set_keys(keys);
        let u = gba.bus.apu.underruns[0];
        crate::icones::remplacer(&mut gba.bus, touche_a);
        gba.run_frame();
        if std::env::var_os("LADYSIA_SUIVI").is_some() {
            let e = gba.bus.read32(0x0300_09A0);
            if e != 0 && std::env::var_os("LADYSIA_SUIVI").is_some_and(|v| v == "3") {
                let vy = gba.bus.read16(e + if std::env::var_os("LADYSIA_SUIVI_VX").is_some() { 0x5A } else { 0x5C });
                if vy != 0 {
                    let mut l = Vec::new();
                    for base in [0x0200_0000u32, 0x0300_0000] {
                        let n = if base >> 24 == 2 { 0x4_0000 } else { 0x8000 };
                        for a in (0..n).step_by(2) {
                            if gba.bus.read16(base + a) == vy {
                                l.push(format!("{:08X}", base + a));
                            }
                        }
                    }
                    eprintln!("[egal] {f} {}", l.join(" "));
                }
            }
            if e != 0 && std::env::var_os("LADYSIA_SUIVI").is_some_and(|v| v == "2") {
                let d: Vec<String> = (0..0x100u32).map(|k| format!("{:02X}", gba.bus.read8(e + k))).collect();
                eprintln!("[ent] {f} {}", d.join(""));
            }
            if e != 0 {
                eprintln!(
                    "[suivi] {f} x {} y {} vx {} vy {} drap {:04X} +66 {:04X} +68 {:04X} +6C {:08X} +70 {:08X}",
                    gba.bus.read32(e + 0x48) as i32 >> 8,
                    gba.bus.read32(e + 0x4C) as i32 >> 8,
                    gba.bus.read16(e + 0x5A) as i16,
                    gba.bus.read16(e + 0x5C) as i16,
                    gba.bus.read16(e + 0x58),
                    gba.bus.read16(e + 0x66),
                    gba.bus.read16(e + 0x68),
                    gba.bus.read32(e + 0x6C),
                    gba.bus.read32(e + 0x70)
                );
            }
        }
        if let Some(o) = &mut jumeau {
            // même image sur la machine jumelle (LADYSIA_COMPARE) puis comparaison
            o.set_keys(gba.bus.keys);
            crate::icones::remplacer(&mut o.bus, touche_a);
            o.run_frame();
            if let (Some(a), Some(b)) = (&mut gba.trace, &mut o.trace) {
                if let Some(k) = (0..a.len().min(b.len())).find(|&k| a[k] != b[k]) {
                    let ctx = |t: &Vec<(u32, u64)>| t[k.saturating_sub(8)..(k + 2).min(t.len())].iter().map(|(p, c)| format!("{p:08X}@{}", c & 0xFF_FFFF)).collect::<Vec<_>>().join(" ");
                    eprintln!("[compare] image {f}, bloc n° {k} :
  ROM     {}
  données {}", ctx(a), ctx(b));
                }
                a.clear();
                b.clear();
            }
            o.bus.apu.buf.clear();
            let mut d: Vec<String> = Vec::new();
            for r in 0..16 {
                if gba.cpu.r[r] != o.cpu.r[r] {
                    d.push(format!("r{r} {:08X}/{:08X}", gba.cpu.r[r], o.cpu.r[r]));
                }
            }
            if gba.bus.cyc != o.bus.cyc {
                d.push(format!("cycles {}/{}", gba.bus.cyc, o.bus.cyc));
            }
            // les copies de code en IWRAM diffèrent par construction
            let copie = |a: u32| [(0x0300_09C0u32, 0x120u32), (0x0300_0DC0, 0x18C), (0x0300_0F4C, 0x98), (0x0300_6CD0, 0x60)].iter().any(|&(s, n)| (s..s + n).contains(&a));
            d.extend(gba.bus.diff(&o.bus, 0).into_iter().filter(|(lo, _, _)| !copie(*lo)).map(|(_, _, t)| t));
            if !d.is_empty() {
                eprintln!(
                    "[compare] image {f} : pc {:08X}/{:08X}, cycles {}/{}, {} écart(s) : {}",
                    gba.cpu.r[15],
                    o.cpu.r[15],
                    gba.bus.cyc,
                    o.bus.cyc,
                    d.len(),
                    d.iter().filter(|t| t.contains("WRAM")).take(30).cloned().collect::<Vec<_>>().join(" ; ")
                );
                jumeau = None;
            }
        }
        if rembob_test.is_some() {
            rembob.noter(gba);
        }
        if trace_fifo && gba.bus.apu.underruns[0] != u {
            eprintln!("image {f} : {} lecture(s) de FIFO vide", gba.bus.apu.underruns[0] - u);
        }
        audio.extend(gba.bus.apu.buf.drain(..));
        if let Some(d) = &shots {
            if (f + 1) % every == 0 {
                let mut vue = gba.frame.clone();
                if let Some(c) = &mods_cfg {
                    crate::mods::image(c, &mut vue);
                }
                std::fs::write(format!("{d}/f{:05}.png", f + 1), png(&vue)).map_err(|e| e.to_string())?;
                if std::env::var_os("LADYSIA_VID").is_some() {
                    // mémoire vidéo : VRAM (96 Ko), OAM (1 Ko), palettes (1 Ko)
                    let mut mem = gba.bus.vram.clone();
                    mem.extend_from_slice(&gba.bus.oam);
                    mem.extend_from_slice(&gba.bus.pal);
                    std::fs::write(format!("{d}/v{:05}.bin", f + 1), mem).map_err(|e| e.to_string())?;
                }
                if std::env::var_os("LADYSIA_RAM").is_some() {
                    // recherche mémoire : IWRAM (32 Ko) puis EWRAM (256 Ko)
                    let mut mem: Vec<u8> = (0..0x8000u32).map(|a| gba.bus.read8(0x0300_0000 + a) as u8).collect();
                    mem.extend((0..0x4_0000u32).map(|a| gba.bus.read8(0x0200_0000 + a) as u8));
                    for p in &gba.frame {
                        mem.extend_from_slice(&p.to_le_bytes());
                    }
                    std::fs::write(format!("{d}/m{:05}.bin", f + 1), mem).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    if let Some(n) = std::env::var("LADYSIA_FUZZ").ok().and_then(|v| v.parse().ok()).filter(|_| !crate::recomp::data_only()) {
        eprint!("Test aléatoire du pilote son :
{}", crate::jeu::fuzz_sound(gba, n));
        eprint!("Test aléatoire de l'affichage :
{}", crate::jeu::fuzz_video(gba, n));
        eprintln!("Test aléatoire de la VRAM des sprites :
{}", crate::jeu::fuzz_vram(gba, n));
    }
    if std::env::var_os("LADYSIA_REGS").is_some() {
        eprintln!("registres : {}", (0..16).map(|i| format!("r{i}={:08X}", gba.cpu.r[i])).collect::<Vec<_>>().join(" "));
    }
    write_save(gba, sav);
    if let Ok(path) = std::env::var("LADYSIA_DUMP") {
        let mem: Vec<u8> = (0..0x8000u32).map(|a| gba.bus.read8(0x0300_0000 + a) as u8).collect();
        std::fs::write(path, mem).map_err(|e| e.to_string())?;
    }
    if let (Some(path), Some(p)) = (&profile, &gba.bus.profile) {
        let mut txt = String::new();
        for (i, n) in p.iter().enumerate() {
            if *n > 0 {
                txt += &format!("{:08X} {n}
", { let o = 2 * i as u32; if o < crate::jeu::CODE_SIZE { 0x0800_0000 + o } else { 0x0300_0000 + o - crate::jeu::CODE_SIZE } });
            }
        }
        std::fs::write(path, txt).map_err(|e| e.to_string())?;
    }
    let peak = audio.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    println!(
        "{frames} images en {:.2} s ({:.0} images/s), pc = {:08X}, son : {} échantillons, crête {peak}",
        t0.elapsed().as_secs_f64(),
        frames as f64 / t0.elapsed().as_secs_f64(),
        gba.cpu.r[15],
        audio.len() / 2
    );
    println!(
        "FIFO A : {} lectures, {} vides ; FIFO B : {} lectures, {} vides ; minuteurs 0x{:04X}/0x{:04X}, SOUNDCNT_H 0x{:04X}",
        gba.bus.apu.pops[0],
        gba.bus.apu.underruns[0],
        gba.bus.apu.pops[1],
        gba.bus.apu.underruns[1],
        gba.bus.io16(0x100),
        gba.bus.io16(0x104),
        gba.bus.io16(0x82)
    );
    {
        use std::sync::atomic::Ordering::Relaxed;
        println!(
            "Code recompilé : {} ({} blocs), {} blocs exécutés, {} vérifiés, {} différent(s) ; {} instructions interprétées",
            if crate::recomp::active() { "actif" } else { "inactif" },
            crate::recomp::block_count(),
            crate::recomp::RUNS.load(Relaxed),
            crate::recomp::CHECKED.load(Relaxed),
            crate::recomp::MISMATCHES.load(Relaxed),
            crate::recomp::INTERPRETED.load(Relaxed)
        );
    }
    if crate::recomp::data_only() {
        let m = crate::recomp::MISSING.load(std::sync::atomic::Ordering::Relaxed);
        println!(
            "Données sans code machine : {}",
            if m == 0 { "tout le code exécuté vient du Rust".to_string() } else { format!("CODE ABSENT en {:08X}", m & 0x7FFF_FFFF) }
        );
    }
    if let Some(a) = &gba.bus.audit {
        println!("Contrôle des données : {} octet(s) de code lus comme des données", a.1.len());
        for (addr, pc) in a.1.iter().take(400) {
            println!("  {addr:08X} lu depuis {pc:08X}");
        }
    }
    print!("Fonctions réécrites ({}) :
{}", if gba.natives.enabled { "actives" } else { "désactivées" }, gba.natives.report());
    if let Some(d) = &shots {
        // capture du menu d'options par-dessus la dernière image
        let mut m = gba.frame.clone();
        crate::menu::Menu::new().draw(&crate::menu::Settings::default(), &mut m);
        std::fs::write(format!("{d}/menu.png"), png(&m)).map_err(|e| e.to_string())?;
        for (nom, capture, manette) in [("commandes", false, false), ("commandes_attente", true, false), ("manette", false, true), ("manette_attente", true, true)] {
            let mut m = gba.frame.clone();
            let mut menu = crate::menu::Menu::new();
            if manette {
                menu.open_pad(4, capture);
            } else {
                menu.open_keys(4, capture);
            }
            menu.draw(&crate::menu::Settings::default(), &mut m);
            std::fs::write(format!("{d}/{nom}.png"), png(&m)).map_err(|e| e.to_string())?;
        }
        for (nom, ligne) in [("mods", 0usize), ("mods_bas", 15)] {
            let mut m = gba.frame.clone();
            let mut menu = crate::menu::Menu::new();
            menu.open_mods(ligne);
            let mut c = crate::menu::Settings::default();
            c.mod_jump = 1;
            c.mod_size = 3;
            menu.draw(&c, &mut m);
            std::fs::write(format!("{d}/{nom}.png"), png(&m)).map_err(|e| e.to_string())?;
        }
        // rendus améliorés de la dernière image (LADYSIA_RENDUS=largeurxhauteur)
        if let Some((w, h)) = std::env::var("LADYSIA_RENDUS").ok().and_then(|v| {
            let (a, b) = v.split_once('x')?;
            Some((a.parse::<i32>().ok()?, b.parse::<i32>().ok()?))
        }) {
            let mut r = crate::filtres::Rendeur::default();
            // planche de détails : un morceau de chaque rendu (360 × 270, au centre)
            let (cw, chh) = (360usize, 270usize);
            let mut planche = vec![0xFFFFFFu32; cw * 4 * chh * 5];
            let mut coller = |out: &[u32], k: usize| {
                let (ox, oy) = ((w as usize / 5).min(w as usize - cw), (h as usize * 5 / 8).min(h as usize - chh));
                for yy in 0..chh {
                    for xx in 0..cw {
                        planche[((k / 4) * chh + yy) * cw * 4 + (k % 4) * cw + xx] = out[(oy + yy) * w as usize + ox + xx];
                    }
                }
            };
            let vives = crate::filtres::vives_table();
            for filtre in 0..4 {
                for effet in 0..4 {
                    r.rendre(&gba.frame, w, h, crate::filtres::Rendu { filtre, effet });
                    let t = Instant::now();
                    for _ in 0..30 {
                        r.rendre(&gba.frame, w, h, crate::filtres::Rendu { filtre, effet });
                    }
                    eprintln!("rendu filtre {filtre} effet {effet} : {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 30.0);
                    std::fs::write(format!("{d}/rendu_f{filtre}_e{effet}.png"), png_wh(&r.out, w as usize, h as usize))
                        .map_err(|e| e.to_string())?;
                    coller(&r.out, (filtre * 4 + effet) as usize);
                }
            }
            let v: Vec<u32> = gba
                .frame
                .iter()
                .map(|&s| vives[(((s >> 19) & 31) | (((s >> 11) & 31) << 5) | (((s >> 3) & 31) << 10)) as usize])
                .collect();
            r.rendre(&v, w, h, crate::filtres::Rendu { filtre: 3, effet: 0 });
            std::fs::write(format!("{d}/rendu_vives.png"), png_wh(&r.out, w as usize, h as usize)).map_err(|e| e.to_string())?;
            coller(&r.out, 16);
            std::fs::write(format!("{d}/planche.png"), png_wh(&planche, cw * 4, chh * 5)).map_err(|e| e.to_string())?;
        }
    }
    if std::env::var("LADYSIA_STATE_TEST").is_ok() {
        // aller-retour : sauver, jouer 300 images, recharger, rejouer : même image ?
        let st = gba.save_state();
        let run = |g: &mut Gba| {
            for f in 0..300u32 {
                g.set_keys(if f % 50 < 25 { key::RIGHT } else { key::A });
                g.run_frame();
            }
            g.frame.clone()
        };
        let a = run(gba);
        let ok = gba.load_state(&st);
        let b = run(gba);
        println!("état instantané : {} octets, rechargé : {ok}, images identiques après 300 images : {}", st.len(), a == b);
    }
    if let Some(d) = &shots {
        let mono: Vec<i16> = audio.chunks(2).map(|c| ((c[0] as i32 + c[1] as i32) / 2) as i16).collect();
        let _ = std::fs::write(format!("{d}/son.wav"), crate::music::wav16(crate::gba::apu::RATE, &mono));
    }
    Ok(())
}

pub fn run(rom_path: &str) -> Result<(), String> {
    let rom = std::fs::read(rom_path).map_err(|e| format!("ROM illisible : {e}"))?;
    run_rom(crate::lzr::load(rom)?, save_path(rom_path))
}

/// Données du jeu intégrées à l'exécutable (fonction `embarque`), sans code machine :
/// fichier écrit par la commande `donnees`. Sauvegarde à côté du .exe.
#[cfg(feature = "embarque")]
pub fn run_embedded() {
    static ROM: &[u8] = include_bytes!(env!("LADYSIA_DONNEES_ABS"));
    let sav = match std::env::var("LADYSIA_SAVE") {
        Ok(p) => p.into(),
        Err(_) => std::env::current_exe().map(|e| e.with_extension("sav")).unwrap_or_else(|_| "Lady Sia.sav".into()),
    };
    if let Err(e) = crate::lzr::load(ROM.to_vec()).and_then(|rom| run_rom(rom, sav)) {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(h: *mut c_void, text: *const u16, title: *const u16, kind: u32) -> i32;
        }
        unsafe { MessageBoxW(std::ptr::null_mut(), wide(&e).as_ptr(), wide("Lady Sia").as_ptr(), 0x10) };
    }
}

fn run_rom(rom: Vec<u8>, sav: std::path::PathBuf) -> Result<(), String> {
    let save = std::fs::read(&sav).ok();
    let mut gba = Gba::new(rom, save);

    if let Some(n) = std::env::var("LADYSIA_FRAMES").ok().and_then(|v| v.parse().ok()) {
        return headless(&mut gba, n, &sav);
    }

    let hwnd = unsafe {
        let inst = win::GetModuleHandleW(std::ptr::null());
        let class = wide("LadySiaJeu");
        let wc = win::WNDCLASSW {
            style: 0,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: std::ptr::null_mut(),
            hCursor: win::LoadCursorW(std::ptr::null_mut(), win::IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        win::RegisterClassW(&wc);
        let mut r = win::RECT { left: 0, top: 0, right: 240 * 3, bottom: 160 * 3 };
        win::AdjustWindowRect(&mut r, win::WS_OVERLAPPEDWINDOW, 0);
        win::CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Lady Sia").as_ptr(),
            win::WS_OVERLAPPEDWINDOW | win::WS_VISIBLE,
            win::CW_USEDEFAULT,
            win::CW_USEDEFAULT,
            r.right - r.left,
            r.bottom - r.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            inst,
            std::ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        return Err("fenêtre impossible à créer".into());
    }
    unsafe { win::timeBeginPeriod(1) };
    let ini = sav.with_extension("ini");
    let mut cfg = crate::menu::Settings::load(&ini);
    let mut applied = cfg.clone();
    let mut menu = crate::menu::Menu::new();
    let lcd = crate::menu::lcd_table();
    let vives = crate::filtres::vives_table();
    let afficheur = Afficheur::lancer(hwnd);
    let mut ghost_prev: Vec<u32> = Vec::new();
    let mut win_rect = win::RECT { left: 0, top: 0, right: 0, bottom: 0 };
    apply_size(hwnd, &cfg, None, &mut win_rect);
    gba.natives.enabled = cfg.natives || crate::recomp::data_only();

    let mut audio = Audio::open();
    // tampons d'avance gardés pour la carte son (≈ 17 ms chacun) ; augmente tout seul
    // si la carte son se retrouve à vide (craquements)
    let mut avance = 3usize;
    // diagnostic : secondes passées à émuler / afficher (dont rendu des filtres)
    let mut temps_emu = 0f64;
    let mut coupures_vues = 0u32;
    if audio.is_none() {
        eprintln!("pas de sortie son disponible : le jeu tourne sans son");
    }
    let pad = xinput();
    let mut rembob = crate::mods::Rembobinage::new();
    // reprise automatique : état sauvé en quittant (fichier « .reprise » à côté de la sauvegarde)
    let reprise = sav.with_extension("reprise");
    let mut message: Option<(String, Instant)> = None;
    if cfg.resume {
        if let Ok(d) = std::fs::read(&reprise) {
            if gba.load_state(&d) {
                message = Some(("PARTIE REPRISE".into(), Instant::now()));
            }
        }
    }
    let autoquit: Option<f64> = std::env::var("LADYSIA_AUTOQUIT").ok().and_then(|v| v.parse().ok());
    let frame_time = Duration::from_secs_f64(280_896.0 / 16_777_216.0);
    let start = Instant::now();
    let mut next = Instant::now();
    let mut paused = false;
    let mut prev = 0u64;
    let mut held = [true; 256];
    let mut pad_prev = 0u16;
    let mut manque_note = false;
    let mut suivi_mods = crate::mods::Suivi::default();
    let mut dette_vitesse = 0u32;
    let mut stock_son: Vec<i16> = Vec::new();
    let mut frames = 0u64;
    let mut fps_t = Instant::now();
    let mut fps_n = 0;
    let mut fps = 0.0;
    let mut last_save = Instant::now();
    let mut show = vec![0u32; 240 * 160];

    'main: loop {
        unsafe {
            let mut msg: win::MSG = std::mem::zeroed();
            while win::PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, win::PM_REMOVE) != 0 {
                if msg.message == win::WM_QUIT {
                    break 'main;
                }
                win::TranslateMessage(&msg);
                win::DispatchMessageW(&msg);
            }
        }
        if let Some(s) = autoquit {
            if start.elapsed().as_secs_f64() > s {
                break;
            }
        }
        let focus = unsafe { win::GetForegroundWindow() } == hwnd;
        let mut keys = if focus && !menu.open { keyboard(&cfg.keys) } else { 0 };
        let pad_bits = pad.map(pad_raw).unwrap_or(0);
        if !menu.open {
            keys |= pad_keys(pad_bits, &cfg.pad);
        }
        // gâchette gauche : rembobinage
        let gachette = pad_bits & 0x0400 != 0;
        let pad_new = pad_bits & !pad_prev;
        pad_prev = pad_bits;
        // touches système (fronts montants) : bit n = n-ième touche de la liste
        let sys_keys = [0x1B, 0x26, 0x28, 0x25, 0x27, 0x0D, b'X' as i32, 0x20, 0x08, 0x74, 0x75, 0x76, 0x7A, b'P' as i32];
        let mut now_keys = 0u64;
        for (i, vk) in sys_keys.iter().enumerate() {
            if focus && down(*vk) {
                now_keys |= 1 << i;
            }
        }
        // manette (boutons physiques, quelle que soit l'attribution) : Back + Start ouvre
        // le menu, croix ou stick et A / B pour naviguer
        let padmap = [(0x0030u16, 0), (0x0001, 1), (0x0002, 2), (0x0004, 3), (0x0008, 4), (0x1000, 5), (0x2000, 8)];
        for (m, bit) in padmap {
            if pad_bits & m == m {
                now_keys |= 1 << bit;
            }
        }
        let hit = |i: u32| now_keys & (1 << i) != 0 && prev & (1 << i) == 0;
        let mut action = crate::menu::Action::None;
        // pendant l'attente d'une touche, F5 / F6 / F7 / F11 ne font rien d'autre
        let cap = menu.capturing();
        // attribution d'une touche : la première touche nouvellement appuyée
        let pressed = if focus && menu.open { new_key(&mut held) } else { None };
        if !(focus && menu.open) {
            held = [true; 256];
        }
        if menu.capturing_pad() {
            if pressed == Some(0x1B) {
                menu.capture_key(&mut cfg, 0x1B);
            } else if pad_new != 0 {
                menu.capture_pad(&mut cfg, 1 << pad_new.trailing_zeros());
            }
        } else if menu.capturing() {
            if let Some(vk) = pressed {
                menu.capture_key(&mut cfg, vk);
            } else if hit(8) {
                // Select sur la manette annule
                menu.capture_key(&mut cfg, 0x1B);
            }
        } else if menu.open {
            action = menu.input(&mut cfg, [hit(1), hit(2), hit(3), hit(4), hit(5) || hit(6) || hit(7), hit(0) || hit(8)]);
        } else if hit(0) {
            menu.open = true;
        }
        if hit(9) && !cap {
            action = crate::menu::Action::SaveState;
        }
        if hit(11) && !cap {
            action = crate::menu::Action::LoadState;
        }
        if hit(10) && !cap {
            cfg.slot = cfg.slot % 9 + 1;
            message = Some((format!("EMPLACEMENT {}", cfg.slot), Instant::now()));
        }
        if hit(12) && !cap {
            cfg.fullscreen = !cfg.fullscreen;
        }
        if hit(13) && !menu.open {
            paused = !paused;
        }
        prev = now_keys;
        match action {
            crate::menu::Action::Quit => break,
            crate::menu::Action::SaveState => {
                let p = crate::menu::state_path(&sav, cfg.slot);
                let msg = match std::fs::write(&p, gba.save_state()) {
                    Ok(()) => format!("ÉTAT {} SAUVEGARDÉ", cfg.slot),
                    Err(_) => "SAUVEGARDE IMPOSSIBLE".into(),
                };
                message = Some((msg, Instant::now()));
            }
            crate::menu::Action::LoadState => {
                let p = crate::menu::state_path(&sav, cfg.slot);
                let ok = std::fs::read(&p).map(|d| gba.load_state(&d)).unwrap_or(false);
                rembob.vider();
                let msg = if ok { format!("ÉTAT {} CHARGÉ", cfg.slot) } else { format!("EMPLACEMENT {} VIDE", cfg.slot) };
                message = Some((msg, Instant::now()));
            }
            crate::menu::Action::Level(n) => {
                // même chose que « recommencer », avec l'état du jeu imposé au premier
                // chargement de scène : le jeu démarre directement dans ce niveau
                write_save(&mut gba, &sav);
                let rom = gba.bus.rom().to_vec();
                gba = Gba::new(rom, std::fs::read(&sav).ok());
                gba.natives.enabled = cfg.natives || crate::recomp::data_only();
                gba.force_state = Some(n);
                rembob.vider();
                message = Some((format!("NIVEAU {}", n + 1 - crate::mods::NIVEAUX.start()), Instant::now()));
            }
            crate::menu::Action::Reset => {
                write_save(&mut gba, &sav);
                let rom = gba.bus.rom().to_vec();
                let natives = gba.natives.enabled;
                gba = Gba::new(rom, std::fs::read(&sav).ok());
                gba.natives.enabled = natives;
                rembob.vider();
                message = Some(("JEU RECOMMENCÉ".into(), Instant::now()));
            }
            _ => {}
        }
        if cfg != applied {
            if cfg.fullscreen != applied.fullscreen || cfg.scale != applied.scale {
                apply_size(hwnd, &cfg, Some(&applied), &mut win_rect);
            }
            gba.natives.enabled = cfg.natives || crate::recomp::data_only();
            cfg.save(&ini);
            applied = cfg.clone();
        }
        let turbo = focus && down(0x09) && !menu.open;
        let inactive = cfg.pause_unfocused && !focus;
        let running = !paused && !menu.open && !inactive;
        let rewinding = running && cfg.mod_rewind && ((focus && down(b'R' as i32)) || gachette);
        if !cfg.mod_rewind {
            rembob.vider();
        }

        let mut pcm: Vec<i16> = Vec::new();
        let mut rembob_vide = false;
        if rewinding {
            rembob_vide = !rembob.reculer(&mut gba);
            // silence de la durée d'une image : le son reste l'horloge
            pcm.resize(2 * 549, 0);
            fps_n += 1;
        } else if running {
            let n = if turbo { crate::menu::TURBO[cfg.turbo] } else { 1 };
            // vitesse du jeu (mod) : nombre d'images de jeu par image affichée
            let vitesse = crate::mods::VITESSES[cfg.mod_speed as usize];
            let n = if turbo {
                n
            } else {
                dette_vitesse += vitesse;
                let k = dette_vitesse / 100;
                dette_vitesse %= 100;
                k
            };
            for _ in 0..n {
                let keys = crate::mods::appliquer(&mut gba, &cfg, &mut suivi_mods, keys);
                gba.set_keys(keys);
                let t_emu = Instant::now();
                crate::icones::remplacer(&mut gba.bus, crate::icones::touche_icone(&cfg.keys[4]));
                gba.run_frame();
                temps_emu += t_emu.elapsed().as_secs_f64();
                frames += 1;
                pcm.extend(gba.bus.apu.buf.drain(..));
                if cfg.mod_rewind {
                    rembob.noter(&gba);
                }
            }
            fps_n += 1;
            if turbo {
                pcm.clear();
            } else if vitesse != 100 {
                pcm = crate::mods::son_vitesse(&mut stock_son, &pcm, vitesse);
            } else {
                stock_son.clear();
            }
            for s in pcm.iter_mut() {
                *s = (*s as i32 * cfg.volume as i32 / 10) as i16;
            }
        }

        // image affichée : correction des couleurs, menu, messages
        if cfg.colors != 0 {
            let table = if cfg.colors == 1 { &lcd } else { &vives };
            for (d, &s) in show.iter_mut().zip(&gba.frame) {
                let i = ((s >> 19) & 31) | (((s >> 11) & 31) << 5) | (((s >> 3) & 31) << 10);
                *d = table[i as usize];
            }
        } else {
            show.copy_from_slice(&gba.frame);
        }
        crate::mods::image(&cfg, &mut show);
        if !cfg.ghost {
            ghost_prev.clear();
        } else if running {
            crate::filtres::remanence(&mut show, &mut ghost_prev);
        }
        if menu.open {
            menu.draw(&cfg, &mut show);
        } else if paused {
            crate::menu::banner(&mut show, "PAUSE (P)");
        } else if inactive {
            crate::menu::banner(&mut show, "PAUSE (FENÊTRE INACTIVE)");
        } else if rewinding {
            let m = if rembob_vide { "<< DÉBUT DE L'HISTORIQUE".to_string() } else { format!("<< REMBOBINAGE ({:.0} S)", rembob.secondes()) };
            crate::menu::banner(&mut show, &m);
        }
        {
            // version finale : du code absent des données aurait dû s'exécuter
            let m = crate::recomp::MISSING.load(std::sync::atomic::Ordering::Relaxed);
            if m != 0 && !manque_note {
                manque_note = true;
                let t = format!("Code non recompilé en {:08X}
", m & 0x7FFF_FFFF);
                let _ = std::fs::write(sav.with_extension("erreur.txt"), &t);
                message = Some((format!("CODE ABSENT EN {:08X}", m & 0x7FFF_FFFF), Instant::now()));
            }
        }
        if let Some((m, t)) = &message {
            if t.elapsed() < Duration::from_secs(2) {
                crate::menu::banner(&mut show, m);
            } else {
                message = None;
            }
        }
        gba.bus.apu.soft = cfg.soft_sound;
        afficheur.envoyer(&show, &cfg);

        // cadence : le son sert d'horloge ; sinon minuterie
        match &mut audio {
            Some(a) if running && !turbo => {
                a.play(&pcm);
                if a.starved != coupures_vues {
                    coupures_vues = a.starved;
                    // les deux premières secondes (démarrage) ne comptent pas
                    if frames > 120 && avance < 5 {
                        avance += 1;
                    }
                }
                let t = Instant::now();
                while a.queued() >= avance && t.elapsed() < Duration::from_millis(100) {
                    attendre(Duration::from_millis(1));
                }
            }
            _ => {
                if !turbo {
                    next += frame_time;
                    let now = Instant::now();
                    if next > now {
                        attendre(next - now);
                    } else if now - next > Duration::from_millis(100) {
                        next = now;
                    }
                }
            }
        }

        if last_save.elapsed() > Duration::from_secs(1) {
            write_save(&mut gba, &sav);
            last_save = Instant::now();
        }
        if fps_t.elapsed() >= Duration::from_secs(1) {
            fps = fps_n as f64 / fps_t.elapsed().as_secs_f64();
            let title = format!(
                "Lady Sia{}{}{}{} — Échap : options",
                if cfg.fps { format!(" — {fps:.0} images/s") } else { String::new() },
                if turbo { " — accéléré" } else { "" },
                if audio.is_none() { " — sans son" } else { "" },
                if paused { " — pause" } else { "" }
            );
            unsafe { win::SetWindowTextW(hwnd, wide(&title).as_ptr()) };
            fps_t = Instant::now();
            fps_n = 0;
        }
    }
    write_save(&mut gba, &sav);
    if cfg.resume {
        let _ = std::fs::write(&reprise, gba.save_state());
    }
    if autoquit.is_some() {
        println!(
            "bilan : {frames} images, {fps:.1} images/s, pc = {:08X}, son {}{}\n{}",
            gba.cpu.r[15],
            if audio.is_some() { "oui" } else { "non" },
            audio.as_ref().map_or(String::new(), |a| format!(
                " (coupures {}, tampons perdus {}, avance {avance}) ; émulation {:.1} ms/image ; {}",
                a.starved,
                a.dropped,
                temps_emu * 1000.0 / frames.max(1) as f64,
                afficheur.bilan()
            )),
            gba.natives.report()
        );
    }
    Ok(())
}

/// Taille de fenêtre, ou plein écran (fenêtre sans bord sur tout le moniteur).
fn apply_size(hwnd: win::HWND, cfg: &crate::menu::Settings, old: Option<&crate::menu::Settings>, saved: &mut win::RECT) {
    unsafe {
        let was_full = old.is_some_and(|o| o.fullscreen);
        if cfg.fullscreen {
            if !was_full {
                win::GetWindowRect(hwnd, saved);
            }
            let mon = win::MonitorFromWindow(hwnd, 2);
            let mut mi: win::MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<win::MONITORINFO>() as u32;
            win::GetMonitorInfoW(mon, &mut mi);
            let r = &mi.rcMonitor;
            win::SetWindowLongPtrW(hwnd, win::GWL_STYLE, (win::WS_POPUP | win::WS_VISIBLE) as isize);
            win::SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                win::SWP_FRAMECHANGED | win::SWP_NOZORDER,
            );
        } else {
            win::SetWindowLongPtrW(hwnd, win::GWL_STYLE, (win::WS_OVERLAPPEDWINDOW | win::WS_VISIBLE) as isize);
            let s = cfg.scale as i32;
            let mut r = win::RECT { left: 0, top: 0, right: 240 * s, bottom: 160 * s };
            win::AdjustWindowRect(&mut r, win::WS_OVERLAPPEDWINDOW, 0);
            let (x, y) = if was_full { (saved.left, saved.top) } else { (0, 0) };
            let flags = win::SWP_FRAMECHANGED | win::SWP_NOZORDER | if was_full { 0 } else { win::SWP_NOMOVE };
            win::SetWindowPos(hwnd, std::ptr::null_mut(), x, y, r.right - r.left, r.bottom - r.top, flags);
        }
    }
}

/// Fil d'affichage : filtres et dessin dans la fenêtre, séparés de l'émulation et du
/// son. Si une image tarde (filtre HD, fil suspendu par Windows), le son continue et
/// l'image suivante remplace simplement celle en attente.
struct Afficheur {
    part: std::sync::Arc<(std::sync::Mutex<Boite>, std::sync::Condvar)>,
    fil: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Boite {
    image: Vec<u32>,
    reglages: Option<crate::menu::Settings>,
    neuve: bool,
    fin: bool,
    affichees: u64,
    sautees: u64,
    temps: f64,
    pire: f64,
}

impl Afficheur {
    fn lancer(hwnd: win::HWND) -> Afficheur {
        let part = std::sync::Arc::new((std::sync::Mutex::new(Boite::default()), std::sync::Condvar::new()));
        let p = part.clone();
        let h = hwnd as usize;
        let fil = std::thread::spawn(move || {
            let hwnd = h as win::HWND;
            let mut rendeur = crate::filtres::Rendeur::default();
            let mut image = Vec::new();
            loop {
                let cfg = {
                    let (m, cv) = &*p;
                    let mut b = m.lock().unwrap();
                    while !b.neuve && !b.fin {
                        b = cv.wait(b).unwrap();
                    }
                    if b.fin {
                        return;
                    }
                    b.neuve = false;
                    std::mem::swap(&mut image, &mut b.image);
                    b.reglages.clone().unwrap_or_default()
                };
                let t = Instant::now();
                present(hwnd, &image, &cfg, &mut rendeur);
                let d = t.elapsed().as_secs_f64();
                let mut b = p.0.lock().unwrap();
                b.affichees += 1;
                b.temps += d;
                b.pire = b.pire.max(d);
            }
        });
        Afficheur { part, fil: Some(fil) }
    }

    /// Dépose l'image à afficher (remplace celle pas encore affichée).
    fn envoyer(&self, img: &[u32], cfg: &crate::menu::Settings) {
        let (m, cv) = &*self.part;
        let mut b = m.lock().unwrap();
        if b.neuve {
            b.sautees += 1;
        }
        b.image.clear();
        b.image.extend_from_slice(img);
        if b.reglages.as_ref() != Some(cfg) {
            b.reglages = Some(cfg.clone());
        }
        b.neuve = true;
        cv.notify_one();
    }

    fn bilan(&self) -> String {
        let b = self.part.0.lock().unwrap();
        format!(
            "affichage {:.1} ms/image (pire {:.0} ms), {} images sautées",
            b.temps * 1000.0 / b.affichees.max(1) as f64,
            b.pire * 1000.0,
            b.sautees
        )
    }
}

impl Drop for Afficheur {
    fn drop(&mut self) {
        {
            let mut b = self.part.0.lock().unwrap();
            b.fin = true;
            self.part.1.notify_one();
        }
        if let Some(f) = self.fil.take() {
            let _ = f.join();
        }
    }
}

fn present(hwnd: win::HWND, frame: &[u32], cfg: &crate::menu::Settings, rendeur: &mut crate::filtres::Rendeur) {
    unsafe {
        let mut rc = win::RECT { left: 0, top: 0, right: 0, bottom: 0 };
        win::GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right, rc.bottom);
        let (w, h) = if cfg.stretch {
            (cw, ch)
        } else {
            let scale = (cw as f64 / 240.0).min(ch as f64 / 160.0);
            ((240.0 * scale) as i32, (160.0 * scale) as i32)
        };
        let (x, y) = ((cw - w) / 2, (ch - h) / 2);
        let rendu = crate::filtres::Rendu { filtre: cfg.filter, effet: cfg.effect };
        let simple = rendu.simple(h);
        let (bw, bh, pixels) = if simple {
            (240, 160, frame)
        } else {
            rendeur.rendre(frame, w, h, rendu);
            (w, h, &rendeur.out[..])
        };
        let bmi = win::BITMAPINFO {
            biSize: 40,
            biWidth: bw,
            biHeight: -bh,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
            colors: [0],
        };
        let dc = win::GetDC(hwnd);
        const BLACKNESS: u32 = 0x0000_0042;
        win::PatBlt(dc, 0, 0, x, ch, BLACKNESS);
        win::PatBlt(dc, x + w, 0, cw - x - w, ch, BLACKNESS);
        win::PatBlt(dc, 0, 0, cw, y, BLACKNESS);
        win::PatBlt(dc, 0, y + h, cw, ch - y - h, BLACKNESS);
        win::SetStretchBltMode(dc, if simple && cfg.filter == 1 { 4 } else { 3 });
        win::SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
        win::StretchDIBits(dc, x, y, w, h, 0, 0, bw, bh, pixels.as_ptr() as *const c_void, &bmi, 0, win::SRCCOPY);
        win::ReleaseDC(hwnd, dc);
    }
}
