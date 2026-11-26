# Lady Sia — portage PC en Rust

[English](README.md) · **Français** · [Deutsch](README.de.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Nederlands](README.nl.md)

Outils d'analyse et portage PC (Windows) du jeu Game Boy Advance *Lady Sia*, version
européenne. Le code du jeu est recompilé en Rust à partir de la cartouche ; le résultat
est un exécutable autonome, sans émulateur à installer.

**Ce dépôt ne contient ni la ROM, ni ses données, ni rien qui en soit extrait.** Il faut
posséder la cartouche et en faire sa propre copie. Tout ce qui vient de la cartouche
(désassemblage, code recompilé, graphismes, niveaux, sons) est généré sur votre machine
et ignoré par Git (voir `.gitignore`).

## Construire le jeu

Il faut Rust (édition 2024) sous Windows, et la ROM européenne
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 Mo).

```
cargo build --release
```

Cette première compilation donne les outils (le code recompilé n'existe pas encore, un
module vide est écrit à sa place par `build.rs`).

```
target/release/ladysia-decomp <rom.gba>
```

Analyse la ROM et écrit le désassemblage annoté dans `asm/ladysia.s` (plus quelques
rapports). Les noms de fonctions viennent de `symbols.txt`.

```
target/release/ladysia-decomp <rom.gba> recomp
```

Traduit le désassemblage en Rust dans `src/recomp/code/`. La commande échoue si une
cible de saut du code traduit n'est pas elle-même traduite.

```
cargo build --release
target/release/ladysia-decomp <rom.gba> donnees ladysia.lsd
```

Recompile avec le code du jeu, puis écrit les données du jeu : la ROM dont toutes les
instructions et l'en-tête sont effacés, compressée.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Donne l'exécutable final `target/release/ladysia-decomp.exe` (à renommer), qui contient
le code Rust et les données. La sauvegarde et les réglages s'écrivent à côté de lui.

Pendant le développement, `ladysia-decomp <rom.gba> play` lance le jeu sur la ROM, et
`ladysia-decomp <rom.gba> view` ouvre la visionneuse de niveaux.

## Jouer

| Touche | Action |
| --- | --- |
| Flèches | Direction |
| X, Espace, K ou A | A |
| W, Z ou J | B |
| Q / S | L / R |
| Entrée | Start |
| Retour arrière | Select |
| Échap | Menu : affichage, son, commandes, mods |
| Tab | Accéléré |
| R | Rembobinage |
| F5 / F7 / F6 | Sauvegarde instantanée / chargement / emplacement suivant |
| F11 | Plein écran |
| P | Pause |

Les touches du clavier et les boutons d'une manette Xbox se règlent dans le menu.

## Mods

Menu Échap › **MODS** :

- santé infinie, vies infinies, mort subite (un seul coup), rembobinage ;
- vitesse du jeu de 50 à 200 % ;
- saut haut ou très haut, gravité « lune » ou « plume », course rapide, vol libre
  (haut et bas pilotent Sia en l'air) ;
- Sia miniature, grande ou géante, toupie, transparente, ou dans une autre palette ;
- monde miroir, ambiances de couleurs (Game Boy, noir et blanc, nuit, négatif, sépia) ;
- départ direct dans l'un des 36 niveaux.

Les mods de mouvement et d'apparence agissent sur Sia à pied.

## Organisation

- `src/gba/` : processeur ARM7TDMI, bus, vidéo et son de la console ;
- `src/recomp_gen.rs`, `src/recomp/` : recompilation statique du code du jeu ;
- `src/jeu/` : fonctions du jeu réécrites à la main en Rust lisible ;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs` : fenêtre, menu, mods,
  filtres d'image ;
- le reste : analyse et extraction (désassembleur, décompresseurs, niveaux, sons) ;
- `NOTES.md` : journal détaillé de la rétro-ingénierie.

*Lady Sia* est une marque de ses ayants droit. Ce projet n'est affilié à aucun d'eux.
