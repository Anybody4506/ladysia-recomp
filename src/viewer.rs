//! Visionneuse de niveaux (Windows, sans dépendance : API Win32 en direct).
//!
//! Affiche une scène comme la console : couches de décor avec leur défilement
//! différentiel (parallaxe), objets placés avec leur animation, musique du niveau.
//!
//! Touches : flèches = se déplacer (Maj = plus vite), Page suivante / précédente ou N / P =
//! scène suivante / précédente, Z = zoom (écran GBA 240×160 ou vue 2× plus large),
//! O = objets, M = musique, C = cadres des objets, Origine = revenir en haut à gauche,
//! Échap = quitter.

use crate::levels::{self, SceneView};
use crate::music;
use crate::objects::{self, Sprite, Sprites};
use crate::packed::Tables;
use crate::rom::{Rom, ROM_BASE};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- Win32 ----
#[allow(non_snake_case, clippy::upper_case_acronyms)]
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

    pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    pub const WS_VISIBLE: u32 = 0x1000_0000;
    pub const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
    pub const WM_DESTROY: u32 = 0x0002;
    pub const WM_QUIT: u32 = 0x0012;
    pub const PM_REMOVE: u32 = 1;
    pub const SRCCOPY: u32 = 0x00CC_0020;
    pub const SND_ASYNC: u32 = 0x0001;
    pub const SND_MEMORY: u32 = 0x0004;
    pub const SND_LOOP: u32 = 0x0008;
    pub const IDC_ARROW: usize = 32512;

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
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetModuleHandleW(name: *const u16) -> HANDLE;
    }
    #[link(name = "winmm")]
    unsafe extern "system" {
        pub fn PlaySoundW(sound: *const u8, module: HANDLE, flags: u32) -> i32;
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

/// Touche enfoncée (seulement si la fenêtre a le focus).
fn key(hwnd: win::HWND, vk: i32) -> bool {
    unsafe { win::GetForegroundWindow() == hwnd && win::GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

// ----------------------------------------------------------- Données ----

/// Animation 0 d'une définition : images et durées.
struct Anim {
    frames: Vec<(Sprite, u32)>,
    total: u32,
}

fn anim(sprites: &mut Sprites, def: u32) -> Option<Anim> {
    let n = *sprites.anims(def).first()?;
    let mut frames = Vec::new();
    for i in 0..n {
        if let Some(s) = sprites.frame(def, 0, i) {
            let d = (s.duration as u32).max(1);
            frames.push((s, d));
        }
    }
    let total = frames.iter().map(|f| f.1).sum();
    (!frames.is_empty()).then_some(Anim { frames, total })
}

/// Une entrée de la table des niveaux qui s'affiche.
struct Entry {
    state: u32,
    scene: SceneView,
    song: u32,
}

/// Morceau rendu (fichier WAV en mémoire, gardé vivant pendant la lecture).
enum Music {
    Off,
    Loading(u32),
    Playing(u32),
}

pub fn run(rom: &'static Rom) -> Result<(), String> {
    println!("Chargement des niveaux…");
    let tables: &'static Tables = Box::leak(Box::new(Tables::from_rom(rom)));
    let mut cache = BTreeMap::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let rd = |a: u32| rom.u32((a - ROM_BASE) as usize).unwrap_or(0);
    for state in 0..levels::LEVEL_COUNT {
        let e = levels::LEVEL_TABLE + 12 * state;
        let (s, song) = (rd(e), rd(e + 4));
        if !seen.insert(s) {
            continue;
        }
        match SceneView::load(rom, tables, &mut cache, s) {
            Some(scene) => entries.push(Entry { state, scene, song }),
            None => println!("  état {state:2} : scène {s:08X} illisible, ignorée"),
        }
    }
    drop(cache);
    if entries.is_empty() {
        return Err("aucune scène lisible".into());
    }
    println!("{} scènes chargées. Touches : flèches, Maj, N/P, Z, O, C, M, Échap.", entries.len());

    let mut sprites = Sprites::new(rom, tables);
    let mut anims: BTreeMap<u32, Option<Anim>> = BTreeMap::new();

    // Mode capture (vérification sans fenêtre) : une image par scène dans le dossier donné.
    if let Ok(dir) = std::env::var("LADYSIA_CAPTURE") {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut fb = Vec::new();
        for e in &entries {
            let r = e.scene.reference();
            // caméra centrée sur le premier objet de la couche de référence, sinon en haut à gauche
            let (ox, oy) = r.objects.first().map_or((0, 0), |o| (o.x - 240, o.y - 160));
            let cam_x = ox.clamp(0, (r.w as i32 - 480).max(0));
            let cam_y = oy.clamp(0, (r.h as i32 - 320).max(0));
            let view = View { cam_x, cam_y, w: 480, h: 320, objects: true, boxes: false, frame: 0 };
            draw_scene(&mut fb, rom, &e.scene, &view, &mut sprites, &mut anims);
            let rgb: Vec<u8> = fb.iter().flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]).collect();
            let name = format!("{dir}/etat{:02}_{:08X}.png", e.state, e.scene.addr);
            std::fs::write(name, crate::png::rgb(480, 320, &rgb)).map_err(|e| e.to_string())?;
        }
        println!("{} captures écrites dans {dir}", entries.len());
        return Ok(());
    }

    // musique : rendu par émulation dans un fil séparé, puis lecture en boucle
    let (to_worker, jobs) = mpsc::channel::<u32>();
    let (to_main, done) = mpsc::channel::<(u32, Option<Vec<u8>>)>();
    std::thread::spawn(move || {
        for song in jobs {
            let info = music::song_info(rom, song);
            let tick_s = 3600.0 / (info.tempo.max(1) as f64 * 96.0) / 59.7275;
            let secs = (info.ticks as f64 * tick_s).clamp(5.0, 180.0);
            let wav = music::render(rom, song, secs, 0.0).ok().map(|r| music::wav16(r.rate, &r.pcm));
            if to_main.send((song, wav)).is_err() {
                break;
            }
        }
    });
    let mut songs: BTreeMap<u32, Option<Vec<u8>>> = BTreeMap::new();
    let mut music = Music::Off;
    let mut music_on = true;

    // fenêtre
    let class = wide("LadySiaVisionneuse");
    let hwnd = unsafe {
        let inst = win::GetModuleHandleW(std::ptr::null());
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
        let title = wide("Lady Sia");
        win::CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
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
        return Err("impossible de créer la fenêtre".into());
    }

    // départ sur le premier niveau jouable (état 16), sinon la première scène
    let mut cur = entries.iter().position(|e| e.state >= 16).unwrap_or(0);
    let (mut cam_x, mut cam_y) = (0i32, 0i32);
    let mut zoom = false;
    let mut show_objects = true;
    let mut show_boxes = false;
    let mut frame: u64 = 0;
    let mut prev_keys = [false; 256];
    let mut scene_changed = true;
    let mut last_title = String::new();
    let mut fb: Vec<u32> = Vec::new();

    // LADYSIA_AUTOQUIT=n : ferme après n secondes en affichant un bilan (test automatique)
    let autoquit = std::env::var("LADYSIA_AUTOQUIT").ok().and_then(|v| v.parse::<f64>().ok());
    let launched = Instant::now();
    'main: loop {
        let start = Instant::now();
        if let Some(limit) = autoquit {
            if launched.elapsed().as_secs_f64() > limit {
                let state = match music {
                    Music::Off => "coupée".to_string(),
                    Music::Loading(s) => format!("morceau {s} en préparation"),
                    Music::Playing(s) => format!("morceau {s} en lecture"),
                };
                println!("bilan : {frame} images affichées en {limit} s, musique {state}");
                break;
            }
        }
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

        // --- entrées ---
        let mut pressed = |vk: i32| {
            let now = key(hwnd, vk);
            let was = std::mem::replace(&mut prev_keys[vk as usize], now);
            now && !was
        };
        if pressed(0x1B) {
            break;
        }
        let n = entries.len();
        if pressed(0x22) || pressed(b'N' as i32) {
            cur = (cur + 1) % n;
            scene_changed = true;
        }
        if pressed(0x21) || pressed(b'P' as i32) {
            cur = (cur + n - 1) % n;
            scene_changed = true;
        }
        if pressed(b'Z' as i32) {
            zoom = !zoom;
        }
        if pressed(b'O' as i32) {
            show_objects = !show_objects;
        }
        if pressed(b'C' as i32) {
            show_boxes = !show_boxes;
        }
        if pressed(b'M' as i32) {
            music_on = !music_on;
            scene_changed = true;
        }
        if pressed(0x24) {
            cam_x = 0;
            cam_y = 0;
        }
        let speed = if key(hwnd, 0x10) { 12 } else { 3 };
        if key(hwnd, 0x25) {
            cam_x -= speed;
        }
        if key(hwnd, 0x27) {
            cam_x += speed;
        }
        if key(hwnd, 0x26) {
            cam_y -= speed;
        }
        if key(hwnd, 0x28) {
            cam_y += speed;
        }

        let entry = &entries[cur];
        let scene = &entry.scene;
        let (vw, vh) = if zoom { (480usize, 320usize) } else { (240, 160) };
        let r = scene.reference();
        cam_x = cam_x.clamp(0, (r.w as i32 - vw as i32).max(0));
        cam_y = cam_y.clamp(0, (r.h as i32 - vh as i32).max(0));

        // --- musique ---
        while let Ok((song, wav)) = done.try_recv() {
            songs.insert(song, wav);
            scene_changed = true;
        }
        if scene_changed {
            scene_changed = false;
            let want = entry.song;
            let playing = matches!(music, Music::Playing(s) if s == want);
            if !music_on {
                unsafe { win::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
                music = Music::Off;
            } else if !playing {
                match songs.get(&want) {
                    Some(Some(wav)) => {
                        unsafe {
                            win::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
                            win::PlaySoundW(wav.as_ptr(), std::ptr::null_mut(), win::SND_ASYNC | win::SND_MEMORY | win::SND_LOOP);
                        }
                        music = Music::Playing(want);
                    }
                    Some(None) => {
                        unsafe { win::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
                        music = Music::Off;
                    }
                    None => {
                        unsafe { win::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
                        if !matches!(music, Music::Loading(s) if s == want) {
                            let _ = to_worker.send(want);
                        }
                        music = Music::Loading(want);
                    }
                }
            }
        }

        // --- rendu ---
        let view = View { cam_x, cam_y, w: vw, h: vh, objects: show_objects, boxes: show_boxes, frame };
        draw_scene(&mut fb, rom, scene, &view, &mut sprites, &mut anims);

        // --- affichage ---
        unsafe {
            let mut rc = win::RECT { left: 0, top: 0, right: 0, bottom: 0 };
            win::GetClientRect(hwnd, &mut rc);
            let bmi = win::BITMAPINFO {
                biSize: 40,
                biWidth: vw as i32,
                biHeight: -(vh as i32),
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
            win::StretchDIBits(
                dc,
                0,
                0,
                rc.right,
                rc.bottom,
                0,
                0,
                vw as i32,
                vh as i32,
                fb.as_ptr() as *const _,
                &bmi,
                0,
                win::SRCCOPY,
            );
            win::ReleaseDC(hwnd, dc);
        }

        let objs: usize = scene.layers.iter().map(|l| l.objects.len()).sum();
        let mus = match music {
            Music::Off if !music_on => "musique coupée (M)".to_string(),
            Music::Off => format!("musique {} indisponible", entry.song),
            Music::Loading(s) => format!("musique {s} : préparation…"),
            Music::Playing(s) => format!("musique {s}"),
        };
        let title = format!(
            "Lady Sia — scène {}/{} (état {}) {:08X} — {} couches, {objs} objets — x={cam_x} y={cam_y} / {}×{} — {mus}",
            cur + 1,
            entries.len(),
            entry.state,
            scene.addr,
            scene.layers.len(),
            r.w,
            r.h
        );
        if title != last_title {
            let w = wide(&title);
            unsafe { win::SetWindowTextW(hwnd, w.as_ptr()) };
            last_title = title;
        }

        frame += 1;
        let target = Duration::from_micros(16_742); // 59,73 Hz comme la GBA
        if let Some(rest) = target.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    unsafe { win::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
    Ok(())
}

/// Paramètres d'une image à dessiner.
struct View {
    cam_x: i32,
    cam_y: i32,
    w: usize,
    h: usize,
    objects: bool,
    boxes: bool,
    frame: u64,
}

/// Dessine la scène vue depuis la caméra (coordonnées de la couche de référence).
fn draw_scene(
    fb: &mut Vec<u32>,
    rom: &Rom,
    scene: &SceneView,
    v: &View,
    sprites: &mut Sprites,
    anims: &mut BTreeMap<u32, Option<Anim>>,
) {
    let (cam_x, cam_y, vw, vh, frame) = (v.cam_x, v.cam_y, v.w, v.h, v.frame);
    let (show_objects, show_boxes) = (v.objects, v.boxes);
    let r = scene.reference();
    fb.clear();
    fb.resize(vw * vh, pack(scene.colors[0]));
    let scroll = |l: &levels::ViewLayer| -> (i32, i32) {
        let s = |c: i32, f: u32, fref: u32| if f == 0 || fref == 0 { 0 } else { c * f as i32 / fref as i32 };
        (s(cam_x, l.fx, r.fx), s(cam_y, l.fy, r.fy))
    };
    let mut order: Vec<&levels::ViewLayer> = scene.layers.iter().collect();
    order.sort_by_key(|l| (std::cmp::Reverse(l.prio), std::cmp::Reverse(l.index)));
    for l in &order {
        let (sx, sy) = scroll(l);
        for y in 0..vh {
            let ly = (sy + y as i32).rem_euclid(l.h as i32) as usize;
            let row = &l.px[ly * l.w..(ly + 1) * l.w];
            let out = &mut fb[y * vw..(y + 1) * vw];
            let mut lx = sx.rem_euclid(l.w as i32) as usize;
            for o in out.iter_mut() {
                let c = row[lx];
                if c != 0 {
                    *o = pack(scene.colors[c as usize]);
                }
                lx += 1;
                if lx == l.w {
                    lx = 0;
                }
            }
        }
    }
    if show_objects {
        for l in &scene.layers {
            let (sx, sy) = scroll(l);
            for o in &l.objects {
                let (x, y) = (o.x - sx, o.y - sy);
                if x < -256 || y < -256 || x > vw as i32 + 256 || y > vh as i32 + 256 {
                    continue;
                }
                let a = anims.entry(o.def).or_insert_with(|| anim(sprites, o.def));
                let flags = (o.params[2] >> 8) & 3;
                let Some(a) = a else {
                    cross(fb, vw, vh, x, y, 0xFF00FF);
                    continue;
                };
                // image courante (60 images/s, durées du jeu)
                let mut t = (frame % a.total as u64) as u32;
                let mut img = &a.frames[0].0;
                for (s, d) in &a.frames {
                    if t < *d {
                        img = s;
                        break;
                    }
                    t -= d;
                }
                let bank = objects::palette_bank(rom, scene.addr, img.type_id) as usize;
                draw_sprite(fb, vw, vh, img, x, y, flags, |c| pack(scene.colors[256 + bank * 16 + c as usize]));
                if show_boxes {
                    cross(fb, vw, vh, x, y, 0xFFFF00);
                }
            }
        }
    }

}

fn pack(c: [u8; 3]) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

fn put(fb: &mut [u32], w: usize, h: usize, x: i32, y: i32, c: u32) {
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
        fb[y as usize * w + x as usize] = c;
    }
}

fn cross(fb: &mut [u32], w: usize, h: usize, x: i32, y: i32, c: u32) {
    for d in -3..=3 {
        put(fb, w, h, x + d, y, c);
        put(fb, w, h, x, y + d, c);
    }
}

/// Dessine une image d'objet ancrée en (x, y) ; `flags` : 1 = miroir H, 2 = miroir V.
fn draw_sprite(fb: &mut [u32], w: usize, h: usize, s: &Sprite, x: i32, y: i32, flags: u32, color: impl Fn(u8) -> u32) {
    for sy in 0..s.h {
        for sx in 0..s.w {
            let c = s.px[sy * s.w + sx];
            if c == 0 {
                continue;
            }
            let (mut dx, mut dy) = (s.ox + sx as i32, s.oy + sy as i32);
            if flags & 1 != 0 {
                dx = -1 - dx;
            }
            if flags & 2 != 0 {
                dy = -1 - dy;
            }
            put(fb, w, h, x + dx, y + dy, color(c));
        }
    }
}
