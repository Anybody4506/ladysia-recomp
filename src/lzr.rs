//! Compression des données embarquées dans l'exécutable : LZ77 (fenêtre = tout le
//! fichier, dernière distance réutilisable) + codeur arithmétique binaire adaptatif, à la
//! manière de LZMA en plus simple. Sans dépendance ; la décompression de 4 Mo prend une
//! fraction de seconde au lancement.
//!
//! Format : `LSD1`, taille d'origine (u32), puis le flux du codeur. Symboles :
//! - bit « répétition ? » (contexte : le symbole précédent en était une) ;
//! - littéral : 8 bits en arbre, contexte = octet précédent ;
//! - répétition : bit « même distance que la précédente ? », sinon distance (nombre de
//!   bits en arbre de 5 bits, bit de poids fort implicite, 4 bits suivants modélisés, le
//!   reste direct), puis longueur − 2 (arbre de 8 bits, 2 à 257).

const MAGIC: &[u8; 4] = b"LSD1";
const MIN: usize = 2;
const MAX: usize = 257;
const P0: u16 = 1024; // probabilité initiale (sur 2048)

struct Models {
    is_match: [u16; 2],
    is_rep: [u16; 2],
    lit: Vec<u16>,        // 256 contextes × 256
    len: [u16; 256],
    len_rep: [u16; 256],
    dist_bits: [u16; 32],
    dist_hi: Vec<u16>,    // 32 × 16 : 4 bits sous le bit de poids fort
}

impl Models {
    fn new() -> Self {
        Models {
            is_match: [P0; 2],
            is_rep: [P0; 2],
            lit: vec![P0; 256 * 256],
            len: [P0; 256],
            len_rep: [P0; 256],
            dist_bits: [P0; 32],
            dist_hi: vec![P0; 32 * 16],
        }
    }
}

// ----------------------------------------------------------------- codeur ----
struct Enc {
    low: u64,
    range: u32,
    cache: u8,
    pending: u64,
    out: Vec<u8>,
}

impl Enc {
    fn new() -> Self {
        Enc { low: 0, range: u32::MAX, cache: 0, pending: 1, out: Vec::new() }
    }
    fn shift(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut c = self.cache;
            loop {
                self.out.push(c.wrapping_add(carry));
                c = 0xFF;
                self.pending -= 1;
                if self.pending == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.pending += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }
    fn bit(&mut self, p: &mut u16, b: u32) {
        let bound = (self.range >> 11) * *p as u32;
        if b == 0 {
            self.range = bound;
            *p += (2048 - *p) >> 5;
        } else {
            self.low += bound as u64;
            self.range -= bound;
            *p -= *p >> 5;
        }
        while self.range < 1 << 24 {
            self.range <<= 8;
            self.shift();
        }
    }
    fn direct(&mut self, v: u32, n: u32) {
        for k in (0..n).rev() {
            self.range >>= 1;
            if (v >> k) & 1 != 0 {
                self.low += self.range as u64;
            }
            while self.range < 1 << 24 {
                self.range <<= 8;
                self.shift();
            }
        }
    }
    fn tree(&mut self, probs: &mut [u16], n: u32, v: u32) {
        let mut m = 1usize;
        for k in (0..n).rev() {
            let b = (v >> k) & 1;
            self.bit(&mut probs[m], b);
            m = m * 2 + b as usize;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift();
        }
        self.out
    }
}

struct Dec<'a> {
    d: &'a [u8],
    p: usize,
    range: u32,
    code: u32,
}

impl<'a> Dec<'a> {
    fn new(d: &'a [u8]) -> Self {
        let mut x = Dec { d, p: 1, range: u32::MAX, code: 0 };
        for _ in 0..4 {
            x.code = (x.code << 8) | x.next();
        }
        x
    }
    #[inline]
    fn next(&mut self) -> u32 {
        let b = self.d.get(self.p).copied().unwrap_or(0);
        self.p += 1;
        b as u32
    }
    #[inline]
    fn bit(&mut self, p: &mut u16) -> u32 {
        let bound = (self.range >> 11) * *p as u32;
        let b = if self.code < bound {
            self.range = bound;
            *p += (2048 - *p) >> 5;
            0
        } else {
            self.code -= bound;
            self.range -= bound;
            *p -= *p >> 5;
            1
        };
        if self.range < 1 << 24 {
            self.range <<= 8;
            self.code = (self.code << 8) | self.next();
        }
        b
    }
    fn direct(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            self.range >>= 1;
            let b = (self.code >= self.range) as u32;
            if b != 0 {
                self.code -= self.range;
            }
            v = v * 2 + b;
            if self.range < 1 << 24 {
                self.range <<= 8;
                self.code = (self.code << 8) | self.next();
            }
        }
        v
    }
    #[inline]
    fn tree(&mut self, probs: &mut [u16], n: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..n {
            m = m * 2 + self.bit(&mut probs[m]) as usize;
        }
        m as u32 - (1 << n)
    }
}

// ------------------------------------------------------------- compression ----
fn hash(d: &[u8], i: usize) -> usize {
    let v = u32::from_le_bytes([d[i], d[i + 1], d[i + 2], 0]);
    (v.wrapping_mul(0x9E37_79B1) >> 14) as usize
}

fn match_len(d: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n < max && d[a + n] == d[b + n] {
        n += 1;
    }
    n
}

/// Coût approximatif (en bits) d'une distance, pour comparer deux répétitions.
fn dist_cost(dist: usize) -> usize {
    6 + (usize::BITS - dist.leading_zeros()) as usize
}

pub fn compress(d: &[u8]) -> Vec<u8> {
    let n = d.len();
    let mut head = vec![u32::MAX; 1 << 18];
    let mut prev = vec![u32::MAX; n];
    let mut m = Models::new();
    let mut e = Enc::new();
    let (mut i, mut rep, mut was_match) = (0usize, 1usize, 0usize);
    let insert = |head: &mut [u32], prev: &mut [u32], i: usize| {
        if i + 2 < n {
            let h = hash(d, i);
            prev[i] = head[h];
            head[h] = i as u32;
        }
    };
    // meilleure répétition en `i` : (longueur, distance), 0 si aucune ne vaut le coup
    let find = |head: &[u32], prev: &[u32], i: usize, rep: usize| -> (usize, usize) {
        let max = MAX.min(n - i);
        let mut best = (0usize, 0usize);
        if rep <= i && max >= MIN {
            let l = match_len(d, i - rep, i, max);
            if l >= MIN {
                best = (l, rep);
            }
        }
        if i + 2 < n {
            let mut c = head[hash(d, i)];
            let mut tries = 96;
            while c != u32::MAX && tries > 0 {
                let c0 = c as usize;
                let l = match_len(d, c0, i, max);
                let dist = i - c0;
                // une répétition plus longue doit payer le prix de sa distance
                let better = if best.1 == rep && best.0 > 0 {
                    l > best.0 + 1 + (dist_cost(dist) / 8)
                } else {
                    l > best.0 || (l == best.0 && dist < best.1)
                };
                if l >= 3 && better && !(l == 3 && dist > 0x1000) && !(l == 4 && dist > 0x4_0000) {
                    best = (l, dist);
                }
                c = prev[c0];
                tries -= 1;
            }
        }
        best
    };
    while i < n {
        let mut b = find(&head, &prev, i, rep);
        if b.0 > 0 && b.0 < 32 && i + 1 < n {
            // évaluation paresseuse : si la position suivante fait nettement mieux, littéral
            insert(&mut head, &mut prev, i);
            let nb = find(&head, &prev, i + 1, rep);
            if nb.0 > b.0 + 1 {
                b = (0, 0);
            }
        } else {
            insert(&mut head, &mut prev, i);
        }
        if b.0 == 0 {
            e.bit(&mut m.is_match[was_match], 0);
            let ctx = if i > 0 { d[i - 1] as usize } else { 0 };
            e.tree(&mut m.lit[ctx * 256..ctx * 256 + 256], 8, d[i] as u32);
            was_match = 0;
            i += 1;
            continue;
        }
        let (l, dist) = b;
        e.bit(&mut m.is_match[was_match], 1);
        if dist == rep {
            e.bit(&mut m.is_rep[was_match], 1);
            e.tree(&mut m.len_rep, 8, (l - MIN) as u32);
        } else {
            e.bit(&mut m.is_rep[was_match], 0);
            let nb = usize::BITS - dist.leading_zeros(); // 1..=32
            e.tree(&mut m.dist_bits, 5, nb - 1);
            let rest = nb - 1; // bits sous le bit de poids fort
            let low = dist as u32 & ((1u32 << rest) - 1);
            if rest <= 4 {
                e.tree(&mut m.dist_hi[(nb as usize - 1) * 16..(nb as usize) * 16], rest, low);
            } else {
                e.tree(&mut m.dist_hi[(nb as usize - 1) * 16..(nb as usize) * 16], 4, low >> (rest - 4));
                e.direct(low & ((1 << (rest - 4)) - 1), rest - 4);
            }
            e.tree(&mut m.len, 8, (l - MIN) as u32);
            rep = dist;
        }
        for k in 1..l {
            insert(&mut head, &mut prev, i + k);
        }
        was_match = 1;
        i += l;
    }
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend(e.finish());
    out
}

// ----------------------------------------------------------- décompression ----
pub fn is_compressed(d: &[u8]) -> bool {
    d.starts_with(MAGIC)
}

pub fn decompress(d: &[u8]) -> Option<Vec<u8>> {
    if !is_compressed(d) || d.len() < 8 {
        return None;
    }
    let n = u32::from_le_bytes(d[4..8].try_into().ok()?) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut m = Models::new();
    let mut r = Dec::new(&d[8..]);
    let (mut rep, mut was_match) = (1usize, 0usize);
    while out.len() < n {
        if r.bit(&mut m.is_match[was_match]) == 0 {
            let ctx = out.last().copied().unwrap_or(0) as usize;
            let b = r.tree(&mut m.lit[ctx * 256..ctx * 256 + 256], 8);
            out.push(b as u8);
            was_match = 0;
            continue;
        }
        let l;
        if r.bit(&mut m.is_rep[was_match]) == 1 {
            l = r.tree(&mut m.len_rep, 8) as usize + MIN;
        } else {
            let nb = r.tree(&mut m.dist_bits, 5) + 1;
            let rest = nb - 1;
            let low = if rest <= 4 {
                r.tree(&mut m.dist_hi[(nb as usize - 1) * 16..(nb as usize) * 16], rest)
            } else {
                let hi = r.tree(&mut m.dist_hi[(nb as usize - 1) * 16..(nb as usize) * 16], 4);
                (hi << (rest - 4)) | r.direct(rest - 4)
            };
            rep = ((1u64 << rest) as usize) | low as usize;
            l = r.tree(&mut m.len, 8) as usize + MIN;
        }
        if rep > out.len() || out.len() + l > n {
            return None;
        }
        let s = out.len() - rep;
        for k in 0..l {
            let b = out[s + k];
            out.push(b);
        }
        was_match = 1;
    }
    Some(out)
}

/// Données telles quelles, ou décompressées si elles sont au format `LSD1`.
pub fn load(d: Vec<u8>) -> Result<Vec<u8>, String> {
    if is_compressed(&d) {
        decompress(&d).ok_or_else(|| "données compressées illisibles".to_string())
    } else {
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn aller_retour() {
        let mut seed = 12345u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            seed >> 16
        };
        // mélange de zéros, de motifs répétés, de texte et de bruit
        let mut d: Vec<u8> = Vec::new();
        d.extend(std::iter::repeat_n(0u8, 5000));
        for _ in 0..300 {
            let n = 1 + rnd() as usize % 40;
            match rnd() % 4 {
                0 => d.extend((0..n).map(|_| rnd() as u8)),
                1 => d.extend(std::iter::repeat_n(rnd() as u8, n * 20)),
                2 => {
                    let s = rnd() as usize % d.len();
                    let e = (s + n * 9).min(d.len());
                    let c = d[s..e].to_vec();
                    d.extend(c);
                }
                _ => d.extend(b"Lady Sia, princesse guerriere. ".iter().cycle().take(n * 3)),
            }
        }
        for data in [&d[..], &d[..1], &[][..], &d[..4999]] {
            let c = super::compress(data);
            assert_eq!(super::decompress(&c).as_deref(), Some(data));
        }
        assert!(super::compress(&d).len() < d.len() / 2);
        assert_eq!(super::load(d.clone()).unwrap(), d);
    }
}
