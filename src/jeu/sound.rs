//! Mixeur du pilote son (code ARM que le pilote assemble en IWRAM).
//!
//! Toutes les 128 périodes d'échantillon (IRQ Timer1), chaque voix active est ajoutée
//! dans un tampon 16 bits (`Mix_Voice`), puis le tampon est écrêté en octets signés
//! pour la FIFO DirectSound et remis à zéro (`Mix_Clip`).

use super::Env;

/// Échantillons produits par appel.
const BLOCK: u32 = 128;

/// `Mix_Voice(voix, tampon)` (copie de 0x08035480 en 0x03000DC0) : ajoute 128
/// échantillons de la voix au tampon 16 bits.
///
/// Voix : `+4` position (octets PCM 8 bits signés), `+8` fin, `+0xC` début de boucle
/// (0 = pas de boucle), `+0x10` longueur de boucle, `+0x14` pas (4.12), `+0x16` fraction,
/// `+0x18/+0x19` volumes gauche / droite (ce mixeur utilise leur moyenne).
/// Lecture à l'envers si la fin est avant la position. Renvoie 1 quand la voix se
/// termine (sans mettre à jour sa position), 0 sinon.
pub fn mix_voice(env: &mut Env) {
    let v = env.cpu.r[0];
    let mut acc = env.cpu.r[1];
    let mut pos = env.ldr(v + 4);
    let mut end = env.ldr(v + 8);
    let mut frac = env.ldrh(v + 0x16);
    let step = env.ldrh(v + 0x14);
    let mut vol = env.ldrb(v + 0x18) + env.ldrb(v + 0x19);
    if vol != 0 {
        vol += 2;
    }
    vol >>= 1;
    let backwards = (end.wrapping_sub(pos) as i32) < 0;

    for _ in 0..BLOCK {
        let s = env.ldsb(pos);
        let a = env.ldsh(acc);
        env.strh(acc, a.wrapping_add(s.wrapping_mul(vol as i32) >> 8) as u32);
        acc = acc.wrapping_add(2);
        frac = frac.wrapping_add(step);
        if frac < 0x1000 {
            continue;
        }
        let adv = frac >> 12;
        frac &= 0xFFFF_0FFF; // comme l'original : seuls les bits 12-15 sont effacés
        pos = if backwards { pos.wrapping_sub(adv) } else { pos.wrapping_add(adv) };
        let past = if backwards { pos <= end } else { pos >= end };
        if past {
            let start = env.ldr(v + 0xC);
            if start == 0 {
                env.cpu.r[0] = 1;
                return;
            }
            pos = pos.wrapping_sub(end).wrapping_add(start);
            let len = env.ldr(v + 0x10);
            end = if backwards { start.wrapping_sub(len) } else { start.wrapping_add(len) };
            env.str(v + 8, end);
            let inside = if backwards { pos > end } else { pos < end };
            if !inside {
                pos = start;
            }
        }
    }
    env.str(v + 4, pos);
    env.strh(v + 0x16, frac);
    env.cpu.r[0] = 0;
}

/// `Mix_Clip(tampon, sortie)` (copie de 0x08034EC8 en 0x03000F4C) : écrête les 128
/// valeurs 16 bits à -128..127, les écrit en octets dans la sortie et remet le tampon
/// à zéro.
pub fn mix_clip(env: &mut Env) {
    let mut acc = env.cpu.r[0];
    let mut out = env.cpu.r[1];
    for _ in 0..BLOCK / 4 {
        let mut word = 0u32;
        for k in 0..4 {
            let s = env.ldsh(acc + 2 * k).clamp(-128, 127);
            word |= (s as u32 & 0xFF) << (8 * k);
        }
        env.str(acc, 0);
        env.str(acc + 4, 0);
        env.str(out, word);
        acc += 8;
        out += 4;
    }
}

const SOUND: u32 = 0x0300_6ED8; // pointeur vers l'état du pilote
const NOTE_RELEASE: u32 = 0x0803_196D;
const NOTE_ON: u32 = 0x0803_4361;
const TEMPO_SET: u32 = 0x0803_3D4D;

/// `[[a] + off]`
fn deref(env: &mut Env, a: u32, off: u32) -> u32 {
    let p = env.ldr(a);
    env.ldr(p.wrapping_add(off))
}

/// Nombre de voix DirectSound du pilote.
fn voices(env: &mut Env) -> u32 {
    let g = env.ldr(SOUND);
    env.ldrb(g + 8)
}

/// Fin de liste : deux halfwords 0xFFFF.
fn is_end(env: &mut Env, p: u32) -> bool {
    env.ldrh(p) == 0xFFFF && env.ldrh(p + 2) == 0xFFFF
}

/// Retire `dt` au compte à rebours `[a]` ; vrai s'il est écoulé.
fn count_down(env: &mut Env, a: u32, dt: u32) -> bool {
    let v = env.ldr(a).wrapping_sub(dt);
    env.str(a, v);
    v as i32 <= 0
}

/// Ajoute `delay` ticks (× 256) au compte à rebours `[a]` ; vrai s'il reste écoulé.
fn add_delay(env: &mut Env, a: u32, delay: u32) -> bool {
    let v = env.ldr(a).wrapping_add(delay << 8);
    env.str(a, v);
    v as i32 <= 0
}

/// `Music_Tick()` (0x08034544) : avance le morceau en cours de `dt` (`+0x14C`).
///
/// État du morceau `S = [pilote + 0x120]`, base du morceau `B = [S + 0x114]` :
/// 1. durée des notes jouées (`[S+0x110]`, 12 octets par voix) : fin -> relâchement ;
/// 2. 17 pistes `S + 8n` (`[entrée][attente]`) : entrée `[délai][motif]`, -1 = fin,
///    -2 = boucle ; le motif choisi va dans `S + 0x88 + 8n` (transposé de `+0x14E`
///    pour les 4 premières) ;
/// 3. 16 motifs en cours : événements `[délai u16][vélocité][note|0x80][durée…]`,
///    vélocité 0 = changement d'instrument (`S+0x118+n`) ou de volume général ;
/// 4. changements de tempo (`[S+0x108]`).
pub fn music_tick(env: &mut Env) {
    let g = env.ldr(SOUND);
    let s = env.ldr(g + 0x120);
    if env.ldrb(s + 0x151) & 2 == 0 {
        return;
    }
    let dt = env.ldrh(s + 0x14C);

    // 1. notes en cours
    let mut i = 0u32;
    while (i as i32) < voices(env) as i32 + 4 {
        let p = env.ldr(s + 0x110).wrapping_add(12 * i);
        if env.ldrb(p + 4) != 0 && count_down(env, p, dt) {
            env.call(NOTE_RELEASE, &[i]);
            env.strb(p + 4, 0);
        }
        i += 1;
    }

    // 2. pistes
    for n in 0..17u32 {
        let tr = s + 8 * n;
        if env.ldr(tr) == 0 || !count_down(env, tr + 4, dt) {
            continue;
        }
        let mut cmd = deref(env, tr, 4);
        loop {
            if cmd == 0xFFFF_FFFF {
                env.str(tr, 0);
                break;
            }
            let e = env.ldr(tr);
            let pat = env.ldr(e + 4);
            let delay;
            if pat == 0xFFFF_FFFE {
                // boucle : retour à l'entrée indiquée
                let next = e + 8;
                env.str(tr, next);
                let state = deref(env, SOUND, 0x120);
                let b = env.ldr(state + 0x114);
                let table = b.wrapping_add(env.ldr(b + 0x410)).wrapping_add(0x410);
                let first = env.ldr(table + 4 * n).wrapping_add(0x410);
                let target = b.wrapping_add(first).wrapping_add(env.ldr(e + 8));
                env.str(tr, target);
                delay = env.ldr(next + 4);
            } else {
                let b = env.ldr(s + 0x114);
                let table = b.wrapping_add(env.ldr(b + 0x414)).wrapping_add(0x410);
                let p = b.wrapping_add(env.ldr(table.wrapping_add(pat << 2)).wrapping_add(0x410));
                if env.ldrh(p) == 0xFFFF && env.ldrh(p + 2) == 0xFFFF {
                    env.str(s + 0x88 + 8 * n, 0);
                } else {
                    env.str(s + 0x88 + 8 * n, p);
                    let first = env.ldrh(p) << 8;
                    env.str(s + 0x8C + 8 * n, first);
                    if n <= 3 {
                        let shift = env.ldrh(s + 0x14E);
                        env.str(s + 0x8C + 8 * n, shift + first);
                    }
                }
                env.str(tr, e + 8);
                delay = env.ldr(e + 8);
            }
            if !add_delay(env, tr + 4, delay) {
                break;
            }
            cmd = deref(env, tr, 4);
        }
    }

    // 3. motifs
    for n in 0..16u32 {
        let m = s + 0x88 + 8 * n;
        let mut ev = env.ldr(m);
        if ev == 0 || !count_down(env, m + 4, dt) {
            continue;
        }
        let instr = s + 0x118 + n;
        loop {
            let vel = env.ldsb(ev + 2);
            let size;
            if vel > 0 {
                let note = env.ldrb(ev + 3);
                if note & 0x80 == 0 {
                    let len = env.ldrh(ev + 4);
                    if len != 0 {
                        let v = env.ldsb(ev + 2) as u32;
                        let note = env.ldrb(ev + 3);
                        env.call(NOTE_ON, &[s, n, v, note, len]);
                    }
                    size = 6;
                } else {
                    // note avec changement d'instrument
                    let ins = env.ldrb(ev + 4);
                    env.strb(instr, ins);
                    if env.ldrh(ev + 4) != 0 {
                        let v = env.ldsb(ev + 2) as u32;
                        let note = env.ldrb(ev + 3) & 0x7F;
                        let len = env.ldrh(ev + 6);
                        env.call(NOTE_ON, &[s, n, v, note, len]);
                    }
                    size = 8;
                }
            } else if vel == 0 {
                let b = env.ldrb(ev + 3);
                if b & 0x80 == 0 {
                    env.strb(instr, b);
                } else {
                    let g = env.ldr(SOUND);
                    let max = env.ldrb(g + 8);
                    env.strb(g + 9, (b & 0x7F).min(max));
                }
                size = 4;
            } else {
                env.str(m, 0);
                break;
            }
            ev = env.ldr(m) + size;
            env.str(m, ev);
            if is_end(env, ev) {
                env.str(m + 4, 0);
                env.str(m, 0);
                break;
            }
            let d = env.ldrh(ev);
            if !add_delay(env, m + 4, d) {
                break;
            }
        }
    }

    // 4. tempo
    let t = s + 0x108;
    let mut p = env.ldr(t);
    if p == 0 || !count_down(env, t + 4, dt) {
        return;
    }
    loop {
        let bpm = env.ldrh(p + 2);
        env.strh(s + 0x14A, bpm);
        env.call(TEMPO_SET, &[bpm, s + 0x14A, p, 0x14A]);
        p = env.ldr(t) + 4;
        env.str(t, p);
        if env.ldrh(p + 2) == 0xFFFF && env.ldrh(p) == 0xFFFF {
            env.str(t + 4, 0);
            env.str(t, 0);
            return;
        }
        let d = env.ldrh(p);
        if !add_delay(env, t + 4, d) {
            return;
        }
    }
}

const VOICE_EFFECT: u32 = 0x0802_FF81;

/// `Sound_Timer1()` (0x08031F64) : IRQ Timer1, toutes les 128 périodes d'échantillon.
///
/// Tampon de sortie en anneau de 3 blocs de 128 octets par FIFO : au premier bloc, les
/// DMA 1 et 2 (FIFO A et B) sont relancées depuis le début. Chaque voix active
/// (`[pilote+0xCC]`) est ajoutée au tampon de mixage par la routine générée (`+0xD8` :
/// voix normale, `+0xDC` : voix avec bit `+0x1B`, qui se termine en renvoyant 1),
/// puis l'écho éventuel (`+0xE4`), puis l'écrêtage vers le bloc courant (`+0xE0`).
pub fn sound_timer1(env: &mut Env) {
    let g = env.ldr(SOUND);
    if env.ldrb(g + 8) == 0 {
        return;
    }
    let p = g + 0xD8;
    let mix = env.ldr(p);
    let mix_alt = env.ldr(p + 4);
    let clip = env.ldr(p + 8);
    let echo = env.ldr(p + 0xC);

    if env.ldrb(g + 0xF9) == 0 {
        // premier appel : démarre le Timer1 (prédiviseur 64, IRQ)
        env.strb(g + 0xF9, 1);
        env.strh(0x0400_0106, 0);
        let reload = env.ldrh(p + 0x22);
        env.str(0x0400_0104, 0x00C1_0000 | reload);
    }
    let blk = (env.ldrb(g + 0xF8) + 1) & 0xFF;
    env.strb(g + 0xF8, blk);
    if blk == 3 {
        env.strb(g + 0xF8, 0);
    }
    if env.ldrb(g + 0xF8) == 1 {
        // relance des DMA son au début du tampon
        env.strh(0x0400_00C6, 0x8440);
        env.strh(0x0400_00D2, 0x8440);
        env.strh(0x0400_00C6, 0x0440);
        env.strh(0x0400_00D2, 0x0440);
        let a = env.ldr(p + 0x14);
        env.str(0x0400_00BC, a);
        let b = env.ldr(p + 0x18);
        env.str(0x0400_00C8, b);
        env.str(0x0400_00C4, 0xB660_0004);
        env.str(0x0400_00D0, 0xB660_0004);
    }

    let mut v = 0u32;
    while (v as i32) < voices(env) as i32 {
        let slot = deref(env, SOUND, 0xCC).wrapping_add(4 * v);
        let voice = env.ldr(slot);
        if voice != 0 {
            let alt = env.ldrb(voice + 0x1B);
            let done = if alt == 0 {
                let acc = env.ldr(p + 0x10);
                env.call(mix, &[voice, acc, v, mix])
            } else {
                let acc = env.ldr(p + 0x24);
                env.call(mix_alt, &[voice, acc, v, mix_alt])
            };
            if done != 0 {
                let slot = deref(env, SOUND, 0xCC).wrapping_add(4 * v);
                env.str(slot, 0);
            }
            let ch = deref(env, SOUND, 0xD0).wrapping_add(v << 6);
            if env.ldr(ch + 0x38) & 0x2_0000 != 0 {
                env.call(VOICE_EFFECT, &[v, voice]);
            }
        }
        v += 1;
    }

    if env.ldrb(g + 0x116) != 0 {
        let acc = env.ldr(p + 0x10);
        env.call(echo, &[p + 0x24, acc, echo]);
    }
    let off = env.ldrb(g + 0xF8) << 7;
    let acc = env.ldr(p + 0x10);
    let out_a = env.ldr(p + 0x14).wrapping_add(off);
    let out_b = env.ldr(p + 0x18).wrapping_add(off);
    env.call(clip, &[acc, out_a, out_b]);
}

const VBLANK_STEPS: [u32; 8] = [
    0x0803_3569, 0x0803_08B1, 0x0803_09FD, 0x0803_0BF9, 0x0803_0CD9, 0x0803_0D6D, 0x0803_0FD1,
    0x0803_1279,
];
const PSG_SET_FREQ: u32 = 0x0803_0555;
const PSG_SET_VOLUME: u32 = 0x0803_0601;
const LOCK: u32 = 0x0803_0391;
const UNLOCK: u32 = 0x0803_036D;
const PITCH_STEP: u32 = 0x0803_2165;
const MUSIC_TICK: u32 = 0x0803_4545;
/// Adresses des registres de déclenchement des 4 canaux PSG.
const PSG_TRIGGER: u32 = 0x083F_42A8;

/// `(a * b) << 8 >> 16` sur 32 bits, comme le code Thumb.
fn mul_hi(a: u32, b: u32) -> u32 {
    (a.wrapping_mul(b) << 8) >> 16
}

/// `Sound_VBlank()` (0x0803142C) : mise à jour du pilote son à chaque image.
///
/// Après 8 étapes (enveloppes, vibratos, portamentos…), applique aux canaux ce qui a
/// changé : fréquence des 3 premiers canaux PSG (bit 0x40 de `+0x18+4n`), volume des 4
/// (bit 0x80), redéclenchement (bit 0x200) ; volume et panoramique des voix DirectSound
/// (bit 0x80 de `voix+0x38`), pas de lecture (bit 0x40), voix à démarrer (bit 0x200),
/// puis avance la musique (`Music_Tick`).
pub fn sound_vblank(env: &mut Env) {
    for f in VBLANK_STEPS {
        env.call(f, &[]);
    }

    // fréquences PSG
    for k in 0..3u32 {
        let g = env.ldr(SOUND);
        let fl = g + 0x18 + 4 * k;
        let v = env.ldr(fl);
        if v & 0x40 == 0 {
            continue;
        }
        env.str(fl, v & !0x40);
        let c = g + 0x24 * k;
        let mut sum = 0u32;
        for off in [0x28, 0x2A, 0x2C, 0x34, 0x3C, 0x44] {
            sum = env.ldrh(c + off).wrapping_add(sum);
        }
        env.call(PSG_SET_FREQ, &[(sum >> 8) & 0xFF, sum & 0xFF, k]);
    }

    // volumes et déclenchement PSG
    for k in 0..4u32 {
        let g = env.ldr(SOUND);
        let fl = g + 0x18 + 4 * k;
        let v = env.ldr(fl);
        if v & 0x80 != 0 {
            let rest = v & !0x80;
            env.str(fl, rest);
            let c = g + 8 * k;
            let mut a = env.ldrh(c + 0x98) >> 8;
            if a != 0 {
                a += 1;
            }
            let vol = mul_hi(env.ldrb(c + 0x9B), a);
            let master = if rest & 0x1_0000 != 0 { env.ldrh(g + 0xC6) } else { env.ldrh(g + 0xC4) };
            env.call(PSG_SET_VOLUME, &[mul_hi(master, vol), k]);
        }
        let g = env.ldr(SOUND);
        if env.ldr(g + 0x18 + 4 * k) & 0x200 != 0 {
            if k != 3 {
                let reg = env.ldr(PSG_TRIGGER + 4 * k);
                let f = env.ldrh(g + 0xBC + 2 * k);
                env.strh(reg, f | 0x8000);
            } else {
                let reg = env.ldr(PSG_TRIGGER + 0xC);
                let f = env.ldrh(reg);
                env.strh(reg, f | 0x8000);
            }
            let g = env.ldr(SOUND);
            let fl = g + 0x18 + 4 * k;
            let v = env.ldr(fl);
            env.str(fl, v & 0xFFFF_FDFF);
        }
    }

    // voix DirectSound
    let mut v = 0u32;
    while (v as i32) < voices(env) as i32 {
        let ch = deref(env, SOUND, 0xD4).wrapping_add(4 * v);
        let c = env.ldr(ch);
        if c != 0 {
            let fade = env.ldrh(c + 0x36);
            if fade != 0 {
                let n = (env.ldrh(c + 0x36) as i32 - env.ldrb(c + 0x3F) as i32).max(0);
                env.strh(c + 0x36, n as u32);
            }
            let fl = env.ldr(c + 0x38);
            if fl & 0x80 != 0 {
                env.str(c + 0x38, fl & !0x80);
                let mut a = env.ldrh(c + 0x28) >> 8;
                if a != 0 {
                    a += 1;
                }
                let vol = mul_hi(env.ldrb(c + 0x2B), a);
                env.strb(c + 0x2A, vol);
                let mixv = deref(env, SOUND, 0xC8).wrapping_add(0x1C * v);
                let pan = env.ldrb(c + 0x3C);
                let (left, right, mono);
                if pan <= 0x3F {
                    let pan = env.ldrb(c + 0x3C);
                    left = (((((0x40 - pan) * 0x4AC8) >> 6) + 0xB538) * vol >> 16) & 0xFF;
                    right = (((0xB818 * pan) >> 6) * vol >> 16) & 0xFF;
                    mono = 0;
                } else if pan < 0x80 {
                    let pan = env.ldrb(c + 0x3C);
                    left = ((((0x7F - pan) * 0xB818) >> 6) * vol >> 16) & 0xFF;
                    right = (((((pan - 0x3F) * 0x4AC8) >> 6) + 0xB538) * vol >> 16) & 0xFF;
                    mono = 0;
                } else {
                    right = (0xB538 * vol >> 16) & 0xFF;
                    left = right;
                    mono = 1;
                }
                let g = env.ldr(SOUND);
                let master = if env.ldr(c + 0x38) & 0x1_0000 != 0 { env.ldrh(g + 0xC6) } else { env.ldrh(g + 0xC4) };
                env.call(LOCK, &[]);
                env.strb(mixv + 0x18, (left * master) >> 8);
                env.strb(mixv + 0x19, (right * master) >> 8);
                env.strb(mixv + 0x1A, mono);
                env.call(UNLOCK, &[]);
            }
            let fl = env.ldr(c + 0x38);
            if fl & 0x40 != 0 {
                let fl = fl & !0x40;
                env.str(c + 0x38, fl);
                if fl & 0x1400 != 0 {
                    let mixv = deref(env, SOUND, 0xC8).wrapping_add(0x1C * v);
                    let mut sum = 0u32;
                    for off in [0, 2, 4, 0xC, 0x14, 0x1C] {
                        sum = env.ldrh(c + off).wrapping_add(sum);
                    }
                    let sum = sum & 0xFFFF;
                    let sample = env.ldr(mixv);
                    let step = env.call(PITCH_STEP, &[sample, sum >> 8, sum & 0xFF]);
                    env.strh(mixv + 0x14, step);
                }
            }
            if env.ldr(c + 0x38) & 0x8000 != 0 {
                let ch = deref(env, SOUND, 0xD4).wrapping_add(4 * v);
                env.str(ch, 0);
            }
        }
        v += 1;
    }

    // voix à démarrer
    env.call(LOCK, &[]);
    let mut v = 0u32;
    while (v as i32) < voices(env) as i32 {
        let g = env.ldr(SOUND);
        let table = env.ldr(g + 0xD4);
        let c = env.ldr(table.wrapping_add(4 * v));
        if c != 0 {
            let fl = env.ldr(c + 0x38);
            if fl & 0x200 != 0 && fl & 0x1400 != 0 {
                env.str(c + 0x38, fl & 0xFFFF_FDFF);
                let slot = env.ldr(g + 0xCC).wrapping_add(4 * v);
                let mixv = env.ldr(g + 0xC8).wrapping_add(0x1C * v);
                env.str(slot, mixv);
            }
        }
        v += 1;
    }
    env.call(UNLOCK, &[]);
    env.call(MUSIC_TICK, &[]);
}

/// Opérations du bytecode des instruments (une fonction par opcode).
const INSTR_OPS: u32 = 0x083F_4568;

/// `Instr_Run()` (0x08033568) : exécute le programme d'instrument de chaque canal
/// (voix DirectSound + 4 PSG) ; `[pilote+0x11C] + 16n` pointe sur l'instruction en
/// cours, dont l'octet 0 est l'opcode. On enchaîne tant que l'opération renvoie ≠ 0.
pub fn instr_run(env: &mut Env) {
    let mut n = 0u32;
    while (n as i32) < voices(env) as i32 + 4 {
        let slot = deref(env, SOUND, 0x11C).wrapping_add(16 * n);
        if env.ldr(slot) != 0 {
            loop {
                let slot = deref(env, SOUND, 0x11C).wrapping_add(16 * n);
                let code = env.ldr(slot);
                let op = env.ldrb(code);
                let f = env.ldr(INSTR_OPS + 4 * op);
                if env.call(f, &[n, slot, f]) == 0 {
                    break;
                }
            }
        }
        n += 1;
    }
}

/// Glissement d'une valeur 16 bits : `[p] += [p+2]`, et si la limite `[p+4]` (signée)
/// est dépassée dans le sens du glissement, on retire `[p+4]`. Vrai si actif.
fn slide(env: &mut Env, p: u32) -> bool {
    if env.ldsh(p + 2) == 0 {
        return false;
    }
    let v = env.ldrh(p + 2).wrapping_add(env.ldrh(p));
    env.strh(p, v);
    let wrap = env.ldrh(p + 4);
    let limit = env.ldsh(p + 4);
    let cur = v as u16 as i16 as i32;
    if (limit > 0 && cur > limit) || (limit <= 0 && cur < limit) {
        env.strh(p, v.wrapping_sub(wrap));
    }
    true
}

/// `Pitch_Slides()` (0x080308B0) : deux glissements de hauteur (vibrato / portamento)
/// pour les 3 canaux PSG mélodiques (`pilote + 0x24n + 0x2C`, `+0x34`) et pour chaque
/// voix DirectSound (`voix + 4`, `+0xC`) ; le canal est marqué « fréquence à
/// recalculer » (bit 0x40).
pub fn pitch_slides(env: &mut Env) {
    for k in 0..3u32 {
        for j in 0..2u32 {
            let g = env.ldr(SOUND);
            if slide(env, g + 0x24 * k + 0x28 + 4 + 8 * j) {
                let g = env.ldr(SOUND);
                let fl = env.ldr(g + 0x18 + 4 * k);
                env.str(g + 0x18 + 4 * k, fl | 0x40);
            }
        }
    }
    let mut v = 0u32;
    while (v as i32) < voices(env) as i32 {
        let table = deref(env, SOUND, 0xD4);
        let c = env.ldr(table.wrapping_add(4 * v));
        if c != 0 {
            for j in 0..2u32 {
                if slide(env, c + 4 + 8 * j) {
                    let fl = env.ldr(c + 0x38);
                    env.str(c + 0x38, fl | 0x40);
                }
            }
        }
        v += 1;
    }
}

/// Voix DirectSound `v` (0 si l'emplacement est libre).
fn voice(env: &mut Env, v: u32) -> u32 {
    let t = deref(env, SOUND, 0xD4);
    env.ldr(t.wrapping_add(4 * v))
}

/// Oscillation commune : `[p] += [q+2]` ; compteur `[q+4]` (recharge `[q+5]`), à
/// chaque recharge le pas change de signe.
fn wobble(env: &mut Env, p: u32, q: u32) -> bool {
    if env.ldsh(q + 2) == 0 {
        return false;
    }
    let v = env.ldrh(q + 2).wrapping_add(env.ldrh(p));
    env.strh(p, v);
    let n = env.ldrb(q + 4).wrapping_sub(1);
    env.strb(q + 4, n);
    if n & 0xFF == 0xFF {
        let r = env.ldrb(q + 5);
        env.strb(q + 4, r);
        let s = env.ldrh(q + 2);
        env.strh(q + 2, s.wrapping_neg());
    }
    true
}

/// Marque le canal PSG `k` (`pilote + 0x18 + 4k |= bit`).
fn psg_flag(env: &mut Env, k: u32, bit: u32) {
    let g = env.ldr(SOUND);
    let a = g + 0x18 + 4 * k;
    let f = env.ldr(a);
    env.str(a, f | bit);
}

fn voice_flag(env: &mut Env, c: u32, bit: u32) {
    let f = env.ldr(c + 0x38);
    env.str(c + 0x38, f | bit);
}

/// `Vibrato()` (0x080309FC) : oscillation de hauteur des 3 canaux PSG mélodiques
/// (`pilote + 0x28 + 0x24k + 0x14`) et des voix DirectSound (`voix + 0x14`).
pub fn vibrato(env: &mut Env) {
    for k in 0..3u32 {
        let c = env.ldr(SOUND) + 0x28 + 0x24 * k;
        if wobble(env, c + 0x14, c + 0x14) {
            psg_flag(env, k, 0x40);
        }
    }
    let mut v = 0;
    while v < voices(env) {
        let c = voice(env, v);
        if c != 0 && wobble(env, c + 0x14, c + 0x14) {
            voice_flag(env, c, 0x40);
        }
        v += 1;
    }
}

/// Rampe `[p] += [p+2]` bornée par `[p+4]` (le pas s'annule à la borne).
fn ramp(env: &mut Env, p: u32) {
    let step = env.ldsh(p + 2);
    let v = env.ldrh(p).wrapping_add(env.ldrh(p + 2));
    env.strh(p, v);
    let target = env.ldrh(p + 4);
    let (cur, lim) = (v as u16 as i16, target as u16 as i16);
    if (step > 0 && cur > lim) || (step <= 0 && cur < lim) {
        env.strh(p, target);
        env.strh(p + 2, 0);
    }
}

/// `Pitch_Ramps()` (0x08030BF8) : glissement de hauteur vers une note cible
/// (`+0x1C` : valeur, pas, cible) pour les 3 canaux PSG mélodiques et les voix.
pub fn pitch_ramps(env: &mut Env) {
    for k in 0..3u32 {
        let c = env.ldr(SOUND) + 0x28 + 0x24 * k;
        if env.ldsh(c + 0x1E) != 0 {
            ramp(env, c + 0x1C);
            psg_flag(env, k, 0x40);
        }
    }
    let mut v = 0;
    while v < voices(env) {
        let c = voice(env, v);
        if c != 0 && env.ldsh(c + 0x1E) != 0 {
            voice_flag(env, c, 0x40);
            ramp(env, c + 0x1C);
        }
        v += 1;
    }
}

/// `Sample_Sweep()` (0x08030CD8) : décalage oscillant du point de départ de
/// l'échantillon (bit 0x400 de `voix + 0x38`) entre `[+0x30]` et `[+0x31]` (×256),
/// écrit dans le descripteur de lecture `[pilote + 0xC8] + 0x1C v` (`+0xC`).
pub fn sample_sweep(env: &mut Env) {
    let mut p = deref(env, SOUND, 0xD4);
    let mut v = 0;
    while v < voices(env) {
        let c = env.ldr(p);
        if c != 0 && env.ldr(c + 0x38) & 0x400 != 0 {
            let s = c + 0x2C;
            let mut pos = env.ldsh(c + 0x2C) + env.ldsh(s + 2);
            let step = env.ldrh(s + 2);
            let up = env.ldsh(s + 2) >= 0;
            let lim = if up { env.ldsb(s + 5) } else { env.ldsb(s + 4) };
            if (up && pos >= lim << 8) || (!up && pos <= lim << 8) {
                pos = (lim << 9) - pos;
                env.strh(s + 2, step.wrapping_neg());
            }
            env.strh(s, pos as u32);
            let d = deref(env, SOUND, 0xC8) + 0x1C * v;
            let base = env.ldr(d);
            env.str(d + 0xC, base.wrapping_add((pos as u32 >> 8).wrapping_add(0x10)));
        }
        v += 1;
        p += 4;
    }
}

/// `Loop_Sweep()` (0x08030D6C) : déplacement du début de boucle (bit 0x800 de
/// `voix + 0x38`) par pas `[+0x2C]`, toutes les `[+0x32]` images, `[+0x30]` fois ; le
/// mouvement s'inverse (bit 0x4000) ou s'arrête ; ignoré s'il sortirait de l'échantillon.
pub fn loop_sweep(env: &mut Env) {
    let mut v = 0;
    while v < voices(env) {
        let this = v;
        v += 1;
        let g = env.ldr(SOUND);
        let c = voice(env, this);
        if c == 0 || env.ldr(c + 0x38) & 0x800 == 0 {
            continue;
        }
        let pt = env.ldr(g + 0xCC);
        let d = env.ldr(pt.wrapping_add(4 * this));
        if d == 0 {
            continue;
        }
        let s = c + 0x2C;
        let t = env.ldrb(s + 7);
        if t != 0 {
            env.strb(s + 7, t - 1);
            continue;
        }
        let r = env.ldrb(s + 6);
        env.strb(s + 7, r);
        let mut n = env.ldrh(s + 4);
        if n == 0 {
            let f = env.ldr(c + 0x38);
            if f & 0x4000 != 0 {
                let a = env.ldrh(s + 2);
                env.strh(s + 4, a);
                let st = env.ldrh(c + 0x2C);
                env.strh(c + 0x2C, st.wrapping_neg());
            } else {
                env.str(c + 0x38, f & 0xFFFF_B7FF);
            }
            n = env.ldrh(s + 4);
            if n == 0 {
                continue;
            }
        }
        let delta = env.ldsh(s);
        let cur = env.ldr(d + 0xC);
        let mut np = cur.wrapping_add(delta as u32);
        let smp = env.ldr(d);
        let lo = smp.wrapping_add(0x10);
        let end = env.ldr(d + 8);
        let out = if cur <= end {
            np < lo || np.wrapping_add(env.ldr(d + 0x10)) > lo.wrapping_add(env.ldr(smp))
        } else {
            np.wrapping_sub(env.ldr(d + 0x10)) < lo || np > lo.wrapping_add(env.ldr(smp))
        };
        if out {
            np = cur;
            let a = env.ldrh(s + 2);
            env.strh(s + 2, a.wrapping_sub(n));
            env.strh(s + 4, 0);
        } else {
            env.strh(s + 4, n - 1);
        }
        env.str(d + 0xC, np);
    }
}

/// `Envelopes()` (0x08030FD0) : enveloppes de volume, 4 canaux PSG (`pilote + 0x94 +
/// 8k` : paramètres, `+0x98` niveau) puis voix DirectSound (`+0x24`, `+0x28`). Bit 1 :
/// ADSR (phase dans les bits 1-2 : attaque, déclin, maintien, relâchement ; bit 0x10 =
/// note relâchée), bit 0x20 : fondu linéaire de pas `[+0]`. Le canal est marqué
/// « volume à recalculer » (0x80) quand le niveau change assez.
pub fn envelopes(env: &mut Env) {
    for k in 0..4u32 {
        let g = env.ldr(SOUND);
        let fa = g + 0x18 + 4 * k;
        adsr(env, fa, g + 0x94 + 8 * k, g + 0x98 + 8 * k, 0xF000, &|env: &mut Env| env.ldr(SOUND) + 0x18 + 4 * k);
    }
    let mut v = 0;
    while v < voices(env) {
        let c = voice(env, v);
        if c != 0 {
            adsr(env, c + 0x38, c + 0x24, c + 0x28, 0xFF00, &move |_: &mut Env| c + 0x38);
        }
        v += 1;
    }
}

/// Une enveloppe : drapeaux `fa`, paramètres (ou pas du fondu) `pa`, niveau `lv`.
/// `sig` : bits du niveau
/// dont le changement doit être signalé ; `flags` relit l'adresse des drapeaux.
fn adsr(env: &mut Env, fa: u32, pa: u32, lv: u32, sig: u32, flags: &dyn Fn(&mut Env) -> u32) {
    let f = env.ldr(fa);
    let a = env.ldrh(lv);
    if f & 1 != 0 {
        match (f & 6) >> 1 {
            0 => {
                let e = env.ldr(pa);
                // comparaisons signées sur 32 bits, comme le code d'origine
                let mut r = env.ldrh(e).wrapping_add(a) as i32;
                if r > 0xFEFF {
                    env.str(fa, (f & !6) | 2);
                    r = 0xFF00;
                }
                env.strh(lv, r as u32);
            }
            1 => {
                let e = env.ldr(pa);
                let mut r = a.wrapping_sub(env.ldrh(e + 2)) as i32;
                let sus = env.ldrh(e + 4) as i32;
                if r <= sus {
                    r = sus;
                    env.str(fa, (f & !6) | 4);
                }
                env.strh(lv, r as u32);
            }
            p => {
                if p == 2 && f & 0x10 == 0 {
                    return signal(env, a, lv, sig, flags);
                }
                if p == 2 {
                    env.str(fa, (f & !6) | 6);
                }
                let e = env.ldr(pa);
                let mut r = env.ldrh(lv).wrapping_sub(env.ldrh(e + 6)) as i32;
                if r <= 0xFF {
                    r = 0;
                    let x = flags(env);
                    let fv = env.ldr(x);
                    env.str(x, fv & !7);
                }
                env.strh(lv, r as u32);
            }
        }
        signal(env, a, lv, sig, flags)
    } else if f & 0x20 != 0 {
        let d = env.ldr(pa) as i32;
        let mut r = (a as i32).wrapping_add(d);
        if d >= 0 {
            if r > 0xFEFF {
                env.str(fa, f & !0x20);
                r = 0xFF00;
            }
        } else if r <= 0x100 {
            env.str(fa, f & !0x20);
            r = 0;
        }
        env.strh(lv, r as u32);
        if a & sig != r as u32 & sig {
            let x = flags(env);
            let fv = env.ldr(x);
            env.str(x, fv | 0x80);
        }
    }
}

fn signal(env: &mut Env, a: u32, lv: u32, sig: u32, flags: &dyn Fn(&mut Env) -> u32) {
    if a & sig != env.ldrh(lv) & sig {
        let x = flags(env);
        let fv = env.ldr(x);
        env.str(x, fv | 0x80);
    }
}

/// `Pan_Sweep()` (0x08031278) : panoramique oscillant des voix (`+0xA` position,
/// `+0x12` pas, bornes `[+0x1A]`/`[+0x1B]` ×256 ; bit 0x2000 : aller-retour, sinon
/// arrêt à la borne) ; `+0x3C` reçoit la position entière, voix marquée 0x80.
pub fn pan_sweep(env: &mut Env) {
    let mut v = 0;
    while v < voices(env) {
        let c = voice(env, v);
        v += 1;
        if c == 0 {
            continue;
        }
        let su = env.ldrh(c + 0x12);
        let step = env.ldsh(c + 0x12);
        if step == 0 {
            continue;
        }
        let mut p = env.ldsh(c + 0xA) + step;
        let bounce = if step > 0 {
            let lim = (env.ldrb(c + 0x1B) << 8) as i32;
            let b = p >= lim;
            if b {
                p = lim - (p - lim);
            }
            b
        } else {
            let lim = (env.ldrb(c + 0x1A) << 8) as i32;
            let b = p <= lim;
            if b {
                p = lim + (lim - p);
            }
            b
        };
        if bounce {
            let back = if env.ldr(c + 0x38) & 0x2000 != 0 { su.wrapping_neg() } else { 0 };
            env.strh(c + 0x12, back);
        }
        env.strh(c + 0xA, p as u32);
        let hi = p >> 8;
        if hi != env.ldrb(c + 0x3C) as i32 {
            env.strb(c + 0x3C, hi as u32);
            voice_flag(env, c, 0x80);
        }
    }
}

/// Générateur pseudo-aléatoire du pilote (`Random(n)`, 0x0803033C).
const RANDOM: u32 = 0x0803_033D;

/// Drapeaux du canal `n` des programmes d'instruments : PSG 0-3, puis voix (`n - 4`).
fn chan_flags(env: &mut Env, n: u32) -> u32 {
    if n as i32 <= 3 {
        let g = env.ldr(SOUND);
        env.ldr(g + 0x18 + 4 * n)
    } else {
        let t = deref(env, SOUND, 0xD0);
        env.ldr((n << 6).wrapping_add(t).wrapping_add(0xFFFF_FF00) + 0x38)
    }
}

/// Le canal `n > 3` n'a plus de descripteur de lecture (`[pilote + 0xCC][n - 4] == 0`).
fn no_player(env: &mut Env, n: u32) -> bool {
    let t = deref(env, SOUND, 0xCC);
    env.ldr((4 * n).wrapping_add(t).wrapping_sub(0x10)) == 0
}

/// Opcode 4 des programmes d'instruments, `Instr_Wait(n, slot)` (0x08032484) : attend
/// `[code+2]` images (au hasard jusqu'à cette valeur si `[code+1] & 2`,
/// 0xFFFF = indéfiniment) dans le compteur `slot + 0xA`. L'attente cesse plus tôt si la
/// note est relâchée (`& 1` : drapeau 0x100 du canal) ou si l'échantillon de la voix
/// est terminé (`& 4`). Renvoie 1 quand on passe à l'instruction suivante (`+4`).
pub fn instr_wait(env: &mut Env) {
    let (n, slot) = (env.cpu.r[0], env.cpu.r[1]);
    let code = env.ldr(slot);
    let cnt = env.ldrh(slot + 0xA);
    let next = |env: &mut Env| {
        let c = env.ldr(slot);
        env.str(slot, c.wrapping_add(4));
        env.cpu.r[0] = 1;
    };
    if cnt == 0 {
        let w = env.ldrh(code + 2);
        if w == 0 {
            return next(env);
        }
        let fl = env.ldrb(code + 1);
        if fl & 1 != 0 && chan_flags(env, n) & 0x100 != 0 {
            return next(env);
        }
        if fl & 4 != 0 && n as i32 > 3 && chan_flags(env, n) & 0x200 == 0 && no_player(env, n) {
            return next(env);
        }
        let v = if fl & 2 != 0 {
            let lim = env.ldrh(code + 2);
            env.call(RANDOM, &[lim])
        } else {
            w
        };
        env.strh(slot + 0xA, v);
        env.cpu.r[0] = 0;
        return;
    }
    if env.ldrh(code + 2) != 0xFFFF {
        let c = cnt.wrapping_sub(1);
        env.strh(slot + 0xA, c);
        if c & 0xFFFF == 0 {
            env.str(slot, code.wrapping_add(4));
            env.cpu.r[0] = 1;
            return;
        }
    }
    let fl = env.ldrb(code + 1);
    let stop = (fl & 1 != 0 && chan_flags(env, n) & 0x100 != 0) || (fl & 4 != 0 && n as i32 > 3 && no_player(env, n));
    if stop {
        env.strh(slot + 0xA, 0);
        return next(env);
    }
    env.cpu.r[0] = 0;
}
