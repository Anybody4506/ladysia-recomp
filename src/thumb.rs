//! Désassembleur Thumb (16 bits) complet pour l'ARM7TDMI de la GBA.

use crate::arm::{reg, COND};

const ALU: [&str; 16] = [
    "and", "eor", "lsl", "lsr", "asr", "adc", "sbc", "ror", "tst", "neg", "cmp", "cmn", "orr",
    "mul", "bic", "mvn",
];

/// Effet d'une instruction sur le flot d'exécution.
#[derive(Clone, Copy, Debug)]
pub enum Flow {
    Next,
    Jump(u32),
    CondJump(u32),
    Call(u32),
    Return,
    /// `bx rN` vers une adresse calculée.
    BxReg(u32),
    /// `mov pc, rN` : saut indirect, souvent une table de `switch`.
    IndirectJump(u32),
    Swi(u32),
    Invalid,
}

pub struct TInsn {
    pub size: u32,
    pub text: String,
    pub flow: Flow,
    /// `ldr rX, [pc, #imm]` : (registre, adresse du littéral).
    pub literal: Option<(u32, u32)>,
    /// Registres écrasés (masque de bits).
    pub clobber: u32,
    /// `add rX, pc, #imm` : adresse calculée.
    pub pc_rel: Option<u32>,
}

fn reglist(mask: u32) -> String {
    let regs: Vec<String> = (0..16).filter(|i| mask & (1 << i) != 0).map(reg).collect();
    format!("{{{}}}", regs.join(", "))
}

/// Décode l'instruction Thumb `h` à l'adresse `addr` ; `next` sert pour la paire BL.
pub fn decode(h: u16, next: Option<u16>, addr: u32) -> TInsn {
    let h = h as u32;
    let pc = addr.wrapping_add(4);
    let lo = |shift: u32| (h >> shift) & 7;
    let r = reg;
    let rd = h & 7;
    let mut i = TInsn { size: 2, text: String::new(), flow: Flow::Next, literal: None, clobber: 0, pc_rel: None };

    i.text = match h >> 11 {
        0b00000..=0b00010 => {
            i.clobber = 1 << rd;
            let op = ["lsl", "lsr", "asr"][((h >> 11) & 3) as usize];
            format!("{op} {}, {}, #{}", r(rd), r(lo(3)), (h >> 6) & 31)
        }
        0b00011 => {
            i.clobber = 1 << rd;
            let op = if h & 0x200 != 0 { "sub" } else { "add" };
            let v = (h >> 6) & 7;
            let src = if h & 0x400 != 0 { format!("#{v}") } else { r(v) };
            format!("{op} {}, {}, {src}", r(rd), r(lo(3)))
        }
        0b00100..=0b00111 => {
            let op = (h >> 11) & 3;
            let rd = lo(8);
            if op != 1 {
                i.clobber = 1 << rd;
            }
            format!("{} {}, #0x{:x}", ["mov", "cmp", "add", "sub"][op as usize], r(rd), h & 0xFF)
        }
        0b01000 if h & 0x400 == 0 => {
            let op = ((h >> 6) & 15) as usize;
            if !matches!(op, 8 | 10 | 11) {
                i.clobber = 1 << rd;
            }
            format!("{} {}, {}", ALU[op], r(rd), r(lo(3)))
        }
        0b01000 => {
            let rd = (h & 7) | ((h >> 4) & 8);
            let rs = (h >> 3) & 15;
            match (h >> 8) & 3 {
                0 | 2 => {
                    i.clobber = 1 << rd;
                    if rd == 15 {
                        i.flow = Flow::IndirectJump(rs);
                    }
                    let op = if (h >> 8) & 3 == 0 { "add" } else { "mov" };
                    format!("{op} {}, {}", r(rd), r(rs))
                }
                1 => format!("cmp {}, {}", r(rd), r(rs)),
                _ => {
                    i.flow = if rs == 14 { Flow::Return } else { Flow::BxReg(rs) };
                    format!("bx {}", r(rs))
                }
            }
        }
        0b01001 => {
            let rd = lo(8);
            let lit = (pc & !3) + (h & 0xFF) * 4;
            i.literal = Some((rd, lit));
            i.clobber = 1 << rd;
            format!("ldr {}, =[0x{lit:08x}]", r(rd))
        }
        0b01010 | 0b01011 => {
            let op = (h >> 10) & 3;
            let name = if h & 0x200 == 0 {
                ["str", "strb", "ldr", "ldrb"][op as usize]
            } else {
                ["strh", "ldsb", "ldrh", "ldsh"][op as usize]
            };
            if name.starts_with('l') {
                i.clobber = 1 << rd;
            }
            format!("{name} {}, [{}, {}]", r(rd), r(lo(3)), r(lo(6)))
        }
        0b01100..=0b01111 => {
            let byte = h & 0x1000 != 0;
            let load = h & 0x800 != 0;
            let imm = (h >> 6) & 31;
            let off = if byte { imm } else { imm * 4 };
            let name = match (load, byte) {
                (false, false) => "str",
                (false, true) => "strb",
                (true, false) => "ldr",
                (true, true) => "ldrb",
            };
            if load {
                i.clobber = 1 << rd;
            }
            format!("{name} {}, [{}, #0x{off:x}]", r(rd), r(lo(3)))
        }
        0b10000 | 0b10001 => {
            let load = h & 0x800 != 0;
            if load {
                i.clobber = 1 << rd;
            }
            let name = if load { "ldrh" } else { "strh" };
            format!("{name} {}, [{}, #0x{:x}]", r(rd), r(lo(3)), ((h >> 6) & 31) * 2)
        }
        0b10010 | 0b10011 => {
            let load = h & 0x800 != 0;
            let rd = lo(8);
            if load {
                i.clobber = 1 << rd;
            }
            let name = if load { "ldr" } else { "str" };
            format!("{name} {}, [sp, #0x{:x}]", r(rd), (h & 0xFF) * 4)
        }
        0b10100 | 0b10101 => {
            let rd = lo(8);
            let off = (h & 0xFF) * 4;
            i.clobber = 1 << rd;
            if h & 0x800 != 0 {
                format!("add {}, sp, #0x{off:x}", r(rd))
            } else {
                let a = (pc & !3) + off;
                i.pc_rel = Some(a);
                format!("add {}, pc, #0x{off:x}  @ =0x{a:08x}", r(rd))
            }
        }
        0b10110 | 0b10111 if h & 0xFF00 == 0xB000 => {
            let sign = if h & 0x80 != 0 { "-" } else { "" };
            format!("add sp, #{sign}0x{:x}", (h & 0x7F) * 4)
        }
        0b10110 | 0b10111 if h & 0x600 == 0x400 => {
            let extra = h & 0x100 != 0;
            if h & 0x800 != 0 {
                let mask = (h & 0xFF) | if extra { 1 << 15 } else { 0 };
                i.clobber = mask;
                if extra {
                    i.flow = Flow::Return;
                }
                format!("pop {}", reglist(mask))
            } else {
                format!("push {}", reglist((h & 0xFF) | if extra { 1 << 14 } else { 0 }))
            }
        }
        0b11000 | 0b11001 => {
            let rb = lo(8);
            let load = h & 0x800 != 0;
            i.clobber = if load { (h & 0xFF) | (1 << rb) } else { 1 << rb };
            let name = if load { "ldmia" } else { "stmia" };
            format!("{name} {}!, {}", r(rb), reglist(h & 0xFF))
        }
        0b11010 | 0b11011 => match (h >> 8) & 15 {
            15 => {
                i.flow = Flow::Swi(h & 0xFF);
                format!("swi 0x{:x}", h & 0xFF)
            }
            14 => {
                i.flow = Flow::Invalid;
                format!(".hword 0x{h:04x}")
            }
            c => {
                let t = pc.wrapping_add((((h & 0xFF) as i8 as i32) * 2) as u32);
                i.flow = Flow::CondJump(t);
                format!("b{} 0x{t:08x}", COND[c as usize])
            }
        },
        0b11100 => {
            let t = pc.wrapping_add((((h & 0x7FF) << 21) as i32 >> 20) as u32);
            i.flow = Flow::Jump(t);
            format!("b 0x{t:08x}")
        }
        0b11110 if next.is_some_and(|n| n >> 11 == 0b11111) => {
            let hi = ((h & 0x7FF) << 21) as i32 >> 9;
            let lo = ((next.unwrap() as u32) & 0x7FF) << 1;
            let t = pc.wrapping_add(hi as u32).wrapping_add(lo);
            i.size = 4;
            i.flow = Flow::Call(t);
            i.clobber = 0x500F; // r0-r3, r12, lr
            format!("bl 0x{t:08x}")
        }
        _ => {
            i.flow = Flow::Invalid;
            format!(".hword 0x{h:04x}")
        }
    };
    i
}
