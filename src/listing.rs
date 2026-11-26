//! Génère le listing assembleur annoté (asm/ladysia.s) et la liste des fonctions.

use crate::analysis::{Analysis, Function, Mode, Origin};
use crate::bios::{io_name, region, swi_name};
use crate::rom::Rom;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

fn annotate_word(an: &Analysis, rom: &Rom, v: u32) -> String {
    if an.funcs.contains_key(&(v & !1)) {
        return format!("-> {}", an.label(v));
    }
    if an.labels.contains(&(v & !1)) {
        return format!("-> {}", an.label(v));
    }
    if let Some(n) = io_name(v) {
        return n.to_string();
    }
    if rom.is_rom_ptr(v) {
        return "ptr ROM (données ?)".into();
    }
    region(v).map(|r| format!("ptr {r}")).unwrap_or_default()
}

pub fn write_asm(an: &Analysis, rom: &Rom) -> String {
    let callers = an.callers();
    let mut out = String::new();
    writeln!(out, "@ Lady Sia (GBA) — désassemblage automatique (étape 2)").unwrap();
    writeln!(out, "@ sub_ = fonction, loc_ = cible de saut interne\n").unwrap();

    for (&addr, f) in &an.funcs {
        let mode = match f.mode {
            Mode::Arm => "ARM",
            Mode::Thumb => "Thumb",
        };
        let origin = match f.origin {
            Origin::Entry => "point d'entrée",
            Origin::Call => "bl",
            Origin::Indirect => "appel indirect",
            Origin::Callback => "pointeur de fonction",
            Origin::Symbol => "symbols.txt",
        };
        let tags = tags(rom, f).join(", ");
        writeln!(
            out,
            "\n@ ---- {} ({mode}, {} appelant(s), trouvée par : {origin}) {}----",
            an.label(addr),
            callers.get(&addr).copied().unwrap_or(0),
            if tags.is_empty() { String::new() } else { format!("[{tags}] ") }
        )
        .unwrap();
        writeln!(out, "    .{}", if f.mode == Mode::Thumb { "thumb_func" } else { "arm" }).unwrap();
        writeln!(out, "{}:", an.label(addr)).unwrap();

        // Instructions et littéraux, triés par adresse.
        let mut items: BTreeMap<u32, String> = BTreeMap::new();
        for (&pc, l) in &f.lines {
            let mut text = l.text.clone();
            if let Some(t) = l.target {
                text = text.replace(&format!("0x{t:08x}"), &an.label(t));
            }
            if let Some(n) = text.strip_prefix("swi 0x").and_then(|n| u32::from_str_radix(n, 16).ok()) {
                // En ARM le numéro est dans les bits 16-23.
                let n = if n > 0xFF { n >> 16 } else { n };
                text = format!("{text}  @ {}", swi_name(n));
            }
            let raw = if l.size == 2 { format!("{:04x}    ", l.raw) } else { format!("{:08x}", l.raw) };
            let lbl = if pc != addr && an.labels.contains(&pc) { format!("{}:\n", an.label(pc)) } else { String::new() };
            items.insert(pc, format!("{lbl}    {pc:08x}: {raw}  {text}"));
        }
        for &la in &f.literals {
            if items.contains_key(&la) {
                continue;
            }
            if let Some(v) = rom.addr_to_off(la).and_then(|o| rom.u32(o)) {
                items.insert(la, format!("    {la:08x}: .word 0x{v:08x}  @ {}", annotate_word(an, rom, v)));
            }
        }
        for line in items.values() {
            writeln!(out, "{line}").unwrap();
        }
    }
    out
}

/// Étiquettes déduites du matériel touché par la fonction (via ses littéraux).
pub fn tags(rom: &Rom, f: &Function) -> Vec<&'static str> {
    let mut t = BTreeSet::new();
    for &la in &f.literals {
        let Some(v) = rom.addr_to_off(la).and_then(|o| rom.u32(o)) else { continue };
        let tag = match v {
            0x0400_0130 | 0x0400_0132 => "touches",
            0x0400_00B0..=0x0400_00DF => "dma",
            0x0400_0000..=0x0400_005F => "vidéo",
            0x0400_0060..=0x0400_00AF => "son",
            0x0400_0100..=0x0400_010F => "timer",
            0x0400_0200..=0x0400_020B | 0x0300_7FFC => "irq",
            _ => match v >> 24 {
                0x05 => "palette",
                0x06 => "vram",
                0x07 => "oam",
                0x0E => "sauvegarde",
                _ => continue,
            },
        };
        t.insert(tag);
    }
    if f.lines.values().any(|l| l.text.starts_with("swi")) {
        t.insert("bios");
    }
    t.into_iter().collect()
}

pub fn write_function_list(an: &Analysis, rom: &Rom) -> String {
    let callers = an.callers();
    let mut out = String::from("adresse     mode   octets  appels  appelants  origine    nom / étiquettes\n");
    for (&a, f) in &an.funcs {
        writeln!(
            out,
            "{a:08X}    {:<5}  {:>6}  {:>6}  {:>9}  {:<9}  {} {}",
            if f.mode == Mode::Thumb { "thumb" } else { "arm" },
            f.code_bytes(),
            f.calls.len(),
            callers.get(&a).copied().unwrap_or(0),
            format!("{:?}", f.origin),
            an.names.get(&a).map(String::as_str).unwrap_or("-"),
            tags(rom, f).join(",")
        )
        .unwrap();
    }
    out
}
