//! Décodeur natif du bloc compressé (0x08260000), réécrit d'après `LoadPacked` /
//! `Packed_Decode` / `Packed_BuildTables` / `Packed_ReadSymbol` / `Packed_Predict`.
//!
//! Format (proche de LZX) :
//! - en-tête : u32 taille décompressée, u32 taille compressée, puis un flux de bits
//!   lu octet de poids fort d'abord ;
//! - le flux est découpé en blocs ; chaque bloc commence par des longueurs de codes
//!   Huffman (codées en différence avec le bloc précédent, codes de répétition 16/17/18) ;
//! - mode « données » : 3 arbres (298 symboles, 48 distances, 28 longueurs) ;
//!   0-255 littéral, 256 = répéter la dernière copie, 257-260 = distance récente,
//!   261-268 = copie courte de 2 octets, 269 = nouveau bloc, 270+ = copie explicite ;
//! - mode « son » : 1 à 4 canaux, un arbre de 257 symboles chacun ; 256 = nouveau bloc,
//!   sinon le symbole est un résidu passé dans un prédicteur adaptatif (filtre à 5 poids).

use crate::rom::Rom;

/// Tables statiques lues dans la ROM (bases et bits supplémentaires).
pub struct Tables {
    len_base: [u8; 28],
    len_bits: [u8; 28],
    dist_base: [u32; 48],
    dist_bits: [u8; 48],
    short_base: [u8; 8],
    short_bits: [u8; 8],
}

impl Tables {
    pub fn from_rom(rom: &Rom) -> Tables {
        let bytes = |off: usize, out: &mut [u8]| out.copy_from_slice(&rom.data[off..off + out.len()]);
        let mut t = Tables {
            len_base: [0; 28],
            len_bits: [0; 28],
            dist_base: [0; 48],
            dist_bits: [0; 48],
            short_base: [0; 8],
            short_bits: [0; 8],
        };
        bytes(0x3545A4, &mut t.len_base);
        bytes(0x3545C0, &mut t.len_bits);
        for (i, d) in t.dist_base.iter_mut().enumerate() {
            *d = rom.u32(0x3545DC + i * 4).unwrap();
        }
        bytes(0x35469C, &mut t.dist_bits);
        bytes(0x3546CC, &mut t.short_base);
        bytes(0x3546D4, &mut t.short_bits);
        t
    }
}

/// Lecteur de bits : fenêtre de 24 bits, octet de poids fort d'abord.
struct Bits<'a> {
    src: &'a [u8],
    byte: usize,
    bit: u32,
}

impl Bits<'_> {
    fn peek16(&self) -> u32 {
        let b = |i: usize| self.src.get(self.byte + i).copied().unwrap_or(0) as u32;
        ((b(0) << 16 | b(1) << 8 | b(2)) >> (8 - self.bit)) & 0xFFFF
    }

    fn skip(&mut self, n: u32) {
        let t = self.bit + n;
        self.byte += (t >> 3) as usize;
        self.bit = t & 7;
    }

    fn read(&mut self, n: u32) -> u32 {
        let v = self.peek16() >> (16 - n);
        self.skip(n);
        v
    }
}

/// Table de décodage Huffman canonique, même disposition qu'en RAM dans le jeu.
#[derive(Clone)]
struct Huff {
    n: u32,
    /// lim[len] : premier code de longueur `len`, aligné sur 16 bits (lim[1] = 0).
    lim: [u32; 17],
    /// base[len] : index du premier symbole de longueur `len`.
    base: [u32; 16],
    /// Symboles triés. Comme dans le jeu, les cases non réécrites gardent leur ancienne valeur.
    syms: Vec<u32>,
}

impl Huff {
    fn new(capacity: usize) -> Huff {
        Huff { n: 0, lim: [0; 17], base: [0; 16], syms: vec![0; capacity] }
    }

    /// `sub_0802EA2C`
    fn build(&mut self, lengths: &[u8]) {
        let mut count = [0u32; 16];
        for &l in lengths {
            count[(l & 15) as usize] += 1;
        }
        count[0] = 0;
        self.lim[1] = 0;
        self.base[0] = 0;
        let mut offs = [0u32; 16];
        let mut code = 0u32;
        for len in 1..=15 {
            code = code.wrapping_add(count[len]) << 1;
            let l = code.wrapping_shl(15 - len as u32);
            self.lim[len + 1] = if l as i32 > 0xFFFF { 0xFFFF } else { l };
            self.base[len] = self.base[len - 1] + count[len - 1];
            offs[len] = self.base[len];
        }
        for (i, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let o = &mut offs[(l & 15) as usize];
                self.syms[*o as usize] = i as u32;
                *o += 1;
            }
        }
        self.n = lengths.len() as u32;
    }

    /// `Packed_ReadSymbol` : la longueur du code est trouvée par la même dichotomie que le jeu.
    fn read(&self, b: &mut Bits) -> u32 {
        let v = b.peek16() & 0xFFFE;
        let ge = |k: usize| v >= self.lim[k];
        let len = if ge(9) {
            if ge(13) {
                if ge(15) {
                    15
                } else if ge(14) {
                    14
                } else {
                    13
                }
            } else if ge(11) {
                if ge(12) {
                    12
                } else {
                    11
                }
            } else if ge(10) {
                10
            } else {
                9
            }
        } else if ge(5) {
            if ge(7) {
                if ge(8) {
                    8
                } else {
                    7
                }
            } else if ge(6) {
                6
            } else {
                5
            }
        } else if ge(3) {
            if ge(4) {
                4
            } else {
                3
            }
        } else if ge(2) {
            2
        } else {
            1
        };
        b.skip(len as u32);
        let mut idx = self.base[len].wrapping_add(v.wrapping_sub(self.lim[len]) >> (16 - len));
        if idx >= self.n {
            idx = 0;
        }
        self.syms[idx as usize]
    }
}

/// État du prédicteur audio d'un canal (`sub_0802EC40`, 0x5C octets en RAM).
#[derive(Default, Clone, Copy)]
struct Predictor {
    w: [i32; 5],
    /// Historique : dernier échantillon, différence, et deux valeurs plus anciennes.
    hist: [i32; 4],
    cur: i32,
    /// Erreurs cumulées : sans ajustement, puis -/+ pour chacun des 5 poids.
    acc: [i32; 11],
    count: i32,
    last: i32,
}

impl Predictor {
    /// Transforme un résidu en octet de sortie et adapte les poids tous les 32 échantillons.
    fn step(&mut self, g: &mut i32, sym: u32) -> u8 {
        self.count = self.count.wrapping_add(1);
        let h3 = self.hist[2];
        self.hist[3] = h3;
        let h2 = self.hist[1];
        self.hist[2] = h2;
        let cur = self.cur;
        let d = cur.wrapping_sub(self.hist[0]);
        self.hist[1] = d;
        self.hist[0] = cur;
        let pred = (self.last << 3)
            .wrapping_add(self.w[0].wrapping_mul(cur))
            .wrapping_add(self.w[1].wrapping_mul(d))
            .wrapping_add(self.w[2].wrapping_mul(h2))
            .wrapping_add(self.w[3].wrapping_mul(h3))
            .wrapping_add(self.w[4].wrapping_mul(*g));
        let v = ((pred >> 3) & 0xFF) - sym as i32;
        let e = ((sym << 24) as i32) >> 21;

        // Ces deux formules reproduisent exactement le code du jeu (asymétrie comprise).
        let minus = |x: i32| if e.wrapping_sub(x) > 0 { e.wrapping_neg().wrapping_sub(x) } else { e.wrapping_sub(x) };
        let plus = |x: i32| if e.wrapping_add(x) > 0 { x.wrapping_sub(e) } else { e.wrapping_add(x) };
        self.acc[0] = self.acc[0].wrapping_sub(e.wrapping_abs());
        let xs = [cur, self.hist[1], self.hist[2], self.hist[3], *g];
        for (k, &x) in xs.iter().enumerate() {
            self.acc[1 + 2 * k] = self.acc[1 + 2 * k].wrapping_add(minus(x));
            self.acc[2 + 2 * k] = self.acc[2 + 2 * k].wrapping_add(plus(x));
        }

        let nh = ((v.wrapping_sub(self.last)) << 24) >> 24;
        self.cur = nh;
        *g = nh;
        self.last = v;

        if self.count & 0x1F == 0 {
            // Comparaison non signée, comme dans le jeu.
            let mut best = self.acc[0] as u32;
            let mut sel = 0u32;
            self.acc[0] = 0;
            for k in 1..=10 {
                if (self.acc[k] as u32) < best {
                    best = self.acc[k] as u32;
                    sel = k as u32;
                }
                self.acc[k] = 0;
            }
            let s = sel.wrapping_sub(1);
            if s <= 9 {
                let w = &mut self.w[(s / 2) as usize];
                if s % 2 == 0 {
                    if *w >= -16 {
                        *w -= 1;
                    }
                } else if *w <= 15 {
                    *w += 1;
                }
            }
        }
        v as u8
    }
}

struct Decoder<'a> {
    t: &'a Tables,
    bits: Bits<'a>,
    /// Longueurs du bloc précédent (codage en différence), 0x404 octets comme en RAM.
    prev: Vec<u8>,
    sound: bool,
    channels: usize,
    chan: usize,
    code_len: Huff,
    main: Huff,
    dist: Huff,
    len: Huff,
    sound_tabs: [Huff; 4],
}

impl Decoder<'_> {
    /// `Packed_BuildTables` : lit l'en-tête d'un bloc et reconstruit les arbres.
    fn new_block(&mut self) {
        let p = self.bits.peek16();
        self.sound = p & 0x8000 != 0;
        if p & 0x4000 == 0 {
            self.prev.fill(0);
        }
        self.bits.skip(2);
        let count = if self.sound {
            self.channels = ((p >> 12) & 3) as usize + 1;
            if self.chan >= self.channels {
                self.chan = 0;
            }
            self.bits.skip(2);
            self.channels * 257
        } else {
            374
        };

        let mut cl = [0u8; 19];
        for c in cl.iter_mut() {
            *c = self.bits.read(4) as u8;
        }
        self.code_len.build(&cl);

        let mut lens = vec![0u8; 0x404];
        let mut i = 0;
        while i < count {
            match self.code_len.read(&mut self.bits) {
                s @ 0..=15 => {
                    lens[i] = ((s as u8).wrapping_add(self.prev[i])) & 15;
                    i += 1;
                }
                16 => {
                    let n = self.bits.read(2) + 3;
                    for _ in 0..n {
                        if i >= count {
                            break;
                        }
                        // Répète la longueur précédente (lens[-1] n'arrive pas en pratique).
                        lens[i] = if i > 0 { lens[i - 1] } else { 0 };
                        i += 1;
                    }
                }
                s => {
                    let n = if s == 17 { self.bits.read(3) + 3 } else { self.bits.read(7) + 11 };
                    for _ in 0..n {
                        if i >= count {
                            break;
                        }
                        lens[i] = 0;
                        i += 1;
                    }
                }
            }
        }

        if self.sound {
            for k in 0..self.channels {
                self.sound_tabs[k].build(&lens[k * 257..(k + 1) * 257]);
            }
        } else {
            self.main.build(&lens[..298]);
            self.dist.build(&lens[298..346]);
            self.len.build(&lens[346..374]);
        }
        self.prev.copy_from_slice(&lens);
    }

    fn extra(&mut self, n: u8) -> u32 {
        if n == 0 {
            0
        } else {
            self.bits.read(n as u32)
        }
    }
}

pub struct Unpacked {
    pub data: Vec<u8>,
    /// Nombre de canaux si le premier bloc est en mode son, sinon 0.
    pub sound_channels: usize,
    /// Octets du flux lus (après l'en-tête de 8 octets), pour vérification.
    pub consumed: usize,
}

/// Décode une ressource ; `src` commence à l'en-tête et peut déborder après la fin du flux.
pub fn decompress(t: &Tables, src: &[u8]) -> Option<Unpacked> {
    let size = u32::from_le_bytes(src.get(0..4)?.try_into().ok()?) as usize;
    let packed = u32::from_le_bytes(src.get(4..8)?.try_into().ok()?) as usize;
    let mut d = Decoder {
        t,
        bits: Bits { src: &src[8..], byte: 0, bit: 0 },
        prev: vec![0; 0x404],
        sound: false,
        channels: 1,
        chan: 0,
        code_len: Huff::new(19),
        main: Huff::new(298),
        dist: Huff::new(48),
        len: Huff::new(28),
        sound_tabs: std::array::from_fn(|_| Huff::new(257)),
    };
    let mut pred = [Predictor::default(); 4];
    let mut g = 0i32;
    let mut recent = [0u32; 4];
    let (mut recent_n, mut last_dist, mut last_len) = (0usize, 0u32, 0u32);
    let mut out: Vec<u8> = Vec::with_capacity(size + 512);

    d.new_block();
    let sound_channels = if d.sound { d.channels } else { 0 };
    let mut remain = size as i64 - 1;
    while remain >= 0 {
        if d.sound {
            let s = d.sound_tabs[d.chan].read(&mut d.bits);
            if s == 0x100 {
                d.new_block();
                continue;
            }
            out.push(pred[d.chan].step(&mut g, s));
            d.chan += 1;
            if d.chan == d.channels {
                d.chan = 0;
            }
            remain -= 1;
            continue;
        }

        let s = d.main.read(&mut d.bits);
        let (len, dist) = match s {
            0..=0xFF => {
                out.push(s as u8);
                remain -= 1;
                continue;
            }
            0x10D => {
                d.new_block();
                continue;
            }
            0x100 => (last_len, last_dist),
            0x101..=0x104 => {
                let dist = recent[(recent_n + 0x100 - s as usize) & 3];
                let ls = d.len.read(&mut d.bits) as usize;
                let mut len = t.len_base[ls] as u32 + 2 + d.extra(t.len_bits[ls]);
                len += (dist > 0x3FFFF) as u32 + (dist > 0x1FFF) as u32 + (dist > 0x100) as u32;
                (len, dist)
            }
            0x105..=0x10C => {
                let i = (s - 0x105) as usize;
                (2, t.short_base[i] as u32 + 1 + d.extra(t.short_bits[i]))
            }
            _ => {
                let i = (s - 0x10E) as usize;
                let mut len = t.len_base[i] as u32 + 3 + d.extra(t.len_bits[i]);
                let ds = d.dist.read(&mut d.bits) as usize;
                let dist = t.dist_base[ds].wrapping_add(1) + d.extra(t.dist_bits[ds]);
                len += (dist > 0x3FFFF) as u32 + (dist > 0x1FFF) as u32;
                (len, dist)
            }
        };
        recent[recent_n & 3] = dist;
        recent_n += 1;
        last_dist = dist;
        last_len = len;
        remain -= len as i64;
        for _ in 0..len {
            // Le jeu remet la destination à zéro avant de décoder : une distance nulle
            // ou trop grande lit donc un 0.
            let from = out.len() as i64 - dist as i64;
            let byte = usize::try_from(from).ok().and_then(|f| out.get(f).copied()).unwrap_or(0);
            out.push(byte);
        }
    }

    // `sub_0802EF64` : lecture d'un éventuel symbole de fin (n'affecte que la position).
    if packed >= d.bits.byte + 5 {
        let s = if d.sound { d.sound_tabs[d.chan].read(&mut d.bits) } else { d.main.read(&mut d.bits) };
        if (d.sound && s == 0x100) || (!d.sound && s == 0x10D) {
            d.new_block();
        }
    }
    out.truncate(size);
    let consumed = d.bits.byte + (d.bits.bit > 0) as usize;
    Some(Unpacked { data: out, sound_channels, consumed })
}
