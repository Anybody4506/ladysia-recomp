# Lady Sia — pc-port in Rust

[English](README.md) · [Français](README.fr.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Italiano](README.it.md) · **Nederlands**

Analysegereedschap en een pc-port (Windows) van het Game Boy Advance-spel *Lady Sia*,
Europese versie. De code van het spel wordt vanuit de cartridge naar Rust
gehercompileerd; het resultaat is een zelfstandig uitvoerbaar bestand, zonder emulator.

**Deze repository bevat niet de ROM, niet de gegevens ervan en niets wat eruit is
gehaald.** Je moet de cartridge bezitten en er zelf een kopie van maken. Alles wat uit
de cartridge komt (disassembly, gehercompileerde code, graphics, levels, geluiden) wordt
op je eigen computer gegenereerd en door Git genegeerd (zie `.gitignore`).

## Het spel bouwen

Je hebt Rust (editie 2024) op Windows nodig en de Europese ROM
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 MB).

```
cargo build --release
```

Deze eerste build levert het gereedschap (de gehercompileerde code bestaat nog niet;
`build.rs` schrijft er een lege module voor in de plaats).

```
target/release/ladysia-decomp <rom.gba>
```

Analyseert de ROM en schrijft de geannoteerde disassembly naar `asm/ladysia.s` (plus
enkele rapporten). De functienamen komen uit `symbols.txt`.

```
target/release/ladysia-decomp <rom.gba> recomp
```

Vertaalt de disassembly naar Rust in `src/recomp/code/`. Het commando mislukt als een
sprongdoel in de vertaalde code zelf niet vertaald is.

```
cargo build --release
target/release/ladysia-decomp <rom.gba> donnees ladysia.lsd
```

Bouwt opnieuw met de spelcode en schrijft daarna de spelgegevens: de ROM waarin alle
instructies en de header zijn gewist, gecomprimeerd.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Levert het uiteindelijke bestand `target/release/ladysia-decomp.exe` (hernoem het naar
wens), met de Rust-code en de gegevens. Opslagbestanden en instellingen komen ernaast.

Tijdens het ontwikkelen start `ladysia-decomp <rom.gba> play` het spel vanaf de ROM, en
opent `ladysia-decomp <rom.gba> view` de levelviewer.

## Spelen

| Toets | Actie |
| --- | --- |
| Pijltjes | Richtingspad |
| X, Spatie, K of A | A |
| W, Z of J | B |
| Q / S | L / R |
| Enter | Start |
| Backspace | Select |
| Esc | Menu: beeld, geluid, besturing, mods |
| Tab | Vooruitspoelen |
| R | Terugspoelen |
| F5 / F7 / F6 | Staat opslaan / laden / volgend vak |
| F11 | Volledig scherm |
| P | Pauze |

Toetsen en knoppen van een Xbox-controller stel je in via het menu. Het menu is in het
Frans.

## Mods

Esc-menu › **MODS**:

- oneindige gezondheid, oneindige levens, plotselinge dood (één treffer), terugspoelen;
- spelsnelheid van 50 tot 200 %;
- hoge of zeer hoge sprong, „maan”- of „veer”-zwaartekracht, snel rennen, vrij vliegen
  (omhoog en omlaag sturen Sia in de lucht);
- piepkleine, grote of reusachtige Sia, tol, doorzichtig of in een ander palet;
- spiegelwereld, kleursferen (Game Boy, zwart-wit, nacht, negatief, sepia);
- direct starten in een van de 36 levels.

Bewegings- en uiterlijk-mods werken op Sia te voet.

## Indeling

- `src/gba/`: ARM7TDMI-processor, bus, beeld en geluid van de console;
- `src/recomp_gen.rs`, `src/recomp/`: statische hercompilatie van de spelcode;
- `src/jeu/`: spelfuncties met de hand herschreven in leesbare Rust;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs`: venster, menu, mods,
  beeldfilters;
- de rest: analyse en extractie (disassembler, decompressors, levels, geluiden);
- `NOTES.md`: uitgebreid logboek van de reverse engineering (in het Frans).

*Lady Sia* is een handelsmerk van de respectieve eigenaren. Dit project is met geen van
hen verbonden.
