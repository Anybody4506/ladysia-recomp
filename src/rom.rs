//! Accès à la ROM et en-tête de cartouche GBA.

pub const ROM_BASE: u32 = 0x0800_0000;

pub struct Rom {
    pub data: Vec<u8>,
}

impl Rom {
    pub fn load(path: &str) -> std::io::Result<Self> {
        Ok(Rom { data: std::fs::read(path)? })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn u8(&self, off: usize) -> Option<u8> {
        self.data.get(off).copied()
    }

    pub fn u16(&self, off: usize) -> Option<u16> {
        let b = self.data.get(off..off + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&self, off: usize) -> Option<u32> {
        let b = self.data.get(off..off + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Convertit une adresse GBA (0x08xxxxxx) en offset dans le fichier.
    pub fn addr_to_off(&self, addr: u32) -> Option<usize> {
        let a = addr & !1; // bit Thumb
        if (ROM_BASE..ROM_BASE + self.len() as u32).contains(&a) {
            Some((a - ROM_BASE) as usize)
        } else {
            None
        }
    }

    pub fn is_rom_ptr(&self, v: u32) -> bool {
        self.addr_to_off(v).is_some()
    }
}

pub struct Header {
    pub entry_branch: u32,
    pub entry_off: usize,
    pub title: String,
    pub game_code: String,
    pub maker: String,
    pub fixed_96: u8,
    pub version: u8,
    pub checksum: u8,
    pub checksum_calc: u8,
}

impl Header {
    pub fn parse(rom: &Rom) -> Header {
        let ascii = |a: usize, b: usize| {
            rom.data[a..b]
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as char)
                .collect::<String>()
        };
        let entry_branch = rom.u32(0).unwrap_or(0);
        // Instruction ARM "B" : offset signé sur 24 bits, ×4, + 8 (pipeline).
        let imm = ((entry_branch & 0x00FF_FFFF) << 8) as i32 >> 6;
        let entry_off = (imm + 8) as usize;
        let checksum_calc = rom.data[0xA0..=0xBC]
            .iter()
            .fold(0u8, |acc, &b| acc.wrapping_sub(b))
            .wrapping_sub(0x19);
        Header {
            entry_branch,
            entry_off,
            title: ascii(0xA0, 0xAC),
            game_code: ascii(0xAC, 0xB0),
            maker: ascii(0xB0, 0xB2),
            fixed_96: rom.data[0xB2],
            version: rom.data[0xBC],
            checksum: rom.data[0xBD],
            checksum_calc,
        }
    }
}
