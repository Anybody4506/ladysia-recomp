//! Noms des appels BIOS (SWI) et des registres matériels de la GBA.

pub fn swi_name(n: u32) -> &'static str {
    const NAMES: [&str; 0x2B] = [
        "SoftReset", "RegisterRamReset", "Halt", "Stop", "IntrWait", "VBlankIntrWait", "Div",
        "DivArm", "Sqrt", "ArcTan", "ArcTan2", "CpuSet", "CpuFastSet", "GetBiosChecksum",
        "BgAffineSet", "ObjAffineSet", "BitUnPack", "LZ77UnCompWram", "LZ77UnCompVram",
        "HuffUnComp", "RLUnCompWram", "RLUnCompVram", "Diff8bitUnFilterWram",
        "Diff8bitUnFilterVram", "Diff16bitUnFilter", "SoundBias", "SoundDriverInit",
        "SoundDriverMode", "SoundDriverMain", "SoundDriverVSync", "SoundChannelClear",
        "MidiKey2Freq", "SoundWhatever0", "SoundWhatever1", "SoundWhatever2", "SoundWhatever3",
        "SoundWhatever4", "MultiBoot", "HardReset", "CustomHalt", "SoundDriverVSyncOff",
        "SoundDriverVSyncOn", "SoundGetJumpList",
    ];
    NAMES.get(n as usize).copied().unwrap_or("?")
}

pub fn io_name(addr: u32) -> Option<&'static str> {
    Some(match addr {
        0x0400_0000 => "REG_DISPCNT",
        0x0400_0004 => "REG_DISPSTAT",
        0x0400_0006 => "REG_VCOUNT",
        0x0400_0008 => "REG_BG0CNT",
        0x0400_000A => "REG_BG1CNT",
        0x0400_000C => "REG_BG2CNT",
        0x0400_000E => "REG_BG3CNT",
        0x0400_0010 => "REG_BG0HOFS",
        0x0400_0040 => "REG_WIN0H",
        0x0400_0048 => "REG_WININ",
        0x0400_004C => "REG_MOSAIC",
        0x0400_0050 => "REG_BLDCNT",
        0x0400_0052 => "REG_BLDALPHA",
        0x0400_0054 => "REG_BLDY",
        0x0400_0060 => "REG_SOUND1CNT_L",
        0x0400_0080 => "REG_SOUNDCNT_L",
        0x0400_0082 => "REG_SOUNDCNT_H",
        0x0400_0084 => "REG_SOUNDCNT_X",
        0x0400_0088 => "REG_SOUNDBIAS",
        0x0400_00A0 => "REG_FIFO_A",
        0x0400_00A4 => "REG_FIFO_B",
        0x0400_00B0 => "REG_DMA0SAD",
        0x0400_00BC => "REG_DMA1SAD",
        0x0400_00C8 => "REG_DMA2SAD",
        0x0400_00D4 => "REG_DMA3SAD",
        0x0400_0100 => "REG_TM0CNT",
        0x0400_0104 => "REG_TM1CNT",
        0x0400_0108 => "REG_TM2CNT",
        0x0400_010C => "REG_TM3CNT",
        0x0400_0130 => "REG_KEYINPUT",
        0x0400_0132 => "REG_KEYCNT",
        0x0400_0200 => "REG_IE",
        0x0400_0202 => "REG_IF",
        0x0400_0204 => "REG_WAITCNT",
        0x0400_0208 => "REG_IME",
        0x0300_7FFC => "INTR_VECTOR",
        _ => return None,
    })
}

/// Zone mémoire d'une adresse, pour annoter les littéraux.
pub fn region(addr: u32) -> Option<&'static str> {
    Some(match addr >> 24 {
        0x02 => "EWRAM",
        0x03 => "IWRAM",
        0x04 => "IO",
        0x05 => "PALETTE",
        0x06 => "VRAM",
        0x07 => "OAM",
        0x08 | 0x09 => "ROM",
        0x0E => "SRAM",
        _ => return None,
    })
}
