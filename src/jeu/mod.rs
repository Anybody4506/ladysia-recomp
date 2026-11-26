//! La vraie décompilation : fonctions du jeu réécrites en Rust.
//!
//! Quand le processeur arrive au début d'une fonction listée ici, la version Rust
//! s'exécute à la place du code d'origine, puis revient à l'appelant (comme `bx lr`).
//! Une fonction réécrite peut à son tour appeler du code du jeu pas encore réécrit
//! (`Env::call`), qui peut lui-même retomber sur des fonctions réécrites.
//! Le jeu reste jouable pendant que les fonctions sont remplacées une à une.
//!
//! Vérification (`LADYSIA_CHECK=1`) : à chaque appel, une copie de la machine exécute
//! la fonction d'origine ; mémoire, registres conservés et valeur de retour doivent être
//! identiques à ceux de la version Rust, sinon la différence est signalée.

mod decomp;
pub mod entity;
mod map;
mod math;
mod sound;
mod video;

use crate::gba::bus::Bus;
use crate::gba::cpu::Cpu;
use std::cell::Cell;

/// Taille de la zone de code de la ROM (0x08000000-0x0803FFFF).
pub const CODE_SIZE: u32 = 0x4_0000;

/// Adresse de retour fictive des appels imbriqués (zone non utilisée).
const RET_MAGIC: u32 = 0x0F00_0000;

/// Données tirées de la ROM dont les fonctions réécrites ont besoin.
pub struct Ctx {
    pub packed: crate::packed::Tables,
}

/// Machine vue par une fonction réécrite.
pub struct Env<'a> {
    pub cpu: &'a mut Cpu,
    pub bus: &'a mut Bus,
    pub nat: &'a Natives,
}

impl Env<'_> {
    /// Appelle une fonction du jeu (Thumb si `addr` est impair) avec jusqu'à 4 arguments
    /// en registres et les suivants sur la pile ; renvoie r0.
    pub fn call(&mut self, addr: u32, args: &[u32]) -> u32 {
        let sp = self.cpu.r[13];
        let extra = args.len().saturating_sub(4) as u32;
        let new_sp = sp.wrapping_sub(((extra * 4) + 7) & !7);
        for (k, a) in args.iter().enumerate().skip(4) {
            self.bus.write32(new_sp + 4 * (k as u32 - 4), *a);
        }
        self.cpu.r[13] = new_sp;
        for (k, a) in args.iter().take(4).enumerate() {
            self.cpu.r[k] = *a;
        }
        self.cpu.r[14] = RET_MAGIC | 1;
        self.cpu.ret(addr);
        while self.cpu.r[15] != RET_MAGIC {
            if self.nat.enabled {
                if let Some(id) = self.nat.lookup(self.cpu.r[15], self.bus) {
                    self.nat.call(id, self.cpu, self.bus);
                    continue;
                }
            }
            if self.nat.enabled {
                if let Some(b) = crate::recomp::lookup(self.cpu.r[15], self.cpu.thumb(), self.bus) {
                    crate::recomp::run(b, self.cpu, self.bus);
                    self.bus.halt_req = false;
                    continue;
                }
            }
            crate::recomp::interpreting(self.cpu.r[15]);
            self.bus.count(self.cpu.r[15]);
            self.cpu.step(self.bus);
            self.bus.halt_req = false;
        }
        self.cpu.r[13] = sp;
        self.cpu.r[0]
    }

    // accès mémoire au format des instructions Thumb, y compris aux adresses non
    // alignées (rotation des mots / demi-mots, comme l'ARM7TDMI)
    pub fn ldr(&mut self, a: u32) -> u32 {
        self.bus.read32(a & !3).rotate_right((a & 3) * 8)
    }
    pub fn ldrh(&mut self, a: u32) -> u32 {
        self.bus.read16(a & !1).rotate_right((a & 1) * 8)
    }
    pub fn ldsh(&mut self, a: u32) -> i32 {
        if a & 1 != 0 {
            self.bus.read8(a) as u8 as i8 as i32
        } else {
            self.bus.read16(a) as u16 as i16 as i32
        }
    }
    pub fn ldrb(&mut self, a: u32) -> u32 {
        self.bus.read8(a)
    }
    pub fn ldsb(&mut self, a: u32) -> i32 {
        self.bus.read8(a) as u8 as i8 as i32
    }
    pub fn str(&mut self, a: u32, v: u32) {
        self.bus.write32(a & !3, v)
    }
    pub fn strh(&mut self, a: u32, v: u32) {
        self.bus.write16(a & !1, v as u16)
    }
    pub fn strb(&mut self, a: u32, v: u32) {
        self.bus.write8(a, v as u8)
    }
}

pub struct Native {
    pub addr: u32,
    pub name: &'static str,
    pub f: fn(&mut Env),
    /// Mémoire de travail privée de l'original (lue par aucun autre code du jeu,
    /// vérifié dans le désassemblage) : son contenu après l'appel peut différer.
    pub private: &'static [(u32, u32)],
    /// La fonction renvoie une valeur dans r0 (sinon r0 n'est pas comparé).
    pub returns: bool,
    /// Code généré en IWRAM : copie de la ROM `(source, longueur)`. La version Rust n'est
    /// utilisée que si l'IWRAM contient exactement ce code au moment de l'appel.
    pub copy_of: Option<(u32, u32)>,
    /// Résultat sur 64 bits (r1:r0) : r1 est aussi comparé.
    pub wide: bool,
}

/// Liste des fonctions remplacées.
fn list() -> Vec<Native> {
    vec![
        Native { addr: 0x0802_F0B0, name: "Decompress", f: decomp::decompress, private: &[], returns: true, copy_of: None, wide: false },
        // 0x02001224-0x02002058 : état de Packed_Decode / BuildTables / BuildHuffman /
        // ReadSymbol / Predict / Finish, fonctions appelées seulement depuis LoadPacked.
        Native { addr: 0x0802_EE7C, name: "LoadPacked", f: decomp::load_packed, private: &[(0x0200_1224, 0x0200_2058)], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0800_16C8, name: "Palette_Fade", f: video::palette_fade, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_A2E0, name: "Entity_OnScreen", f: video::entity_on_screen, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0802_A50C, name: "Entity_Hide", f: entity::entity_hide, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_A580, name: "VRAM_Alloc", f: entity::vram_alloc, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0802_A694, name: "VRAM_Free", f: entity::vram_free, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_9CD8, name: "Entity_Frame", f: entity::entity_frame, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_99E8, name: "Entities_Update", f: entity::entities_update, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_7498, name: "Map_DrawColumn", f: map::draw_column, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_7A50, name: "Layer_Scroll", f: map::layer_scroll, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_4544, name: "Music_Tick", f: sound::music_tick, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_1F64, name: "Sound_Timer1", f: sound::sound_timer1, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_142C, name: "Sound_VBlank", f: sound::sound_vblank, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_3568, name: "Instr_Run", f: sound::instr_run, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_08B0, name: "Pitch_Slides", f: sound::pitch_slides, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_93E8, name: "Map_TileAt", f: map::tile_at, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0803_5C74, name: "__divsi3", f: math::divsi3, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0803_5D40, name: "__modsi3", f: math::modsi3, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0803_6270, name: "__udivsi3", f: math::udivsi3, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0803_62E8, name: "__umodsi3", f: math::umodsi3, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0803_5E10, name: "__muldi3", f: math::muldi3, private: &[], returns: true, copy_of: None, wide: true },
        Native { addr: 0x0802_956C, name: "Map_Collide", f: map::collide, private: &[], returns: true, copy_of: None, wide: false },
        Native { addr: 0x0802_8D3C, name: "Entity_Probe", f: entity::entity_probe, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_D074, name: "Layers_Parallax", f: map::layers_parallax, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_83B0, name: "Bg_Regs", f: video::bg_regs_native, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_D858, name: "Bgs_Regs", f: video::bgs_regs, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_CFA8, name: "Camera_Move", f: video::camera_move, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0800_19E8, name: "Frame_Camera", f: video::frame_camera, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_C284, name: "Sprites_Upload", f: video::sprites_upload, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0802_D2A4, name: "Layers_VBlank", f: video::layers_vblank, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_09FC, name: "Vibrato", f: sound::vibrato, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_0BF8, name: "Pitch_Ramps", f: sound::pitch_ramps, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_0CD8, name: "Sample_Sweep", f: sound::sample_sweep, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_0D6C, name: "Loop_Sweep", f: sound::loop_sweep, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_0FD0, name: "Envelopes", f: sound::envelopes, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_1278, name: "Pan_Sweep", f: sound::pan_sweep, private: &[], returns: false, copy_of: None, wide: false },
        Native { addr: 0x0803_2484, name: "Instr_Wait", f: sound::instr_wait, private: &[], returns: true, copy_of: None, wide: false },
        // mixeur ARM assemblé en IWRAM par le pilote son
        Native { addr: 0x0300_0DC0, name: "Mix_Voice", f: sound::mix_voice, private: &[], returns: true, copy_of: Some((0x0803_5480, 0x18C)), wide: false },
        Native { addr: 0x0300_0F4C, name: "Mix_Clip", f: sound::mix_clip, private: &[], returns: true, copy_of: Some((0x0803_4EC8, 0x98)), wide: false },
    ]
}

pub struct Natives {
    pub enabled: bool,
    pub check: bool,
    pub list: Vec<Native>,
    index: Vec<u16>,
    /// Fonctions générées en IWRAM (cherchées par adresse exacte).
    iwram: Vec<usize>,
    pub ctx: Ctx,
    pub calls: Vec<Cell<u64>>,
    pub mismatches: Vec<Cell<u64>>,
    reports: Cell<u32>,
    /// Fonctions à mémoire privée appelées (bit = rang) pendant l'appel vérifié en cours :
    /// leur mémoire privée est aussi ignorée dans la vérification des fonctions appelantes.
    inner_private: Cell<u64>,
}

impl Natives {
    pub fn new(rom: &[u8]) -> Self {
        let list = list();
        let mut index = vec![0u16; (CODE_SIZE / 2) as usize];
        let mut iwram = Vec::new();
        for (i, n) in list.iter().enumerate() {
            if n.copy_of.is_some() {
                iwram.push(i);
            } else {
                index[((n.addr - 0x0800_0000) / 2) as usize] = i as u16 + 1;
            }
        }
        let r = crate::rom::Rom { data: rom.to_vec() };
        let n = list.len();
        assert!(n <= 64, "inner_private : 64 fonctions au plus");
        Natives {
            enabled: std::env::var("LADYSIA_NATIVE").map(|v| v != "0").unwrap_or(true),
            check: std::env::var("LADYSIA_CHECK").is_ok_and(|v| v == "1"),
            list,
            index,
            iwram,
            ctx: Ctx { packed: crate::packed::Tables::from_rom(&r) },
            calls: (0..n).map(|_| Cell::new(0)).collect(),
            mismatches: (0..n).map(|_| Cell::new(0)).collect(),
            reports: Cell::new(0),
            inner_private: Cell::new(0),
        }
    }

    /// Fonction réécrite qui commence à `off` (décalage dans la ROM).
    #[inline]
    pub fn at(&self, off: u32) -> Option<usize> {
        match self.index[(off >> 1) as usize] {
            0 => None,
            i => Some(i as usize - 1),
        }
    }

    /// Fonction réécrite qui commence à l'adresse `pc` (ROM ou code généré en IWRAM).
    #[inline]
    pub fn lookup(&self, pc: u32, bus: &Bus) -> Option<usize> {
        let off = pc.wrapping_sub(0x0800_0000);
        if off < CODE_SIZE {
            return self.at(off);
        }
        if pc.wrapping_sub(0x0300_0000) < 0x8000 {
            for &i in &self.iwram {
                let n = &self.list[i];
                if n.addr == pc {
                    let (src, len) = n.copy_of.unwrap();
                    if bus.iwram_is_copy(pc, src, len) {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    pub fn call(&self, id: usize, cpu: &mut Cpu, bus: &mut Bus) {
        let lr = cpu.r[14];
        let sp = cpu.r[13];
        let shadow = self.check.then(|| {
            let (mut c, mut b) = (cpu.clone(), bus.clone());
            let ok = run_original(&mut c, &mut b, lr, sp);
            (c, b, ok)
        });
        let outer = self.inner_private.replace(0);
        let mut env = Env { cpu: &mut *cpu, bus: &mut *bus, nat: self };
        (self.list[id].f)(&mut env);
        let inner = self.inner_private.get();
        let own = if self.list[id].private.is_empty() { 0 } else { 1u64 << id };
        self.inner_private.set(outer | inner | own);
        cpu.r[13] = sp;
        cpu.ret(lr);
        self.calls[id].set(self.calls[id].get() + 1);

        if let Some((c, b, ok)) = shadow {
            let mut diffs = Vec::new();
            if !ok {
                diffs.push("la fonction d'origine n'est pas revenue".to_string());
            }
            for r in [0usize, 1, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15] {
                if (r == 0 && !self.list[id].returns) || (r == 1 && !self.list[id].wide) {
                    continue;
                }
                if c.r[r] != cpu.r[r] {
                    diffs.push(format!("r{r} : origine {:08X}, Rust {:08X}", c.r[r], cpu.r[r]));
                }
            }
            let private: Vec<(u32, u32)> = (0..self.list.len())
                .filter(|&j| j == id || inner & (1 << j) != 0)
                .flat_map(|j| self.list[j].private.iter().copied())
                .collect();
            diffs.extend(
                b.diff(bus, sp)
                    .into_iter()
                    .filter(|(lo, hi, _)| !private.iter().any(|&(a, z)| *lo >= a && *hi <= z))
                    .map(|(_, _, t)| t),
            );
            if !diffs.is_empty() {
                self.mismatches[id].set(self.mismatches[id].get() + 1);
                if self.reports.get() < 30 {
                    self.reports.set(self.reports.get() + 1);
                    eprintln!(
                        "[vérif] {} (r0={:08X} à l'entrée) : {} différence(s) : {}",
                        self.list[id].name,
                        c.r[0],
                        diffs.len(),
                        diffs.iter().take(6).cloned().collect::<Vec<_>>().join(" ; ")
                    );
                }
            }
        }
    }

    pub fn report(&self) -> String {
        let mut s = String::new();
        for (i, n) in self.list.iter().enumerate() {
            s += &format!(
                "  {:08X} {:<16} {:>7} appels{}\n",
                n.addr,
                n.name,
                self.calls[i].get(),
                if self.check { format!(", {} différent(s) de l'original", self.mismatches[i].get()) } else { String::new() }
            );
        }
        s
    }
}

/// Exécute la fonction d'origine jusqu'à son retour (sans interruptions).
fn run_original(c: &mut Cpu, b: &mut Bus, lr: u32, sp: u32) -> bool {
    for _ in 0..200_000_000u64 {
        c.step(b);
        b.halt_req = false;
        if c.r[15] == lr & !1 && c.r[13] == sp {
            return true;
        }
    }
    false
}

/// Lit un mot de 32 bits non aligné (octet par octet, comme le jeu).
pub fn read_u32(bus: &mut Bus, a: u32) -> u32 {
    (0..4).fold(0, |v, k| v | (bus.read8(a + k) << (8 * k)))
}

/// Copie de `n` octets à partir de `a` (directement dans la ROM si possible).
pub fn bytes(bus: &mut Bus, a: u32, n: usize) -> Vec<u8> {
    if (0x0800_0000..0x0A00_0000).contains(&a) {
        let o = (a - 0x0800_0000) as usize;
        let rom = bus.rom();
        if o < rom.len() {
            let v = rom[o..(o + n).min(rom.len())].to_vec();
            bus.audited(o, v.len());
            return v;
        }
    }
    (0..n as u32).map(|k| bus.read8(a + k) as u8).collect()
}

/// Test aléatoire (`LADYSIA_FUZZ=n`) des fonctions du pilote son appelées sans argument :
/// à partir de l'état courant, les données du pilote (canaux PSG, voix, descripteurs de
/// lecture) sont tirées au hasard, puis l'original et la version Rust sont comparés.
/// Couvre les branches que la musique du test ne parcourt pas.
pub fn fuzz_sound(gba: &mut crate::gba::Gba, n: u32) -> String {
    const SOUND: u32 = 0x0300_6ED8;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };
    let targets = [0x0803_09FCu32, 0x0803_0BF8, 0x0803_0CD8, 0x0803_0D6C, 0x0803_0FD0, 0x0803_1278, 0x0803_08B0];
    let mut out = String::new();
    let check = gba.natives.check;
    gba.natives.check = true;
    for addr in targets {
        let id = gba.natives.at(addr - 0x0800_0000).expect("fonction réécrite");
        let before = gba.natives.mismatches[id].get();
        for _ in 0..n {
            let (mut cpu, mut bus) = (gba.cpu.clone(), gba.bus.clone());
            let g = bus.read32(SOUND);
            let fill = |bus: &mut Bus, a: u32, len: u32, rnd: &mut dyn FnMut() -> u32| {
                for k in (0..len).step_by(4) {
                    let mut v = rnd();
                    // petits pas et compteurs : les bornes sont atteintes plus souvent
                    if rnd() & 1 == 0 {
                        v &= 0x8007_0F0F;
                    }
                    bus.write32(a + k, v);
                }
            };
            fill(&mut bus, g + 0x18, 0xB0, &mut rnd);
            let voices = bus.read8(g + 8);
            for v in 0..voices {
                let t = bus.read32(g + 0xD4);
                let c = bus.read32(t + 4 * v);
                if c != 0 {
                    fill(&mut bus, c, 0x40, &mut rnd);
                }
                for t in [0xC8u32, 0xCC] {
                    let base = bus.read32(g + t);
                    let d = if t == 0xC8 { base + 0x1C * v } else { bus.read32(base + 4 * v) };
                    if d != 0 {
                        // +8 fin, +0xC position, +0x10 longueur de boucle (près de l'échantillon)
                        let s = bus.read32(d);
                        for (o, w) in [(8u32, 0x400u32), (0xC, 0x400)] {
                            bus.write32(d + o, s.wrapping_add(rnd() % w));
                        }
                        bus.write32(d + 0x10, rnd() % 0x100);
                    }
                }
            }
            // enveloppes : `[+0]` est un pointeur (ADSR) ou un pas de fondu ; jamais une
            // adresse sans mémoire (le bus ouvert y dépend de l'instruction en cours)
            let mut slots: Vec<(u32, u32)> = (0..4).map(|k| (g + 0x18 + 4 * k, g + 0x94 + 8 * k)).collect();
            for v in 0..voices {
                let t = bus.read32(g + 0xD4);
                let c = bus.read32(t + 4 * v);
                if c != 0 {
                    slots.push((c + 0x38, c + 0x24));
                }
            }
            for (fa, pa) in slots {
                match rnd() % 3 {
                    0 => bus.write32(pa, 0x0800_0000 + (rnd() % 0x3F_0000 & !1)),
                    1 => bus.write32(pa, rnd() & 0xFFF),
                    _ => {
                        bus.write32(pa, (rnd() & 0xFFF).wrapping_neg());
                        let f = bus.read32(fa);
                        bus.write32(fa, f & !1);
                    }
                }
            }
            cpu.r[13] = 0x0300_7E00;
            cpu.r[14] = RET_MAGIC | 1;
            cpu.ret(addr | 1);
            if gba.bus.profile.is_some() {
                // couverture : instructions de l'original parcourues pendant le test
                let (mut oc, mut ob) = (cpu.clone(), bus.clone());
                ob.profile = None;
                while oc.r[15] != RET_MAGIC {
                    gba.bus.count(oc.r[15]);
                    oc.step(&mut ob);
                    ob.halt_req = false;
                }
            }
            gba.natives.call(id, &mut cpu, &mut bus);
        }
        let n_bad = gba.natives.mismatches[id].get() - before;
        out += &format!("  {:08X} {:<16} {n} tirages, {n_bad} différent(s)\n", addr, gba.natives.list[id].name);
    }
    // Instr_Wait(n, slot) : instruction factice (opcode 4) en IWRAM libre
    let id = gba.natives.at(0x0003_2484).expect("Instr_Wait");
    let before = gba.natives.mismatches[id].get();
    for _ in 0..n {
        let (mut cpu, mut bus) = (gba.cpu.clone(), gba.bus.clone());
        let g = bus.read32(SOUND);
        let voices = bus.read8(g + 8);
        let ch = rnd() % (voices + 4);
        let slot = bus.read32(g + 0x11C) + 16 * ch;
        let code = 0x0300_7A00;
        let w = [0u32, 1, 2, 5, 0xFFFF, rnd() & 0xFFFF][(rnd() % 6) as usize];
        bus.write32(code, 4 | (rnd() & 7) << 8 | w << 16);
        bus.write32(slot, code);
        bus.write16(slot + 0xA, [0u16, 1, 2, rnd() as u16][(rnd() % 4) as usize]);
        for a in [g + 0x18 + 4 * ch.min(3)] {
            bus.write32(a, rnd());
        }
        if ch > 3 {
            let t = bus.read32(g + 0xD0);
            let c = (ch << 6).wrapping_add(t).wrapping_sub(0x100);
            bus.write32(c + 0x38, rnd());
            let p = bus.read32(g + 0xCC) + 4 * ch - 0x10;
            if rnd() & 1 == 0 {
                bus.write32(p, 0);
            }
        }
        cpu.r[0] = ch;
        cpu.r[1] = slot;
        cpu.r[13] = 0x0300_7E00;
        cpu.r[14] = RET_MAGIC | 1;
        cpu.ret(0x0803_2485);
        gba.natives.call(id, &mut cpu, &mut bus);
    }
    out += &format!("  08032484 Instr_Wait       {n} tirages, {} différent(s)
", gba.natives.mismatches[id].get() - before);
    gba.natives.check = check;
    out
}

/// Test aléatoire des fonctions d'affichage (`LADYSIA_FUZZ`, à lancer depuis une partie) :
/// variables des couches tirées au hasard (plan d'eau, ondulation, compteurs, poussée de
/// l'image, jeu figé), puis original et version Rust comparés `n` fois par fonction.
pub fn fuzz_video(gba: &mut crate::gba::Gba, n: u32) -> String {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };
    let mut out = String::new();
    let check = gba.natives.check;
    gba.natives.check = true;
    for addr in [0x0802_D2A4u32, 0x0802_CFA8, 0x0800_19E8] {
        let id = gba.natives.at(addr - 0x0800_0000).expect("fonction réécrite");
        let before = gba.natives.mismatches[id].get();
        for _ in 0..n {
            let (mut cpu, mut bus) = (gba.cpu.clone(), gba.bus.clone());
            let count = bus.read16(0x0300_6C94) as u32;
            let frozen = bus.read32(0x0300_6CA8);
            bus.write32(0x0300_6CA8, (frozen & !1) | (rnd() % 4 == 0) as u32);
            match addr {
                0x0802_D2A4 => {
                    let water = if rnd() % 4 == 0 || count == 0 { 0x100 } else { rnd() % count };
                    bus.write16(0x0300_6C70, water as u16);
                    bus.write16(0x0300_6C9C, if rnd() & 1 == 0 { 0x100 } else { (rnd() % 4) as u16 });
                    for a in [0x0300_6AFC, 0x0300_6C90, 0x0300_6AD0, 0x0300_6AD4] {
                        bus.write16(a, (rnd() % 5) as u16);
                    }
                    for a in [0x0300_6B00, 0x0300_6C7C, 0x0300_6A90, 0x0300_6AF8] {
                        bus.write16(a, rnd() as u16);
                    }
                    if rnd() & 1 == 0 {
                        bus.write16(0x0300_6AF8, (rnd() % 0x300) as u16);
                    }
                    bus.write32(0x0300_6B10 + 0x1C, rnd() % 0x4_0000);
                    bus.write16(0x0400_0004, rnd() as u16 & 0xFF3F);
                }
                0x0802_CFA8 => {
                    for a in [0x0300_6AE0, 0x0300_6AE2] {
                        bus.write16(a, (rnd() % 0x2000) as u16 ^ 0xF000 * (rnd() & 1) as u16);
                    }
                    cpu.r[0] = (rnd() % 0x1800).wrapping_sub(0xC00);
                    cpu.r[1] = (rnd() % 0x1800).wrapping_sub(0xC00);
                    cpu.r[2] = rnd();
                    cpu.r[3] = rnd();
                }
                _ => {
                    // sans cinématique (un pointeur au hasard lancerait un script quelconque)
                    bus.write32(0x0300_09A0, 0);
                    bus.write16(0x0300_1C88, rnd() as u16);
                    for a in [0x0300_6B30, 0x0300_6B32] {
                        bus.write16(a, (rnd() % 0x1000).wrapping_sub(0x800) as u16);
                    }
                }
            }
            cpu.r[13] = 0x0300_7E00;
            cpu.r[14] = RET_MAGIC | 1;
            cpu.ret(addr | 1);
            if gba.bus.profile.is_some() {
                let (mut oc, mut ob) = (cpu.clone(), bus.clone());
                ob.profile = None;
                while oc.r[15] != RET_MAGIC {
                    gba.bus.count(oc.r[15]);
                    oc.step(&mut ob);
                    ob.halt_req = false;
                }
            }
            gba.natives.call(id, &mut cpu, &mut bus);
        }
        let n_bad = gba.natives.mismatches[id].get() - before;
        out += &format!("  {:08X} {:<16} {n} tirages, {n_bad} différent(s)\n", addr, gba.natives.list[id].name);
    }
    gba.natives.check = check;
    out
}

/// Test aléatoire de la gestion de la VRAM des sprites (`LADYSIA_FUZZ`) : table
/// d'occupation tirée au hasard (blocs libres ou occupés, souvent pleine pour forcer la
/// fusion des blocs libres), puis `VRAM_Alloc` (taille au hasard) ou `VRAM_Free` (bloc au
/// hasard) comparés à l'original `n` fois chacune.
pub fn fuzz_vram(gba: &mut crate::gba::Gba, n: u32) -> String {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };
    let mut out = String::new();
    let check = gba.natives.check;
    gba.natives.check = true;
    for addr in [0x0802_A580u32, 0x0802_A694, 0x0802_A50C] {
        let id = gba.natives.at(addr - 0x0800_0000).expect("fonction réécrite");
        let before = gba.natives.mismatches[id].get();
        for _ in 0..n {
            let (mut cpu, mut bus) = (gba.cpu.clone(), gba.bus.clone());
            // table : blocs de 1 à 64 tuiles, occupés avec une probabilité variable
            let busy = rnd() % 100;
            let mut starts = Vec::new();
            let mut i = 0u32;
            while i < 1024 {
                let len = (1 + rnd() % 64).min(1024 - i);
                let used = rnd() % 100 < busy;
                for k in 0..len {
                    bus.write16(0x0300_4E50 + 2 * (i + k), if k == 0 { len as u16 | if used { 0x8000 } else { 0 } } else { (rnd() & 0x7FFF) as u16 });
                }
                starts.push(i);
                i += len;
            }
            let ent = 0x0203_F000;
            let typ = 0x0203_F100;
            match addr {
                0x0802_A580 => {
                    bus.write32(ent, typ);
                    bus.write16(typ + 8, ((1 + rnd() % 96) * 32) as u16);
                    cpu.r[0] = ent;
                }
                0x0802_A694 => {
                    let s = starts[rnd() as usize % starts.len()];
                    cpu.r[0] = if rnd() % 16 == 0 { 0 } else { 0x0601_0000 + 32 * s };
                }
                _ => {
                    bus.write8(0x0300_5A54, (rnd() % 34) as u8);
                    for k in 0..32 {
                        bus.write32(0x0300_1E50 + 8 * k, if rnd() % 8 == 0 { ent } else { 0x0203_E000 + 0x80 * (rnd() % 16) });
                    }
                    bus.write16(ent + 0x58, (rnd() & 0xFFFF) as u16);
                    for o in [0x30, 0x34] {
                        bus.write32(ent + o, if rnd() & 1 == 0 { 0 } else { 0x0203_E800 + 0x40 * (rnd() % 8) });
                    }
                    cpu.r[0] = ent;
                    cpu.r[1] = rnd();
                }
            }
            cpu.r[13] = 0x0300_7E00;
            cpu.r[14] = RET_MAGIC | 1;
            cpu.ret(addr | 1);
            gba.natives.call(id, &mut cpu, &mut bus);
        }
        let n_bad = gba.natives.mismatches[id].get() - before;
        out += &format!("  {:08X} {:<16} {n} tirages, {n_bad} différent(s)\n", addr, gba.natives.list[id].name);
    }
    gba.natives.check = check;
    out
}
