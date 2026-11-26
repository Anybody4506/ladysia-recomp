# Notes de rétro-ingénierie — Lady Sia (GBA, Europe, ALDP)

Les noms de fonctions sont dans `symbols.txt` (relu à chaque lancement de l'outil).

## Organisation de la ROM

| Zone | Contenu |
|---|---|
| `0x08000000`–`0x08037B36` | Code (Thumb, compilé avec GCC) |
| `0x08037B3C`–… | Descripteurs (niveaux / ressources ?) pointant vers `0x0825xxxx` |
| `0x08040000`–`0x080A0000` | Tables : définitions d'objets passées à `Entity_Alloc` |
| `0x0825xxxx` | Répertoire des ressources compressées |
| `0x08260000`–`0x08350000` | 397 ressources compressées (format maison, voir plus bas) |
| `0x0835182C` | Table des 15 gestionnaires d'interruptions |
| `0x083538A8` / `0x083538C0` | Tables du texte : 6 langues |

## Démarrage

1. `crt0_Start` (ARM) : piles IRQ et système, puis `bx` vers `AgbMain` (Thumb).
2. `AgbMain` : `REG_WAITCNT = 0x4014`, `InitIntrTable(IntrHandlerTable)`, active les interruptions, puis boucle de jeu.
3. `InitIntrTable` copie `IntrMain` en IWRAM (`0x030009C0`) et met son adresse dans `INTR_VECTOR`.
   - `IntrHandler_00` (VBlank) = `0x08001A34`
   - `IntrHandler_04` (Timer1) = `0x080019A0`, sans doute le mixage du son maison
   - les autres pointent vers un gestionnaire vide

## Variables globales repérées

| Adresse | Rôle |
|---|---|
| `0x03000910` | `IntrTable` (15 pointeurs) |
| `0x0300094C` | touches nouvellement appuyées (`~KEYINPUT & ~précédent`) |
| `0x03001B54` | langue courante (0–5) |
| `0x03002FB0` | compteur d'entités ; pool de 64 entités × `0x78` octets |

## Graphismes : RLE maison (résolu, étape 4)

Chaîne d'appel : `Decompress(dest, src)` (Thumb) → `Decomp_Entry_veneer` (`bx pc`) →
`Decomp_Entry` (ARM, `0x08000248`) → `Decomp_Core` (ARM `0x08000290`, copié en IWRAM
`0x03006CD0` par `Decomp_Init`). Appelé par l'affichage des sprites (`sub_08029CD8`).

```
u32  taille décompressée
u8   méthode : bits 0-3 = m (0 = brut, 1..4 = RLE), bits hauts = motif du marqueur
...  flux : octet dont les m bits hauts == motif -> n = bits bas, octet suivant écrit n+1 fois
            (n = 0 : échappement, littéral) ; sinon l'octet est copié tel quel
```

Masques lus en ROM à `0x0800023C`. Réécrit dans `src/rle.rs` (tests inclus).
`ladysia-decomp <rom> extract` sort 2 723 flux (1,5 Mo -> 2,8 Mo) dans `extract/rle/`
en `.bin` + `.png` (tuiles 4 bpp, palette grise provisoire). Vérification : 99 % des
fichiers ont une cohérence entre pixels voisins > 30 % (médiane 74 %, aléatoire = 6 %).
11 fichiers sous 15 % : probablement des faux positifs du balayage.

## Bloc `0x08260000` : LZ + Huffman maison (résolu, étape 5)

397 ressources `[u32 taille décompressée][u32 taille compressée][flux]`, référencées
par le répertoire `0x0825xxxx`. Fonctions : `LoadPacked` (`0x0802EE7C`) →
`Packed_Decode` (`0x0802E070`), `Packed_BuildTables` (`0x0802E6E0`),
`Packed_BuildHuffman` (`0x0802EA2C`), `Packed_ReadSymbol` (`0x0802EB24`),
`Packed_Predict` (`0x0802EC40`), `Packed_Finish` (`0x0802EF64`).

Format (proche de LZX), réécrit dans `src/packed.rs` :
- flux de bits lu octet de poids fort d'abord (fenêtre de 24 bits) ;
- blocs : 2 bits de drapeaux (bit 15 = mode son, bit 14 = garder les longueurs du bloc
  précédent), 19 longueurs sur 4 bits pour le code des longueurs, puis les longueurs
  des symboles **en différence avec le bloc précédent** (codes 16/17/18 = répétitions) ;
- Huffman canonique, longueurs 1 à 15, seuils alignés sur 16 bits ;
- **mode données** : 3 arbres (298 / 48 / 28 symboles). 0-255 littéral, 256 = répéter
  la dernière copie, 257-260 = une des 4 dernières distances (+ arbre des longueurs),
  261-268 = copie de 2 octets à petite distance, 269 = nouveau bloc, 270+ = longueur et
  distance explicites ; +1 à la longueur si distance > 0x1FFF, +1 si > 0x3FFFF
  (et si > 0x100 pour les distances récentes). Tables de bases/bits en ROM
  `0x083545A4`–`0x083546DC` ;
- **mode son** (géré par le décodeur mais **inutilisé par les 397 ressources** du bloc) : 1 à 4 canaux entrelacés, un arbre de 257 symboles par canal
  (256 = nouveau bloc) ; chaque symbole est un résidu passé dans un prédicteur
  adaptatif à 5 poids (historique, différence, échantillon de l'autre canal), qui ajuste
  un poids tous les 32 échantillons selon l'erreur cumulée la plus faible.

Validation :
- `src/emu.rs` (émulateur Thumb minimal) exécute la vraie routine du jeu : 397/397
  décodées, flux consommé exactement pour 396/397 (exception `0x08320B23`, faux
  pointeur probable du répertoire) ;
- **le décodeur natif donne une sortie identique à l'émulateur pour 397/397 ressources**
  (vérifié à chaque `extract`).

Sorties dans `extract/unpacked/`.

## Contenu des 397 ressources (étape 6)

Aucune ressource n'est en mode son : **le bloc ne contient que des décors**.
Le son est ailleurs (pilote `sub_08031A34`, Timer1 rechargé à `16777216 / fréquence`,
fréquence = u16 en +0 d'un descripteur ; tables candidates `0x08034DB4`–`0x08035760`).

Répertoire `0x0825xxxx` : 117 enregistrements de 7 mots
`[tuiles, carte, attributs, extra|0, (h << 16) | w, nb_tuiles, 6]`.
- tuiles : 4 bpp, `nb_tuiles × 32` octets ;
- carte (lue par `sub_08029310`) : h + 1 mots d'en-tête = nombre de mots de chaque ligne,
  puis par ligne : mot avec bit 15 = n entrées d'écran GBA qui suivent (tuile 10 bits,
  miroirs bits 10/11, palette bits 12-15), sinon n cases vides. Vérifié sur 115/115 cartes ;
- attributs : 12 bits par tuile (`nb_tuiles × 1,5` octets) ;
- un enregistrement (`0x0825A1F4`) a une disposition différente (nb_tuiles aberrant).

51 ressources hors répertoire (référencées ailleurs) : images plein écran en tuiles
**8 bpp** + cartes de 1 282 octets (écran 30×20), choisies par cohérence des pixels.

Bilan (`extract/catalogue.txt`) : 110 jeux de tuiles (PNG dans `extract/decors/`, palette
grise), 121 cartes, 35 attributs, 11 extra, 20 inconnues, 100 vides.

## Scènes, palettes et rendu des niveaux (étape 7)

Scène (argument de `sub_0802C974`, table `0x08351868` pour les écrans de base) :
`+0x00` liste des couches (entrées de 0x20 octets, 1er mot = enregistrement de décor),
`+0x08` **palette** compressée au format packed (1 024 octets = 256 couleurs BG + 256 OBJ,
BGR555, bit 15 à ignorer), `+0x10` (u16) nombre de couches (1 à 4).

`src/levels.rs` repère 51 scènes et 51 palettes, et rend 109 des 117 décors en couleur dans
`extract/niveaux/` (une image par couche). Les 8 décors restants ne sont référencés par
aucune scène trouvée par le balayage.

Entrée de couche (0x20 octets) : `+0x00` décor, `+0x10` / `+0x12` facteurs de défilement
X / Y (8.8, `0x100` = suit la caméra, 0 = fixe), `+0x14` drapeaux (bits 0-1 = priorité BG).

Superposition (`compose` dans `src/levels.rs`) : panorama à la taille de la plus grande
couche ; chaque couche est lue en X × f / f_ref (répétée si plus petite), dans l'ordre du
matériel (priorité 3 -> 0, puis BG de plus petit numéro devant). 36 scènes à plusieurs
couches -> `extract/niveaux/scene_XXXXXXXX_complete.png`. C'est une approximation : le
décalage initial (`+0x16`) et la position exacte de la caméra ne sont pas pris en compte.

## Son (étape 8)

Pilote maison (≈ `0x0802F51C`–`0x08036300`, état IWRAM `0x03006ED8`, mixage sur Timer1).
Démarrage d'une musique : `sub_08003A20(n°)` -> `sub_08033DB4`.

Banque à `0x083546DC` (offsets relatifs à la base) :
- `+0x18` -> `0x08355688` : 12 musiques `[n][offsets]` (`0x083556BC`–`0x08375838`) ;
  chaque morceau : 0x400 octets de table, puis `+0x410` / `+0x414`, et `+0x418` = tempo
  (120 à 160 BPM) ; le tempo sert au calcul `… / 3600` de `sub_08033DB4` ;
- `+0x1C` -> `0x08375838` : 78 échantillons (`0x08376394`–`0x083DB2D0`, fin confirmée par
  un littéral du code). En-tête de 16 octets `[longueur][début de boucle, -1 = aucune]
  [u16 fréquence 11 397 Hz][u16 ?][0]`, puis PCM 8 bits signé (signe vérifié : 78/78).

`src/sound.rs` exporte les échantillons dans `extract/son/` (WAV + `index.txt`).
`0x083DD000`–`0x083F5000` : texte (6 langues) ; ensuite la ROM est vide (`0xFF`).

## Musiques (étape 9)

Format d'un morceau (voir l'en-tête de `src/music.rs`) : tables de programmes
(`+0x10`, batterie canal 9 à `+0x210`), 17 pistes `[délai][motif]` (-1 fin, -2 boucle),
motifs d'événements style MIDI (`[delta][vélocité][note|0x80][instrument][durée]`),
96 ticks par noire. Séquenceur : `sub_08034544` ; note : `sub_08034360` ; instrument =
petit programme en bytecode (`B + [B+0x20+4n]`, octet 0 = canal PSG 0-3 ou DirectSound).

Rendu par **émulation du vrai pilote** (`src/music.rs`) :
- `AgbMain` exécuté jusqu'à `0x080003D0` (fin de l'init son) ; l'EEPROM répond « prête » ;
- `sub_08003A20(n°)`, puis VBlank (`sub_0803142C`) à 59,73 Hz et IRQ Timer1
  (`sub_08031F64`) toutes les 128 périodes d'échantillon (Timer1 = 2 944 × 64 cycles) ;
- le mixeur est du **code ARM** copié en IWRAM (`0x08034DB4`–`0x08035760`) : l'émulateur
  a maintenant un interpréteur ARM ;
- sortie DirectSound : tampon en anneau 3 × 128 octets à `0x030016B0`, 11 397 Hz,
  envoyé aux FIFO A et B ;
- 8 morceaux sur 12 utilisent aussi les canaux PSG (carrés, onde, bruit) : écritures
  horodatées dans `0x04000060`–`0x0400009F`, synthétisées par `src/psg.rs` et mélangées
  selon `SOUNDCNT_L/H`.

`ladysia-decomp <rom> music` -> `extract/musique/musique_NN.wav` (16 bits, 22 795 Hz ;
un passage + jusqu'à 20 s de reprise avec fondu). Vérifié : pas de saturation, tempo et
notes visibles au spectrogramme. Pas encore écouté sur une vraie console pour comparer ;
la synthèse PSG est une approximation (pas de balayage de fréquence du canal 1).

## Objets placés et sprites (étape 10, `src/objects.rs`)

Table des niveaux à `0x08351868` (lue par `AgbMain` vers 0x08000420) : entrées de 12 octets
`[scène][n° de musique][?]`, indexées par le n° d'état en `0x03000B50`. Les états 16 à 51 sont
les niveaux jouables (le joueur est créé à 0x08000C54 avec la définition `0x0805B304`).

Chaque entrée de couche (0x20 octets) contient sa liste d'objets : `+0x04` pointeur,
`+0x0C` nombre. Objet = 0x14 octets `[définition][x | y << 16][3 paramètres]`, x/y en pixels
de la couche (l'entité les garde en 24.8 dans `+0x48/+0x4C`).

Définition : `+0x00` animations (`[liste d'images][nombre]`), `+0x04` banque graphique
« packed » (adresse parfois impaire, `ReadU32Unaligned`) chargée en RAM si `+0x0E & 2`,
`+0x0A` n° de type, `+0x0D` nombre d'animations. Image d'animation (0x14 octets) :
`+0x00` image, `+0x0D/+0x0E` décalage x/y, `+0x10` miroirs. Image : `+0x00` graphismes
(pointeur ROM, ou décalage dans la banque), `+0x04` pièces, `+0x12` nombre de pièces ;
graphismes = flux RLE contenant les tuiles 4 bits de toutes les pièces à la suite
(OBJ en mode 1D). Pièce : x, y (s16), largeur, hauteur (u8), taille en octets (u16).
Le tracé est fait par `sub_08029CD8` (DMA vers la VRAM OBJ allouée par `sub_0802A580`).

Palette : `sub_0802C824` lit le quartet n° `type` de la table pointée par scène `+0x0C`,
qui donne la banque de 16 couleurs dans la moitié OBJ de la palette de la scène.

Résultat : 848 objets dans 34 scènes (165 définitions différentes), tous dessinés avec la
1re image de leur 1re animation sur `extract/niveaux/scene_*_objets.png` ; liste complète
dans `extract/niveaux/objets.txt`. Vérifié à l'œil : gemmes, crochets, ennemis posés sur les
planches. Paramètres des objets (sens, trajets, n° d'interrupteur…) non interprétés.

## Planches de sprites (étape 11, `objects::sheet` / `export_sheets`)

- Table des animations d'une définition : `n` = octet `+0x0D`, entrée de 8 octets
  `[liste d'images][nombre d'images (u16)]`. Image d'animation : 0x14 octets, durée en `+0x0C`.
- Une planche par définition (`extract/sprites/def_XXXXXXXX_typeNNN.png`, PNG indexé 16
  couleurs, couleur 0 transparente) : une ligne par animation ; dans une ligne, cases de même
  taille avec le point d'ancrage au même endroit. `index.txt` liste animations et durées.
- 166 définitions (les 165 placées + le joueur 0x0805B304), 1 693 images ; 2 images illisibles
  (dernière image d'animation sans image : marqueur de fin probable).
- Palette : celle de la première scène où l'objet apparaît. Le joueur ne passe pas par la
  table de types : il utilise la banque OBJ 0 (essai des 16 banques), prise dans la palette
  la plus courante parmi les scènes.

## Paramètres des objets (étape 12, `src/behaviour.rs`)

- Création des objets d'une couche : `sub_080299E8` (0x08029B64). L'enregistrement de 20 octets
  est recopié tel quel en entité `+0x08..+0x1B` ; paramètres en `+0x10`, `+0x14`, `+0x18`.
- Comportement : table `0x083DB2D0` indexée par le n° de type, rangé en entité `+0x28`
  (0 = pas de comportement : 52 des 165 types placés, décors et objets passifs).
- Code générique : param 3 bit 8 -> entité `+0x58` |= 4 = miroir H, bit 9 -> `+0x58` |= 8 =
  miroir V (XOR avec le miroir de l'image dans `sub_08029CD8`). Appliqué aux `scene_*_objets.png`.
- Disposition commune des champs (toutes les fonctions suivent la même convention) :
  `+0x12` (u16) mode / variante ; `+0x14` délai en images (0 => défaut, souvent 60) ;
  `+0x15` / `+0x16` / `+0x17` / `+0x18` distances haut / gauche / bas / droite en tuiles de 8 px
  (lues puis `lsl #11` et ajoutées à la position de départ `+0x40`/`+0x44`) ; `+0x19` options
  (bits 0x20/0x40/0x80 selon le type).
- `behaviour::analyse` suit le code depuis la fonction du type (flot, tables de `switch`,
  appels avec r0 = entité, fonctions d'état rangées en `+0x28`/`+0x2C`) et relève les champs
  lus, leur largeur, s'ils servent de distance, leur valeur par défaut, les bits testés et
  les constantes comparées. Un champ écrit par la fonction d'initialisation avant d'être lu
  est une variable de l'objet : la valeur du niveau est ignorée (ex. type 420 : `+0x12` forcé à 2).
- Résultat : 113 types avec comportement, 71 lisent des paramètres -> `extract/niveaux/types.txt` ;
  chaque ligne de `objets.txt` porte les valeurs décodées et les octets non nuls que le code
  ne lit pas (« non lus »).
- Non résolu : `+0x10` (u16 bas du param 1 : 0x0A, 0x28, 0x2D, 0x32, 0x40), présent surtout
  sur des types sans comportement ; aucune lecture trouvée, peut-être lu par un autre objet
  (le joueur ?) ou inutilisé.

## Sens des modes (étape 13, `behaviour::mode_meaning`)

Lu à la main dans le code, pour les comportements les plus fréquents (138 objets annotés) :

| fonction | types | mode | preuve |
|---|---|---|---|
| 08007CC4 | lieux de la carte (375-395, 508-553) | n° du lieu ; 0 = décor | écrit x / y−`haut`×8 dans `0x03000080 + 8n` |
| 08007C74 | flèche de lieu (378) | n° du lieu | animation 1 si bit n−1 de `0x030000BC` (lieu ouvert) |
| 0800C128 | soldat-rat (421), à bouclier (422) | impair : `sub_0800D528` ; 22 : `sub_0800D57C` ; 10-19 : garde immobile qui se retourne toutes les `délai` images et attaque si le joueur est à < 40 px ; 20-29 : patrouille + palette 9 ; sinon patrouille | 0x0800C150-0x0800C1E0 |
| 08020D14 | canon (186) | 2 : boulet rebondissant (bit 8 : vitesse ×254/256 et vy inversée au contact du décor) ; 3 : boulet à 3,5 px/image ; sinon 2,5 px/image | 0x08020608, 0x0801B7E6 |
| 08020D3C | archer (187) | 0 : flèche à 3 px/image, vy −0,5 ; 1 : flèche à 45° vers le haut, partie 8 px plus haut ; > 1 : `sub_0800B928` | 0x08020736 |
| 08020C78 / 08020548 | tonneau (15), 189 | n° d'objet unique : drapeau `0x03000314 + 2(n−1)` ; mis => remplacé (15) ou supprimé (189) | 0x08020C7C, 0x0802054C |
| 08020D00 | portes (192, 544) | destination : `0x030002D0` = n−1, `0x03000950` = n ; n−1 = 11 => `sub_08001DE4` | 0x08020EE2 |
| 0800ED88 | oiseau (1) | 1 : caché et inactif ; sinon `sub_0800EB6C` | 0x0800ED9E |
| 08021D94 | taupe (308) | 0 : animation 4 ; sinon animation 6 (+ cas spécial dans l'état de jeu 20) | 0x08021DBC |

## Visionneuse de niveaux (étape 14, `src/viewer.rs`, commande `view`)

- Lancement : `visionneuse/Visionneuse Lady Sia.bat` (ou `ladysia-decomp <rom> view`).
- Fenêtre Win32 sans dépendance (user32/gdi32), 240×160 agrandi ×3, ~60 images/s.
- Scènes prises dans la table des niveaux `0x08351868` (60 états, scène + n° de musique) et
  chargées par `levels::SceneView::load` avec le décodeur natif (pas d'émulation, ~1 s) :
  58 scènes lisibles, dont les niveaux que la détection automatique de l'étape 7 manquait.
- Défilement différentiel : couche décalée de caméra × facteur / facteur de la couche de
  référence (facteur 0 = fixe à l'écran), ordre de priorité du matériel.
- Objets : animation 0 jouée avec les durées du jeu, miroirs H/V, palette de la scène.
- Musique : rendue par émulation du pilote dans un fil séparé (un passage, 180 s max), puis
  jouée en boucle par `PlaySoundW` (winmm) depuis la mémoire.
- Tests sans écran : `LADYSIA_CAPTURE=<dossier>` écrit une image par scène ;
  `LADYSIA_AUTOQUIT=<s>` ferme après s secondes et affiche le nombre d'images et l'état de
  la musique.

À faire : décors restants, `+0x10`, modes des autres types, comportement (code des objets).

## Le jeu complet sur PC (étape 15, `src/gba/`, `src/game.rs`, commande `play`)

Le code machine d'origine du jeu (ARM et Thumb) s'exécute sur une GBA réécrite en Rust ;
le BIOS de Nintendo n'est pas nécessaire.

- `gba/cpu.rs` : ARM7TDMI complet (ARM + Thumb, modes et registres en banque, IRQ).
- `gba/bus.rs` : carte mémoire, registres IO, 4 DMA (immédiat, VBlank, HBlank, FIFO son),
  4 minuteurs en cascade, interruptions, KEYINPUT/KEYCNT. Le vecteur IRQ 0x18 est le même
  petit code ARM que le BIOS d'origine (appelle le gestionnaire lu en `0x03007FFC`).
- `gba/hle.rs` : appels BIOS réécrits (Halt, IntrWait/VBlankIntrWait, Div, Sqrt, ArcTan,
  CpuSet, CpuFastSet, Bg/ObjAffineSet, BitUnPack, LZ77, Huffman, RLE, Diff, MidiKey2Freq…).
- `gba/ppu.rs` : rendu par ligne, modes 0-5, fonds texte/affines, sprites normaux/affines,
  fenêtres WIN0/WIN1/OBJ, transparence, éclaircir/assombrir.
- `gba/apu.rs` : FIFO A/B (le jeu : minuteur 0, rechargement 0xFA40 = 11 398 Hz, A à droite,
  B à gauche) + PSG de `psg.rs`, sortie stéréo 32 768 Hz.
- `gba/eeprom.rs` : EEPROM_V122 (512 o pour ce jeu), fichier `.sav` à côté de la ROM.
- `game.rs` : fenêtre Win32, son `waveOut` (sert d'horloge : ~59,7 images/s), manette XInput.

Vérifié : logos, intro, menus, nouvelle partie, niveau 1 (dialogue, déplacement, saut,
épée, transparences) ; 330 à 480 images/s sans fenêtre ; 9 FIFO vides sur ~500 000 lectures.
Tests sans fenêtre : `LADYSIA_FRAMES`, `LADYSIA_SHOTS`, `LADYSIA_EVERY`, `LADYSIA_PRESS`,
`LADYSIA_SAVE`, `LADYSIA_AUTOQUIT`.

Exécutable unique (ROM intégrée, sans console, sauvegarde `Lady Sia.sav` à côté du .exe) :
`LADYSIA_DONNEES=<données> cargo build --release --features embarque` → `jeu/Lady Sia.exe` (depuis l'étape 20 : données sans code, voir plus bas ; avant : `LADYSIA_ROM`).
Lancé sans argument, il démarre le jeu ; avec des arguments, il garde toutes les commandes.

## Options du portage et remplacement du code d'origine (étape 16)

Menu d'options (Échap, `src/menu.rs`, réglages dans `<sauvegarde>.ini`) : plein écran (F11),
taille ×1-×8, image lissée, proportions, couleurs « écran GBA », volume, accéléré ×2/3/4/8
(Tab), sauvegardes instantanées sur 9 emplacements (F5 / F7, F6 = emplacement suivant ;
fichiers `.etat1`…, la sauvegarde `.sav` du jeu n'est jamais écrasée), recommencer, quitter,
et activation des fonctions réécrites. Manette : Select + Start ouvre le menu.

Décompilation (`src/jeu/`) : une fonction réécrite en Rust est appelée à la place du code
d'origine dès que le processeur atteint son adresse, puis revient à l'appelant. Un appel
natif compte 20 cycles (sinon les interruptions du mixeur son sont retardées).
Vérification `LADYSIA_CHECK=1` : une copie de la machine exécute l'original et compare
mémoire, registres conservés et r0 (pile morte et registres DMA en écriture seule exclus,
plus la mémoire privée déclarée de la fonction).

| adresse | nom | remplace aussi | appels vérifiés identiques |
|---|---|---|---|
| 0802F0B0 | Decompress (RLE) | Decomp_Entry, Decomp_Core (ARM) | 585 / 585 |
| 0802EE7C | LoadPacked (LZ + Huffman) | Packed_Decode, BuildTables, BuildHuffman, ReadSymbol, Predict, Finish | 64 / 64 (hors état privé 0x02001224-0x02002058, lu par aucune autre fonction) |
| 080016C8 | Palette_Fade | — | 192 / 192 |
| 0802A2E0 | Entity_OnScreen | — | 49 973 / 49 973 |
| 08029CD8 | Entity_Frame (comportement, animation, envoi VRAM, sprites OAM) | — | 49 973 / 49 973 |
| 080299E8 | Entities_Update (files d'attente, apparition des objets, 4 couches) | — | 4 508 / 4 508 |
| 08027498 | Map_DrawColumn (colonne de tuiles depuis les lignes compressées) | — | 1 298 / 1 298 |
| 08027A50 | Layer_Scroll (défilement, bornes / boucle, rotation-zoom) | — | 7 456 / 7 456 |
| 08034544 | Music_Tick (séquenceur : notes, 17 pistes, 16 motifs, tempo) | — | 4 500 / 4 500 |
| 08031F64 | Sound_Timer1 (IRQ : relance DMA son, mixage des voix, écrêtage) | — | 6 713 / 6 713 |
| 0803142C | Sound_VBlank (fréquences / volumes PSG, volume et panoramique des voix, démarrages) | — | 4 500 / 4 500 |
| 08033568 | Instr_Run (programmes d'instruments, table d'opcodes 0x083F4568) | — | 4 500 / 4 500 |
| 080308B0 | Pitch_Slides (vibratos / portamentos PSG et DirectSound) | — | 4 500 / 4 500 |
| 080293E8 | Map_TileAt (attribut de collision d'une tuile, cache, lignes compressées) | — | 21 887 / 21 887 |
| 0802956C | Map_Collide (collision au pixel : masque 8×8 par attribut, nature du sol) | — | 21 887 / 21 887 |
| 08028D3C | Entity_Probe (déplacement contre le décor, sondage 3×3, « au sol » prolongé) | — | 1 055 / 1 055 |
| 0802D074 | Layers_Parallax (défilement proportionnel des couches secondaires) | — | 4 508 / 4 508 |
| 080309FC | Vibrato (oscillation de hauteur PSG et voix, pas inversé à chaque recharge) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08030BF8 | Pitch_Ramps (glissement vers une note cible, borné) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08030CD8 | Sample_Sweep (départ d'échantillon oscillant, bit 0x400) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08030D6C | Loop_Sweep (début de boucle déplacé, bits 0x800 / 0x4000) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08030FD0 | Envelopes (ADSR et fondus des 4 PSG et des voix) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08031278 | Pan_Sweep (panoramique oscillant des voix) | — | 4 500 / 4 500, aléatoire 10 000 / 10 000 |
| 08032484 | Instr_Wait (opcode 4 des programmes d'instruments : attente) | — | 19 279 / 19 279, aléatoire 5 000 / 5 000 |
| 080283B0 | Bg_Regs (défilement d'un fond, point de référence et matrice affine) | — | appelée par Bgs_Regs |
| 0802D858 | Bgs_Regs (Bg_Regs pour chaque couche) | — | 4 505 / 4 505, partie 6 802 / 6 802 |
| 0802CFA8 | Camera_Move (couche principale : poussée, bornes ±8 px, rappel, défilement) | — | 4 505 / 4 505, partie 4 733 / 4 733, aléatoire 6 000 / 6 000 |
| 080019E8 | Frame_Camera (cinématique ou Camera_Move à la vitesse demandée) | — | 4 505 / 4 505, partie 6 802 / 6 802, aléatoire 6 000 / 6 000 |
| 0802C284 | Sprites_Upload (VBlank : graphismes des sprites et OAM par DMA 0, sprites cachés) | — | 4 510 / 4 510, partie 6 804 / 6 804 |
| 0802D2A4 | Layers_VBlank (VBlank : animations de tuiles, plan d'eau : ligne VCOUNT, DISPCNT, ondulation) | — | 4 628 / 4 628, partie 6 864 / 6 864, aléatoire 6 000 / 6 000 |
| 08035C74 / 08035D40 | __divsi3 / __modsi3 (libgcc) | — | 12 996 / 93 |
| 08036270 / 080362E8 | __udivsi3 / __umodsi3 (libgcc) | — | 6 525 / 9 949 |
| 08035E10 | __muldi3 (produit 64 bits, r1 comparé aussi) | — | 19 836 / 19 836 |
| 03000DC0 | Mix_Voice (ARM généré en IWRAM, copie de 0x08035480) | — | 17 149 / 17 149 |
| 03000F4C | Mix_Clip (ARM généré en IWRAM, copie de 0x08034EC8) | — | 6 713 / 6 713 |

Mixeur : le pilote assemble en IWRAM, selon sa configuration, des routines ARM copiées de
la ROM (`0x08034DB4`–`0x08035760`). Une fonction réécrite en IWRAM déclare la copie dont
elle tient lieu (`copy_of`) : la version Rust n'est appelée que si l'IWRAM contient
exactement ces octets. Le mixeur ARM représentait 67 % des instructions exécutées (le
profileur compte maintenant aussi l'IWRAM, `LADYSIA_DUMP=fichier` sauve l'IWRAM).
Le son produit n'est pas identique échantillon par échantillon à celui de l'original (le
code Rust prend un temps nul, le séquenceur tombe à un autre moment du bloc de mixage),
mais même musique (corrélation des enveloppes 0,92, même niveau moyen).

Les fonctions réécrites appellent le code pas encore réécrit avec `Env::call` (arguments
r0-r3 + pile, retour sur une adresse fictive 0x0F000000), qui peut lui-même retomber sur
des fonctions réécrites. Chiffres : test de 4 500 images (`LADYSIA_CHECK=1`).

Profil (`LADYSIA_PROFILE=fichier`, instructions interprétées par adresse en ROM et en
IWRAM, y compris le code appelé depuis une fonction réécrite) sur 3 000 images :
86,1 millions sans les fonctions réécrites, 11,6 millions avec (−87 %). Plus de fonction
dominante : `0802BD68` (sprites affines) 5 %, `08030FD0` (pilote son) 5 %, `080232B4` 4 %,
`08025D88` 4 %, `080309FC` / `08030BF8` (pilote son) 3 %…
Au passage titre → menu (image 990) le pilote son est réinitialisé pendant un chargement
devenu instantané : ~300 échantillons (9 ms) de FIFO vide au lieu de 9, au changement de
musique.
Étape 16 bis : les 6 étapes restantes de `Sound_VBlank` et l'attente des instruments
réécrites ; 14,2 → 9,9 millions d'instructions interprétées sur le test de 4 500 images
(−30 %). Première fonction restante : `0802BD68` (sprites affines, 10 %).

Test aléatoire `LADYSIA_FUZZ=n` (à la fin du mode sans fenêtre) : les données du pilote
son (canaux PSG, voix, descripteurs de lecture) sont tirées au hasard à partir d'un vrai
état, puis l'original et la version Rust sont comparés `n` fois par fonction. La musique
du test ne parcourt qu'une partie des branches (relâchement, rampes, balayages) ; avec
`LADYSIA_PROFILE`, le test compte aussi les instructions de l'original qu'il exécute :
toutes celles des 6 étapes le sont (sauf 3 sauts inatteignables). Il a trouvé un écart
(comparaison signée du niveau d'enveloppe), corrigé. Les accès mémoire des fonctions
réécrites (`Env::ldr`, `ldrh`, `ldsh`…) suivent maintenant l'ARM7 aux adresses non
alignées (rotation, octet signé), comme le code d'origine.

Restant : ~1 500 fonctions sur 1 537, les données (graphismes, niveaux, sons) restent lues
dans la ROM.

### Bus ouvert (correctif : chargement sans fin à la fin du Royaume de l'air)

À la fin du Royaume de l'air, un objet (`0x03006B10`) passe sous le bas de la carte
(ligne 0x15). `Map_AttrAt` (`0x080296DC`, appelée depuis `0x0802853A`) lit alors un
pointeur de ligne hors du tableau (`0x001E0000`) et parcourt la ligne compressée jusqu'à
trouver une plage assez longue. Dans une zone sans mémoire, la GBA renvoie l'instruction
préchargée (« bus ouvert ») et la recherche s'arrête aussitôt ; l'émulateur renvoyait 0,
et la recherche durait des millions d'itérations (écran « Merci de patienter » figé).
Le bus reproduit maintenant la console : zones 0x00004000-0x01FFFFFF, E/S au-delà de
0x400 et adresses ≥ 0x10000000 renvoient `[pc+4]` (Thumb, doublé) ou `[pc+8]` (ARM) ;
le BIOS lu hors du BIOS renvoie `0xE25EF004`. Options de test ajoutées :
`LADYSIA_ETAT=fichier` (part d'un état instantané), `LADYSIA_PC=n` (avec
`LADYSIA_PROFILE`, profil des n dernières images), `LADYSIA_REGS=1` (registres à la fin).

### Icône « appuie vite sur A » (`src/icones.rs`)

Course du Royaume de l'air : le jeu montre une flèche rouge au-dessus d'un bouton qui
s'enfonce (sprite 16×32 + bout 8×8, 4 images envoyées en VRAM toutes les 4 images) ; il
faut marteler A (~10 fois/s ; maintenir, B ou ↓ ne font rien). Avant chaque image, le
sprite est reconnu à l'empreinte FNV-1a de ses tuiles et remplacé par une touche de
clavier « A » qui s'enfonce au même rythme (aussi quand le jeu ne renvoie pas l'image) ;
le petit sprite du bout du bouton (8×8 ou 8×16, même palette, en bas à droite de la
touche, à n'importe quelle tuile) est masqué dans l'OAM. Vérifié image par image sur
toute la course (1 784 images avec la touche, aucun pixel rouge autour). Seul
l'affichage change (images de fin de course identiques). La touche A du clavier commande
maintenant aussi le bouton A (avec X, Espace, K) ; L reste sur Q. Option de test
`LADYSIA_VID=1` : VRAM, OAM et palettes sauvées avec les captures.

## Améliorations graphiques et sonores (étape 17, `src/filtres.rs`)

Tout se passe dans la couche PC (sortie image et son) : le jeu lui-même n'est pas modifié,
et les réglages d'origine restent ceux par défaut (« RÉGLAGES D'ORIGINE » dans le menu).

| Option (menu Échap / `.ini`) | Valeurs | Détail |
|---|---|---|
| IMAGE (`filtre`) | pixels nets, lissée, nets adoucis, HD (xBR) | nets adoucis = bilinéaire « net » (transition d'un seul pixel de sortie, pas de pixels inégaux) ; HD = xBR niveau 2 (Hyllian) : détection des contours à 45°, 30° et 60° par coin de pixel, puis tracé en droites anticrénelées directement à la taille de la fenêtre (pas d'agrandissement intermédiaire). Le premier essai, Scale2x/3x + bilinéaire, rendait mal (tramages bruités, flou) et a été remplacé |
| EFFET (`effet`) | aucun, lignes de balayage, grille LCD, écran cathodique | calculé par pixel de sortie d'après la position dans le pixel GBA ; inactif sous ×2 |
| COULEURS (`couleurs`) | originales, écran GBA, vives | vives = saturation ×1,3 + légère courbe en S (table 15 bits) ; l'ancienne clé `couleurs_ecran_gba=1` est reprise |
| RÉMANENCE LCD (`remanence`) | oui / non | moyenne de l'image courante et de la précédente |
| SON ADOUCI (`son_adouci`, activé par défaut) | oui / non | chaque échantillon du mixeur (11,4 kHz) est daté au cycle près ; la sortie 32 768 Hz est interpolée dans le temps (retard 4096 cycles) puis passe dans un Butterworth d'ordre 4 à 45 % de la fréquence du mixeur, comme le filtre analogique de la console. Variations brusques (dérivée seconde) : 302 → 96 en valeur efficace, parasites au-dessus de 5,7 kHz divisés par 3 |

Le menu a 19 lignes et défile (14 visibles). Rendu sur plusieurs cœurs : à 1620×1080,
2,5 ms (pixels nets), 5 ms (lissée / nets adoucis) et 9 ms (HD xBR, effets compris) ; fenêtre ×6 avec tout
activé : 59,4 images/s. Test : `LADYSIA_RENDUS=1620x1080` avec `LADYSIA_SHOTS` écrit chaque
rendu de la dernière image et une planche de détails ; `LADYSIA_SON_ADOUCI=1` en mode sans fenêtre.

### Grésillements de la musique (étape 17 bis)

Mesures : le flux du jeu est propre (le DMA lit exactement 384 octets entre deux relances,
tampon de 3 blocs de 128 échantillons, minuteur 0 = FA40 → 11 397 Hz, minuteur 1 = F480 ;
aucun écrêtage ; FIFO vide seulement pendant deux chargements). Les grésillements venaient
de la sortie PC :

1. **coupures de la carte son** : avec le filtre HD, un fil de rendu suspendu par Windows
   bloquait toute la boucle (pics de 50 à 110 ms) et la file waveOut se vidait (15 coupures
   en 15 s). Le filtre et le dessin tournent maintenant dans un **fil d'affichage séparé**
   (`Afficheur`, la dernière image remplace celle pas encore affichée) : 1 coupure (au
   démarrage) en 15 s avec HD + écran cathodique, moins de 1 % d'images sautées ; rendu sur
   la moitié des cœurs (6 au plus). La file son s'allonge aussi d'elle-même (3 → 5 tampons)
   si des coupures surviennent ;
2. **marches d'escalier** à 11,4 kHz rejouées à 32 kHz (2 ou 3 échantillons de sortie par
   échantillon, d'où une gigue) : nouveau « son adouci » ci-dessus.

Le bilan de `LADYSIA_AUTOQUIT` donne coupures, avance, temps d'émulation et d'affichage.

## Mods activables (étape 18, `src/mods.rs`)

Variables trouvées par recherche mémoire : la RAM (IWRAM + EWRAM) et l'image sont vidées
toutes les 10 images pendant une partie scriptée (`LADYSIA_RAM=1` avec `LADYSIA_SHOTS`),
le remplissage du cœur (pixels rouges du HUD) donne 5 niveaux (138, 124, 92, 46, 11 px) et
un seul octet les suit pendant toute une vie :

| Adresse | Rôle | Vérifié |
|---|---|---|
| `0x03001E08` | santé, 5 = cœur plein, 0 = mort | 5→4→3→2→1→0 aux coups (images 2210, 2400, 2650, 2960, 3140), remis à 5 à la nouvelle vie |
| `0x03001DE0` | vies | 3 → 2 à la mort (image 3260), chiffre du HUD |

| Option (menu / `.ini`) | Défaut | Effet |
|---|---|---|
| MOD SANTÉ INFINIE (`mod_sante_infinie`) | non | santé remise à 5 avant chaque image (sauf à 0 : chute mortelle) ; partie scriptée sans perte de vie jusqu'à 4500 images |
| MOD VIES INFINIES (`mod_vies_infinies`) | non | vies maintenues à 9 (le HUD affiche 9) ; les vies peuvent être écrites dans la sauvegarde du jeu |
| MOD REMBOBINAGE (`mod_rembobinage`) | oui | un état toutes les 4 images, 10 s d'historique (≈ 60 Mo) ; maintenir **R** ou la gâchette gauche recule à vitesse ×2, le son reste l'horloge (silence) |
| PAUSE HORS FENÊTRE (`pause_hors_fenetre`) | non | le jeu s'arrête quand la fenêtre n'est pas au premier plan |
| REPRISE AUTO (`reprise_auto`) | non | état sauvé dans `<sauvegarde>.reprise` en quittant, rechargé au lancement |

Tests sans fenêtre : `LADYSIA_MODS=sante,vies`, `LADYSIA_REMBOBINE=image:pas` (ex. `3100:40`
ramène 160 images en arrière : santé 1 → 2, image identique à celle de l'image 2950 à
quelques images près). Non retenu : « tous les lieux ouverts » (bits de `0x030000BC`, 6 lieux
par carte) car la progression dépend d'autres variables et serait écrite dans la sauvegarde.

## Commandes du clavier configurables

Menu Échap > « Commandes du clavier... » : pour chaque bouton GBA (croix, A, B, L, R,
Start, Select), jusqu'à 4 touches. Entrée = remplacer, → = ajouter, ← = retirer la
dernière, « Touches d'origine » remet la disposition de départ. Pendant l'attente,
la première touche nouvellement appuyée (`new_key`, balayage des codes VK) est
attribuée et retirée des autres boutons ; Échap annule. Touches réservées au portage
refusées : Échap, Tab, F5, F6, F7, F11, P, et R si le mod rembobinage est actif. La
navigation du menu reste sur les touches fixes (flèches, Entrée, X, Espace, Échap,
Retour arrière). Gardées dans le `.ini` (`touche_a=58,20,4B,41`, codes VK en
hexadécimal) ; un `.ini` sans ces lignes garde les touches d'origine. La manette
n'est pas concernée. Test : `menu::tests::touches_ini` ; captures `commandes.png`,
`commandes_attente.png` avec `LADYSIA_SHOTS`.

L'icône « appuie vite sur A » porte la touche attribuée au bouton A
(`icones::touche_icone` : « A » si elle en fait partie, sinon la première) : un
caractère en police 5×7 du menu, ou une abréviation de 2-3 caractères en police 3×5
(ESP, ENT, MAJ, F12…). Test sans fenêtre : `LADYSIA_TOUCHE_A=code VK hexadécimal`.

### Étape 16 ter : affichage (caméra, fonds, VBlank)

Six fonctions de plus (tableau ci-dessus). Vérifiées sur le test de 4 500 images et
depuis deux états en pleine partie (course du Royaume de l'air, sauvegarde 1). La branche
« plan d'eau » de `Layers_VBlank` n'étant parcourue par aucun de ces tests, un test
aléatoire `fuzz_video` (lancé avec `LADYSIA_FUZZ`, depuis un état de partie) tire les
variables des couches au hasard : avec lui, toutes les instructions d'origine des six
fonctions sont exécutées, sans écart. Il a trouvé une erreur : `0x0802CEEC` (avance de la
cinématique) reçoit la cinématique en r0, corrigée.
Instructions interprétées : test titre / menus 4,79 → 2,78 millions (−42 %) ; partie
(3 000 images) 12,8 → 11,3 millions (−12 % ; 108,6 millions sans fonctions réécrites).
Écart connu, antérieur : depuis l'état de la course, 1 appel sur 34 304 d'`Entity_Frame`
différait en EWRAM `0x02001B13`–`0x02001C4E`. Expliqué et corrigé plus loin (fausse
alerte de la vérification, voir « Améliorations diverses »).

## Recompilation statique complète (étape 19, `src/recomp_gen.rs`, `src/recomp/`)

Tout le code du jeu est maintenant compilé en code machine PC : plus aucune instruction du
jeu n'est interprétée en partie (0 sur 3 000 images depuis la sauvegarde 1 ou la course ;
1 seule sur le test titre / menus : le saut de démarrage de la cartouche).

- **Génération** : `ladysia-decomp <rom> recomp` lit `asm/ladysia.s` et écrit
  `src/recomp/code/` (16 fichiers, ~6 Mo) : 90 346 instructions Thumb et 206 ARM en
  11 331 blocs. Un bloc = suite d'instructions coupée aux étiquettes, après les sauts,
  appels, retours et SWI. Chaque instruction devient `c.thumb_insn(b, opcode, adresse)`
  (ou `arm_insn`) : la même routine que l'interpréteur, marquée `#[inline(always)]`,
  avec des constantes ; le compilateur n'en garde que le travail de cette instruction
  (plus de lecture ni de décodage). Comportement identique par construction, cycles
  compris (lecture d'instruction comptée : 2 cycles Thumb en ROM, 4 ARM en ROM, 1 en
  IWRAM / BIOS).
- **Code hors désassemblage** : les pointeurs de la ROM vers du code Thumb non répertorié
  (fonctions appelées seulement par une table, ex. `0x0803382C`) sont suivis jusqu'au
  premier retour : 1 084 instructions de plus. Un bloc traduit est correct quelle que soit
  l'adresse, même si elle n'est jamais exécutée.
- **Code copié en IWRAM** : `IntrMain` (répartiteur d'interruptions, ARM, ROM 0x0800012C →
  IWRAM 0x030009C0) est recompilé à son adresse d'exécution, utilisé seulement si l'IWRAM
  contient exactement la copie. Le vecteur d'interruption du mini-BIOS aussi.
- **Exécution** : quand le processeur arrive au début d'un bloc (table par demi-mot, mode
  ARM / Thumb vérifié), la fonction du bloc s'exécute ; interruptions, temps (minuteurs,
  son) et arrêt du processeur sont traités entre deux blocs. Un bloc s'arrête aussi après
  une écriture qui demande l'arrêt (HALTCNT). Les fonctions réécrites à la main (`jeu/`)
  restent prioritaires ; le code d'origine qu'elles appellent passe aussi par les blocs.
  Tout ce qui n'a pas de bloc (adresse inconnue) retombe sur l'interpréteur.
- **Garde-fous** : empreinte de la zone de code (autre ROM → tout interprété),
  `LADYSIA_RECOMP=0` pour désactiver ; le menu « FONCTIONS RÉÉCRITES : NON » coupe aussi
  le code recompilé (jeu d'origine interprété).
- **Vérification** `LADYSIA_RECOMP_CHECK=1` : les 3 premières exécutions de chaque bloc
  sont refaites par l'interpréteur sur une copie de la machine et comparées (registres,
  drapeaux, cycles, arrêt, mémoire). Titre / menus 2 504, course 2 528, sauvegarde 1
  3 032 blocs vérifiés : 0 différence. Fonctions réécrites (`LADYSIA_CHECK=1`) : 0.
- **Profil** : `LADYSIA_PROFILE` ne compte plus que les instructions encore interprétées.
- Compilation : ~1 min 10 s de plus en `--release` ; exe 5,8 Mo (10 Mo avec la ROM).
- Vitesse : le processeur ne pèse presque plus rien ; le temps restant est surtout
  l'affichage (PPU) et le son. Fenêtre : 59,7 images/s, 1,1 ms d'émulation par image.

Suite possible : rendre lisibles (réécriture à la main dans `jeu/`) les fonctions
importantes, qui remplacent alors leurs blocs ; remplacer le rendu GBA ligne par ligne.


## Correction : barre bleue à la surface de l'eau

Dans les niveaux aquatiques, une barre bleue opaque de 3 lignes apparaissait à la surface
de l'eau (avec ou sans recompilation : défaut de l'émulation vidéo). Le gestionnaire
d'interruption de ligne `0x0802D726` active le fond de l'eau (BG2) dans DISPCNT à la
ligne de la surface. Sur la vraie GBA, un fond activé en cours d'image n'apparaît
qu'après 3 lignes. Le jeu compte sur ce délai : les 3 premières lignes du fond de
l'eau, opaques, ne doivent jamais être visibles. Ce délai est maintenant émulé
(`Bus::bg_delay`, décompté à chaque ligne ; un fond désactivé disparaît tout de suite).
Vérifié sur l'état 2 du joueur : 0 image touchée sur 60, contre 140 sur 140 avant. Le
menu, la course et le niveau des machines donnent des images identiques à la version
précédente.

## Améliorations diverses (son, manette, vérification, balayage des niveaux)

- **Coupures de son** : le bilan comptait toujours 1 coupure au lancement. La file de la
  carte son est forcément vide au premier envoi (et en sortie de pause ou de menu), donc
  ce n'en était pas une. Une file vide ne compte plus que si le son tournait déjà
  (envoi précédent il y a moins de 40 ms).
- **Manette configurable** : menu Échap › « COMMANDES DE LA MANETTE... », sur le modèle de
  l'écran du clavier (A remplacer, → ajouter, ← retirer, 4 boutons au plus par bouton
  GBA, un bouton pris à un autre lui est retiré). `.ini` : `manette_a=A,Y` etc. (noms
  A B X Y LB RB LT RT START BACK L3 R3 CROIX_HAUT/BAS/GAUCHE/DROITE). Le stick gauche
  compte comme la croix ; la gâchette gauche rembobine toujours. La navigation dans le
  menu utilise les boutons physiques (Back + Start, croix ou stick, A, B), quelle que
  soit l'attribution, pour qu'une mauvaise configuration ne bloque jamais le menu.
- **Écart d'`Entity_Frame` expliqué** : la zone `0x02001224`–`0x02002058` est la mémoire
  de travail privée de `LoadPacked` (tables Huffman de l'original, inutiles à la version
  Rust). Elle était ignorée quand `LoadPacked` est vérifiée seule, mais pas quand
  elle est appelée plus bas, depuis le comportement d'un objet dans `Entity_Frame`. La
  vérification ignore maintenant aussi la mémoire privée des fonctions réécrites appelées
  pendant l'appel vérifié (`inner_private`). Course : 0 différence sur 35 104 appels.
- **Balayage des niveaux** : `LADYSIA_NIVEAU=n` (tests) impose l'état du jeu
  `0x03000B50` au premier chargement de scène (0x0800043E), ce qui démarre directement au
  niveau n. Niveaux 16 à 51 joués 1 500 images (en avançant et en sautant), captures
  toutes les 50 images, détecteurs automatiques (bandes unies, lignes isolées, barre
  d'eau) et revue à l'œil : aucun autre défaut d'affichage. Les cas suspects étaient des
  éléments du jeu : bordure de dialogue, fenêtres à barreaux, eau translucide devant le
  sol, image accrochée au mur, interface du mini-jeu de la tortue.
- **Nouvelles fonctions réécrites** (`src/jeu/entity.rs`) : `Entity_Hide` (0x0802A50C,
  file des entités à cacher), `VRAM_Alloc` (0x0802A580, premier bloc libre assez grand,
  avec fusion des blocs libres puis seconde recherche si rien ne convient) et `VRAM_Free`
  (0x0802A694, libération et fusion avec le bloc suivant). La table d'occupation
  `0x03004E50` a une entrée de 16 bits par tuile de sprite, en tête de chaque bloc
  (bit 15 = occupé, bits 0-14 = longueur). Test aléatoire `LADYSIA_FUZZ` (`fuzz_vram`,
  tables tirées au hasard et souvent pleines) : 3 000 tirages par fonction, 0 différence.
- **`Decompress`, copie brute** (méthode 0, ex. `0x080F9AFC` au niveau 17) : l'original
  lit des mots de 32 bits à `src + 5`, adresse non alignée, et l'ARM7 fait alors tourner
  les octets du mot lu. La version Rust copiait les octets dans l'ordre : 7 octets sur
  320 différaient (quelques pixels). Elle reproduit maintenant la lecture tournée. Le
  balayage `LADYSIA_CHECK` des niveaux 16 à 51 (450 images chacun) et de la course donne
  0 différence sur 1 375 865 appels vérifiés.

## Version finale : plus de ROM dans l'exécutable (étape 20)

L'exécutable du jeu (`--features embarque`) n'embarque plus la ROM mais un fichier de
**données sans code machine**, écrit par la commande `donnees` :

```
ladysia-decomp <rom.gba> recomp                 # régénère src/recomp/code/
ladysia-decomp <rom.gba> donnees ladysia.lsd    # données : ROM sans instructions ni en-tête, compressée
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Dans ce fichier, chaque instruction recompilée, l'en-tête de la cartouche (logo compris)
et les routines ARM du mixeur sont à zéro : 180 082 octets effacés. Il reste les
graphismes, niveaux, sons, textes et les constantes mêlées au code (adresses, tables).
Tout ce qui s'exécute vient du Rust (`src/recomp/code/` et `src/jeu/`) ; avec ces données
rien ne peut être interprété (`recomp::data_only()`, reconnu par `DATA_HASH`). Si du code
absent devait s'exécuter, le jeu l'affiche (« CODE ABSENT EN … ») et l'écrit dans
`Lady Sia.erreur.txt`. L'option du menu devient « CODE DU JEU : 100 % RUST ».

Ce qu'il a fallu régler :
- **Bus ouvert** : la console relit l'instruction préchargée quand le jeu lit une adresse
  sans mémoire, et Lady Sia en dépend. La valeur est maintenant une constante passée par
  le code recompilé (`thumb_insn(.., ob)`, `Bus::ob`), plus lue dans la ROM.
- **Saut d'entrée de la cartouche** (0x08000000) recompilé : plus aucune instruction
  interprétée, même au démarrage.
- **Constantes prises pour du code** : la recherche de code « par pointeurs » décodait
  l'en-tête, des constantes et des tables. Sans conséquence tant que la ROM était là, mais
  ces octets auraient été effacés. Les `.word` du désassemblage sont protégés, et une
  suite n'est gardée que si elle est plausible (zone de code, pas de mot nul ni
  d'instruction indéfinie, `bl` complets vers la zone de code, fin par un retour).
- **Instruction dans une constante** : le gestionnaire de ligne `0x0802D726` commence 2
  octets avant sa vraie première instruction, au milieu de la constante `0x03006C94`. Ces
  2 octets sont à la fois une adresse et une instruction (`lsl r0, r0, #12`, sans effet) :
  un octet répertorié `.word` n'est jamais effacé.
- **Générateur aléatoire** (0x0802DA28, tableau de 312 mots en `0x030003F0`) : il lit 25
  mots après la fin de son tableau, c'est-à-dire le début de la copie du répartiteur
  d'interruptions en IWRAM (`0x030009C0`). Ces 100 octets (`0x0800012C`-`0x0800018F`) sont
  gardés dans les données, comme graines de hasard : jamais exécutés, mais sans eux les
  tirages ne seraient plus ceux de l'original.
- **Fonctions mortes** : tout `push {.., lr}` hors du code connu est essayé comme début
  de fonction ; 5 461 instructions de plus sont recompilées (95 242 Thumb, 207 ARM,
  11 787 blocs) et effacées. Reste environ 3 Ko de fragments jamais exécutés (constantes
  de fonctions mortes, quelques fonctions sans `push`).
- **Sauvegardes instantanées anciennes** : elles contiennent le code d'origine recopié en
  IWRAM (répartiteur, mixeur). Au chargement, ces copies, reconnues par leur empreinte,
  sont remplacées par leur version des données (`Bus::strip_copies`).

Outils de test ajoutés : `LADYSIA_AUDIT=1` (avec la ROM : signale tout octet effacé lu
comme une donnée), `LADYSIA_COMPARE=<données>` (seconde machine menée en parallèle,
état complet comparé à chaque image, plus trace des blocs).
Vérification : 40 parcours côte à côte ROM / données (niveaux 16 à 51, démarrage et
menus avec et sans sauvegarde, deux états du joueur), 96 000 images : 0 écart d'état
(mémoire, registres, cycles), 0 instruction interprétée, 0 octet de code lu comme donnée
hors recopies attendues. Attention : `LADYSIA_NIVEAU` saute l'initialisation du jeu, le
générateur aléatoire n'est alors pas initialisé (sans effet sur la comparaison).

## Suites de l'étape 20 : code oublié, taille, mods, dépôt publiable

- **« Code non recompilé en 0800B988 »** (journal `Lady Sia.erreur.txt` du joueur) : le
  filtre de plausibilité rejetait une suite qui retombait sur du code retenu par la même
  recherche. Corrigé (`kept`), et le générateur refuse maintenant de produire le code si
  une cible de saut ou d'appel direct, ou la suite d'une instruction, n'est pas
  recompilée. Les cibles d'appels sont suivies au-delà de la fin du désassemblage
  (fonctions de bibliothèque) : 96 060 instructions Thumb, 207 ARM, 11 900 blocs.
- **Taille de l'exécutable : 11,1 → 8,4 Mo.**
  - Données compressées (`src/lzr.rs`, LZ77 + codeur arithmétique binaire adaptatif, sans
    dépendance) : 4 194 304 → 2 617 747 octets, mieux que zlib (2 683 103). La commande
    `donnees` écrit ce format et relit pour vérifier ; décompression au lancement en
    moins de 0,4 s. `play` et `LADYSIA_COMPARE` acceptent les deux formats.
  - Les outils de développement ne sont plus dans l'exécutable `embarque`.
  - `panic = "abort"`, `strip = true`. `opt-level = "s"` et `codegen-units = 1`
    n'apportent rien (le code recompilé fait déjà ~50 octets par instruction du jeu) et
    triplent le temps de compilation : non retenus.
  - Instructions de calcul pur (`Cpu::thumb_reg`) : elles ne notent plus l'instruction en
    cours ni la valeur de bus ouvert, utiles seulement aux accès mémoire et en fin de
    bloc. Gain faible (1 %), comportement identique.
  - Le reste (6,2 Mo) est le code recompilé lui-même ; descendre nettement plus bas
    demanderait une autre approche (interpréter des tables d'instructions, ce qui
    remettrait le code de la cartouche dans l'exécutable).
- **Attente précise** (`game::attendre`) : `sleep(1 ms)` dure 15,6 ms quand Windows
  n'honore pas `timeBeginPeriod` (fenêtre cachée, économie d'énergie) et le jeu tombait
  à 43 images/s. Remplacé par une minuterie haute résolution : 60 images/s dans tous les
  cas.
- **Page MODS** (`src/mods.rs`, menu Échap › MODS) : santé infinie, vies infinies, mort
  subite, rembobinage, vitesse du jeu (50 à 200 %, son ramené à la durée d'une image),
  saut (44 px d'origine, 81 ou 129), gravité (saut de 80 ou 152 px), course rapide
  (202 → 354 px sur 120 images), vol libre, taille de Sia, toupie, transparence, palette,
  monde miroir, ambiances de couleurs, départ direct dans un niveau.
  Variables : entité du joueur à pied en `[0x030009A0]` (type `0x0805B304`), vitesses
  tenues par le jeu en `0x03001E00` (horizontale, 512 au plus) et `0x03001E10`
  (verticale : +64 par pas de logique, saut à environ −900) ; les champs `+0x5A/+0x5C`
  de l'entité n'en sont que la copie. `+0x6C` : octet non nul au sol. Drapeaux `+0x58` :
  0x100 échelle (`+0x66/+0x68`, agrandissement 8.8), 0x80 rotation (`+0x3C`, tour =
  0x1000), 0x20 transparence, 0x200 palette imposée (`+0x72`). La course rapide passe
  par `jeu::entity::COURSE_PLUS` (déplacement amplifié dans `Entity_Frame`).
  Tests : `LADYSIA_MODS=saut=2,gravite=1,course,vol,taille=3,toupie,transparente,
  palette=5,miroir,ambiance=1,subite`, `LADYSIA_SUIVI=1` (position et vitesse du joueur à
  chaque image).
- **Dépôt publiable** : rien de ce qui vient de la cartouche n'est suivi par Git
  (`.gitignore` : ROM, données, sauvegardes, `asm/`, `src/recomp/code/`, `extract/`,
  rapports, exécutables). `build.rs` écrit un module de code vide quand il manque, pour
  que les outils compilent, et donne le chemin absolu des données à intégrer. Procédure
  du `README.md` rejouée sur une copie ne contenant que les 49 fichiers publiables :
  mêmes données au bit près, exécutable final fonctionnel.
- Vérification de l'ensemble : 310 images et pistes son de 10 niveaux identiques à la
  version précédente, 0 écart ROM / données.
