# OpenWWX

Implementación en Rust del juego de cartas coleccionables **Raw Deal** (WWF/WWE), para escritorio.

Estado: motor de reglas (Quick Start y Advanced), app de escritorio y los sets Premiere, Fully Loaded, Backlash,
Survivor Series, It's All About The Game, Mania, Summerslam, Ruthless Aggression, Velocity, Survivor Series 2,
Insurrextion y Absolutely Raw and Ultimate Smackdown con sus efectos programados. El diseño completo está en
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

It's All About The Game: no se puede jugar todavía Calling You Out. Se juegan simplificadas: Two-Man Power Trip
(puede llevar todas las cartas de Stone Cold y Triple H, no solo las Foil), Rollin' - Rollin' - Rollin'! (el +2D va a
tu próxima maniobra de ese tipo, no solo a la carta buscada), Throttled Within an Inch of Your Life (se juega como
Strike o como Grapple) y Backed by Mr. McMahon (la mano boca arriba solo se muestra una vez).

Mania: no se pueden jugar todavía Playing by the Rules, Veteran Referee: Tim White, Messing With the Champ (no hay
Title Belts aún), Keibler's Cookies, Product Endorsements y Snubbed by the Fans (cambian el Superstar Value), Not
Done with Any Flair, Indian Strap Match, Fortitude Surge, Second Wind, Sustained Damage, Happy You're Here, Happier
You're Gone, Test of Strength, Foley is Good!, Would You Please ... Shut the Hell Up!, TLC Match y Censorship Match.
Se juegan sin una parte de su texto: Rob Van Dam (Chair Shot sigue siendo Heel, sin espacios extra de Mid-match),
Spike Dudley (sin cambio del límite de Mid-match), APA (las "Hand cards" son las maniobras), Pump Kick, Indian
Deathlock, X-treme Measures, Trailer Park Trash!, Torrie Wilson, On It!, Five Star Frog Splash, Cerebral Assassin,
Gettin' Cheeky with It!, Superior Training, Slandered Online, No Disqualification Match, Personal Appearance,
Chicago Street Fight, Four Corners Match y los Touch Turnbuckle (no vuelven al Backlash), Hell in a Cell Match (no
bloquea el Ringside) y Flying Tope (su texto no dice qué revierte). Están simplificadas: Catapult y Over the Top
Rope (la carta siguiente ignora cualquier "solo después de"), Out Think the Fink, Stratusfied (mira 5 cartas),
Totally Bogus! (revierte cualquier carta que haga robar), Cole Calls It Right!, Crimson Goddess y Spinnerooni (la
maniobra revertida no va al Ring rival), What???, You Will Respect Me, Duchess of Queensbury Rules y Handicap Match.

Summerslam: no se pueden jugar todavía Tag Out e It's All in the Teamwork! (Tag Team), Living Legend y Just Who in
the Blue Hell Do You Think You Are? (Superstar Value y límite de cartas Mid-match), Use 'em or Lose 'em y That's a
Near Fall! (saber qué intentó jugar el rival o cuánto daño recibiste), Hulkin' Up (jugarla tras una maniobra que
tuvo éxito), Been There, Done That (títulos de tu propio Ring), Play the Game! y Smart Mark (reaccionar a las
Reversals o barajadas del rival), I'm Gonna Try That Again (copias de una carta del Ringside), Give It All I Got, My
God! He's Broken in Half!, In the Presence of the Kanenites y The Mood Is About to Change (contar cartas de la mano o
la mitad de tu Fortitude), Bra and Panties Match y Managed by Stephanie McMahon-Helmsley (dejar en blanco una
Stipulation), Lethal nWo Poison (sacar cartas del juego), Outsider Distraction (cambiar el Draw Segment desde el Ring)
y Raven's Shopping Cart (cartas debajo de otra). Se juegan sin una parte de su texto: Inverted Body Block, Booby
Trap, Managed by Torrie Wilson y Managed by Terri Runnels (revertir cartas que miran tu mano, quitan cartas del Ring
o buscan en el Arsenal), Back Splash y Kangaroo Kick (no revertir desde el Backlash), Knife-Edge Chop (límite tras
Irish Whip), Springboard (Strike como High Risk), Mind Games (las copias del Arsenal), Beating the Odds, Go For the
Cover! (Kick Out!), There Are Two Things You Can Do y Whatchoo Talkin' 'bout, Ese? (la F de la acción revertida),
Silent, But Violent (las acciones de la mano al fondo del Arsenal), Head y Do YOU Live By the Three I's? (Permanent),
Edge-acution (volver desde el Ringside), Hardcore 'Til the End, Stratusfaction Bulldog, To Be the Man y Tell Me...
(Title Belts), y los Superstars Ric Flair, Hall and Nash, Billy and Chuck y Al Snow. Están simplificadas: las Feuds
Do You Know What My Watch Says? e I'm the Biggest Dog on This Block! (añaden la habilidad nueva sin quitar la
propia), los cambios de mano inicial (se aplican al final del Pre-match), Pick Your Spots (dura toda la partida),
It's Showtime! (el rival roba antes de perder el turno), Ring General (devuelve las 2 cartas antes de robar), Atomic
Back Body Drop, Tilt-a-Whirl Powerslam y Caped Body Press (las Reversals permitidas van por título), Not According to
the Fine Print! (revierte cualquier carta) y la habilidad de Hurricane (se usa tras la maniobra, en el Main Segment).

Ruthless Aggression: todas sus cartas se juegan, pero la habilidad de Dangerous Alliance no hace nada todavía (robar
cuando el rival roba, Handicap Match contra Tag Teams). Se juegan sin una parte de su texto: Do You Know Who I Am? (el
requisito de 3 de Stun Value en tu Ring), Look Into My Eyes ... Do You Trust Me? (buscar cuando el rival la quita del
Ring), So Long ... Goodbye! (retirar del juego la carta revertida) y Managed by Vince McMahon (el límite de Pre-match
menor). Están simplificadas: la habilidad de The People's Team (con 3 tipos ya impide también revertir desde el
Backlash y un híbrido cuenta por dos tipos), Do You Know Who I Am? (el rival puede pasar del tope de 4 cartas mientras
busca un Reversal) y Managed by Vince McMahon (anular una Stipulation, el Title Belt y el hueco extra de Mid-match son
una opción que no hace nada).

Velocity: no se pueden jugar todavía Did I Just Say ... Three Minutes?, Managed by Shane O'Mac y My Sacrifice (quitar
cartas del juego), I'm the Champ - Nothin's Gonna Change That! y Title Belt Clubberin' (no hay Title Belts aún), Hot
Tag! (Tag Team), Backstage Warm-up Routine y Red Hook's Premiere Announcer (cambian la mano inicial del rival), A
Victim of the Crippler (cambia la habilidad del Superstar), Hold the Phone!, Ring Psychology: Neck, Pay-Per-View Main
Event, Raw or Smackdown - Pick a Brand!, See How It Feels, The Ref Got in the Way!, Hold On! It's Not Time to Show
Yet!, Beg For Mercy y Luche Libre Extravaganza. Se juegan sin una parte de su texto: Rey Mysterio (seguir el turno
tras un Reversal), 3 Minute Warning y Jamie Noble (Title Belts y Rico Enters!), Flying Mare, Shoot Punch, Shoot
Slam, Raisin' the Roof y Everything's Cool When You're... (reaccionar cuando el rival las saca del Ring), Shoot
Headlock y Regal Upper Class Punch (recuperarlas del Ringside), Roll Up y Lunging Choke Hold (su tipo no es Reversal,
así que no revierten), Lariat Takedown (no comprueba las 4 cartas del Backlash), Waist Lock y Singapore Cane (no
dejan texto en blanco), Gettin' Beat Like a Government Mule (no repite el daño), Get the 'F' Out! (no revierte del
todo una Multi), Back-and-Forth Action, Defensive Stance, He's Runnin' Like a Scalded Dawg, Ring Psychology: Back
(solo la parte de 6 cartas), Managed by Eric Bischoff, Managed by Nidia, Dragged to the Center of the Ring, The
Game's Sleeper y Hurrislam ("puede ser revertida por"), Remove the People's Elbow Pad, Jamal's Top Rope Splash, Now
I'm Gonna Get Some Respect, Just Hold On a Second, Mister!, Yo, Ese, I Know You, Homes!, Dudley Tough, Never Gonna
Stop Me, Five Time! Five Time! Five Time! Five Time! Five Time!, When Hulkamania Runs Wild on You! y !eM pleH. Están
simplificadas: Big Freak'n Machine, The Game y The People's Champ (cuentan como Kane, Triple H y The Rock), Bionic
Elbow, Director's Cut y F-5 (los Reversals del Backlash son los de Mid-match), Breather Hold (juegas con la
Fortitude del rival, no con una menos), Here's a Ratings Booster! (-2F por cada mitad de la Fortitude rival), Don't
You Usually Wrestle in Dark Matches? e It Doesn't Matter What Your Name Is! (el Superstar Value 2 o menos se mira
por nombre), Wanna Know What I'm Gonna Do? (las cartas se barajan en el Arsenal), Velocity (sin tope de 5), I Aims ta
be Startin' Sump'tin', A Shattered Dreams Production (el rival elige la carta), He's as Crazy as a Pet Raccoon,
Measure Him, Unorthodox Style of Wrestling, Check This Out!, The Fire Still Burns, It's All About Control (va al
Ringside al usarla), The Next Big Thing, The West Coast Pop y Go On - Give 'im Some Sugar!.

Survivor Series 2: no se puede jugar todavía Split Personalities Have Benefits (Superstars ocultos en el Backstage). Se
juega sin una parte de su texto la habilidad de Dude Love (el robo extra del rival y llevar a Mankind o Cactus Jack
ocultos).

Insurrextion: no se pueden jugar todavía Blind Tag, Pump Up the Crowd, Give and Take, Cut the Ring in Half y Glad
Someone Has My Back (Tag Team), Dude, Try Something Else y Fall into the Corner (seguir el turno tras revertir), What's
Mine is Mine and What's Yours is Mine, We Lie, We Cheat, We Steal, Raw Roulette Wheel y Stacy Keibler, Marketing
Manager (cartas debajo de otra o sacadas del Ringside al Ring), Mangled Intro by Lillian Garcia, Series of
Stipulations, Stephanie McMahon Decrees..., According to Linda and the Board of Executives y You're a Human Vacuum
Cleaner ... (límites de Pre-match/Mid-match, perder la partida, Superstar Value), I'm the Underdog! (Superstar Value),
Toothless Aggression y There's No Holding Me Back (jugarse después del daño), Hey, Whatcha Doin'?, You Make Me Sick!,
Chief Morley Says..., Real Predictable..., I Can't Be Reading This Right..., Rhyno's Garbage Can y Screw the Rules!. Se
juegan sin una parte de su texto: Snap Slam, Body Vise, Inverted Leg Lock, Collegiate Champions y The Ref Takes Control
(quitar cartas del juego), Noggin Knocker (Title Belts y Tag Team), Tossed Between the Ropes (no revertir desde el
Backlash, cartas con efecto activado), Hangman's Neck Breaker (límite de Reversals tras From the Top Rope), Don't Try
This at Home (revertir efectos activados), Always y Pass the Buck (su efecto en el Ring), No One Wants to Watch Women
Wrestle, Ring Psychology: Arm (el máximo de 4D del rival), Take Care of Number One, Man of 1000 Holds, Backstage
Politics, Holla If Ya Hear Me! (mirar el Backlash), The Icon ... (Superstar Value), Chavo's Inverted Powerbomb (buscarla
como High Risk), Dudleyz3 (tres Reversals), X-treme Sendoff y You Can't Manhandle the Big Show (límites), Dude ...
Whatever, X-treme Hedonism y Nidia Interferes (efectos desde el Ringside), Yo Dawg, Respect da Bookerman y Proper
Planning Prevents Piss-poor Performance (al ser quitadas por el rival), No Gimmicks Needed (anular la habilidad
rival), Electrifying (doblar números), 3 Minutes and We're Out y Gimme a Hell Yeah! (el máximo de 6), y las
habilidades de The Rattlesnake, Los Guerreros, Team Angle, Test (reglas de mazo) y Shawn Michaels. Están simplificadas:
Series of Punches, The Devil's in the Detail, Crowd Pleaser, Perplex Opponent, Beat You Raw, Forget the Smackdown,
Jabronie!, Backstage Donnybrook, Chase Off Manager, Mental Acumen, Don't Cross a McMahon, Give You the Runaround,
Outside Interference, The Big, Bad Booty Daddy is Your Hook Up, Rabid Attack in the Corner, The Manbeast, You're Going
to Space Mountain y las habilidades de Big Poppa Pump, The Crippler y Rhyno.

Absolutely Raw and Ultimate Smackdown: no se pueden jugar todavía las 4 cartas Enforcer (el importador las salta),
Elimination Chamber (cambia los límites de Pre-match y Mid-match), Undermine the Competition y Hitting the G-Spot
(revertir después del daño de la maniobra), ni You're Fired! como Action (solo como Reversal). Se juegan sin una parte
de su texto: las habilidades de Eric Bischoff y Stephanie McMahon (Enforcers y prohibir Stipulations), la de Mae Young
(Arsenal de 45 cartas y Puppies! Puppies! como Set-up), Redemption (pasar al Ring rival), Banned from Ringside (mandar
la carta revertida al fondo de tu Arsenal), Escape to the Side (descartarla en lugar de Escape Move o Step Aside) y Raw
Deal! (anular una carta del Ring rival). Están simplificadas: las cartas que dependen de Superstars femeninas (se
reconocen por nombre: Chyna, Lita, Stephanie McMahon y Mae Young), Get Out of Here! (nunca se retira nada del juego,
así que # es 0), Vince McMahon Interferes! (solo en tu turno), Redemption (también impide revertir desde el Backlash) y
Banned from Ringside (el rival devuelve 1 carta de su mano inicial al Arsenal).

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
