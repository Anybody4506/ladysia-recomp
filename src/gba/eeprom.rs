//! EEPROM de sauvegarde (« EEPROM_V122 » dans la ROM), pilotée bit à bit par DMA3
//! en 0x0D000000. Taille 512 octets (adresses 6 bits) ou 8 Ko (14 bits), déduite de
//! la longueur des transferts.

#[derive(Clone)]
pub struct Eeprom {
    pub data: Vec<u8>,
    bits: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
    /// Largeur d'adresse détectée (6 ou 14), 0 si inconnue.
    pub width: usize,
    pub dirty: bool,
}

impl Eeprom {
    pub fn new(save: Option<Vec<u8>>) -> Self {
        let mut data = vec![0xFF; 0x2000];
        let mut width = 0;
        if let Some(s) = save {
            let n = s.len().min(0x2000);
            data[..n].copy_from_slice(&s[..n]);
            width = if s.len() <= 512 { 6 } else { 14 };
        }
        Eeprom { data, bits: Vec::new(), out: Vec::new(), pos: 0, width, dirty: false }
    }

    /// Contenu à écrire dans le fichier de sauvegarde.
    pub fn image(&self) -> &[u8] {
        if self.width == 14 { &self.data } else { &self.data[..512] }
    }

    pub fn state(&self, w: &mut Vec<u8>) {
        w.extend_from_slice(&self.data);
        w.push(self.width as u8);
        w.extend_from_slice(&(self.out.len() as u32).to_le_bytes());
        w.extend_from_slice(&self.out);
        w.extend_from_slice(&(self.pos as u32).to_le_bytes());
    }

    pub fn load(&mut self, r: &mut super::State) -> Option<()> {
        // la sauvegarde du jeu (.sav) n'est jamais écrasée par une sauvegarde instantanée
        r.bytes(0x2000)?;
        self.width = r.u8()? as usize;
        let n = r.u32()? as usize;
        self.out = r.bytes(n)?.to_vec();
        self.pos = r.u32()? as usize;
        self.bits.clear();
        Some(())
    }

    pub fn begin(&mut self, _count: u32) {
        self.bits.clear();
    }

    pub fn write(&mut self, v: u32) {
        self.bits.push((v & 1) as u8);
    }

    pub fn read(&mut self) -> u32 {
        if self.pos < self.out.len() {
            self.pos += 1;
            self.out[self.pos - 1] as u32
        } else {
            1 // prêt
        }
    }

    fn number(bits: &[u8]) -> usize {
        bits.iter().fold(0, |a, &b| (a << 1) | b as usize)
    }

    pub fn finish(&mut self) {
        let b = std::mem::take(&mut self.bits);
        let n = b.len();
        if n < 3 || b[0] != 1 {
            return;
        }
        if b[1] == 1 {
            // lecture : 11 + adresse + 0
            let w = n - 3;
            if w == 6 || w == 14 {
                self.width = w;
            }
            let block = Self::number(&b[2..2 + w]) & if w == 6 { 0x3F } else { 0x3FF };
            self.out = vec![0; 4];
            for k in 0..64 {
                let byte = self.data[block * 8 + k / 8];
                self.out.push((byte >> (7 - k % 8)) & 1);
            }
            self.pos = 0;
        } else if n > 67 {
            // écriture : 10 + adresse + 64 bits + 0
            let w = n - 67;
            if w == 6 || w == 14 {
                self.width = w;
            }
            let block = Self::number(&b[2..2 + w]) & if w == 6 { 0x3F } else { 0x3FF };
            for k in 0..8 {
                self.data[block * 8 + k] = Self::number(&b[2 + w + 8 * k..2 + w + 8 * k + 8]) as u8;
            }
            self.out.clear();
            self.pos = 0;
            self.dirty = true;
        }
    }
}
