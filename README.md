# OpenWWX

Implementación en Rust del juego de cartas coleccionables **Raw Deal** (WWF/WWE), para escritorio.

Estado: motor de reglas (Quick Start y Advanced), app de escritorio y los sets Premiere, Fully Loaded, Backlash y
Survivor Series con sus efectos programados. El diseño completo está en
[docs/motor-de-reglas.md](docs/motor-de-reglas.md).

## Crates

| Crate | Qué hace |
|---|---|
| `rawdeal-core` | Motor de reglas puro y determinista: estado, turnos, daño, reversals, victoria. |
| `rawdeal-cards` | Carga de cartas desde archivos RON. Incluye un set de prueba con cartas y Superstars inventados. |
| `rawdeal-ai` | Oponentes por computadora. Por ahora, un bot aleatorio. |
| `rawdeal-cli` | Cliente de texto para jugar contra el bot y depurar el motor. |
| `openwwx` | App de escritorio (egui): menú, tablero, mano, decisiones y relato de la lucha. |

## Jugar

Descarga la versión para tu sistema desde [Releases](https://github.com/Hiigaaran/openwwx/releases), descomprímela,
pon tus sets `.ron` en la carpeta `cards` y abre `openwwx`.

La mesa repite cada jugada con animaciones: la carta vuela de la mano a "En juego", las cartas volteadas por daño
caen al Ringside con un destello rojo, y los avisos grandes marcan turnos, reversals, daño recibido y manos mostradas.
La velocidad se cambia arriba del relato (lentas, normales, rápidas o sin animaciones) y "Saltar" las termina al
instante.

Las Superstars de la lucha aparecen como cartas junto a sus pilas, y el visor a la izquierda del relato muestra en grande
la última carta por la que pasó el mouse. Sin arte, cada carta se dibuja con la estructura de Raw Deal (título,
Fortitude y Daño, dibujo según su tipo, línea de tipo, texto y Stun Value). Si en la carpeta de cartas (o en una
subcarpeta, por ejemplo `art`) hay imágenes `.png` o `.jpg` cuyo nombre es el GUID de OCTGN de la carta, su id o su
título, se usa la imagen tal cual.

Para compilarla tú mismo:

```sh
cargo run --release -p openwwx
```

Para publicar una versión nueva, sube un tag (`git tag v0.1.0 && git push origin v0.1.0`) o ejecuta el workflow
"Release" desde la pestaña Actions con la versión: GitHub Actions compila Windows, macOS (Apple Silicon e Intel) y
Linux y crea el release con los archivos.

## Probarlo

```sh
cargo test                                  # tests de reglas + 200 partidas aleatorias entre todos los Superstars
cargo run -p rawdeal-cli                    # jugar contra el bot
cargo run -p rawdeal-cli -- --watch --seed 3  # ver una partida bot contra bot
```

## Qué funciona

- Setup Quick Start: Hand Size, Superstar Value decide quién empieza (moneda en empate).
- Turno: robo, Main Segment con chequeo de Fortitude, fin de turno.
- Daño volteando cartas del Arsenal al Ringside.
- Reversals desde la mano y al voltearlos durante el daño, con Stun Value.
- Cartas híbridas, cartas Irreversible.
- Victoria por Pinfall y Count Out (se puede rechazar), y empate.
- Efectos: robar (o robar hasta N), descartar (elegido, al azar, elegido por el rival tras mirar la mano, o
  todas las cartas de un tipo), recuperar del Ringside, barajar cartas en el Arsenal, poner cartas en el fondo
  del Arsenal, daño extra, mandar cartas del Arsenal al Ringside, buscar en el Arsenal, mirar la mano rival,
  ordenar las cartas de arriba del Arsenal, quitar cartas del Ring, terminar el turno, saltarse el turno
  rival, efectos al final del turno, ganar por descalificación, efectos opcionales y "elige una".
- Bonos: +D/-F a la próxima carta, a la próxima maniobra, a las cartas del resto del turno o del turno
  siguiente; +D a los Reversals contra una carta y +F para poder revertirla; cartas que no se pueden revertir
  (o no desde el Arsenal); bonos impresos según la carta anterior o las cartas de tu Ring; cartas que
  mientras están en tu Ring dan bonos o efectos al inicio del turno.
- Requisitos para jugar una carta (Fortitude menor que el rival, cartas en mano, carta en tu Ring, Superstar
  rival no femenina), cartas híbridas jugadas como Acción con su propio texto, y cartas que "cuentan como
  jugadas después de" otra en tu siguiente turno (Irish Whip, Jockeying for Position).
- Habilidades de Superstar: al inicio del turno (antes de robar), activadas una vez por turno con costo,
  robo extra en el Draw Segment y reducción del daño recibido.
- Palabras clave: follow-up (la carta anterior debe cumplir una condición; usar la habilidad no rompe la
  cadena), Active, Universally Unique, Irreversible.
- Multi (hay que revertirla dos veces, desde la mano o al voltear daño; las Superstars femeninas no pueden
  jugar maniobras Multi) y Chain (revertirla desde la mano o el Backlash cuesta descartar 2; si tiene éxito,
  puedes descartar 2 y buscar otra Chain en el Arsenal).
- Modo avanzado (`GameSetup::advanced`, o `--advanced` en la CLI): mazo Backlash de hasta 20 cartas
  (10 Pre-match y 10 Mid-match), fase Pre-match completa (Venue, Feud, Stipulation y Manager por turnos,
  luego Events y Objects hasta dos pases seguidos, robo de Hand Size y efectos de fin de Pre-match),
  límite de 5 cartas Pre-match y 5 Mid-match en el Ring, una sola Venue y un solo Pay-Per-View, cartas
  Mid-match jugadas y revertidas desde el Backlash, y count out avanzado.
- Mecánicas de Fully Loaded: cartas del Ring con habilidad propia ("Usar"), auras mientras están en el Ring,
  cartas que no se pueden revertir según la Fortitude o la primera del turno, Reversals solo desde el Arsenal o
  solo por una carta, bonos por varios turnos, prohibiciones de jugar cartas al rival, mostrar la mano, buscar
  varias cartas, devolver cartas del Ring a la mano, y las habilidades de las 9 Superstars nuevas (incluida la de
  Chyna, que revierte descartando una carta del mismo tipo).
- Mecánicas de Backlash: cartas del Ring que se mandan al Ringside para revertir (botón "Revertir con … (del
  Ring)"), efectos después del daño si la carta no fue revertida, efectos antes del Reversal (Half Hour Suplex),
  "tu próxima carta tiene que ser …", bonos para las próximas N cartas, daño doble, cartas con logos de varias
  Superstars, cartas que solo ciertas Superstars pueden (o no pueden) llevar, Hand Size según el Superstar Value
  rival (Edge), habilidades una vez por partida y las de las 10 Superstars nuevas, cartas Pre-match y Mid-match del
  set (incluida la que anula la próxima carta Pre-match del rival).
- Survivor Series: Cactus Jack, Deadman Inc y sus 10 cartas, y Stephanie Interferes (Triple H), que no puede ir en
  el mismo mazo que Chyna Interferes.
- Validación de mazos: 60 cartas, máx. 3 copias (las reimpresiones con el mismo título cuentan juntas), Unique, Set-Up, conflictos Face/Heel, Raw/SmackDown,
  Fan Favorite/Cheater, cartas específicas de Superstar.

## Interpretaciones de reglas (confirmadas por Simon, 2026-10-14 y 2026-10-15)

- Un Reversal volteado durante el daño detiene el daño automáticamente (no es opcional).
- Las Acciones hacen su D impreso como daño (casi todas tienen D 0).
- Rechazar un Count Out vale para el resto de la partida.
- El primer jugador también roba en su primer turno.
- Multi: el primer Reversal jugado desde la mano queda en el Ring sin efectos ni daño, y el rival puede
  jugar otro. Un Reversal volteado durante el daño también cuenta como uno de los dos.
- Chain: el costo de descartar 2 solo aplica al revertir desde la mano o el Backlash, no al voltear un
  Reversal durante el daño.
- Las cartas Backlash que llegan al Ringside nunca vuelven a la mano ni al Arsenal.
- El texto de un Reversal que no dice "If played from your hand" también se aplica cuando se voltea durante
  el daño (por ejemplo, Manager Interferes roba 1 en los dos casos).
- Una carta híbrida jugada como Acción no hace daño (Chop, Undertaker's Tombstone Piledriver).
- Jockeying for Position: el +4D o el +8F se elige al jugar la carta.
- Disqualification! solo revierte maniobras HEEL; revertir un Reversal todavía no existe en el motor.

## Pendiente

Volley, Enforcers, Tag Team (y sus cartas de Premiere: Cheap Shot From the Corner, Tag in Partner, Double Team,
Trip, Partner Interference y la parte de compañero de Combination Attack), revertir Reversals, y modificadores
de Superstar Value durante el Pre-match.

Fully Loaded: no se pueden jugar todavía Surprise Hit y Double Clothesline (rival inactivo de Tag Team), Stare Down
Opponent, Do You Like Pie?, Doing the Job, Predictable Opponent, Defensive Posture, Inferno Match, Kick Out!, Rabid
Wolverine y Let's Make Some Noise .... Se juegan sin una parte de su texto: Trash Can (volver a usarla desde el
Ring), Series of Suplexes (repetirla), Small Package (ganar con tres seguidas), Attitude Adjustment (el -#F del turno
siguiente), Huge Bump, Crippler Crossface (Maintain Hold), Ref KO'd! y That's Gonna Cost You! (revertir Reversals), y
Masked Vengeance descarta al azar de toda la mano.

Backlash: no se pueden jugar todavía Pop the Guy On the Apron, Tandem Atomic Drop, Propel Partner y Jump the Bell
(Tag Team), Don't Mess with the Champ! (no hay Title Belts aún), The Brahma Bull y Over Sell Maneuver. Se juegan sin
una parte de su texto: la habilidad de Edge and Christian, la de Edge (empezar después del Pre-match), la de Dudley
Boyz (sus Foreign Objects siguen siendo Heel), Crucifix Rollup (el pin solo como maniobra), Clutch onto Opponent
(Maintain Hold), Blown Spot (el -#F), Lita to the Xtreme (seguir el turno tras un Reversal), Swanton Bomb (buscarla
como High Risk), Con-Chair-To (solo si la revierten desde la mano) y Captive Tag Out (sin compañero). Adrenaline Rush,
Study the Tapes, Student of the Sport y Big Slide Into the Ring están simplificadas.

Survivor Series: todas sus cartas se juegan. Dead Man Walking manda 4 cartas del Arsenal rival al Ringside sin
contarlas como daño.

## Cartas y arte

El texto y el arte de las cartas de Raw Deal pertenecen a sus dueños. Este repositorio solo trae
cartas de prueba inventadas; la base de cartas real se carga desde archivos del usuario.

Para jugar con tus propias cartas, pon un archivo `.ron` por set en una carpeta (mismo formato que
[`crates/rawdeal-cards/data/test-set.ron`](crates/rawdeal-cards/data/test-set.ron)) y tus mazos en
archivos `.ron` como este:

```ron
(superstar: "premiere/stone-cold", arsenal: ["premiere/001", ...], backlash: [])
```

```sh
cargo run -p rawdeal-cli -- --cards ~/raw-deal/sets --deck1 mazo1.ron --deck2 mazo2.ron --check  # validar
cargo run -p rawdeal-cli -- --cards ~/raw-deal/sets --deck1 mazo1.ron --deck2 mazo2.ron          # jugar
```

Sin archivos de mazo, `--superstar1`/`--superstar2` arman un mazo rápido para esa Superstar:

```sh
cargo run -p rawdeal-cli -- --cards ~/raw-deal/sets --superstar1 premiere/stone-cold-steve-austin --superstar2 premiere/the-rock
```

### Importar desde OCTGN

Los sets del juego Raw Deal de OCTGN (`set.xml`) se convierten con:

```sh
cargo run -p rawdeal-cards --bin octgn-import -- <set.xml> Premiere premiere ~/raw-deal/sets/01-premiere.ron \
    --overlay crates/rawdeal-cards/scripts/premiere.ron
```

El importador copia tipo, F, D, SV, rasgos, logo de Superstar y texto. Deduce qué revierte cada Reversal a
partir del tipo y de frases comunes del texto. Las habilidades de Superstar y los efectos de cada carta vienen
del archivo de scripts (`crates/rawdeal-cards/scripts/premiere.ron` para Premiere), escrito a mano a partir del
texto de las cartas. En otros sets, el texto sin script es solo informativo.

Para probar partidas aleatorias con tus sets: `OPENWWX_CARDS=~/raw-deal/sets cargo test -p rawdeal-cards --test user_sets`.
