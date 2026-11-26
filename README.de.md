# Lady Sia — PC-Portierung in Rust

[English](README.md) · [Français](README.fr.md) · **Deutsch** · [Español](README.es.md) · [Italiano](README.it.md) · [Nederlands](README.nl.md)

Analysewerkzeuge und eine PC-Portierung (Windows) des Game-Boy-Advance-Spiels
*Lady Sia*, europäische Version. Der Code des Spiels wird aus dem Modul nach Rust
rekompiliert; das Ergebnis ist eine eigenständige ausführbare Datei, ohne Emulator.

**Dieses Repository enthält weder das ROM noch dessen Daten noch irgendetwas daraus
Extrahiertes.** Du musst das Modul besitzen und deine eigene Kopie erstellen. Alles,
was aus dem Modul stammt (Disassembly, rekompilierter Code, Grafiken, Level, Sounds),
wird auf deinem Rechner erzeugt und von Git ignoriert (siehe `.gitignore`).

## Spiel bauen

Benötigt werden Rust (Edition 2024) unter Windows und das europäische ROM
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 MB).

```
cargo build --release
```

Dieser erste Build liefert die Werkzeuge (der rekompilierte Code existiert noch nicht;
`build.rs` schreibt an seiner Stelle ein leeres Modul).

```
target/release/ladysia-recomp <rom.gba>
```

Analysiert das ROM und schreibt das kommentierte Disassembly nach `asm/ladysia.s`
(plus einige Berichte). Die Funktionsnamen stammen aus `symbols.txt`.

```
target/release/ladysia-recomp <rom.gba> recomp
```

Übersetzt das Disassembly nach Rust in `src/recomp/code/`. Der Befehl schlägt fehl,
wenn ein Sprungziel im übersetzten Code nicht selbst übersetzt ist.

```
cargo build --release
target/release/ladysia-recomp <rom.gba> donnees ladysia.lsd
```

Baut erneut mit dem Spielcode und schreibt dann die Spieldaten: das ROM, in dem alle
Instruktionen und der Header gelöscht sind, komprimiert.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Erzeugt die fertige Datei `target/release/ladysia-recomp.exe` (beliebig umbenennen),
die den Rust-Code und die Daten enthält. Spielstände und Einstellungen werden daneben
gespeichert.

Während der Entwicklung startet `ladysia-recomp <rom.gba> play` das Spiel aus dem ROM,
und `ladysia-recomp <rom.gba> view` öffnet den Level-Betrachter.

## Spielen

| Taste | Aktion |
| --- | --- |
| Pfeiltasten | Steuerkreuz |
| X, Leertaste, K oder A | A |
| W, Z oder J | B |
| Q / S | L / R |
| Eingabe | Start |
| Rücktaste | Select |
| Esc | Menü: Anzeige, Ton, Steuerung, Mods |
| Tab | Schneller Vorlauf |
| R | Zurückspulen |
| F5 / F7 / F6 | Zustand speichern / laden / nächster Platz |
| F11 | Vollbild |
| P | Pause |

Tastatur und Xbox-Controller lassen sich im Menü belegen. Das Menü ist auf Französisch.

## Mods

Esc-Menü › **MODS**:

- unendliche Gesundheit, unendliche Leben, plötzlicher Tod (ein Treffer), Zurückspulen;
- Spielgeschwindigkeit von 50 bis 200 %;
- hoher oder sehr hoher Sprung, „Mond“- oder „Feder“-Schwerkraft, schnelles Laufen,
  freies Fliegen (hoch und runter steuern Sia in der Luft);
- winzige, große oder riesige Sia, Kreiseln, durchsichtig oder in anderer Palette;
- Spiegelwelt, Farbstimmungen (Game Boy, Schwarzweiß, Nacht, Negativ, Sepia);
- direkter Start in einem der 36 Level.

Bewegungs- und Aussehens-Mods wirken auf Sia zu Fuß.

## Aufbau

- `src/gba/`: ARM7TDMI-Prozessor, Bus, Grafik und Ton der Konsole;
- `src/recomp_gen.rs`, `src/recomp/`: statische Rekompilierung des Spielcodes;
- `src/jeu/`: von Hand in lesbares Rust umgeschriebene Spielfunktionen;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs`: Fenster, Menü, Mods,
  Bildfilter;
- der Rest: Analyse und Extraktion (Disassembler, Dekompressoren, Level, Sounds);
- `NOTES.md`: ausführliches Reverse-Engineering-Protokoll (auf Französisch).

*Lady Sia* ist eine Marke ihrer jeweiligen Inhaber. Dieses Projekt steht mit keinem
von ihnen in Verbindung.
