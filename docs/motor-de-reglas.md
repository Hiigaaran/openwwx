# OpenWWX: diseño del motor de reglas

Estado: propuesta v0.1 (2026-10-14). Paso 1 del roadmap: motor de reglas.

## 1. Resumen de las reglas de Raw Deal que el motor debe modelar

Fuentes: [reglamento en rulespal](https://rulespal.com/wwe-raw-deal/rulebook) y [resumen enciclopédico](https://classicistranieri.com/en/r/a/w/Raw_Deal_aee6.html). Conviene contrastar con el reglamento oficial impreso de Comic Images si Simon lo tiene, sobre todo para los casos raros.

**Mazos y zonas.** Cada jugador tiene una carta de Superstar (fuera del mazo) y un Arsenal de 60 cartas. Zonas: Arsenal, Mano, Ring (cartas jugadas con éxito), Ringside (descarte), Backstage (Superstar y cartas Backstage, no son "Ring"), Backlash (mazo opcional de 20 cartas: 10 Pre-match y 10 Mid-match) y "fuera del juego".

**Construcción.** Máx. 3 copias por carta (Set-Up sin límite, Unique solo 1); no se mezclan Face/Heel, Raw/SmackDown ni Fan Favorite/Cheater; las cartas específicas de Superstar se saltan esas restricciones.

**Superstar.** Hand Size (cartas iniciales, sin límite de mano después), Superstar Value (quién empieza; empate al azar) y una habilidad, normalmente una vez por turno, obligatoria u opcional.

**Tipos de carta.** Maniobra (Strike, Grapple, Submission, High Risk, Trademark Finisher), Acción, Reversal, Híbridos (varios tipos, se juegan según la función elegida), cartas específicas de Superstar, Backlash (Pre-match: Venue, Feud, Stipulation, Manager, Event, Object; Mid-match).

**Estadísticas.** F (Fortitude requerida), D (daño), SV (Stun Value, 0 a 5 estrellas). Tu *Fortitude Rating* es la suma del D impreso de las cartas en tu Ring. Solo puedes jugar una carta si tu rating ≥ su F. Las bonificaciones de daño no suman a la Fortitude; solo el D impreso.

**Turno.**
1. Inicio de turno: efectos de inicio y habilidades usables antes de robar.
2. Draw Segment: robar 1 (algunas Superstars roban más).
3. Main Segment: en cualquier orden jugar Maniobras, Acciones, activar ACEs y la habilidad.
4. Fin: el turno termina cuando te revierten una carta, cuando no quieres o no puedes jugar más.

**Resolución de una carta.** Se juega (paga requisitos) → ventana de reversal del rival desde la mano → si no se revierte, se resuelven sus efectos → luego el daño → la carta queda en el Ring.

**Daño.** Por cada punto, el defensor voltea la carta superior de su Arsenal al Ringside, una a una. Si voltea un Reversal que podría revertir esa carta y cumple su F, el daño se detiene y el turno del atacante acaba; la carta atacante se queda en el Ring, el Reversal va al Ringside sin efectos ni daño, y el atacante puede robar hasta el SV de la carta revertida.

**Reversal desde la mano.** La carta atacante va al Ringside del atacante, el turno acaba, el Reversal queda en el Ring del defensor (suma su D a su Fortitude), aplica sus efectos y puede hacer daño. Los Reversals no se pueden revertir.

**Victoria.** Pin: el rival debe voltear una carta y su Arsenal está vacío. Count Out: el rival termina un turno sin cartas en el Arsenal (se puede rechazar el count out para buscar el pin). Empate: ambos terminan un turno sin cartas y nadie fue pineado.

**Palabras clave que el motor debe soportar.** Follow-up / cadenas ("debe jugarse justo después de X"; jugar otra carta rompe la cadena, usar habilidades no), Chain (revertir descartando 2; buscar otra Chain descartando 2), Multi (hay que revertirla dos veces), Volley, Active, Universally Active, Permanent, Unique, Universally Unique (cinturones), Restricted Modification, Heat, Throwback, ACE (Activated Card Effects), Irreversible y efectos obligatorios/opcionales ("haz lo que puedas e ignora el resto"). Regla general: el texto de la carta prevalece sobre el reglamento.

**Modo avanzado.** Fase Pre-match (Venue, Feud, Stipulation, Manager por turnos, luego cartas restantes alternadas hasta dos pases seguidos), máx. 5 Pre-match y 5 Mid-match en el Ring, Enforcers desde Backstage. Tag Team queda fuera del alcance inicial.

## 2. Principios de diseño

1. **El motor es una biblioteca pura**: sin E/S, sin UI, sin red, sin hilos. Entra un estado y una decisión, salen un estado nuevo y eventos. Todo lo demás (UI, IA, red) es un cliente del motor.
2. **Determinista**: el RNG (barajar, monedas, descartes al azar) es un `ChaCha8Rng` con semilla guardada dentro del estado. Misma semilla + mismas decisiones = misma partida. Esto da repeticiones, tests reproducibles y red por lockstep.
3. **El motor pregunta, los jugadores responden**: cada vez que una regla requiere una elección, el motor se detiene con una `Decision` pendiente que dice quién elige y entre qué opciones. Humanos, IA y red responden por la misma vía.
4. **Información oculta explícita**: el estado completo nunca sale del motor hacia un jugador; se exponen vistas (`PlayerView`) que ocultan mano y Arsenal ajenos.
5. **Cartas como datos + escape hatch**: la mayoría de efectos se describe con un lenguaje de efectos declarativo; las cartas imposibles de describir así usan funciones Rust registradas por id.
6. **Reglas incrementales**: primero Quick Start con un set pequeño de cartas, después palabras clave y modo avanzado.

## 3. Estructura del workspace (Cargo)

```
openwwx/
├── crates/
│   ├── rawdeal-core/    # tipos, estado, motor, lenguaje de efectos (sin dependencias pesadas)
│   ├── rawdeal-cards/   # carga de definiciones de cartas (RON/TOML) + efectos en Rust por id
│   ├── rawdeal-ai/      # bots: aleatorio, heurístico, MCTS (paso 4)
│   ├── rawdeal-net/     # protocolo y transporte (paso 5)
│   └── openwwx-app/     # cliente de escritorio (UI, arte cargable)
└── data/                # cartas de prueba; los datos reales se cargan aparte
```

Solo `rawdeal-core` existe al inicio del paso 1. `rawdeal-cards` nace con un puñado de cartas de prueba para ejercitar el motor; la base completa es el paso 2.

## 4. Modelo de datos

```rust
/// Definición estática de una carta (lo que está impreso). Compartida, inmutable.
pub struct CardDef {
    pub id: CardDefId,            // p. ej. "premiere/042"
    pub title: String,
    pub kinds: Vec<CardKind>,     // Maneuver(Strike), Action, Reversal… (varios = híbrido)
    pub fortitude: Option<u8>,    // None = "X" o sin valor
    pub damage: Option<u8>,
    pub stun_value: u8,
    pub traits: TraitSet,         // Face/Heel, Raw/SmackDown, Unique, Active, Multi, Chain…
    pub superstar_logo: Option<SuperstarId>,
    pub text: String,             // texto original, para mostrar
    pub script: CardScript,       // efectos interpretables (ver §6)
}

/// Instancia física dentro de una partida.
pub struct CardInstance { pub def: CardDefId, pub owner: PlayerId, pub zone: Zone }
pub type CardId = u16;            // índice en un arena por partida

pub enum Zone { Arsenal, Hand, Ring, Ringside, Backstage, Backlash, OutOfGame, Stack }

pub struct PlayerState {
    pub superstar: SuperstarId,
    pub arsenal: Vec<CardId>,     // índice 0 = fondo, last() = tope
    pub hand: Vec<CardId>,
    pub ring: Vec<CardId>,
    pub ringside: Vec<CardId>,
    pub backstage: Vec<CardId>,
    pub backlash: Vec<CardId>,
    pub out_of_game: Vec<CardId>,
    pub ability_uses_this_turn: u8,
}

pub struct GameState {
    pub cards: Vec<CardInstance>, // arena
    pub players: [PlayerState; 2],
    pub active: PlayerId,
    pub turn: u32,
    pub phase: Phase,             // PreMatch(..), StartOfTurn, Draw, Main, EndOfTurn, GameOver(Outcome)
    pub chain: ChainState,        // última carta jugada, para follow-ups / cadenas
    pub modifiers: Vec<Modifier>, // efectos continuos con duración
    pub pending: Vec<Step>,       // pila de pasos pendientes del intérprete (§5)
    pub decision: Option<Decision>,
    pub rng: ChaCha8Rng,
}
```

La Fortitude no se guarda: se calcula (suma del D impreso del Ring + modificadores que afecten Fortitude), para que nunca se desincronice. `GameState` es `Clone` barato (vectores pequeños de `u16`, definiciones compartidas por `Arc`), lo que necesita la IA.

## 5. Bucle del motor: máquina de pasos con decisiones

```rust
pub struct Engine { db: Arc<CardDb> }

impl Engine {
    pub fn new_game(&self, setup: GameSetup, seed: u64) -> (GameState, Vec<Event>);
    pub fn legal_actions(&self, s: &GameState, p: PlayerId) -> Vec<Action>;
    pub fn apply(&self, s: &mut GameState, p: PlayerId, a: Action) -> Result<Vec<Event>, RuleError>;
    pub fn view(&self, s: &GameState, p: PlayerId) -> PlayerView;
}

pub enum Action {
    PlayCard { card: CardId, as_kind: CardKind },   // el híbrido elige función
    UseAbility, ActivateAce { card: CardId },
    Reverse { card: CardId },                       // desde la mano, en ventana de reversal
    DeclineReversal, EndTurn,
    Choose(ChoiceAnswer),                           // respuesta a una Decision
    RefuseCountOut,
}
```

Internamente, jugar una carta empuja una secuencia de `Step` a `pending`:

```
PayCosts → ReversalWindow(defensor) → ResolveEffects → DealDamage(n) → MoveToRing → CheckChain
```

`apply` ejecuta pasos hasta que uno necesite una elección (crea `Decision` y devuelve el control) o la pila se vacíe. `DealDamage(n)` se descompone en `n` pasos `OverturnOne`; cada uno comprueba pin, mira si la carta volteada es un Reversal válido contra la carta en curso (si lo es, pregunta al defensor si quiere usarlo cuando sea opcional), y aplica SV. Así se modelan sin casos especiales el daño interrumpido, Multi (dos reversals necesarios) y los efectos que dicen "después de que esta carta haga daño…".

Después de cada paso se ejecuta `check_state()`: pin, count out al final del turno, empate, límites de Backlash, Universally Unique.

**Eventos.** Cada cambio visible emite un `Event` (`CardMoved`, `DamageOverturned`, `Reversed`, `TurnEnded`, `GameOver`…). La UI anima a partir de eventos, la red puede retransmitirlos y el log de partida es la lista de `Action` + semilla.

## 6. Lenguaje de efectos

Las ~cientos de cartas comparten patrones. Un `CardScript` contiene:

```rust
pub struct CardScript {
    pub play_requirements: Vec<Condition>,     // "solo si tu último maniobra fue Grapple"
    pub on_play: Option<Effect>,               // se resuelve antes del daño
    pub damage_bonus: Vec<(Condition, i8)>,    // "+2D si…"
    pub reverses: Option<ReverseSpec>,         // solo Reversals: qué puede revertir
    pub triggers: Vec<Trigger>,                // "al inicio de tu turno…", "cuando hagas daño…"
    pub continuous: Vec<StaticAbility>,        // mientras esté en el Ring…
    pub ace: Option<(Cost, Effect)>,
    pub follow_up_of: Option<Condition>,       // debe jugarse justo después de…
    pub custom: Option<&'static str>,          // id de función Rust para lo inexpresable
}

pub enum Effect {
    Seq(Vec<Effect>),
    Choose { who: Who, options: Vec<Effect> },
    If(Condition, Box<Effect>, Option<Box<Effect>>),
    Optional(Box<Effect>),                     // "puedes…"
    Draw { who: Who, n: Amount },
    Discard { who: Who, n: Amount, how: Pick },        // elegido / al azar
    Search { from: ZoneRef, filter: CardFilter, to: ZoneRef, n: Amount, shuffle: bool },
    Move { what: CardSel, to: ZoneRef },
    Damage { who: Who, n: Amount },                    // daño extra fuera de la carta
    AddModifier(Modifier),                             // "+2D a tu próxima maniobra este turno"
    EndTurn, Shuffle(ZoneRef),
    Custom(&'static str),
}

pub struct ReverseSpec {
    pub targets: CardFilter,      // p. ej. Maneuver(Strike) con D ≤ 7, o título concreto
    pub from_hand_effect: Option<Effect>,
    pub damage_override: Option<Amount>,  // "#" o daño igual al de la carta revertida
}
```

`CardFilter` y `Condition` son árboles pequeños (tipo, subtipo, título, rasgos, rangos de F/D, "la última carta jugada…"). Los modificadores tienen duración (`ThisCard`, `ThisTurn`, `NextManeuver`, `WhileInRing(CardId)`) y se aplican en orden fijo: valor impreso → modificadores de la carta → efectos continuos → Restricted Modification anula.

Las cartas se escriben en RON, con el texto original al lado, y un test por carta. Cuando una carta no encaja, `Custom("premiere/042")` llama a una función Rust registrada en `rawdeal-cards`. La meta es que el 85-90 % sea solo datos; si muchas cartas acaban en `Custom`, es señal de que falta un primitivo.

## 7. Cómo se enchufan los pasos siguientes

**Paso 2, base de cartas.** `rawdeal-cards` lee `CardDef` desde archivos RON/TOML (o SQLite si crece) con un esquema versionado. El motor solo depende del trait `CardDb`. Recomendación legal: el texto y el arte de las cartas son propiedad de WWE / Comic Images, así que el repositorio debería incluir solo un set de prueba y permitir cargar el resto desde una carpeta del usuario.

**Paso 3, arte cargable.** Un manifiesto separado `CardDefId → ruta de imagen` en la carpeta del usuario, leído solo por `openwwx-app`. El motor nunca ve imágenes; si falta el arte, la UI dibuja la carta desde sus datos.

**Paso 4, IA con dificultades.** La IA solo usa `view`, `legal_actions` y `apply` sobre clones.
- Fácil: acción legal al azar con sesgo simple.
- Media: heurística de un paso (daño esperado, Fortitude ganada, riesgo de reversal según cartas ya vistas).
- Difícil: MCTS con determinización (ISMCTS): se rellenan la mano y el Arsenal ocultos del rival con cartas compatibles con lo visto y se simulan partidas. La dificultad se regula con tiempo/iteraciones.
El determinismo y el `Clone` barato del estado son lo que hacen viable esto.

**Paso 5, en línea.** Como el motor es determinista y solo cambia por `Action`, el protocolo puede ser un intercambio de acciones. El problema es la información oculta: si ambos clientes tienen el estado completo, uno podría leer la mano del otro. Opciones a evaluar entonces:
- Anfitrión autoritativo: un jugador (o un servidor) ejecuta el motor y envía a cada uno su `PlayerView` + eventos. Simple, funciona por TCP directo o relay; el anfitrión podría hacer trampa.
- Servidor dedicado autoritativo: igual, sin ventaja para nadie; requiere alojar algo.
- P2P con barajado criptográfico ("mental poker"): sin servidor ni confianza, mucho más complejo.
El motor sirve para las tres sin cambios; la decisión se toma en el paso 5. Mi recomendación previa: anfitrión autoritativo sobre TCP (con QUIC/relay más adelante para NAT).

## 8. Plan de implementación del paso 1

1. Workspace + `rawdeal-core` con tipos, zonas, arena de cartas, RNG con semilla.
2. Setup Quick Start: revelar Superstars, barajar, robar Hand Size, decidir quién empieza.
3. Turno básico: robar, jugar maniobras/acciones con chequeo de F, daño por volteo, fin de turno.
4. Reversals desde la mano y durante el daño, Stun Value.
5. Victoria: pin, count out (con rechazo), empate.
6. Lenguaje de efectos con los primitivos más comunes y ~15 cartas de prueba.
7. Habilidades de Superstar (2-3 Superstars de prueba).
8. Palabras clave: follow-up/cadena, Multi, Chain, Active, Unique, Permanent, ACE, Irreversible.
9. CLI de texto mínima para jugar humano vs. bot aleatorio y depurar.
10. Modo avanzado: Pre-match y Backlash.

**Pruebas.** Tests de escenario por regla ("dado este estado, esta acción produce estos eventos"), un test por carta, y pruebas de propiedades con `proptest`: miles de partidas aleatorias que no deben paniquear, conservan el número de cartas y dan el mismo resultado al repetir la misma semilla.

## 9. Preguntas abiertas para Simon

- ¿Qué set de cartas usamos primero para probar? Propuesta: Premiere (el set base).
- ¿Quick Start primero y Advanced después? Propuesta: sí.
- Framework de UI de escritorio (Bevy, egui, macroquad): no bloquea el paso 1; se decide cuando toque el cliente.
- ¿Tienes el reglamento oficial (PDF o impreso) y FAQs/erratas? Son la referencia para casos raros.
