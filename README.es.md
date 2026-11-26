# Lady Sia — port para PC en Rust

[English](README.md) · [Français](README.fr.md) · [Deutsch](README.de.md) · **Español** · [Italiano](README.it.md) · [Nederlands](README.nl.md)

Herramientas de análisis y port para PC (Windows) del juego de Game Boy Advance
*Lady Sia*, versión europea. El código del juego se recompila a Rust a partir del
cartucho; el resultado es un ejecutable autónomo, sin emulador que instalar.

**Este repositorio no contiene la ROM, ni sus datos, ni nada extraído de ella.** Hay
que poseer el cartucho y hacer una copia propia. Todo lo que procede del cartucho
(desensamblado, código recompilado, gráficos, niveles, sonidos) se genera en tu equipo
y Git lo ignora (ver `.gitignore`).

## Compilar el juego

Se necesita Rust (edición 2024) en Windows y la ROM europea
(`Lady Sia (Europe) (En,Fr,De,Es,It,Nl).gba`, 4 MB).

```
cargo build --release
```

Esta primera compilación genera las herramientas (el código recompilado aún no existe;
`build.rs` escribe un módulo vacío en su lugar).

```
target/release/ladysia-recomp <rom.gba>
```

Analiza la ROM y escribe el desensamblado anotado en `asm/ladysia.s` (más algunos
informes). Los nombres de funciones vienen de `symbols.txt`.

```
target/release/ladysia-recomp <rom.gba> recomp
```

Traduce el desensamblado a Rust en `src/recomp/code/`. El comando falla si un destino
de salto del código traducido no está a su vez traducido.

```
cargo build --release
target/release/ladysia-recomp <rom.gba> donnees ladysia.lsd
```

Recompila con el código del juego y luego escribe los datos del juego: la ROM con todas
las instrucciones y la cabecera borradas, comprimida.

```
LADYSIA_DONNEES=ladysia.lsd cargo build --release --features embarque
```

Genera el ejecutable final `target/release/ladysia-recomp.exe` (renómbralo a tu gusto),
que contiene el código Rust y los datos. Las partidas y los ajustes se guardan a su lado.

Durante el desarrollo, `ladysia-recomp <rom.gba> play` lanza el juego desde la ROM y
`ladysia-recomp <rom.gba> view` abre el visor de niveles.

## Jugar

| Tecla | Acción |
| --- | --- |
| Flechas | Cruceta |
| X, Espacio, K o A | A |
| W, Z o J | B |
| Q / S | L / R |
| Intro | Start |
| Retroceso | Select |
| Esc | Menú: imagen, sonido, controles, mods |
| Tab | Avance rápido |
| R | Rebobinar |
| F5 / F7 / F6 | Guardar estado / cargar / ranura siguiente |
| F11 | Pantalla completa |
| P | Pausa |

Las teclas y los botones de un mando Xbox se configuran en el menú. El menú está en
francés.

## Mods

Menú Esc › **MODS**:

- salud infinita, vidas infinitas, muerte súbita (un solo golpe), rebobinado;
- velocidad del juego de 50 a 200 %;
- salto alto o muy alto, gravedad «luna» o «pluma», carrera rápida, vuelo libre
  (arriba y abajo dirigen a Sia en el aire);
- Sia diminuta, grande o gigante, peonza, transparente o con otra paleta;
- mundo espejo, ambientes de color (Game Boy, blanco y negro, noche, negativo, sepia);
- inicio directo en cualquiera de los 36 niveles.

Los mods de movimiento y apariencia afectan a Sia a pie.

## Estructura

- `src/gba/`: procesador ARM7TDMI, bus, vídeo y sonido de la consola;
- `src/recomp_gen.rs`, `src/recomp/`: recompilación estática del código del juego;
- `src/jeu/`: funciones del juego reescritas a mano en Rust legible;
- `src/game.rs`, `src/menu.rs`, `src/mods.rs`, `src/filtres.rs`: ventana, menú, mods,
  filtros de imagen;
- el resto: análisis y extracción (desensamblador, descompresores, niveles, sonidos);
- `NOTES.md`: diario detallado de ingeniería inversa (en francés).

*Lady Sia* es una marca de sus respectivos titulares. Este proyecto no está afiliado a
ninguno de ellos.
