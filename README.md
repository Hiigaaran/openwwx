# OpenWWX

Implementación en Rust del juego de cartas coleccionables **Raw Deal** (WWF/WWE), para escritorio.

Estado: motor de reglas (Quick Start, Advanced, Virtual y Revolution) con los 53 sets de OCTGN (los 26 oficiales,
de Premiere a Revolution, los Virtual hechos por fans, Promos y Prize Cards) y sus efectos programados; app de escritorio para jugar contra
la computadora en tres niveles de dificultad, con animaciones y arte opcional importado de OCTGN; y juego en línea
directo o por un servidor común, con espectadores, chat, cuentas, historial, ranking y amigos. El diseño completo está
en [docs/motor-de-reglas.md](docs/motor-de-reglas.md).

![Una lucha contra la computadora: mano, Ring de cada Superstar, visor de la carta y relato](docs/img/partida.png)

*Las capturas usan el set de prueba del repositorio, con cartas y Superstars inventados: OpenWWX no incluye texto
ni arte de Raw Deal.*

## Crates

| Crate | Qué hace |
|---|---|
| `rawdeal-core` | Motor de reglas puro y determinista: estado, turnos, daño, reversals, victoria. |
| `rawdeal-cards` | Carga de cartas desde archivos RON. Incluye un set de prueba con cartas y Superstars inventados. |
| `rawdeal-ai` | Oponentes por computadora en tres niveles: Fácil, Normal y Difícil. |
| `rawdeal-net` | Juego en línea: protocolo, partida hospedada (quien hospeda corre el motor), conexión de jugadores y espectadores, y el servidor común `openwwx-server` (lista de partidas y reenvío de mensajes). |
| `rawdeal-cli` | Cliente de texto para jugar contra el bot y depurar el motor. |
| `openwwx` | App de escritorio (egui): menú, tablero, mano, decisiones y relato de la lucha. |

## Jugar

Descarga la versión para tu sistema desde [Releases](https://github.com/Hiigaaran/openwwx/releases), descomprímela,
pon tus sets `.ron` en la carpeta `cards` y abre `openwwx`.

![El menú: cartas, Superstars, dificultad, reglas y la sección En línea](docs/img/menu.png)

Antes de empezar eliges la dificultad de la computadora:

- **Fácil**: juega cartas al azar.
- **Normal**: revierte siempre que puede, ataca con sus maniobras más fuertes y descarta las cartas que menos le sirven.
- **Difícil**: prueba cada jugada posible contra varias suposiciones de tu mano y de su Arsenal, juega el resto del
  turno en cada una y elige la que mejor le resulta. No ve tus cartas ocultas: solo sabe lo que sabría un jugador en
  la mesa.

En 800 partidas entre bots con los 53 sets (`cargo run --release -p rawdeal-cards --example arena -- <carpeta> 800`),
Normal le gana a Fácil el 66% de las veces, y Difícil le gana a Fácil el 73% y a Normal el 58%.

La mesa repite cada jugada con animaciones: la carta vuela de la mano a "En juego", las cartas volteadas por daño
caen al Ringside con un destello rojo, y los avisos grandes marcan turnos, reversals, daño recibido y manos mostradas.
La velocidad se cambia arriba del relato (lentas, normales, rápidas o sin animaciones) y "Saltar" las termina al
instante.

![Aviso de turno durante las animaciones](docs/img/animacion.png)

Las Superstars de la lucha aparecen como cartas junto a sus pilas, y el visor a la izquierda del relato muestra en grande
la última carta por la que pasó el mouse. Sin arte, cada carta se dibuja con la estructura de Raw Deal (título,
Fortitude y Daño, dibujo según su tipo, línea de tipo, texto y Stun Value). Si en la carpeta de cartas (o en una
subcarpeta, por ejemplo `art`) hay imágenes `.png` o `.jpg` cuyo nombre es el GUID de OCTGN de la carta, su id o su
título, se usa la imagen tal cual. Con "Importar imágenes de OCTGN (.o8c)…" en el menú eliges uno o varios paquetes de
imágenes de OCTGN y el juego copia sus imágenes a la subcarpeta `art` de tu carpeta de cartas; como vienen nombradas
por el GUID de cada carta, todas las cartas importadas desde OCTGN quedan con su imagen.

### En línea

En la sección "En línea" del menú se juega contra otra persona:

- **Hospedar partida**: tu computadora corre la partida con las reglas elegidas arriba, y juegas con tu Superstar y tu
  mazo. La sala de espera muestra la dirección (IP y puerto, 7777 por defecto) a la que se conectan los demás. Fuera de
  tu red local hay que abrir ese puerto en el router.
- **Jugar**: te unes a la partida de esa dirección como el otro jugador, con tu Superstar y tu mazo.
- **Mirar**: entras como espectador. Ves lo mismo que vería alguien sentado junto al ring: el Ring, el Ringside, cuántas
  cartas tiene cada uno en la mano y lo que se juega, pero no las manos.

Con un **servidor común** nadie tiene que abrir puertos (solo la máquina del servidor). Alguien corre
`openwwx-server` (viene en cada release; escucha en el puerto 7778, o el que se le pase como argumento) en una máquina
accesible desde internet. En la app se escribe su dirección en "Servidor común": "Publicar partida" crea una partida
con las reglas elegidas y la pone en la lista, y "Ver partidas" muestra las partidas abiertas (quién juega, si están
esperando rival o jugando, cuántos miran) con botones para jugar o mirar. La lista se actualiza sola. El servidor no
corre las partidas ni necesita las cartas: solo reenvía los mensajes entre quien hospeda y quienes se unen.

En el servidor común cada jugador tiene una **cuenta**: en "Tu cuenta" se escribe nombre y contraseña y se pulsa "Crear
cuenta" la primera vez y "Entrar" después. El nombre queda reservado en ese servidor (sin distinguir mayúsculas) y es
el que ven los demás; la app recuerda la sesión, así que la contraseña se escribe una vez. Publicar, jugar y mirar en
el servidor piden haber entrado. La conexión con el servidor va cifrada (TLS): el servidor crea su propio certificado
la primera vez, la app lo recuerda, y si después cambia avisa antes de conectarse ("Confiar en el certificado nuevo"
solo si el administrador dijo que lo cambió). Las contraseñas se guardan como hash Argon2. No hay correo para
recuperarlas: el administrador pone una nueva con `openwwx-server --reset-password NOMBRE CONTRASEÑA`. El servidor
guarda cuentas y certificado en la carpeta `openwwx-server-data` (o la que se pase con `--data`), y al arrancar
muestra la huella del certificado. Las partidas directas (Hospedar / Unirse directo) siguen sin cuentas ni cifrado.

![Sesión abierta en el servidor: amigos, una invitación, una solicitud y la lista de partidas](docs/img/servidor.png)

El servidor guarda el **historial** de cada partida terminada entre dos cuentas y lleva un **ranking** con puntaje Elo
(todos empiezan con 1000). "Ranking" muestra los mejores jugadores del servidor y "Mi historial" tus últimas
partidas (rival, resultado, reglas y puntos ganados o perdidos); un clic en un nombre abre su historial. Si quien
hospeda se va con la partida empezada, pierde por abandono. El resultado lo informa la app de quien hospeda, así que
una app modificada podría mentir; el servidor no corre las partidas.

![Ranking del servidor e historial de una jugadora](docs/img/ranking.png)

Con la sesión abierta aparece la sección **Amigos**: "Agregar amigo" le manda una solicitud a ese nombre, y son amigos
cuando la acepta. La lista muestra quién está en línea. Mientras esperas rival en una partida publicada en el servidor,
la ventana "Invitar amigos" invita a los que están en línea, y a ellos les aparece la invitación con botones para
jugar o mirar.

![Sala de espera de una partida publicada, con la ventana para invitar amigos](docs/img/sala.png)

Durante la partida hay un chat arriba del relato. Si a un jugador se le corta la conexión, la app vuelve a conectarse
sola y lo devuelve a su asiento; si no vuelve en 2 minutos, pierde por abandono, y "Salir" en medio de la partida
también cuenta como abandono. Si sale quien hospeda, la partida se cierra para todos.

![Partida en línea vista por el invitado, con el chat en el relato](docs/img/en-linea.png)

Todos necesitan la misma versión de OpenWWX y los mismos sets de cartas; si no coinciden, la sala de espera dice qué
sets faltan o son distintos. Quien hospeda valida los mazos con las reglas de la partida. Nadie recibe las cartas
ocultas de los demás.

### Compilar

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
cargo run -p rawdeal-cli                    # jugar contra el bot (--level easy|normal|hard)
cargo run -p rawdeal-cli -- --watch --seed 3  # ver una partida bot contra bot
cargo run -p rawdeal-cli -- --host --name Ana          # hospedar una partida en línea (puerto 7777)
cargo run -p rawdeal-cli -- --join 192.168.0.10 --name Beto  # unirse como jugador
cargo run -p rawdeal-cli -- --spectate 192.168.0.10    # mirar
cargo run -p rawdeal-net --bin openwwx-server           # servidor común (puerto 7778)
cargo run -p rawdeal-cli -- --list mi-servidor.com      # partidas abiertas en el servidor
cargo run -p rawdeal-cli -- --server mi-servidor.com --host --name Ana    # publicar una partida
cargo run -p rawdeal-cli -- --server mi-servidor.com --join 3 --name Beto # unirse a la partida 3
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
  jugar maniobras Multi, salvo las de su propio logo) y Chain (revertirla desde la mano o el Backlash cuesta descartar 2; si tiene éxito,
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
- It's All About The Game: Superstars que cuentan como otras (Two-Man Power Trip es Stone Cold y Triple H;
  Brothers of Destruction es Undertaker, Deadman Inc y Kane) para los efectos y para armar el mazo, efectos de
  Superstar al terminar el Pre-match, cartas del Ring que dejan en blanco la habilidad de la Superstar rival
  (Critical Injury) o el texto de sus maniobras (Debilitating Injury: Arm/Elbow), opciones que elige el rival
  (Backed by Mr. McMahon) y maniobras Strike + Grapple.
- Mania: las 9 Superstars nuevas (Big Show, Lita, Rob Van Dam, Booker T, APA, Spike Dudley, William Regal y
  Raven), costo en descartes para jugar maniobras, tope de daño a las maniobras rivales, cartas que no se
  pueden llevar según tipo y daño, efectos que cuentan cartas (mano, Fortitude, Stun Value, cartas del Ring),
  habilidades que reaccionan a las maniobras del rival, cartas del Ring que te limitan a ti, Hand Size extra por
  cartas del Ring y bonos que impiden revertir desde la mano o el Backlash.
- Summerslam, Ruthless Aggression, Velocity, Survivor Series 2, Insurrextion y Absolutely Raw and Ultimate
  Smackdown: sus Superstars y cartas con los mecanismos que ya existían. Las Superstars femeninas se reconocen por
  nombre (OCTGN no lo indica), y los títulos que nombra una carta se comparan sin mayúsculas ni puntuación.
- Divas Overload, Wrestlemania Tin, Vengeance, Summerslam Tin, Lethal Library, Survivor Series 3, Armageddon,
  Unforgiven, Royal Rumble, No Way Out y Great American Bash: sus Superstars y cartas con los mecanismos que ya
  existían. Ahora se leen los rasgos Fan Favorite, Cheater y Universally Unique, y "Volley" reconoce "Volley: 6".
- Mecánicas nuevas en todos los sets: cartas retiradas del juego, cartas que se juegan o se usan desde el
  Ringside, Superstars femeninas, Divas y otros rasgos de Superstar (Tag Team, Legend, GM) para armar el mazo y en
  los efectos, llevar cartas de otra Superstar sin contar como ella, cambios de Superstar Value, de los límites de
  Pre-match y Mid-match y del Hand Size rival, cartas que bajan su F descartando, reacciones cuando el rival saca
  una carta de tu Ring y cartas que no se pueden revertir desde el Backlash.
- Segundo lote: mover cartas entre zonas (devolver cartas retiradas, poner cartas del Ringside en el Ring),
  Reversals usados desde el Ringside, efectos de "jugada desde el Ringside", condiciones por cualquier conteo,
  cartas que vuelven arriba o al fondo del Arsenal sin barajar y costos de retirar cartas. Unas 80 cartas más de
  todos los sets quedan completas o jugables.
- Tercer lote: cartas jugadas que van arriba o al fondo de su Arsenal, mover cartas al fondo del Arsenal, elegir
  cartas del Arsenal, Reversals que solo se usan desde el Ringside, Reversals que retiran del juego la carta
  revertida o a sí mismos, comparar dos conteos y nuevos conteos (cartas retiradas por tipo, Fortitude rival, media
  mano). Unas 60 cartas más quedan completas o jugables.
- Cuarto lote: cartas del Ring que reaccionan cuando el rival revierte (por ejemplo, recuperar la carta revertida),
  mover cartas encima del Arsenal y aritmética de conteos (restar, mitad, tope). Unas 25 cartas más quedan
  completas o jugables.
- Quinto lote: saber de dónde vino el Reversal del rival (mano, Backlash, Ringside o volteado del Arsenal) y qué
  carta revirtió, reacciones a los Reversals también en Superstars y en el Ringside, y sumas de conteos. Unas 15
  cartas más quedan completas.
- Sexto lote: seguir jugando el turno después de un Reversal, reacciones cuando tú reviertes y saber de dónde vino
  tu último Reversal. Unas 25 cartas más quedan completas o jugables (por ejemplo Rey Mysterio, Lita to the Xtreme,
  I Won't Stop y J.R. Style Push).
- Cartas debajo de otras: poner cartas debajo de una carta (de la mano, el Arsenal o el Ringside), recuperarlas,
  jugarlas como si estuvieran en la mano y que se vayan con su carta cuando sale del Ring. Unas 15 cartas que no se
  podían jugar ahora se juegan (por ejemplo We Lie, We Cheat, We Steal, Hide and Seek y Size Matters).
- Heat, Volley, Permanent y Title Belts (reglas del rulebook y del OmniFAQ 3.7.1): con 6 o más cartas Heat en tu
  Ringside juegas una desde allí por turno, y si un efecto tuyo saca cartas de tu Ringside primero se retiran tus Heat;
  las cartas Volley se pueden revertir retirando # cartas al azar del Ringside; las cartas Permanent y las
  Stipulations solo salen del Ring por su propio texto; los Title Belts son Universally Unique. Unas 25 cartas nuevas
  se juegan o quedan completas (por ejemplo Messing With the Champ, Here I Stand: The Champion y Double Axe Handle).
- Responder a un Reversal: justo después de que el rival revierta tu carta, puedes revertir su Reversal
  (Disqualification! contra Reversals HEEL) o anularlo con una Action como Ref KO'd! (ni tu carta ni su Reversal hacen
  daño y tu turno sigue). That's Gonna Cost You! también se juega ya.
- Enforcers y Backstage Area: las cartas Enforcer van en el Backstage Area del mazo; las Superstars que pueden
  revelarlas (GMs y Divas como Eric Bischoff, Stephanie McMahon, Stacy Keibler o Paul Heyman) eligen 1 Enforcer antes
  de la partida, que suma a su mano inicial, su Superstar Value y su habilidad, y algunas cartas cambian de Enforcer
  (por ejemplo You're Fired! y Suplex into the Ring).
- Sets Revolution (24-26): cartas Antic (solo en tu turno, las Reversals de Action no las paran), maniobras Throw,
  Hold, Assault, Trademark y Extreme con sus tamaños Core, Mean y Colossal, Momentum (solo lo revierten las cartas que
  revierten Momentum), Follow-up (solo después del tipo de carta indicado, sin límite de copias si no es Unique),
  Targets, Storylines (reemplazan la Superstar Ability mientras están en el Ring) y Pre-match Corner y Faction. El
  formato Revolution se elige en el menú (o `--revolution` en el CLI): solo cartas Revolution, Backlash de 6 Pre-match y
  6 Mid-match, máximo 3 de cada en el Ring, manos antes del Pre-match y regla de agresión (si terminas el turno sin
  intentar una maniobra, volteas 6 cartas). Cada una de esas cinco reglas se puede activar o desactivar antes de
  empezar la partida (casillas bajo "Formato Revolution", o en el CLI `--no-revolution-only`, `--no-small-backlash`,
  `--no-ring-limit`, `--no-early-hands` y `--no-aggression`). Las mecánicas de las cartas (Antic, Momentum, Targets,
  etc.) siempre se aplican, porque están en el texto de las cartas.
- Sets Virtual (27-51, hechos por fans en teamcanadaonline.net), Promos y Prize Cards: cartas Backstage (van en el
  Backstage Area, se revelan al empezar y dan habilidades; algunas se esconden para usar su efecto una vez), Pre-match
  Allegiance (su propio paso antes del Venue), TMF, tipos con rasgos ("Strike: Heel", "Action: Movie") y el formato
  Virtual en el menú (o `--virtual` en el CLI): Backlash de 12 Pre-match y 12 Mid-match, máximo 6 de cada en el Ring.
  Dos cartas distintas con el mismo título en un set se distinguen por su último logo ("Dream Street (The Legacy)") o
  por un número ("Trip (2)").
- Textos que cambian cómo se revierte una carta: Multi o Chain con condición ("si es la primera carta del turno"),
  cartas que necesitan 3 Reversals, cartas que solo se revierten desde ciertos lugares ("solo desde el Arsenal",
  "no desde la mano"), bonos y auras multiplicados por un conteo ("-#F, donde # es el Superstar Value del rival") y
  cartas Pre-match que no ocupan espacio en el Ring.
- Cartas que se juegan después de una maniobra exitosa del rival (desde la mano, antes de sus efectos, antes de su
  daño o después de él) y terminan su turno; algunas anulan lo que falta de la maniobra. Cartas boca abajo en el Ring:
  quedan en blanco (sin tipo, rasgos ni texto, F: 0 D: 0) hasta que se dan vuelta o salen del Ring. Cartas debajo de
  la Superstar, que algunas habilidades dejan jugar como si estuvieran en la mano ("debajo de una carta Backstage" se
  trata igual).
- Revertir Activated Card Effects (ACEs) de cartas del Ring, del Ringside o Backstage, desde la mano o desde el
  Ring (el costo se paga igual y el efecto no ocurre); cartas del Ring que paran una maniobra exitosa; cartas que solo
  ciertos Reversals pueden revertir ("no por cartas non-Unique"); Reversals que cuentan como 2 y Reversals que
  ignoran las restricciones de reversión.
- ACEs de cartas de tu Ring que el rival puede usar en su turno pagando el costo, ACEs que no se pueden revertir,
  bonos que dejan la próxima maniobra revertible solo por ciertas cartas ("solo por cartas Unique"), cartas del Ring
  que reaccionan cuando el rival juega una carta (al jugarla, antes de cualquier Reversal, o una vez por turno, con
  su dueño eligiendo) y Reversals que cuentan como 2 solo al voltearse recibiendo daño.
- Validación de mazos: 60 cartas, máx. 3 copias (las reimpresiones con el mismo título cuentan juntas), Unique, Set-Up, conflictos Face/Heel, Raw/SmackDown,
  Fan Favorite/Cheater, cartas específicas de Superstar.

## Interpretaciones de reglas (confirmadas por Simon, 2026-10-14 y 2026-10-15)

- Un Reversal volteado durante el daño detiene el daño automáticamente (no es opcional).
- Las Acciones hacen su D impreso como daño (casi todas tienen D 0).
- Rechazar un Count Out vale para ese turno: se vuelve a ofrecer al final del turno siguiente (confirmado por
  Simon el 2026-10-21; antes valía para toda la partida).
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
- Disqualification! solo revierte maniobras HEEL.
- Una carta boca abajo en el Ring es una carta en blanco: sin tipo, rasgos ni texto, F: 0 y D: 0 (confirmado por
  Simon el 2026-10-24).
- "Debajo de una carta Backstage" se trata igual que debajo de la Superstar (confirmado por Simon el 2026-10-24).

## Pendiente

Tag Team (y sus cartas de Premiere: Cheap Shot From the Corner, Tag in Partner, Double Team, Trip, Partner
Interference y la parte de compañero de Combination Attack). Todos los sets de OCTGN (01-51, Promos y Prize Cards)
están importados.

Las cartas de cada set que todavía no se pueden jugar o que se juegan simplificadas están en
[docs/pendiente.md](docs/pendiente.md).

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
