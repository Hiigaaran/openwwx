//! Spanish text for the interface: card kinds, effects, decisions and the
//! match log.

use rawdeal_core::{
    Boost, CardFilter, CardId, CardKind, ChoicePurpose, Duration, Effect, Event, ManeuverType, Outcome, Pick, PlayerId,
    PreMatchType, Who, WinMethod, YesNoPurpose, Zone,
};

pub fn kind(k: CardKind) -> String {
    match k {
        CardKind::Maneuver(t) => format!("Maniobra {}", maneuver(t)),
        CardKind::Action => "Acción".into(),
        CardKind::Reversal => "Reversal".into(),
        CardKind::PreMatch(t) => format!(
            "Pre-match {}",
            match t {
                PreMatchType::Venue => "Venue",
                PreMatchType::Feud => "Feud",
                PreMatchType::Stipulation => "Stipulation",
                PreMatchType::Manager => "Manager",
                PreMatchType::Event => "Event",
                PreMatchType::Object => "Object",
            }
        ),
    }
}

fn maneuver(t: ManeuverType) -> &'static str {
    match t {
        ManeuverType::Strike => "Strike",
        ManeuverType::Grapple => "Grapple",
        ManeuverType::Submission => "Submission",
        ManeuverType::HighRisk => "High Risk",
        ManeuverType::TrademarkFinisher => "Trademark Finisher",
    }
}

thread_local! {
    /// Set while describing effects controlled by the other player.
    static SWAPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn who(w: Who) -> &'static str {
    match (w, SWAPPED.with(|s| s.get())) {
        (Who::You, false) | (Who::Opponent, true) => "tú",
        _ => "el rival",
    }
}

fn cards(n: u8) -> String {
    if n == 1 { "1 carta".into() } else { format!("{n} cartas") }
}

pub fn filter(f: &CardFilter) -> String {
    match f {
        CardFilter::Any => "cualquier carta".into(),
        CardFilter::Maneuver => "maniobra".into(),
        CardFilter::ManeuverType(t) => maneuver(*t).into(),
        CardFilter::Action => "Acción".into(),
        CardFilter::Reversal => "Reversal".into(),
        CardFilter::Title(t) => format!("\"{t}\""),
        CardFilter::TitleContains(t) => format!("con \"{t}\" en el título"),
        CardFilter::Trait(t) => format!("{t:?}"),
        CardFilter::DamageAtMost(n) => format!("de {n}D o menos"),
        CardFilter::DamageAtLeast(n) => format!("de {n}D o más"),
        CardFilter::FortitudeAtMost(n) => format!("de {n}F o menos"),
        CardFilter::FortitudeAtLeast(n) => format!("de {n}F o más"),
        CardFilter::DamageAtMostYourFortitude => "con D no mayor que tu Fortitude".into(),
        CardFilter::After(f) => format!("jugada después de {}", filter(f)),
        CardFilter::PlayerRingHas { filter: f, n } => format!("si hay {n} {} en su Ring", filter(f)),
        CardFilter::All(fs) => fs.iter().map(filter).collect::<Vec<_>>().join(" "),
        CardFilter::AnyOf(fs) => fs.iter().map(filter).collect::<Vec<_>>().join(" o "),
        CardFilter::Not(f) => format!("no {}", filter(f)),
        CardFilter::Keyword(k) => k.clone(),
        CardFilter::EarlierThisTurn { filter: f, n } => format!("si ya jugó {n} {} este turno", filter(f)),
        CardFilter::PlayedBy(name) => format!("jugada por {name}"),
        CardFilter::StunValueAtLeast(n) => format!("con Stun Value {n} o más"),
        CardFilter::Draws => "que hace robar cartas".into(),
        CardFilter::ForcesDiscard => "que obliga a descartar".into(),
        CardFilter::SameTitleInRing => "con el mismo título que una carta de su Ring".into(),
        CardFilter::SuperstarSpecific => "específica de Superstar".into(),
        CardFilter::Holds(_) => "si se cumple la condición".into(),
    }
}

fn boost(b: &Boost) -> String {
    let mut parts = Vec::new();
    if b.damage != 0 {
        parts.push(format!("{:+}D", b.damage));
    }
    if b.fortitude != 0 {
        parts.push(format!("{:+}F", b.fortitude));
    }
    if b.reversal_damage != 0 {
        parts.push(format!("Reversals en contra {:+}D", b.reversal_damage));
    }
    if b.reversal_fortitude != 0 {
        parts.push(format!("Reversals en contra {:+}F", b.reversal_fortitude));
    }
    if b.irreversible {
        parts.push("no se puede revertir".into());
    }
    if b.no_hand_reversal {
        parts.push("no se puede revertir desde la mano".into());
    }
    if b.no_backlash_reversal {
        parts.push("no se puede revertir desde el Backlash".into());
    }
    if b.no_arsenal_reversal {
        parts.push("no se puede revertir desde el Arsenal".into());
    }
    if b.stun_value != 0 {
        parts.push(format!("{:+} Stun Value", b.stun_value));
    }
    if b.ignore_play_after {
        parts.push("ignora \"solo después de…\"".into());
    }
    if b.use_opponent_fortitude {
        parts.push("se juega con la Fortitude del rival".into());
    }
    if b.double_fortitude {
        parts.push("F doble".into());
    }
    if b.double_damage {
        parts.push("daño doble".into());
    }
    if b.end_turn {
        parts.push("luego termina el turno".into());
    }
    let when = match b.duration {
        Duration::NextCard => "tu próxima carta, si es",
        Duration::NextMatching => "tu próxima",
        Duration::ThisTurn => "este turno, cada",
        Duration::NextTurn => "el próximo turno, cada",
        Duration::NextTurnFirstCard => "tu primera carta del próximo turno, si es",
        Duration::Turns(_) => "durante varios turnos, cada",
        Duration::NextCards(_) => "tus próximas cartas, si son",
        Duration::NextTurnCards(_) => "tus primeras cartas del próximo turno, si son",
    };
    format!("{when} {}: {}", filter(&b.applies_to), parts.join(", "))
}

pub fn effect(e: &Effect) -> String {
    match e {
        Effect::Draw { who: w, n } => format!("roba {} ({})", cards(*n), who(*w)),
        Effect::DrawUpTo { who: w, n } => format!("roba hasta {} ({})", cards(*n), who(*w)),
        Effect::Discard { who: w, n, pick } => {
            let how = match pick {
                Pick::Chosen => "",
                Pick::Random => " al azar",
                Pick::Controller => " elegidas por ti",
            };
            format!("descarta {}{how} ({})", cards(*n), who(*w))
        }
        Effect::DiscardAll { who: w, filter: f } => format!("descarta de la mano: {} ({})", filter(f), who(*w)),
        Effect::RingsideToHand { n, up_to, .. } => {
            format!("{}{} del Ringside a la mano", if *up_to { "hasta " } else { "" }, cards(*n))
        }
        Effect::RingsideToArsenal { n, up_to, .. } => {
            format!("baraja {}{} del Ringside en el Arsenal", if *up_to { "hasta " } else { "" }, cards(*n))
        }
        Effect::HandToArsenal { n, .. } => format!("baraja {} de la mano en el Arsenal", cards(*n)),
        Effect::HandToArsenalBottom { n, .. } => format!("{} de la mano al fondo del Arsenal", cards(*n)),
        Effect::RingsideToArsenalBottom { n, .. } => format!("{} del Ringside al fondo del Arsenal", cards(*n)),
        Effect::Damage { who: w, n } => format!("{} {n} de daño", who(*w)),
        Effect::Mill { who: w, n } => format!("{} del Arsenal al Ringside ({})", cards(*n), who(*w)),
        Effect::MillChosen { n, .. } => format!("elige {} del Arsenal rival y mándalas al Ringside", cards(*n)),
        Effect::Search { filter: f, .. } => format!("busca {} y ponla en tu mano", filter(f)),
        Effect::LookAtHand { .. } => "mira la mano del rival".into(),
        Effect::Arrange { who: w, n } => format!("ordena las {n} cartas de arriba del Arsenal ({})", who(*w)),
        Effect::Shuffle { who: w } => format!("baraja el Arsenal ({})", who(*w)),
        Effect::RemoveFromRing { n, .. } => format!("quita {} del Ring rival", cards(*n)),
        Effect::LevelFortitude => "iguala la Fortitude".into(),
        Effect::OverturnUntilReversal { .. } => "el rival voltea hasta encontrar un Reversal".into(),
        Effect::Boost(b) => boost(b),
        Effect::EndTurn => "termina tu turno".into(),
        Effect::SkipNextTurn { .. } => "el rival se salta su próximo turno".into(),
        Effect::AtEndOfTurn(es) => format!("al final del turno: {}", effects(es)),
        Effect::DiscardThis => "descarta esta carta".into(),
        Effect::RemoveThis => "retira esta carta del juego".into(),
        Effect::MoveCards { who: w, from, to, n, .. } => {
            format!("pasa {} de {} a {} ({})", cards(*n), zone(*from), zone(*to), who(*w))
        }
        Effect::ThisToArsenal { bottom } => {
            if *bottom {
                "pon esta carta debajo de tu Arsenal".into()
            } else {
                "pon esta carta encima de tu Arsenal".into()
            }
        }
        Effect::ReturnThis { to } => format!("pon esta carta en tu {}", zone(*to)),
        Effect::RemoveFromGame { who: w, from, n, .. } => {
            let zone = match from {
                Zone::Arsenal => "de arriba del Arsenal",
                Zone::Hand => "de la mano",
                Zone::Ring => "del Ring",
                _ => "del Ringside",
            };
            format!("retira del juego {} {zone} ({})", cards(*n), who(*w))
        }
        Effect::ShuffleThisIntoArsenal => "baraja esta carta en tu Arsenal".into(),
        Effect::NextTurnAfterThis => "tu próxima carta cuenta como jugada después de esta".into(),
        Effect::WinGame => "ganas por descalificación".into(),
        Effect::Optional(es) => format!("puedes: {}", effects(es)),
        Effect::Choice(os) => os.iter().map(|o| effects(o)).collect::<Vec<_>>().join(" / "),
        Effect::If { effects: es, .. } => format!("si se cumple: {}", effects(es)),
        Effect::RevealHand { who: w } => format!("muestra la mano ({})", who(*w)),
        Effect::DiscardMatching { who: w, n, filter: f, .. } => {
            format!("descarta {} ({}) ({})", cards(*n), filter(f), who(*w))
        }
        Effect::DiscardPerRingCard { who: w, filter: f } => {
            format!("descarta 1 carta por cada {} en su Ring ({})", filter(f), who(*w))
        }
        Effect::DiscardMirror { max } => format!("descarta hasta {}; el rival descarta las mismas", cards(*max)),
        Effect::HandToArsenalTop { who: w, n, random } => {
            let how = if *random { " al azar" } else { "" };
            format!("{}{how} de la mano arriba del Arsenal ({})", cards(*n), who(*w))
        }
        Effect::ShuffleHandAndRedraw { who: w } => format!("baraja la mano en el Arsenal y roba igual ({})", who(*w)),
        Effect::Dig { n, take } => format!("mira {} del Arsenal y quédate {}", cards(*n), take),
        Effect::DamageSuperstarValue { who: w } => format!("{} voltea tantas cartas como su Superstar Value", who(*w)),
        Effect::RingToHand { n, .. } => format!("devuelve {} del Ring rival a su mano", cards(*n)),
        Effect::ClearRing { who: w } => format!("todo el Ring al Ringside ({})", who(*w)),
        Effect::RevealUntil { filter: f } => format!("revela del Arsenal hasta encontrar {}", filter(f)),
        Effect::CannotPlay { who: w, filter: f, turns } => {
            format!("{} no puede jugar {} durante {turns} turno(s)", who(*w), filter(f))
        }
        Effect::BoostOpponent(b) => format!("al rival: {}", boost(b)),
        Effect::WinByPin => "ganas por Pinfall".into(),
        Effect::DiscardForDamage { per } => format!("descarta las cartas que quieras: +{per}D por cada una"),
        Effect::NextCardMustBe(f) => format!("tu próxima carta tiene que ser {} o termina tu turno", filter(f)),
        Effect::NextCardMustBeChosen => "tu próxima carta tiene que ser la elegida o termina tu turno".into(),
        Effect::RepeatPreviousEffects => "repite el texto de tu carta anterior".into(),
        Effect::MillAll { who: w, filter: f } => {
            format!("todas las {} del Arsenal al Ringside ({})", filter(f), who(*w))
        }
        Effect::SacrificeThis => "esta carta va al Ringside".into(),
        Effect::BlankNextPreMatch { .. } => "la próxima carta Pre-match del rival no tiene efecto".into(),
        Effect::RingsideToBacklash { n, .. } => format!("{} Mid-match del Ringside al Backlash", cards(*n)),
        Effect::BacklashToRingside { n, .. } => format!("{} del Backlash al Ringside", cards(*n)),
        Effect::Mulligan { n } => format!("baraja hasta {} de la mano en el Arsenal y roba igual", cards(*n)),
        Effect::DrawToMatchOpponent => "roba hasta tener tantas cartas como el rival".into(),
        Effect::RingsideToArsenalCount { .. } => "baraja cartas del Ringside en el Arsenal".into(),
        Effect::BuryHand { .. } => "pon tu mano al fondo del Arsenal y reponla".into(),
        Effect::DiscardHandAndDraw { min } => format!("descarta tu mano y roba igual (al menos {min})"),
        Effect::PurgeReversedTitle => "manda al Ringside todas las copias rivales de la carta revertida".into(),
        Effect::ShuffleRingsideIntoArsenal { who: w } => format!("baraja todo el Ringside en el Arsenal ({})", who(*w)),
        Effect::NextPlayWatch { filter: f, .. } => format!("según cómo salga tu próxima {}", filter(f)),
        Effect::DrawRingCards { less } => format!("roba 1 por cada carta en los Rings, menos {less}"),
        Effect::OpponentChooses { n, .. } => format!("el rival elige {n} opciones"),
        Effect::Counted { effect: e, .. } => format!("{} (según la cuenta)", effect(e)),
        Effect::IrreversibleThis => "esta carta no se puede revertir".into(),
        Effect::MillUntil { who: w, filter: f } => format!("voltea hasta una {} ({})", filter(f), who(*w)),
        Effect::AsOpponent(es) => {
            let shown: Vec<Effect> =
                es.iter().filter(|e| !matches!(e, Effect::OpponentChooses { .. })).cloned().collect();
            let before = SWAPPED.with(|s| s.replace(!s.get()));
            let text = effects(&shown);
            SWAPPED.with(|s| s.set(before));
            text
        }
    }
}

pub fn effects(es: &[Effect]) -> String {
    if es.is_empty() {
        return "nada".into();
    }
    es.iter().map(effect).collect::<Vec<_>>().join(", ")
}

pub fn purpose(p: ChoicePurpose) -> &'static str {
    match p {
        ChoicePurpose::Discard | ChoicePurpose::ForcedDiscard => "para descartar",
        ChoicePurpose::RingsideToHand => "del Ringside para tu mano",
        ChoicePurpose::RingsideToArsenal => "del Ringside para barajar en el Arsenal",
        ChoicePurpose::HandToArsenalBottom => "para poner al fondo del Arsenal",
        ChoicePurpose::RingsideToArsenalBottom => "del Ringside para el fondo del Arsenal",
        ChoicePurpose::SearchToHand => "del Arsenal para tu mano",
        ChoicePurpose::HandToArsenal => "para barajar en el Arsenal",
        ChoicePurpose::ArsenalToRingside => "del Arsenal para mandar al Ringside",
        ChoicePurpose::RingToRingside => "del Ring para mandar al Ringside",
        ChoicePurpose::RemoveFromGame => "para retirar del juego",
        ChoicePurpose::MoveTo(Zone::Hand) => "para poner en la mano",
        ChoicePurpose::MoveTo(Zone::Ring) => "para poner en el Ring",
        ChoicePurpose::MoveTo(Zone::Arsenal) => "para barajar en el Arsenal",
        ChoicePurpose::MoveTo(Zone::Backlash) => "para poner en el Backlash",
        ChoicePurpose::MoveTo(Zone::Removed) => "para retirar del juego",
        ChoicePurpose::MoveTo(_) => "para mandar al Ringside",
        ChoicePurpose::Arrange => "para poner arriba del Arsenal (la siguiente de arriba hacia abajo)",
        ChoicePurpose::DiscardMirror => "para descartar (el rival descartará las mismas)",
        ChoicePurpose::HandToArsenalTop => "para poner arriba del Arsenal",
        ChoicePurpose::RingToHand => "del Ring para devolver a la mano",
        ChoicePurpose::DigToHand => "de arriba del Arsenal para tu mano",
        ChoicePurpose::DiscardForDamage { .. } => "para descartar (+D por cada una)",
        ChoicePurpose::RingsideToBacklash => "del Ringside para tu Backlash",
        ChoicePurpose::Mulligan => "para barajar en el Arsenal y robar otras",
    }
}

pub fn yes_no(p: &YesNoPurpose) -> String {
    match p {
        YesNoPurpose::OptionalEffect { effects: es, .. } => format!("¿Quieres aplicar: {}?", effects(es)),
        YesNoPurpose::ClaimCountOut => "Tu rival terminó su turno sin Arsenal. ¿Ganar por Count Out?".into(),
        YesNoPurpose::ChainSearch => "Tu Chain tuvo éxito. ¿Descartar 2 cartas para buscar otra Chain?".into(),
    }
}

pub fn player(p: PlayerId, me: PlayerId) -> &'static str {
    if p == me { "Tú" } else { "Rival" }
}

fn zone(z: Zone) -> &'static str {
    match z {
        Zone::Arsenal => "Arsenal",
        Zone::Hand => "mano",
        Zone::Ring => "Ring",
        Zone::Ringside => "Ringside",
        Zone::Backlash => "Backlash",
        Zone::Removed => "fuera del juego",
        Zone::InPlay => "juego",
    }
}

/// One line of the match log, or `None` for events not worth showing.
pub fn event(ev: &Event, me: PlayerId, name: &dyn Fn(CardId) -> String) -> Option<String> {
    // The same sentence for you ("Robas …") and for the opponent ("El rival roba …").
    let say = |p: PlayerId, mine: String, theirs: String| if p == me { mine } else { format!("El rival {theirs}") };
    Some(match ev {
        Event::PreMatchStarted { first } => format!("Empieza el Pre-match. Juega primero: {}.", player(*first, me)),
        Event::PreMatchEnded => "Termina el Pre-match.".into(),
        Event::GameStarted { first } => format!("Empieza la lucha. Comienza: {}.", player(*first, me)),
        Event::TurnStarted { player: p, turn } => {
            format!("— Turno {turn}: {} —", if *p == me { "tú" } else { "rival" })
        }
        Event::CardDrawn { player: p, card: Some(c) } => {
            say(*p, format!("Robas {}.", name(*c)), format!("roba {}.", name(*c)))
        }
        Event::CardDrawn { player: p, card: None } => say(*p, "Robas una carta.".into(), "roba una carta.".into()),
        Event::CardPlayed { player: p, card, kind: k } => {
            let what = format!("{} ({})", name(*card), kind(*k));
            say(*p, format!("Juegas {what}."), format!("juega {what}."))
        }
        Event::Reversed { player: p, reversal, reversed, from_hand } => {
            let what = format!(
                "{} con {}{}",
                name(*reversed),
                name(*reversal),
                if *from_hand { "" } else { ", que salió volteada durante el daño" }
            );
            say(*p, format!("¡REVIERTES {what}!"), format!("¡REVIERTE {what}!"))
        }
        Event::PartialReversal { player: p, reversal, card } => {
            let what = format!("{} con {}, pero es Multi: hace falta otro Reversal.", name(*card), name(*reversal));
            say(*p, format!("Frenas {what}"), format!("frena {what}"))
        }
        Event::Revealed { player: p, card } => say(
            *p,
            format!("Muestras {} y la pones en tu mano.", name(*card)),
            format!("muestra {} y la pone en su mano.", name(*card)),
        ),
        Event::Overturned { player: p, card } => {
            say(*p, format!("  Volteas {}.", name(*card)), format!("voltea {}.", name(*card)))
        }
        Event::CardMoved { player: p, card: Some(c), from: Zone::Hand, to: Zone::Ringside } => {
            say(*p, format!("Descartas {}.", name(*c)), format!("descarta {}.", name(*c)))
        }
        Event::CardMoved { player: p, card: Some(c), from, to } if *to != Zone::Ring && *from != Zone::InPlay => {
            let what = format!("{} de {} a {}", name(*c), zone(*from), zone(*to));
            say(*p, format!("Mueves {what}."), format!("mueve {what}."))
        }
        Event::ArsenalShuffled { player: p } => say(*p, "Barajas tu Arsenal.".into(), "baraja su Arsenal.".into()),
        Event::HandShown { player: p, viewer, cards } if *viewer == me && *p != me => {
            let names: Vec<String> = cards.iter().map(|&c| name(c)).collect();
            format!("Mano del rival: {}.", if names.is_empty() { "vacía".into() } else { names.join(", ") })
        }
        Event::HandShown { viewer, .. } if *viewer != me => "El rival mira tu mano.".into(),
        Event::TurnSkipped { player: p } => say(*p, "Te saltas el turno.".into(), "se salta el turno.".into()),
        Event::CountOutRefused { player: p } => {
            say(*p, "Rechazas el Count Out y vas por el pin.".into(), "rechaza el Count Out y va por el pin.".into())
        }
        Event::TurnEnded { player: p } => say(*p, "Terminas el turno.".into(), "termina el turno.".into()),
        Event::GameOver(o) => outcome(o, me),
        _ => return None,
    })
}

pub fn outcome(o: &Outcome, me: PlayerId) -> String {
    match o {
        Outcome::Draw => "¡Empate!".into(),
        Outcome::Win { winner, method } => {
            let how = match method {
                WinMethod::Pinfall => "por Pinfall",
                WinMethod::CountOut => "por Count Out",
                WinMethod::Disqualification => "por descalificación",
            };
            if *winner == me { format!("¡Ganaste {how}!") } else { format!("Perdiste: el rival gana {how}.") }
        }
    }
}
