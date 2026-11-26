//! Carte mémoire de la GBA, registres d'entrée/sortie, DMA, minuteurs,
//! interruptions, manette et EEPROM de sauvegarde.

use super::apu::Apu;
use super::eeprom::Eeprom;

/// Mini-BIOS : vecteur d'interruption identique à celui du BIOS d'origine
/// (sauvegarde r0-r3/r12/lr, appelle le gestionnaire du jeu lu en 0x03007FFC).
/// Mini-BIOS : vecteur d'interruption (0x18), appelle le gestionnaire du jeu (`[0x03007FFC]`).
pub const BIOS_IRQ: [u32; 6] = [
    0xE92D_500F, // stmfd sp!, {r0-r3, r12, lr}
    0xE3A0_0301, // mov r0, #0x04000000
    0xE28F_E000, // add lr, pc, #0
    0xE510_F004, // ldr pc, [r0, #-4]
    0xE8BD_500F, // ldmfd sp!, {r0-r3, r12, lr}
    0xE25E_F004, // subs pc, lr, #4
];

#[derive(Default, Clone, Copy)]
pub struct Dma {
    src: u32,
    dst: u32,
    count: u32,
}

#[derive(Default, Clone, Copy)]
pub struct Timer {
    counter: u32,
    reload: u32,
    acc: u32,
}

#[derive(Clone)]
pub struct Bus {
    rom: std::rc::Rc<[u8]>,
    bios: Vec<u8>,
    ewram: Vec<u8>,
    iwram: Vec<u8>,
    pub io: Vec<u8>,
    pub pal: Vec<u8>,
    pub vram: Vec<u8>,
    pub oam: Vec<u8>,
    /// Compteur global de cycles (16,78 MHz).
    pub cyc: u64,
    dma: [Dma; 4],
    tm: [Timer; 4],
    pub apu: Apu,
    pub eeprom: Eeprom,
    /// Boutons enfoncés (1 = enfoncé), bits de KEYINPUT.
    pub keys: u16,
    pub halt_req: bool,
    /// Profil : instructions exécutées par adresse (ROM 0x08000000-0x0803FFFF puis IWRAM).
    pub profile: Option<Vec<u32>>,
    /// Points de référence internes des fonds affines BG2 / BG3 (x, y).
    pub bg_ref: [[i32; 2]; 2],
    /// Lignes restantes avant qu'un fond activé dans DISPCNT n'apparaisse (le matériel
    /// attend 3 lignes ; l'eau des niveaux aquatiques compte dessus).
    pub bg_delay: [u8; 4],
    sample_acc: u32,
    /// Instruction en cours (bit 0 = Thumb) : sert au « bus ouvert ».
    pub pc_now: u32,
    /// Valeur de « bus ouvert » fournie par le code recompilé pour l'instruction en cours
    /// (l'instruction préchargée d'origine) ; `None` quand l'instruction est interprétée :
    /// elle est alors relue en mémoire. Évite de dépendre du code de la ROM.
    pub ob: Option<u32>,
    /// Contrôle `LADYSIA_AUDIT` : octets de la ROM effacés dans les données sans code, et
    /// lectures qui les ont touchés (adresse lue -> adresse de l'instruction en cours).
    pub audit: Option<Box<(Vec<bool>, std::collections::BTreeMap<u32, u32>)>>,
}

fn rd16(m: &[u8], a: usize) -> u32 {
    u16::from_le_bytes([m[a], m[a + 1]]) as u32
}
fn rd32(m: &[u8], a: usize) -> u32 {
    u32::from_le_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]])
}
fn wr16(m: &mut [u8], a: usize, v: u32) {
    m[a..a + 2].copy_from_slice(&(v as u16).to_le_bytes());
}
fn wr32(m: &mut [u8], a: usize, v: u32) {
    m[a..a + 4].copy_from_slice(&v.to_le_bytes());
}
fn vaddr(a: u32) -> usize {
    let a = (a & 0x1_FFFF) as usize;
    if a >= 0x1_8000 { a - 0x8000 } else { a }
}

/// Coût d'un accès selon la zone (16 bits, 32 bits), en cycles.
fn cost(a: u32, wide: bool) -> u64 {
    match a >> 24 {
        2 => {
            if wide {
                6
            } else {
                3
            }
        }
        5 | 6 => 1 + wide as u64,
        8..=0xD => {
            if wide {
                4
            } else {
                2
            }
        }
        0xE | 0xF => 5,
        _ => 1,
    }
}

impl Bus {
    pub fn new(rom: Vec<u8>, eeprom: Eeprom) -> Self {
        let mut bios = vec![0u8; 0x4000];
        for (k, w) in BIOS_IRQ.iter().enumerate() {
            wr32(&mut bios, 0x18 + 4 * k, *w);
        }
        let mut b = Bus {
            rom: rom.into(),
            bios,
            ewram: vec![0; 0x4_0000],
            iwram: vec![0; 0x8000],
            io: vec![0; 0x400],
            pal: vec![0; 0x400],
            vram: vec![0; 0x1_8000],
            oam: vec![0; 0x400],
            cyc: 0,
            dma: [Dma::default(); 4],
            tm: [Timer::default(); 4],
            apu: Apu::new(),
            eeprom,
            keys: 0,
            halt_req: false,
            profile: None,
            bg_ref: [[0; 2]; 2],
            bg_delay: [0; 4],
            sample_acc: 0,
            pc_now: 0x0800_0001,
            ob: None,
            audit: None,
        };
        wr16(&mut b.io, 0x88, 0x200); // SOUNDBIAS
        wr16(&mut b.io, 0x20, 0x100); // BG2PA
        wr16(&mut b.io, 0x26, 0x100); // BG2PD
        wr16(&mut b.io, 0x30, 0x100);
        wr16(&mut b.io, 0x36, 0x100);
        b
    }

    /// Compte une instruction à l'adresse `pc` dans le profil (s'il est actif).
    #[inline]
    pub fn count(&mut self, pc: u32) {
        if let Some(p) = &mut self.profile {
            let i = if pc.wrapping_sub(0x0800_0000) < 0x4_0000 {
                pc - 0x0800_0000
            } else if pc.wrapping_sub(0x0300_0000) < 0x8000 {
                0x4_0000 + pc - 0x0300_0000
            } else {
                return;
            };
            p[(i >> 1) as usize] += 1;
        }
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Note une lecture de `n` octets de la ROM à partir du décalage `o` (contrôle).
    #[inline]
    pub fn audited(&mut self, o: usize, n: usize) {
        if let Some(a) = &mut self.audit {
            for k in o..(o + n).min(a.0.len()) {
                // les recopies de code en IWRAM sont attendues (vides dans les données)
                // (répartiteur d'interruptions avec la fin de sa copie DMA, décompresseur, mixeur)
                let copied = [(0x0800_012Cu32, 0x0800_024C), (0x0800_0290, 0x0800_02F0), (0x0803_4DB4, 0x0803_5804)]
                    .iter()
                    .any(|&(s, e)| (s..e).contains(&(0x0800_0000 + k as u32)));
                if a.0[k] && !copied {
                    a.1.entry(0x0800_0000 + k as u32).or_insert(self.pc_now);
                }
            }
        }
    }

    /// Sauvegarde instantanée faite avec la ROM complète : le code d'origine recopié en
    /// IWRAM (reconnu par son empreinte) est remplacé par sa version des données sans code.
    pub fn strip_copies(&mut self, copies: &[(u32, u32, u32, u64)]) {
        for &(d, s, n, h) in copies {
            let (i, r, n) = ((d - 0x0300_0000) as usize, (s - 0x0800_0000) as usize, n as usize);
            if r + n <= self.rom.len() && crate::recomp_gen::fnv(&self.iwram[i..i + n]) == h {
                let (rom, iwram) = (&self.rom, &mut self.iwram);
                iwram[i..i + n].copy_from_slice(&rom[r..r + n]);
            }
        }
    }

    /// La mémoire IWRAM à partir de `a` contient-elle exactement la copie de la ROM
    /// `src..src+len` ? (code généré par le jeu, vérifié sans compter de cycles)
    pub fn iwram_is_copy(&self, a: u32, src: u32, len: u32) -> bool {
        let i = (a - 0x0300_0000) as usize;
        let r = (src - 0x0800_0000) as usize;
        let n = len as usize;
        i + n <= self.iwram.len() && r + n <= self.rom.len() && self.iwram[i..i + n] == self.rom[r..r + n]
    }

    /// Image complète de l'état (sauvegarde instantanée).
    pub fn state(&self, w: &mut Vec<u8>) {
        for m in [&self.ewram, &self.iwram, &self.io, &self.pal, &self.vram, &self.oam] {
            w.extend_from_slice(m);
        }
        w.extend_from_slice(&self.cyc.to_le_bytes());
        for d in &self.dma {
            for v in [d.src, d.dst, d.count] {
                w.extend_from_slice(&v.to_le_bytes());
            }
        }
        for t in &self.tm {
            for v in [t.counter, t.reload, t.acc] {
                w.extend_from_slice(&v.to_le_bytes());
            }
        }
        for v in self.bg_ref.iter().flatten() {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w.extend_from_slice(&self.sample_acc.to_le_bytes());
        self.eeprom.state(w);
        self.apu.state(w);
    }

    pub fn load(&mut self, r: &mut super::State) -> Option<()> {
        for (m, n) in [(&mut self.ewram, 0x4_0000), (&mut self.iwram, 0x8000), (&mut self.io, 0x400), (&mut self.pal, 0x400), (&mut self.vram, 0x1_8000), (&mut self.oam, 0x400)] {
            m.copy_from_slice(r.bytes(n)?);
        }
        self.cyc = r.u64()?;
        for d in self.dma.iter_mut() {
            d.src = r.u32()?;
            d.dst = r.u32()?;
            d.count = r.u32()?;
        }
        for t in self.tm.iter_mut() {
            t.counter = r.u32()?;
            t.reload = r.u32()?;
            t.acc = r.u32()?;
        }
        for v in self.bg_ref.iter_mut().flatten() {
            *v = r.u32()? as i32;
        }
        self.sample_acc = r.u32()?;
        self.eeprom.load(r)?;
        self.apu.load(r)?;
        // registres du son « Game Boy » (sans relancer les notes)
        self.apu.psg = crate::psg::Psg::default();
        for o in 0x60..0xA0usize {
            if ![0x65, 0x6D, 0x75, 0x7D].contains(&o) {
                self.apu.psg.write(o - 0x60, self.io[o]);
            }
        }
        Some(())
    }

    /// Différences de mémoire avec une autre machine (plages), en ignorant la pile
    /// sous `sp` (zone morte après le retour d'une fonction).
    pub fn diff(&self, o: &Bus, sp: u32) -> Vec<(u32, u32, String)> {
        let mut out = Vec::new();
        let regions: [(&str, u32, &[u8], &[u8]); 6] = [
            ("EWRAM", 0x0200_0000, &self.ewram, &o.ewram),
            ("IWRAM", 0x0300_0000, &self.iwram, &o.iwram),
            ("IO", 0x0400_0000, &self.io, &o.io),
            ("PAL", 0x0500_0000, &self.pal, &o.pal),
            ("VRAM", 0x0600_0000, &self.vram, &o.vram),
            ("OAM", 0x0700_0000, &self.oam, &o.oam),
        ];
        for (name, base, a, b) in regions {
            let mut i = 0;
            while i < a.len() {
                if a[i] != b[i] {
                    let addr = base + i as u32;
                    // pile morte, et registres DMA en écriture seule (illisibles sur la console)
                    let dma_wo = name == "IO" && (0xB0..0xE0).contains(&i) && (i - 0xB0) % 12 < 10;
                    if (sp.saturating_sub(0x800)..sp).contains(&addr) || dma_wo {
                        i += 1;
                        continue;
                    }
                    while i < a.len() && (a[i] != b[i] || (i + 8 < a.len() && a[i + 1..i + 8] != b[i + 1..i + 8])) {
                        i += 1;
                    }
                    out.push((addr, base + i as u32, format!("{name} {:08X}-{:08X}", addr, base + i as u32)));
                }
                i += 1;
            }
        }
        out
    }

    pub fn io16(&self, o: usize) -> u32 {
        rd16(&self.io, o)
    }
    pub fn io32(&self, o: usize) -> u32 {
        rd32(&self.io, o)
    }
    pub fn set_io16(&mut self, o: usize, v: u32) {
        wr16(&mut self.io, o, v)
    }

    pub fn irq(&mut self, bit: u32) {
        let v = self.io16(0x202) | (1 << bit);
        self.set_io16(0x202, v);
    }

    pub fn irq_pending(&self) -> bool {
        self.io16(0x200) & self.io16(0x202) & 0x3FFF != 0
    }

    pub fn ime(&self) -> bool {
        self.io[0x208] & 1 != 0
    }

    fn rom_at(&self, a: u32) -> Option<usize> {
        let o = (a & 0x01FF_FFFF) as usize;
        (o < self.rom.len()).then_some(o)
    }

    // -------------------------------------------------------------- lecture --
    /// Zones 0x00000000-0x01FFFFFF : BIOS (illisible hors du BIOS : renvoie sa dernière
    /// instruction, `subs pc, lr, #4` de retour d'interruption) puis bus ouvert.
    fn unmapped(&mut self, a: u32) -> u32 {
        let a = a & !3;
        if a < 0x4000 {
            if self.pc_now & !1 < 0x4000 {
                rd32(&self.bios, a as usize)
            } else {
                0xE25E_F004
            }
        } else {
            self.open_bus()
        }
    }

    /// Bus ouvert (adresse sans mémoire) : la console relit l'instruction préchargée,
    /// `[pc + 4]` en Thumb (sur les deux moitiés du mot), `[pc + 8]` en ARM. Certains
    /// jeux lisent ces zones par erreur ; avec 0, Lady Sia bouclait des millions de fois
    /// (chargement sans fin à la fin du Royaume de l'air, objet sous le bas de la carte).
    fn open_bus(&mut self) -> u32 {
        if let Some(v) = self.ob {
            return v;
        }
        let cyc = self.cyc;
        let pc = self.pc_now & !1;
        let v = if self.pc_now & 1 != 0 {
            let h = if (pc.wrapping_add(4) >> 24) < 2 { 0 } else { self.read16(pc.wrapping_add(4)) };
            h | (h << 16)
        } else if (pc.wrapping_add(8) >> 24) < 2 {
            0
        } else {
            self.read32(pc.wrapping_add(8))
        };
        self.cyc = cyc;
        v
    }

    #[inline(never)]
    pub fn read8(&mut self, a: u32) -> u32 {
        self.cyc += cost(a, false);
        match a >> 24 {
            0 | 1 => (self.unmapped(a) >> ((a & 3) * 8)) & 0xFF,
            2 => self.ewram[(a & 0x3_FFFF) as usize] as u32,
            3 => self.iwram[(a & 0x7FFF) as usize] as u32,
            4 => {
                let o = a & 0x00FF_FFFF;
                if o < 0x400 {
                    (self.io_read16(o & !1) >> ((o & 1) * 8)) & 0xFF
                } else {
                    (self.open_bus() >> ((a & 3) * 8)) & 0xFF
                }
            }
            5 => self.pal[(a & 0x3FF) as usize] as u32,
            6 => self.vram[vaddr(a)] as u32,
            7 => self.oam[(a & 0x3FF) as usize] as u32,
            8..=0xC => match self.rom_at(a) {
                Some(o) => {
                    self.audited(o, 1);
                    self.rom[o] as u32
                }
                None => ((a >> 1) >> ((a & 1) * 8)) & 0xFF,
            },
            0xD => self.eeprom.read(),
            0xE | 0xF => 0xFF,
            _ => (self.open_bus() >> ((a & 3) * 8)) & 0xFF,
        }
    }

    #[inline(never)]
    pub fn read16(&mut self, a: u32) -> u32 {
        let a = a & !1;
        self.cyc += cost(a, false);
        match a >> 24 {
            0 | 1 => (self.unmapped(a) >> ((a & 2) * 8)) & 0xFFFF,
            2 => rd16(&self.ewram, (a & 0x3_FFFF) as usize),
            3 => rd16(&self.iwram, (a & 0x7FFF) as usize),
            4 => {
                let o = a & 0x00FF_FFFF;
                if o < 0x400 { self.io_read16(o) } else { (self.open_bus() >> ((a & 2) * 8)) & 0xFFFF }
            }
            5 => rd16(&self.pal, (a & 0x3FF) as usize),
            6 => rd16(&self.vram, vaddr(a)),
            7 => rd16(&self.oam, (a & 0x3FF) as usize),
            8..=0xC => match self.rom_at(a) {
                Some(o) if o + 1 < self.rom.len() => {
                    self.audited(o, 2);
                    rd16(&self.rom, o)
                }
                _ => (a >> 1) & 0xFFFF,
            },
            0xD => self.eeprom.read(),
            0xE | 0xF => 0xFFFF,
            _ => (self.open_bus() >> ((a & 2) * 8)) & 0xFFFF,
        }
    }

    #[inline(never)]
    pub fn read32(&mut self, a: u32) -> u32 {
        let a = a & !3;
        self.cyc += cost(a, true);
        match a >> 24 {
            0 | 1 => self.unmapped(a),
            2 => rd32(&self.ewram, (a & 0x3_FFFF) as usize),
            3 => rd32(&self.iwram, (a & 0x7FFF) as usize),
            4 => {
                let o = a & 0x00FF_FFFF;
                if o < 0x400 { self.io_read16(o) | (self.io_read16(o + 2) << 16) } else { self.open_bus() }
            }
            5 => rd32(&self.pal, (a & 0x3FF) as usize),
            6 => rd32(&self.vram, vaddr(a)),
            7 => rd32(&self.oam, (a & 0x3FF) as usize),
            8..=0xC => match self.rom_at(a) {
                Some(o) if o + 3 < self.rom.len() => {
                    self.audited(o, 4);
                    rd32(&self.rom, o)
                }
                _ => ((a >> 1) & 0xFFFF) | ((((a + 2) >> 1) & 0xFFFF) << 16),
            },
            0xD => self.eeprom.read(),
            0xE | 0xF => 0xFFFF_FFFF,
            _ => self.open_bus(),
        }
    }

    // ------------------------------------------------------------ écriture --
    #[inline(never)]
    pub fn write8(&mut self, a: u32, v: u8) {
        self.cyc += cost(a, false);
        match a >> 24 {
            2 => self.ewram[(a & 0x3_FFFF) as usize] = v,
            3 => self.iwram[(a & 0x7FFF) as usize] = v,
            4 => {
                let o = a & 0x00FF_FFFF;
                match o {
                    0x301 => self.halt_req = true,
                    0x202 | 0x203 => {
                        let cur = self.io16(0x202);
                        self.set_io16(0x202, cur & !((v as u32) << ((o & 1) * 8)));
                    }
                    0xA0..=0xA7 => self.apu.push(((o - 0xA0) / 4) as usize, v as i8),
                    0..=0x3FF => {
                        let al = o & !1;
                        let cur = rd16(&self.io, al as usize);
                        let sh = (o & 1) * 8;
                        let nv = (cur & !(0xFF << sh)) | ((v as u32) << sh);
                        self.io_write16(al, nv, Some(o & 1));
                    }
                    _ => {}
                }
            }
            5 => {
                let o = (a & 0x3FE) as usize;
                self.pal[o] = v;
                self.pal[o + 1] = v;
            }
            6 => {
                let o = vaddr(a) & !1;
                if o < 0x1_0000 {
                    self.vram[o] = v;
                    self.vram[o + 1] = v;
                }
            }
            0xD => self.eeprom.write(v as u32),
            _ => {}
        }
    }

    #[inline(never)]
    pub fn write16(&mut self, a: u32, v: u16) {
        let a = a & !1;
        let v = v as u32;
        self.cyc += cost(a, false);
        match a >> 24 {
            2 => wr16(&mut self.ewram, (a & 0x3_FFFF) as usize, v),
            3 => wr16(&mut self.iwram, (a & 0x7FFF) as usize, v),
            4 => {
                let o = a & 0x00FF_FFFF;
                if o < 0x400 {
                    self.io_write16(o, v, None);
                }
            }
            5 => wr16(&mut self.pal, (a & 0x3FF) as usize, v),
            6 => wr16(&mut self.vram, vaddr(a), v),
            7 => wr16(&mut self.oam, (a & 0x3FF) as usize, v),
            0xD => self.eeprom.write(v),
            _ => {}
        }
    }

    #[inline(never)]
    pub fn write32(&mut self, a: u32, v: u32) {
        let a = a & !3;
        self.cyc += cost(a, true);
        match a >> 24 {
            2 => wr32(&mut self.ewram, (a & 0x3_FFFF) as usize, v),
            3 => wr32(&mut self.iwram, (a & 0x7FFF) as usize, v),
            4 => {
                let o = a & 0x00FF_FFFF;
                if o < 0x400 {
                    self.io_write16(o, v & 0xFFFF, None);
                    self.io_write16(o + 2, v >> 16, None);
                }
            }
            5 => wr32(&mut self.pal, (a & 0x3FF) as usize, v),
            6 => wr32(&mut self.vram, vaddr(a), v),
            7 => wr32(&mut self.oam, (a & 0x3FF) as usize, v),
            0xD => self.eeprom.write(v),
            _ => {}
        }
    }

    // ------------------------------------------------------- registres IO --
    fn io_read16(&mut self, o: u32) -> u32 {
        let o = o as usize;
        match o {
            0x100 | 0x104 | 0x108 | 0x10C => self.tm[(o - 0x100) / 4].counter & 0xFFFF,
            0x130 => !self.keys as u32 & 0x3FF,
            0xA0..=0xA7 => 0,
            _ => rd16(&self.io, o),
        }
    }

    /// `byte` : écriture d'un seul octet (Some(0) bas, Some(1) haut).
    fn io_write16(&mut self, o: u32, v: u32, byte: Option<u32>) {
        let o = o as usize;
        let old = rd16(&self.io, o);
        if o == 0 {
            for i in 0..4 {
                let bit = 0x100 << i;
                if v & bit == 0 {
                    self.bg_delay[i] = 0;
                } else if old & bit == 0 {
                    self.bg_delay[i] = 3;
                }
            }
        }
        match o {
            0x004 => wr16(&mut self.io, o, (old & 7) | (v & !7)),
            0x006 | 0x130 => {}
            0x028..=0x02E | 0x038..=0x03E => {
                wr16(&mut self.io, o, v);
                let k = if o < 0x38 { 0 } else { 1 };
                let base = 0x28 + 0x10 * k;
                let x = ((rd32(&self.io, base) << 4) as i32) >> 4;
                let y = ((rd32(&self.io, base + 4) << 4) as i32) >> 4;
                if o < base + 4 {
                    self.bg_ref[k][0] = x;
                } else {
                    self.bg_ref[k][1] = y;
                }
            }
            0x060..=0x09F => {
                let mut v = v;
                if o == 0x84 {
                    v = (v & 0x80) | (old & 0x0F);
                }
                wr16(&mut self.io, o, v);
                let lo = byte.is_none_or(|b| b == 0);
                let hi = byte.is_none_or(|b| b == 1);
                if lo {
                    self.apu.psg.write(o - 0x60, v as u8);
                }
                if hi {
                    self.apu.psg.write(o + 1 - 0x60, (v >> 8) as u8);
                }
                if o == 0x82 {
                    if v & 0x800 != 0 {
                        self.apu.reset(0);
                    }
                    if v & 0x8000 != 0 {
                        self.apu.reset(1);
                    }
                    wr16(&mut self.io, o, v & !0x8800);
                }
            }
            0x0A0..=0x0A7 => {
                let ch = (o - 0xA0) / 4;
                match byte {
                    Some(b) => self.apu.push(ch, (v >> (8 * b)) as i8),
                    None => {
                        self.apu.push(ch, v as i8);
                        self.apu.push(ch, (v >> 8) as i8);
                    }
                }
            }
            0x0BA | 0x0C6 | 0x0D2 | 0x0DE => {
                let n = (o - 0xBA) / 12;
                wr16(&mut self.io, o, v);
                if v & 0x8000 != 0 && old & 0x8000 == 0 {
                    let base = 0xB0 + 12 * n;
                    let cnt = rd16(&self.io, base + 8);
                    let max = if n == 3 { 0x1_0000 } else { 0x4000 };
                    self.dma[n] = Dma {
                        src: rd32(&self.io, base) & if n == 0 { 0x07FF_FFFF } else { 0x0FFF_FFFF },
                        dst: rd32(&self.io, base + 4) & if n == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF },
                        count: if cnt == 0 { max } else { cnt & (max - 1) },
                    };
                    if (v >> 12) & 3 == 0 {
                        self.run_dma(n);
                    }
                }
            }
            0x100 | 0x104 | 0x108 | 0x10C => {
                wr16(&mut self.io, o, v);
                self.tm[(o - 0x100) / 4].reload = v;
            }
            0x102 | 0x106 | 0x10A | 0x10E => {
                let n = (o - 0x102) / 4;
                wr16(&mut self.io, o, v);
                if v & 0x80 != 0 && old & 0x80 == 0 {
                    self.tm[n].counter = self.tm[n].reload;
                    self.tm[n].acc = 0;
                }
            }
            0x202 => wr16(&mut self.io, o, old & !v),
            0x300 => {
                wr16(&mut self.io, o, v & 0xFF);
                if byte != Some(0) {
                    self.halt_req = true;
                }
            }
            _ => wr16(&mut self.io, o, v),
        }
    }

    // ----------------------------------------------------------------- DMA --
    fn run_dma(&mut self, n: usize) {
        let ctl_off = 0xBA + 12 * n;
        let ctl = rd16(&self.io, ctl_off);
        let timing = (ctl >> 12) & 3;
        let sound = timing == 3 && (n == 1 || n == 2);
        let word = sound || ctl & 0x400 != 0;
        let count = if sound { 4 } else { self.dma[n].count };
        let step: i32 = if word { 4 } else { 2 };
        let dctl = (ctl >> 5) & 3;
        let sctl = (ctl >> 7) & 3;
        let dinc = if sound {
            0
        } else {
            match dctl {
                1 => -step,
                2 => 0,
                _ => step,
            }
        };
        let sinc = match sctl {
            1 => -step,
            2 => 0,
            _ => step,
        };
        let (mut s, mut d) = (self.dma[n].src, self.dma[n].dst);
        let to_eeprom = d >> 24 == 0xD;
        if to_eeprom {
            self.eeprom.begin(count);
        }
        for _ in 0..count {
            if word {
                let v = self.read32(s);
                self.write32(d, v);
            } else {
                let v = self.read16(s);
                self.write16(d, v as u16);
            }
            s = s.wrapping_add(sinc as u32);
            d = d.wrapping_add(dinc as u32);
        }
        self.dma[n].src = s;
        self.dma[n].dst = d;
        if to_eeprom {
            self.eeprom.finish();
        }
        if ctl & 0x4000 != 0 {
            self.irq(8 + n as u32);
        }
        if ctl & 0x200 != 0 && timing != 0 {
            let base = 0xB0 + 12 * n;
            let cnt = rd16(&self.io, base + 8);
            let max = if n == 3 { 0x1_0000 } else { 0x4000 };
            self.dma[n].count = if cnt == 0 { max } else { cnt & (max - 1) };
            if dctl == 3 {
                self.dma[n].dst = rd32(&self.io, base + 4) & if n == 3 { 0x0FFF_FFFF } else { 0x07FF_FFFF };
            }
        } else {
            wr16(&mut self.io, ctl_off, ctl & 0x7FFF);
        }
    }

    /// Déclenche les DMA programmés sur un événement (1 = VBlank, 2 = HBlank).
    pub fn dma_event(&mut self, timing: u32) {
        for n in 0..4 {
            let ctl = rd16(&self.io, 0xBA + 12 * n);
            if ctl & 0x8000 != 0 && (ctl >> 12) & 3 == timing {
                self.run_dma(n);
            }
        }
    }

    fn fifo_request(&mut self, ch: usize) {
        let target = 0x0400_00A0 + 4 * ch as u32;
        for n in 1..=2 {
            let ctl = rd16(&self.io, 0xBA + 12 * n);
            if ctl & 0x8000 != 0 && (ctl >> 12) & 3 == 3 && self.dma[n].dst & !3 == target {
                self.run_dma(n);
            }
        }
    }

    // ----------------------------------------------------------- minuteurs --
    /// Fait avancer minuteurs et son de `cycles`.
    pub fn tick(&mut self, cycles: u32) {
        let mut prev = 0u32;
        for n in 0..4 {
            let ctl = rd16(&self.io, 0x102 + 4 * n);
            if ctl & 0x80 == 0 {
                prev = 0;
                continue;
            }
            let incs = if n > 0 && ctl & 4 != 0 {
                prev
            } else {
                let shift = [0, 6, 8, 10][(ctl & 3) as usize];
                let t = &mut self.tm[n];
                t.acc += cycles;
                let k = t.acc >> shift;
                t.acc &= (1 << shift) - 1;
                k
            };
            let mut ov = 0;
            if incs > 0 {
                let t = &mut self.tm[n];
                let mut c = t.counter + incs;
                while c >= 0x1_0000 {
                    c = c - 0x1_0000 + t.reload;
                    ov += 1;
                }
                t.counter = c;
            }
            if ov > 0 {
                if ctl & 0x40 != 0 {
                    self.irq(3 + n as u32);
                }
                if n < 2 {
                    // instant exact du débordement (précis au cycle avec le prédiviseur 1)
                    let t = &self.tm[n];
                    let period = ((0x1_0000 - t.reload) as u64) << [0, 6, 8, 10][(ctl & 3) as usize];
                    let since = ((t.counter - t.reload) as u64) << [0, 6, 8, 10][(ctl & 3) as usize];
                    let k = ov.min(8);
                    for i in 0..k {
                        let at = self.cyc.saturating_sub(since + period * (k - 1 - i) as u64);
                        self.sound_timer(n as u32, at);
                    }
                }
            }
            prev = ov;
        }
        self.sample_acc += cycles;
        while self.sample_acc >= 512 {
            self.sample_acc -= 512;
            let (h, x) = (self.io16(0x82), self.io[0x84]);
            let at = self.cyc - self.sample_acc as u64;
            self.apu.sample(h, x, at);
        }
    }

    fn sound_timer(&mut self, n: u32, at: u64) {
        let h = self.io16(0x82);
        for ch in 0..2 {
            if (h >> (10 + 4 * ch)) & 1 == n {
                if self.apu.pop(ch as usize, at) <= 16 {
                    self.fifo_request(ch as usize);
                }
            }
        }
    }

    // ---------------------------------------------------- outils pour HLE --
    pub fn clear(&mut self, what: u32) {
        if what & 1 != 0 {
            self.ewram.fill(0);
        }
        if what & 2 != 0 {
            self.iwram[..0x7E00].fill(0);
        }
        if what & 4 != 0 {
            self.pal.fill(0);
        }
        if what & 8 != 0 {
            self.vram.fill(0);
        }
        if what & 16 != 0 {
            self.oam.fill(0);
        }
    }
}
