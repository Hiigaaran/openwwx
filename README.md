# OpenWWX

Implementación en Rust del juego de cartas coleccionables **Raw Deal** (WWF/WWE), para escritorio.

Estado: paso 1 del roadmap, el motor de reglas (reglas Quick Start). El diseño completo está en
[docs/motor-de-reglas.md](docs/motor-de-reglas.md).

## Crates

| Crate | Qué hace |
|---|---|
| `rawdeal-core` | Motor de reglas puro y determinista: estado, turnos, daño, reversals, victoria. |
| `rawdeal-cards` | Carga de cartas desde archivos RON. Incluye un set de prueba con cartas y Superstars inventados. |
| `rawdeal-ai` | Oponentes por computadora. Por ahora, un bot aleatorio. |
| `rawdeal-cli` | Cliente de texto para jugar contra el bot y depurar el motor. |

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
- Efectos: robar, descartar (elegido o al azar), recuperar del Ringside, poner cartas en el fondo del Arsenal,
  daño extra, efectos opcionales.
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
- Validación de mazos: 60 cartas, máx. 3 copias, Unique, Set-Up, conflictos Face/Heel, Raw/SmackDown,
  Fan Favorite/Cheater, cartas específicas de Superstar.

## Interpretaciones de reglas (confirmadas por Simon, 2026-10-14)

- Un Reversal volteado durante el daño detiene el daño automáticamente (no es opcional).
- Las Acciones hacen su D impreso como daño (casi todas tienen D 0).
- Rechazar un Count Out vale para el resto de la partida.
- El primer jugador también roba en su primer turno.

## Interpretaciones de reglas por confirmar

- Multi: el primer Reversal jugado desde la mano queda en el Ring sin efectos ni daño, y el rival puede
  jugar otro. Un Reversal volteado durante el daño también cuenta como uno de los dos.
- Chain: el costo de descartar 2 solo aplica al revertir desde la mano o el Backlash, no al voltear un
  Reversal durante el daño.
- Las cartas Backlash que llegan al Ringside nunca vuelven a la mano ni al Arsenal.

## Pendiente

Volley, Enforcers, Tag Team y modificadores de Superstar Value durante el Pre-match.

## Cartas y arte

El texto y el arte de las cartas de Raw Deal pertenecen a sus dueños. Este repositorio solo trae
cartas de prueba inventadas; la base de cartas real se carga desde archivos del usuario.
