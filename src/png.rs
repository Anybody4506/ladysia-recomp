//! Écriture PNG minimale sans dépendance (zlib en blocs « stockés », non compressés).

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Image 8 bits indexée (`pixels` = un index de palette par pixel).
pub fn indexed(width: usize, height: usize, pixels: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
    encode(width, height, pixels, 1, Some(palette), false)
}

/// Image indexée dont la couleur 0 est transparente (chunk tRNS).
pub fn indexed_alpha0(width: usize, height: usize, pixels: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
    encode(width, height, pixels, 1, Some(palette), true)
}

/// Image RVB 24 bits (`pixels` = 3 octets par pixel).
pub fn rgb(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    encode(width, height, pixels, 3, None, false)
}

fn encode(width: usize, height: usize, pixels: &[u8], bpp: usize, palette: Option<&[[u8; 3]]>, alpha0: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * bpp + 1) * height);
    for row in pixels.chunks(width * bpp).take(height) {
        raw.push(0); // filtre « aucun »
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(0xFFFF).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push((i + 1 == chunks.len()) as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    // 8 bits par composante ; type 3 = palette, type 2 = RVB
    ihdr.extend_from_slice(&[8, if palette.is_some() { 3 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if let Some(p) = palette {
        chunk(&mut out, b"PLTE", &p.concat());
        if alpha0 {
            chunk(&mut out, b"tRNS", &[0]);
        }
    }
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Tuiles GBA 8x8 en 4 bits/pixel -> planche de `cols` tuiles de large.
pub fn tiles_4bpp(data: &[u8], cols: usize) -> (usize, usize, Vec<u8>) {
    let n = data.len() / 32;
    let rows = n.div_ceil(cols).max(1);
    let (w, h) = (cols * 8, rows * 8);
    let mut px = vec![0u8; w * h];
    for (t, tile) in data.chunks_exact(32).enumerate() {
        let (tx, ty) = ((t % cols) * 8, (t / cols) * 8);
        for (i, &byte) in tile.iter().enumerate() {
            let (x, y) = ((i % 4) * 2, i / 4);
            px[(ty + y) * w + tx + x] = byte & 0xF;
            px[(ty + y) * w + tx + x + 1] = byte >> 4;
        }
    }
    (w, h, px)
}

/// Palette de gris (16 niveaux) en attendant les vraies palettes du jeu.
pub fn gray16() -> Vec<[u8; 3]> {
    (0..16).map(|i| [i * 17, i * 17, i * 17]).collect()
}

/// Tuiles 8x8 en 8 bits par pixel (64 octets par tuile).
pub fn tiles_8bpp(data: &[u8], cols: usize) -> (usize, usize, Vec<u8>) {
    let n = data.len() / 64;
    let rows = n.div_ceil(cols).max(1);
    let (w, h) = (cols * 8, rows * 8);
    let mut px = vec![0u8; w * h];
    for (t, tile) in data.chunks_exact(64).enumerate() {
        let (tx, ty) = ((t % cols) * 8, (t / cols) * 8);
        for (i, &byte) in tile.iter().enumerate() {
            px[(ty + i / 8) * w + tx + i % 8] = byte;
        }
    }
    (w, h, px)
}

/// Palette de gris (256 niveaux).
pub fn gray256() -> Vec<[u8; 3]> {
    (0..=255u8).map(|i| [i, i, i]).collect()
}

/// Part des paires de pixels voisins identiques (dans une ligne de tuile).
pub fn coherence(data: &[u8], bpp8: bool) -> f64 {
    let px: Vec<u8> = if bpp8 { data.to_vec() } else { data.iter().flat_map(|b| [b & 15, b >> 4]).collect() };
    let same = (0..px.len().saturating_sub(1)).filter(|&i| i % 8 != 7 && px[i] == px[i + 1]).count();
    same as f64 / px.len().max(1) as f64
}
