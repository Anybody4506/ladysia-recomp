//! Améliorations de l'image du portage PC, sans rien changer au jeu lui-même :
//! filtre HD xBR (contours détectés puis redessinés en vectoriel, anticrénelés à la
//! taille exacte de la fenêtre), pixels nets adoucis, lissage bilinéaire, lignes de
//! balayage, grille LCD, masque d'écran cathodique, couleurs vives, rémanence.

pub const W: usize = 240;
pub const H: usize = 160;

/// Découpe `dst` en blocs de `bloc` éléments et appelle `f(numéro, bloc)` sur
/// plusieurs cœurs.
fn en_parallele<T: Send, F: Fn(usize, &mut [T]) + Sync>(dst: &mut [T], bloc: usize, f: F) {
    let n = dst.len() / bloc;
    let fils = std::thread::available_parallelism().map_or(1, |v| v.get() / 2).clamp(1, 6);
    if fils == 1 || n < 16 {
        for (i, b) in dst.chunks_mut(bloc).enumerate() {
            f(i, b);
        }
        return;
    }
    let par = n.div_ceil(fils);
    std::thread::scope(|sc| {
        for (k, part) in dst.chunks_mut(bloc * par).enumerate() {
            let f = &f;
            sc.spawn(move || {
                for (i, b) in part.chunks_mut(bloc).enumerate() {
                    f(k * par + i, b);
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------
// xBR (niveau 2, d'après l'algorithme de Hyllian) : pour chaque coin de chaque pixel
// d'origine, on décide s'il est traversé par un contour (45°, 30° ou 60°) et de
// quelle couleur est l'autre côté. Le contour est ensuite tracé comme une droite,
// avec un anticrénelage d'un pixel de sortie.
// ---------------------------------------------------------------------------

/// Coin d'un pixel : bits 1 = contour à 45°, 2 = pente douce (30°), 4 = pente raide (60°).
#[derive(Clone, Copy, Default)]
struct Coin {
    forme: u8,
    px: u32,
}

#[inline]
fn yuv(c: u32) -> [f32; 3] {
    let (r, g, b) = (((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32);
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    [y, 0.492 * (b - y), 0.877 * (r - y)]
}

/// Analyse des contours : 4 coins par pixel (bas-droite, bas-gauche, haut-droite,
/// haut-gauche), chacun traité par symétrie comme le coin bas-droite.
fn xbr_coins(src: &[u32], coins: &mut Vec<[Coin; 4]>) {
    let yuvs: Vec<[f32; 3]> = src.iter().map(|&c| yuv(c)).collect();
    coins.resize(W * H, [Coin::default(); 4]);
    en_parallele(coins, W, |y, ligne| {
        let y = y as i32;
        for x in 0..W as i32 {
            for (k, &(sx, sy)) in [(1i32, 1i32), (-1, 1), (1, -1), (-1, -1)].iter().enumerate() {
                let at = |dx: i32, dy: i32| -> usize {
                    let xx = (x + dx * sx).clamp(0, W as i32 - 1) as usize;
                    let yy = (y + dy * sy).clamp(0, H as i32 - 1) as usize;
                    yy * W + xx
                };
                let (e, f, h) = (at(0, 0), at(1, 0), at(0, 1));
                let (ce, cf, ch) = (src[e], src[f], src[h]);
                if ce == cf || ce == ch {
                    ligne[x as usize][k] = Coin::default();
                    continue;
                }
                let (b, c, d, g, i) = (at(0, -1), at(1, -1), at(-1, 0), at(-1, 1), at(1, 1));
                let (f4, i4, h5, i5) = (at(2, 0), at(2, 1), at(0, 2), at(1, 2));
                let df = |p: usize, q: usize| -> f32 {
                    let (a, b) = (&yuvs[p], &yuvs[q]);
                    48.0 * (a[0] - b[0]).abs() + 7.0 * (a[1] - b[1]).abs() + 6.0 * (a[2] - b[2]).abs()
                };
                let eq = |p: usize, q: usize| src[p] == src[q];
                // restriction de niveau 1 (évite d'arrondir les coins voulus)
                let lv1 = (!eq(f, b) && !eq(h, d)) || (eq(e, i) && !eq(f, i4) && !eq(h, i5)) || eq(e, g) || eq(e, c);
                if !lv1 {
                    ligne[x as usize][k] = Coin::default();
                    continue;
                }
                let cote_e = df(e, c) + df(e, g) + df(i, h5) + df(i, f4) + 4.0 * df(h, f);
                let cote_i = df(h, d) + df(h, i5) + df(f, i4) + df(f, b) + 4.0 * df(e, i);
                if cote_e >= cote_i {
                    ligne[x as usize][k] = Coin::default();
                    continue;
                }
                let mut forme = 1u8;
                let (dfg, dhc) = (df(f, g), df(h, c));
                if 2.0 * dfg <= dhc && !eq(e, g) && !eq(d, g) {
                    forme |= 2;
                }
                if dfg >= 2.0 * dhc && !eq(e, c) && !eq(b, c) {
                    forme |= 4;
                }
                let px = if df(e, f) <= df(e, h) { cf } else { ch };
                ligne[x as usize][k] = Coin { forme, px };
            }
        }
    });
}

/// Couverture (0..1) du coin bas-droite par le contour, en coordonnées locales u, v
/// (0..1 dans le pixel d'origine) ; `ppx` = pixels de sortie par pixel d'origine.
#[inline]
fn couverture(forme: u8, u: f32, v: f32, ppx: f32) -> f32 {
    let lisse = |dist: f32| (0.5 + dist * ppx).clamp(0.0, 1.0);
    let mut a = 0.0f32;
    if forme & 1 != 0 {
        a = lisse((u + v - 1.5) * std::f32::consts::FRAC_1_SQRT_2);
    }
    if forme & 2 != 0 {
        a = a.max(lisse((v + 0.5 * u - 1.0) * 0.894_427));
    }
    if forme & 4 != 0 {
        a = a.max(lisse((u + 0.5 * v - 1.0) * 0.894_427));
    }
    a
}

/// Réglages de rendu (copiés depuis le menu).
#[derive(Clone, Copy, PartialEq)]
pub struct Rendu {
    /// 0 pixels nets, 1 lissée, 2 nets adoucis, 3 HD (xBR).
    pub filtre: u32,
    /// 0 aucun, 1 lignes de balayage, 2 grille LCD, 3 écran cathodique.
    pub effet: u32,
}

impl Rendu {
    /// Le rendu GDI simple suffit (pas de calcul sur le processeur).
    pub fn simple(&self, h: i32) -> bool {
        self.filtre < 2 && (self.effet == 0 || h < 2 * H as i32)
    }
}

/// Image de sortie à la taille de la fenêtre, avec tampons réutilisés.
#[derive(Default)]
pub struct Rendeur {
    coins: Vec<[Coin; 4]>,
    pub out: Vec<u32>,
    /// par colonne / ligne de sortie : pixels source, poids du second (sur 256), position dans le pixel
    cols: Vec<(u32, u32, u32, f32)>,
    rows: Vec<(u32, u32, u32, f32)>,
    col_w: Vec<[u32; 3]>,
    row_w: Vec<u32>,
    key: (i32, i32, u32, u32),
}

impl Rendeur {
    /// Produit `out` (w × h) à partir de l'image 240 × 160.
    pub fn rendre(&mut self, src: &[u32], w: i32, h: i32, r: Rendu) {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        let zoom = h as f64 / H as f64;
        let ppx = (w as f32 / W as f32).min(h as f32 / H as f32);
        let hd = r.filtre == 3;
        if hd {
            xbr_coins(src, &mut self.coins);
        }
        // tables d'échantillonnage et de pondération (recalculées si la taille change)
        let key = (w as i32, h as i32, r.filtre, r.effet);
        if key != self.key {
            self.key = key;
            let mode = r.filtre;
            let axe = |dst: usize, srcn: usize| -> Vec<(u32, u32, u32, f32)> {
                let k = dst as f64 / srcn as f64;
                (0..dst)
                    .map(|i| {
                        let c = (i as f64 + 0.5) / k; // centre du pixel de sortie, en pixels source
                        let cell = (c.floor() as usize).min(srcn - 1);
                        let pos = (c - c.floor()) as f32;
                        match mode {
                            1 | 2 => {
                                let s = (c - 0.5).max(0.0);
                                let i0 = (s.floor() as usize).min(srcn - 1);
                                let i1 = (i0 + 1).min(srcn - 1);
                                let mut f = s - i0 as f64;
                                if mode == 2 {
                                    // bilinéaire « net » : transition sur un seul pixel de sortie
                                    f = ((f - 0.5) * k + 0.5).clamp(0.0, 1.0);
                                }
                                let f = (f * 256.0).round().clamp(0.0, 256.0) as u32;
                                (i0 as u32, i1 as u32, f, pos)
                            }
                            _ => (cell as u32, cell as u32, 0, pos),
                        }
                    })
                    .collect()
            };
            self.cols = axe(w, W);
            self.rows = axe(h, H);
            // pondérations des effets, d'après la position dans le pixel GBA d'origine
            let fx = zoom >= 2.0;
            let dernier = |i: usize, dst: usize, srcn: usize| -> bool {
                let v = (i as f64 + 0.5) * srcn as f64 / dst as f64;
                let cell = v.floor() as usize;
                let end = ((cell + 1) * dst).div_ceil(srcn); // premier pixel de la case suivante
                i + 1 >= end
            };
            self.row_w = (0..h)
                .map(|y| {
                    if !fx {
                        return 256;
                    }
                    let p = self.rows[y].3 as f64;
                    let d = (2.0 * p - 1.0).powi(2);
                    match r.effet {
                        1 => (256.0 * (1.0 - 0.45 * d)) as u32,
                        2 => {
                            if dernier(y, h, H) {
                                200
                            } else {
                                256
                            }
                        }
                        3 => (256.0 * 1.3 * (1.0 - 0.6 * d.powf(0.8))) as u32,
                        _ => 256,
                    }
                })
                .collect();
            self.col_w = (0..w)
                .map(|x| {
                    if !fx {
                        return [256; 3];
                    }
                    match r.effet {
                        2 => {
                            if dernier(x, w, W) {
                                [200; 3]
                            } else {
                                [256; 3]
                            }
                        }
                        3 => {
                            let mut m = [200; 3];
                            m[x % 3] = 256;
                            m
                        }
                        _ => [256; 3],
                    }
                })
                .collect();
        }
        // échantillonnage à la taille exacte puis effets
        self.out.resize(w * h, 0);
        let (rows, cols, row_w, col_w, coins) = (&self.rows, &self.cols, &self.row_w, &self.col_w, &self.coins);
        en_parallele(&mut self.out, w, |y, line| {
            let (y0, y1, fy, v) = rows[y];
            let rw = row_w[y];
            let r0 = &src[y0 as usize * W..][..W];
            let r1 = &src[y1 as usize * W..][..W];
            for (x, &(x0, x1, fxw, u)) in cols.iter().enumerate() {
                let c = if hd {
                    let e = r0[x0 as usize];
                    let cs = &coins[y0 as usize * W + x0 as usize];
                    let mut best = 0.0f32;
                    let mut col = e;
                    for (k, coin) in cs.iter().enumerate() {
                        if coin.forme == 0 {
                            continue;
                        }
                        let uu = if k & 1 == 0 { u } else { 1.0 - u };
                        let vv = if k < 2 { v } else { 1.0 - v };
                        let a = couverture(coin.forme, uu, vv, ppx);
                        if a > best {
                            best = a;
                            col = coin.px;
                        }
                    }
                    lerp(e, col, (best * 256.0) as u32)
                } else if fy == 0 && fxw == 0 {
                    r0[x0 as usize]
                } else {
                    let (p00, p01, p10, p11) = (r0[x0 as usize], r0[x1 as usize], r1[x0 as usize], r1[x1 as usize]);
                    lerp(lerp(p00, p01, fxw), lerp(p10, p11, fxw), fy)
                };
                let cw = &col_w[x];
                if rw == 256 && cw[0] == 256 && cw[1] == 256 {
                    line[x] = c;
                    continue;
                }
                let ch = |sh: u32, k: usize| (((c >> sh) & 255) * rw * cw[k] >> 16).min(255);
                line[x] = (ch(16, 0) << 16) | (ch(8, 1) << 8) | ch(0, 2);
            }
        });
    }
}

#[inline]
fn lerp(a: u32, b: u32, f: u32) -> u32 {
    if f == 0 || a == b {
        return a;
    }
    if f >= 256 {
        return b;
    }
    let g = 256 - f;
    let rb = ((a & 0xFF00FF) * g + (b & 0xFF00FF) * f) >> 8 & 0xFF00FF;
    let gg = ((a & 0x00FF00) * g + (b & 0x00FF00) * f) >> 8 & 0x00FF00;
    rb | gg
}

/// Rémanence de l'écran LCD : moyenne de l'image courante et de la précédente.
pub fn remanence(img: &mut [u32], prev: &mut Vec<u32>) {
    if prev.len() != img.len() {
        prev.clear();
        prev.extend_from_slice(img);
        return;
    }
    for (c, p) in img.iter_mut().zip(prev.iter_mut()) {
        let cur = *c;
        *c = ((cur & 0xFEFEFE) >> 1) + ((*p & 0xFEFEFE) >> 1) + (cur & *p & 0x010101);
        *p = cur;
    }
}

/// Table « couleurs vives » (saturation et contraste renforcés), indexée par couleur
/// 15 bits comme la table « écran GBA ».
pub fn vives_table() -> Vec<u32> {
    let mut t = vec![0u32; 32768];
    for (i, out) in t.iter_mut().enumerate() {
        let c = |v: usize| {
            let v = v & 31;
            ((v << 3) | (v >> 2)) as f64 / 255.0
        };
        let (r, g, b) = (c(i), c(i >> 5), c(i >> 10));
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        let s = 1.3;
        let f = |v: f64| {
            let v = l + (v - l) * s;
            // légère courbe en S pour le contraste
            let v = v.clamp(0.0, 1.0);
            let v = v + 0.12 * v * (1.0 - v) * (2.0 * v - 1.0);
            (v.clamp(0.0, 1.0) * 255.0).round() as u32
        };
        *out = (f(r) << 16) | (f(g) << 8) | f(b);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbr_garde_les_aplats() {
        let src = vec![0x123456; W * H];
        let mut r = Rendeur::default();
        r.rendre(&src, 960, 640, Rendu { filtre: 3, effet: 0 });
        assert!(r.out.iter().all(|&c| c == 0x123456));
    }

    #[test]
    fn xbr_arrondit_une_diagonale() {
        // demi-plan sous la diagonale : les marches doivent devenir une droite lissée
        let src: Vec<u32> = (0..W * H).map(|i| if i % W > i / W { 0xFFFFFF } else { 0 }).collect();
        let mut r = Rendeur::default();
        r.rendre(&src, 960, 640, Rendu { filtre: 3, effet: 0 });
        let gris = r.out.iter().filter(|&&c| c != 0 && c != 0xFFFFFF).count();
        assert!(gris > 0, "pas d'anticrénelage");
        // loin du contour : couleurs d'origine
        assert_eq!(r.out[100 * 960 + 900], 0xFFFFFF);
        assert_eq!(r.out[600 * 960 + 20], 0);
    }

    #[test]
    fn rendu_taille_exacte() {
        let src: Vec<u32> = (0..W * H).map(|i| (i as u32).wrapping_mul(2654435761) & 0xFFFFFF).collect();
        let mut r = Rendeur::default();
        for filtre in 0..4 {
            for effet in 0..4 {
                r.rendre(&src, 1000, 700, Rendu { filtre, effet });
                assert_eq!(r.out.len(), 1000 * 700);
            }
        }
        // pixels nets à l'échelle 3 sans effet : identique à l'agrandissement simple
        r.rendre(&src, 720, 480, Rendu { filtre: 0, effet: 0 });
        assert_eq!(r.out[(3 * 50 + 1) * 720 + 3 * 70 + 2], src[50 * W + 70]);
        // nets adoucis à l'échelle entière : aussi identique au centre des pixels
        r.rendre(&src, 720, 480, Rendu { filtre: 2, effet: 0 });
        assert_eq!(r.out[(3 * 50 + 1) * 720 + 3 * 70 + 1], src[50 * W + 70]);
    }
}
