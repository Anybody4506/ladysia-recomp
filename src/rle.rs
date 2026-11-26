//! Décompresseur RLE de Lady Sia, réécrit d'après `Decomp_Entry` / `Decomp_Core` (ARM, 0x08000248).
//!
//! Format, à une adresse quelconque (non alignée) :
//! ```text
//! u32  taille décompressée
//! u8   méthode : bits 0-3 = m (0 = copie brute, 1..4 = RLE), bits hauts = motif du marqueur
//! ...  flux
//! ```
//! Avec m >= 1, les m bits de poids fort d'un octet servent de marqueur :
//! - octet dont les bits hauts == motif : n = bits bas, puis l'octet suivant est écrit n + 1 fois
//!   (n = 0 sert d'échappement pour un octet littéral qui ressemble à un marqueur) ;
//! - sinon l'octet est copié tel quel.

/// (masque des bits hauts, masque des bits bas), lu dans la ROM à 0x0800023C.
const MASKS: [(u8, u8); 5] = [(0x00, 0xFF), (0x80, 0x7F), (0xC0, 0x3F), (0xE0, 0x1F), (0xF0, 0x0F)];

pub struct Decoded {
    pub data: Vec<u8>,
    /// Octets lus en entrée, en-tête compris.
    pub consumed: usize,
}

pub fn decompress(src: &[u8]) -> Option<Decoded> {
    let size = u32::from_le_bytes(src.get(0..4)?.try_into().ok()?) as usize;
    let method = *src.get(4)?;
    let m = (method & 0xF) as usize;
    let mut out = Vec::with_capacity(size);
    if m == 0 {
        // Copie brute par mots de 4 octets, comme la boucle ARM.
        let len = size.div_ceil(4) * 4;
        out.extend_from_slice(src.get(5..5 + len)?);
        out.truncate(size);
        return Some(Decoded { data: out, consumed: 5 + len });
    }
    let (hi, lo) = *MASKS.get(m)?;
    let marker = method & hi;
    let mut pos = 5;
    while out.len() < size {
        let mut b = *src.get(pos)?;
        pos += 1;
        if b & hi == marker {
            let n = (b & lo) as usize;
            b = *src.get(pos)?;
            pos += 1;
            out.extend(std::iter::repeat_n(b, n));
        }
        out.push(b);
    }
    // Le code ARM compte à rebours et peut dépasser : on coupe à la taille annoncée.
    out.truncate(size);
    Some(Decoded { data: out, consumed: pos })
}

/// Vérifie qu'un flux est plausible avant de l'extraire (balayage de la ROM).
pub fn plausible(src: &[u8]) -> Option<Decoded> {
    let size = u32::from_le_bytes(src.get(0..4)?.try_into().ok()?) as usize;
    let method = *src.get(4)?;
    let m = (method & 0xF) as usize;
    if !(64..=0x4000).contains(&size) || size % 32 != 0 || !(1..=4).contains(&m) {
        return None;
    }
    // Les bits entre le motif et la méthode doivent être nuls.
    if method & !(MASKS[m].0 | 0x0F) != 0 {
        return None;
    }
    let d = decompress(src)?;
    (d.consumed < size).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_and_escape() {
        // taille 6, méthode 1 (marqueur = bit 7 à 0)... on utilise le motif 0x80 :
        // 0x83 0x41 -> 'A' x4 ; 0x80 0x05 -> littéral 0x05 ; 0x42 n'est pas un marqueur.
        let src = [6, 0, 0, 0, 0x81, 0x83, 0x41, 0x80, 0x05, 0x42];
        let d = decompress(&src).unwrap();
        assert_eq!(d.data, b"AAAA\x05B");
        assert_eq!(d.consumed, src.len());
    }

    #[test]
    fn raw_copy() {
        let src = [3, 0, 0, 0, 0x00, 1, 2, 3, 4];
        assert_eq!(decompress(&src).unwrap().data, [1, 2, 3]);
    }
}
