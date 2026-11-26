# Lady Sia — port per PC in Rust

[English](README.md) · [Français](README.fr.md) · [Deutsch](README.de.md) · [Español](README.es.md) · **Italiano** · [Nederlands](README.nl.md)

Strumenti di analisi e port per PC (Windows) del gioco per Game Boy Advance
*Lady Sia*, versione europea. Il codice del gioco viene ricompilato in Rust a partire
dalla cartuccia; il risultato è un eseguibile autonomo, senza emulatore da installare.

**Questo repository non contiene la ROM, né i suoi dati, né nulla di estratto da essa.**
Bisogna possedere la cartuccia e farne una propria copia. Tutto ciò che proviene dalla
cartuccia (disassemblato, codice ricompilato, grafica, livelli, suoni) viene generato
sul tuo computer e ignorato da Git (vedi `.gitignore`).

## Compilare il gioco

Servono Rust (edizione 2024) su Windows e la ROM europea
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 MB).

```
cargo build --release
```

Questa prima compilazione produce gli strumenti (il codice ricompilato non esiste
ancora; `build.rs` scrive al suo posto un modulo vuoto).

```
target/release/ladysia-decomp <rom.gba>
```

Analizza la ROM e scrive il disassemblato commentato in `asm/ladysia.s` (più alcuni
rapporti). I nomi delle funzioni vengono da `symbols.txt`.

```
target/release/ladysia-decomp <rom.gba> recomp
```

Traduce il disassemblato in Rust in `src/recomp/code/`. Il comando fallisce se una
destinazione di salto del codice tradotto non è a sua volta tradotta.

```
cargo build --release
target/release/ladysia-decomp <rom.gba> donnees ladysia.lsd
```

Ricompila con il codice del gioco, poi scrive i dati del gioco: la ROM con tutte le
istruzioni e l'intestazione cancellate, compressa.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Produce l'eseguibile finale `target/release/ladysia-decomp.exe` (rinominalo a piacere),
che contiene il codice Rust e i dati. Salvataggi e impostazioni vengono scritti accanto.

Durante lo sviluppo, `ladysia-decomp <rom.gba> play` avvia il gioco dalla ROM e
`ladysia-decomp <rom.gba> view` apre il visualizzatore di livelli.

## Giocare

| Tasto | Azione |
| --- | --- |
| Frecce | Croce direzionale |
| X, Spazio, K o A | A |
| W, Z o J | B |
| Q / S | L / R |
| Invio | Start |
| Backspace | Select |
| Esc | Menu: immagine, audio, comandi, mod |
| Tab | Avanti veloce |
| R | Riavvolgi |
| F5 / F7 / F6 | Salva stato / carica / slot successivo |
| F11 | Schermo intero |
| P | Pausa |

Tasti e pulsanti di un controller Xbox si configurano nel menu. Il menu è in francese.

## Mod

Menu Esc › **MODS**:

- salute infinita, vite infinite, morte improvvisa (un solo colpo), riavvolgimento;
- velocità di gioco da 50 a 200 %;
- salto alto o altissimo, gravità «luna» o «piuma», corsa veloce, volo libero
  (su e giù guidano Sia in aria);
- Sia minuscola, grande o gigante, trottola, trasparente o con un'altra palette;
- mondo specchio, atmosfere di colore (Game Boy, bianco e nero, notte, negativo, seppia);
- partenza diretta in uno qualsiasi dei 36 livelli.

Le mod di movimento e di aspetto agiscono su Sia a piedi.

## Struttura

- `src/gba/`: processore ARM7TDMI, bus, video e audio della console;
- `src/recomp_gen.rs`, `src/recomp/`: ricompilazione statica del codice del gioco;
- `src/jeu/`: funzioni del gioco riscritte a mano in Rust leggibile;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs`: finestra, menu, mod,
  filtri d'immagine;
- il resto: analisi ed estrazione (disassemblatore, decompressori, livelli, suoni);
- `NOTES.md`: diario dettagliato del reverse engineering (in francese).

*Lady Sia* è un marchio dei rispettivi titolari. Questo progetto non è affiliato a
nessuno di essi.
