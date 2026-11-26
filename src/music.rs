//! Rendu des musiques en exécutant le vrai pilote son du jeu dans l'émulateur.
//!
//! - `AgbMain` est exécuté jusqu'à la fin de l'initialisation du son (`sub_0802F51C`) ;
//! - `sub_08003A20(n°)` lance le morceau ;
//! - on simule ensuite le matériel : à chaque trame (59,73 Hz) le gestionnaire VBlank du
//!   son (`sub_0803142C` : séquenceur, instruments, canaux PSG), et l'IRQ Timer1
//!   (`sub_08031F64` : mixage DirectSound, code ARM copié en IWRAM) toutes les 128
//!   périodes d'échantillon ;
//! - les octets que le mixeur écrit dans le tampon lu par le DMA vers les FIFO sont remis
//!   bout à bout ; les écritures dans les registres PSG sont horodatées puis synthétisées
//!   (`psg.rs`) et mélangées comme sur la console.
//!
//! Format d'un morceau (banque `sound::BANK`, table `+0x18`) :
//! `+0x10` programmes (128 × `[instrument u16][b2][b3]`), `+0x210` idem pour le canal 9
//! (batterie), `+0x410` offset de la table des 17 pistes, `+0x414` offset de la table des
//! motifs, `+0x418` tempo. Piste : couples `[délai u32][motif u32]`, motif -1 = fin,
//! -2 = boucle (mot suivant = offset de reprise). Motif : événements
//! `[delta u16][vélocité s8][note (bit 7 = instrument suit)]…`, fin = `FFFF FFFF`.
//! 96 ticks par noire ; le séquenceur avance de tempo × 96 / 3600 ticks par trame.

use crate::emu::{Cpu, EmuError};
use crate::psg::Psg;
use crate::rom::{Rom, ROM_BASE};
use crate::sound;
use std::fmt::Write as _;

pub const AGB_MAIN: u32 = 0x0800_0308;
/// Juste après l'appel de l'initialisation du son dans `AgbMain`.
pub const AFTER_SOUND_INIT: u32 = 0x0800_03D0;
const PLAY_SONG: u32 = 0x0800_3A21;
const SOUND_VBLANK: u32 = 0x0803_142D;
const SOUND_TIMER1: u32 = 0x0803_1F65;
const CPU_HZ: f64 = 16_777_216.0;
const FRAME_HZ: f64 = 59.7275;
/// Suréchantillonnage de la sortie (les canaux PSG montent haut).
const OVERSAMPLE: usize = 2;

pub fn io16(cpu: &Cpu, o: usize) -> u32 {
    u16::from_le_bytes([cpu.io[o], cpu.io[o + 1]]) as u32
}
pub fn io32(cpu: &Cpu, o: usize) -> u32 {
    io16(cpu, o) | io16(cpu, o + 2) << 16
}

pub fn boot(rom: &Rom) -> Result<Cpu<'_>, EmuError> {
    let mut cpu = Cpu::new(rom);
    cpu.lenient = true;
    cpu.r[13] = 0x0300_7F00;
    cpu.r[14] = 0x0F00_0001;
    cpu.run_until(AGB_MAIN, AFTER_SOUND_INIT, 50_000_000)?;
    Ok(cpu)
}

fn u32_at(rom: &Rom, a: u32) -> u32 {
    rom.u32((a - ROM_BASE) as usize).unwrap_or(0)
}

pub struct SongInfo {
    pub addr: u32,
    pub tempo: u32,
    /// Durée d'un passage complet (jusqu'à la fin ou au point de boucle), en ticks.
    pub ticks: u32,
    /// Début de la partie qui boucle, en ticks (None : le morceau s'arrête).
    pub loop_from: Option<u32>,
    pub tracks: usize,
}

/// Lit la structure d'un morceau (sans l'exécuter).
pub fn song_info(rom: &Rom, n: u32) -> SongInfo {
    let table = sound::BANK + u32_at(rom, sound::BANK + 0x18);
    let addr = sound::BANK + u32_at(rom, table + 4 + 4 * n);
    let base = addr + 0x410;
    let tracks_tab = base + u32_at(rom, base);
    let (mut ticks, mut loop_from, mut count) = (0, None, 0);
    for k in 0..17 {
        let off = u32_at(rom, tracks_tab + 4 * k);
        if off == 0 {
            continue;
        }
        count += 1;
        let start = base + off;
        let mut p = start;
        let mut t = u32_at(rom, p);
        let mut times = vec![(start, 0u32)];
        for _ in 0..1000 {
            let pat = u32_at(rom, p + 4);
            if pat == u32::MAX {
                break;
            }
            if pat == 0xFFFF_FFFE {
                let target = start + u32_at(rom, p + 8);
                let at = times.iter().find(|&&(a, _)| a == target).map(|&(_, t)| t).unwrap_or(0);
                loop_from = Some(loop_from.map_or(at, |l: u32| l.min(at)));
                break;
            }
            p += 8;
            t += u32_at(rom, p);
            times.push((p, t));
        }
        ticks = ticks.max(t);
    }
    SongInfo { addr, tempo: u32_at(rom, base + 8) & 0xFFFF, ticks, loop_from, tracks: count }
}

pub struct Rendered {
    pub rate: u32,
    pub pcm: Vec<i16>,
}

/// Joue le morceau `song` pendant `seconds` secondes (fondu sur les `fade` dernières).
pub fn render(rom: &Rom, song: u32, seconds: f64, fade: f64) -> Result<Rendered, EmuError> {
    let mut cpu = boot(rom)?;
    let sample_cycles = (0x10000 - io16(&cpu, 0x100)) as f64;
    let rate = CPU_HZ / sample_cycles;
    let buf = io32(&cpu, 0xBC);
    let mut psg = Psg::default();
    for o in 0..0x40 {
        psg.write(o, cpu.io[0x60 + o]);
    }
    cpu.log_range = buf..buf + 0x1000;
    cpu.log2_range = 0x0400_0060..0x0400_00A0;
    cpu.call(PLAY_SONG, &[song], 20_000_000)?;

    let prescale = |ctrl: u32| [1.0, 64.0, 256.0, 1024.0][(ctrl & 3) as usize];
    let frame_cycles = CPU_HZ / FRAME_HZ;
    let frames = (seconds * FRAME_HZ) as usize + 2;
    // écritures PSG : (instant en échantillons DirectSound, registre, valeur)
    let mut events: Vec<(f64, usize, u8)> = Vec::new();
    let take_psg = |cpu: &mut Cpu, t: f64, ev: &mut Vec<(f64, usize, u8)>| {
        for (a, v) in cpu.log2.drain(..) {
            ev.push((t, (a - 0x0400_0060) as usize, v));
        }
    };
    let mut next_irq = 0.0f64;
    for f in 0..frames {
        let frame_start = f as f64 * frame_cycles;
        // IRQ Timer1 survenant avant cette trame
        while next_irq < frame_start {
            cpu.call(SOUND_TIMER1, &[], 50_000_000)?;
            take_psg(&mut cpu, next_irq / sample_cycles, &mut events);
            next_irq += (0x10000 - io16(&cpu, 0x104)) as f64 * prescale(io16(&cpu, 0x106));
        }
        cpu.call(SOUND_VBLANK, &[], 20_000_000)?;
        take_psg(&mut cpu, frame_start / sample_cycles, &mut events);
    }

    let dma: Vec<i8> = cpu.log.iter().map(|&(_, v)| v as i8).collect();
    let total = ((seconds * rate) as usize).min(dma.len());
    let out_rate = rate * OVERSAMPLE as f64;
    let dt = 1.0 / out_rate;
    let mut out = Vec::with_capacity(total * OVERSAMPLE);
    let mut e = 0;
    let (mut hp_x, mut hp_y) = (0.0f64, 0.0f64);
    let fade_start = (seconds - fade) * out_rate;
    for i in 0..total * OVERSAMPLE {
        let t = i as f64 / OVERSAMPLE as f64;
        while e < events.len() && events[e].0 <= t {
            psg.write(events[e].1, events[e].2);
            e += 1;
        }
        let x = psg.sample(dt, dma[i / OVERSAMPLE]);
        // passe-haut (comme le condensateur de sortie) : supprime la composante continue
        let y = x - hp_x + 0.999 * hp_y;
        hp_x = x;
        hp_y = y;
        let mut v = y * 48.0;
        if fade > 0.0 && i as f64 > fade_start {
            v *= (1.0 - (i as f64 - fade_start) / (fade * out_rate)).max(0.0);
        }
        out.push(v.clamp(-32768.0, 32767.0) as i16);
    }
    Ok(Rendered { rate: out_rate.round() as u32, pcm: out })
}

/// WAV mono 16 bits.
pub fn wav16(rate: u32, pcm: &[i16]) -> Vec<u8> {
    let n = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + n as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + n).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&n.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// Rend les 12 morceaux dans `extract/musique/`.
pub fn run(rom: &Rom) -> std::io::Result<String> {
    std::fs::create_dir_all("extract/musique")?;
    let mut idx = String::from("n°  adresse   tempo  pistes  passage  boucle\n");
    let mut ok = 0;
    for n in 0..12 {
        let info = song_info(rom, n);
        let tick_s = 3600.0 / (info.tempo.max(1) as f64 * 96.0) / FRAME_HZ;
        let pass = info.ticks as f64 * tick_s;
        // un passage complet, puis (si ça boucle) jusqu'à 20 s de reprise avec fondu
        let (secs, fade) = match info.loop_from {
            Some(l) => (pass + ((info.ticks - l) as f64 * tick_s).min(20.0), 8.0),
            None => (pass + 3.0, 0.0),
        };
        let secs = secs.clamp(5.0, 400.0);
        let lp = info.loop_from.map_or("non".to_string(), |l| format!("dès {:.1} s", l as f64 * tick_s));
        match render(rom, n, secs, fade) {
            Ok(r) => {
                std::fs::write(format!("extract/musique/musique_{n:02}.wav"), wav16(r.rate, &r.pcm))?;
                ok += 1;
                writeln!(idx, "{n:2}  {:08X}  {:5}  {:6}  {pass:6.1} s  {lp}", info.addr, info.tempo, info.tracks).unwrap();
            }
            Err(e) => writeln!(idx, "{n:2}  erreur d'émulation : {e:?}").unwrap(),
        }
    }
    std::fs::write("extract/musique/index.txt", &idx)?;
    Ok(format!("Musiques : {ok}/12 rendues par émulation du pilote -> extract/musique/\n"))
}
