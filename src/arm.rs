//! Désassembleur ARM (32 bits) minimal : suffisant pour le crt0 de démarrage.
//! Le code du jeu lui-même est surtout en Thumb ; ce sera l'étape suivante.

pub const COND: [&str; 16] = [
    "eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "", "nv",
];
const DP_OPS: [&str; 16] = [
    "and", "eor", "sub", "rsb", "add", "adc", "sbc", "rsc", "tst", "teq", "cmp", "cmn", "orr",
    "mov", "bic", "mvn",
];
const SHIFTS: [&str; 4] = ["lsl", "lsr", "asr", "ror"];

pub fn reg(r: u32) -> String {
    match r {
        13 => "sp".into(),
        14 => "lr".into(),
        15 => "pc".into(),
        n => format!("r{n}"),
    }
}

pub struct Insn {
    pub text: String,
    /// Valeur chargée par un `ldr rX, [pc, #imm]` : (registre, adresse du littéral).
    pub literal: Option<(u32, u32)>,
    /// Cible d'un branchement direct B/BL.
    pub branch: Option<u32>,
    /// `bx rN` : registre utilisé.
    pub bx_reg: Option<u32>,
    /// Condition « toujours » (AL).
    pub always: bool,
    /// Branchement avec lien (BL = appel).
    pub link: bool,
    /// L'instruction écrit dans pc (retour ou saut indirect).
    pub writes_pc: bool,
    /// Registres écrasés (masque de bits).
    pub clobber: u32,
    /// `add rX, pc, #imm` : adresse calculée.
    pub pc_rel: Option<u32>,
    pub swi: Option<u32>,
}

fn reglist(mask: u32) -> String {
    let regs: Vec<String> = (0..16).filter(|i| mask & (1 << i) != 0).map(reg).collect();
    format!("{{{}}}", regs.join(", "))
}

fn operand2(ins: u32) -> String {
    if ins & (1 << 25) != 0 {
        let rot = ((ins >> 8) & 0xF) * 2;
        let imm = (ins & 0xFF).rotate_right(rot);
        format!("#0x{imm:x}")
    } else {
        let rm = reg(ins & 0xF);
        let st = SHIFTS[((ins >> 5) & 3) as usize];
        if ins & 0x10 != 0 {
            format!("{rm}, {st} {}", reg((ins >> 8) & 0xF))
        } else {
            let amt = (ins >> 7) & 0x1F;
            if amt == 0 && (ins >> 5) & 3 == 0 {
                rm
            } else {
                format!("{rm}, {st} #{amt}")
            }
        }
    }
}

/// Décode une instruction ARM située à l'adresse `addr`.
pub fn decode(ins: u32, addr: u32) -> Insn {
    let mut out = Insn {
        text: String::new(),
        literal: None,
        branch: None,
        bx_reg: None,
        always: ins >> 28 == 0xE,
        link: false,
        writes_pc: false,
        clobber: 0,
        pc_rel: None,
        swi: None,
    };
    let c = COND[(ins >> 28) as usize];

    out.text = if ins & 0x0FFF_FFF0 == 0x012F_FF10 {
        let r = ins & 0xF;
        out.bx_reg = Some(r);
        format!("bx{c} {}", reg(r))
    } else if ins & 0x0FBF_0FFF == 0x010F_0000 {
        let psr = if ins & (1 << 22) != 0 { "spsr" } else { "cpsr" };
        format!("mrs{c} {}, {psr}", reg((ins >> 12) & 0xF))
    } else if ins & 0x0DB0_F000 == 0x0120_F000 {
        let psr = if ins & (1 << 22) != 0 { "spsr" } else { "cpsr" };
        let fields: String = ['c', 'x', 's', 'f']
            .iter()
            .enumerate()
            .filter(|(i, _)| ins & (1 << (16 + i)) != 0)
            .map(|(_, ch)| *ch)
            .collect();
        format!("msr{c} {psr}_{fields}, {}", operand2(ins))
    } else if (ins >> 25) & 7 == 0b101 {
        out.link = ins & (1 << 24) != 0;
        let link = if out.link { "l" } else { "" };
        let off = (((ins & 0x00FF_FFFF) << 8) as i32) >> 6;
        let tgt = addr.wrapping_add(8).wrapping_add(off as u32);
        out.branch = Some(tgt);
        format!("b{link}{c} 0x{tgt:08x}")
    } else if (ins >> 24) & 0xF == 0xF {
        out.swi = Some((ins >> 16) & 0xFF);
        format!("swi{c} 0x{:x}", ins & 0x00FF_FFFF)
    } else if (ins >> 25) & 7 == 0b100 {
        let load = ins & (1 << 20) != 0;
        let mode = ["da", "ia", "db", "ib"][((ins >> 23) & 3) as usize];
        let wb = if ins & (1 << 21) != 0 { "!" } else { "" };
        if load {
            out.clobber = ins & 0xFFFF;
            out.writes_pc = ins & 0x8000 != 0;
        }
        let op = if load { "ldm" } else { "stm" };
        format!("{op}{mode}{c} {}{wb}, {}", reg((ins >> 16) & 0xF), reglist(ins & 0xFFFF))
    } else if (ins >> 26) & 3 == 0b01 {
        let load = ins & (1 << 20) != 0;
        let byte = if ins & (1 << 22) != 0 { "b" } else { "" };
        let rn = (ins >> 16) & 0xF;
        let rd = (ins >> 12) & 0xF;
        let up = ins & (1 << 23) != 0;
        let sign = if up { "" } else { "-" };
        let op = if load { "ldr" } else { "str" };
        if load {
            out.clobber = 1 << rd;
            out.writes_pc = rd == 15;
        }
        if ins & (1 << 25) == 0 {
            let off = ins & 0xFFF;
            if rn == 15 && load && ins & (1 << 24) != 0 {
                let base = addr.wrapping_add(8);
                let lit = if up { base.wrapping_add(off) } else { base.wrapping_sub(off) };
                out.literal = Some((rd, lit));
                format!("{op}{c}{byte} {}, =[0x{lit:08x}]", reg(rd))
            } else if ins & (1 << 24) == 0 {
                // Post-indexé : accès à [rn], puis rn += off.
                format!("{op}{c}{byte} {}, [{}], #{sign}0x{off:x}", reg(rd), reg(rn))
            } else {
                let wb = if ins & (1 << 21) != 0 { "!" } else { "" };
                format!("{op}{c}{byte} {}, [{}, #{sign}0x{off:x}]{wb}", reg(rd), reg(rn))
            }
        } else {
            format!("{op}{c}{byte} {}, [{}, {sign}{}]", reg(rd), reg(rn), operand2(ins & !(1 << 25)))
        }
    } else if (ins >> 26) & 3 == 0 {
        let opc = ((ins >> 21) & 0xF) as usize;
        let s = if ins & (1 << 20) != 0 && !(8..=11).contains(&opc) { "s" } else { "" };
        let rn = reg((ins >> 16) & 0xF);
        let rd = reg((ins >> 12) & 0xF);
        let op2 = operand2(ins);
        if !(8..=11).contains(&opc) {
            let rdn = (ins >> 12) & 0xF;
            out.clobber = 1 << rdn;
            out.writes_pc = rdn == 15;
            if opc == 4 && (ins >> 16) & 0xF == 15 && ins & (1 << 25) != 0 {
                let imm = (ins & 0xFF).rotate_right(((ins >> 8) & 0xF) * 2);
                out.pc_rel = Some(addr.wrapping_add(8).wrapping_add(imm));
            }
        }
        match opc {
            8..=11 => format!("{}{c} {rn}, {op2}", DP_OPS[opc]),
            13 | 15 => format!("{}{c}{s} {rd}, {op2}", DP_OPS[opc]),
            _ => format!("{}{c}{s} {rd}, {rn}, {op2}", DP_OPS[opc]),
        }
    } else {
        format!(".word 0x{ins:08x}")
    };
    out
}
