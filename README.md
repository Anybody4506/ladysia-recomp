# Lady Sia — PC port in Rust

**English** · [Français](README.fr.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Nederlands](README.nl.md)

Analysis tools and a PC (Windows) port of the Game Boy Advance game *Lady Sia*,
European release. The game's code is recompiled to Rust from the cartridge; the result
is a standalone executable, with no emulator to install.

**This repository contains neither the ROM, nor its data, nor anything extracted from
it.** You need to own the cartridge and make your own copy. Everything that comes from
the cartridge (disassembly, recompiled code, graphics, levels, sounds) is generated on
your machine and ignored by Git (see `.gitignore`).

## Building the game

You need Rust (2024 edition) on Windows, and the European ROM
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 MB).

```
cargo build --release
```

This first build produces the tools (the recompiled code does not exist yet; an empty
module is written in its place by `build.rs`).

```
target/release/ladysia-decomp <rom.gba>
```

Analyses the ROM and writes the annotated disassembly to `asm/ladysia.s` (plus a few
reports). Function names come from `symbols.txt`.

```
target/release/ladysia-decomp <rom.gba> recomp
```

Translates the disassembly to Rust in `src/recomp/code/`. The command fails if a branch
target in the translated code is not itself translated.

```
cargo build --release
target/release/ladysia-decomp <rom.gba> donnees ladysia.lsd
```

Rebuilds with the game's code, then writes the game data: the ROM with every
instruction and the header erased, compressed.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Produces the final executable `target/release/ladysia-decomp.exe` (rename it as you
like), containing the Rust code and the data. Saves and settings are written next to it.

During development, `ladysia-decomp <rom.gba> play` runs the game from the ROM, and
`ladysia-decomp <rom.gba> view` opens the level viewer.

## Playing

| Key | Action |
| --- | --- |
| Arrows | D-pad |
| X, Space, K or A | A |
| W, Z or J | B |
| Q / S | L / R |
| Enter | Start |
| Backspace | Select |
| Esc | Menu: display, sound, controls, mods |
| Tab | Fast forward |
| R | Rewind |
| F5 / F7 / F6 | Save state / load state / next slot |
| F11 | Fullscreen |
| P | Pause |

Keyboard keys and Xbox controller buttons can be remapped in the menu. The in-game
menu is in French.

## Mods

Esc menu › **MODS**:

- infinite health, infinite lives, sudden death (one hit), rewind;
- game speed from 50 to 200 %;
- high or very high jump, "moon" or "feather" gravity, fast run, free flight
  (up and down steer Sia in the air);
- tiny, tall or giant Sia, spinning, transparent, or in another palette;
- mirror world, colour looks (Game Boy, black and white, night, negative, sepia);
- start directly in any of the 36 levels.

Movement and appearance mods apply to Sia on foot.

## Layout

- `src/gba/`: the console's ARM7TDMI processor, bus, video and sound;
- `src/recomp_gen.rs`, `src/recomp/`: static recompilation of the game's code;
- `src/jeu/`: game functions rewritten by hand in readable Rust;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs`: window, menu, mods,
  image filters;
- the rest: analysis and extraction (disassembler, decompressors, levels, sounds);
- `NOTES.md`: detailed reverse-engineering log (in French).

*Lady Sia* is a trademark of its respective owners. This project is not affiliated with
any of them.
