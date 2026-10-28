//! What a deck is built for: the focus hexagon, the Fortitude curve and
//! statistics over many opening hands, for the deck builder.

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use rawdeal_core::{CardDb, CardDef, CardFilter, CardKind, DeckList, ManeuverType, printed_match};

pub const AXES: [&str; 6] = ["Ataque", "Potencia", "Velocidad", "Defensa", "Control", "Aguante"];

/// Effects that draw, discard, search or look at cards, by their names in the
/// card scripts.
const CONTROL: [&str; 17] = [
    "Draw {",
    "DrawUpTo",
    "Discard {",
    "DiscardAll",
    "DiscardMatching",
    "RingsideToHand",
    "Search",
    "LookAtHand",
    "RevealHand",
    "Mill {",
    "MillChosen",
    "Dig",
    "BuryHand",
    "CannotPlay",
    "Mulligan",
    "ShuffleHandAndRedraw",
    "DrawToMatchOpponent",
];

/// The maneuvers a Reversal is checked against for the Defensa axis.
const PROBES: [ManeuverType; 4] =
    [ManeuverType::Strike, ManeuverType::Grapple, ManeuverType::Submission, ManeuverType::HighRisk];

/// Hands dealt for the statistics.
pub const HANDS: usize = 1000;

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Each axis from 0 to 10, in the order of [`AXES`].
    pub focus: [f32; 6],
    /// What each axis counted, in words.
    pub why: [String; 6],
    /// Arsenal cards by printed Fortitude: 0, 1, ... 12 and 13 or more.
    pub curve: [usize; 14],
    pub stats: HandStats,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandStats {
    pub hand_size: usize,
    /// Average cards in the opening hand playable at 0 Fortitude.
    pub playable: f32,
    /// Share of hands with at least one Reversal.
    pub with_reversal: f32,
    /// Share of hands with at least one maneuver playable at 0 Fortitude.
    pub with_opener: f32,
    /// Average Damage of the best maneuver playable at 0 Fortitude.
    pub opener_damage: f32,
}

fn is_maneuver(def: &CardDef) -> bool {
    def.kinds.iter().any(|k| matches!(k, CardKind::Maneuver(_)))
}

fn is_finisher(def: &CardDef) -> bool {
    def.kinds.iter().any(|k| {
        matches!(k, CardKind::Maneuver(ManeuverType::TrademarkFinisher) | CardKind::Maneuver(ManeuverType::Trademark))
    })
}

/// Cards it takes to fill an axis.
fn share(n: f32, full: f32) -> f32 {
    (10.0 * n / full).clamp(0.0, 10.0)
}

fn controls(def: &CardDef) -> bool {
    let script = format!("{:?}", def.script);
    CONTROL.iter().any(|name| script.contains(name))
}

/// Could a card matching `def` meet `filter`? Parts that depend on the
/// match (the player's Fortitude, what was played before) may.
fn may_match(def: &CardDef, filter: &CardFilter) -> bool {
    use CardFilter::*;
    match filter {
        All(fs) => fs.iter().all(|f| may_match(def, f)),
        AnyOf(fs) => fs.iter().any(|f| may_match(def, f)),
        Not(f) => !printed(f) || !printed_match(def, f),
        f if printed(f) => printed_match(def, f),
        _ => true,
    }
}

/// Is `filter` about the printed card only?
fn printed(filter: &CardFilter) -> bool {
    use CardFilter::*;
    match filter {
        All(fs) | AnyOf(fs) => fs.iter().all(printed),
        Not(f) => printed(f),
        Any | Maneuver | ManeuverType(_) | Action | Reversal | Title(_) | TitleContains(_) | Trait(_)
        | DamageAtMost(_) | DamageAtLeast(_) | FortitudeAtMost(_) | FortitudeAtLeast(_) | Keyword(_)
        | StunValueAtLeast(_) | SuperstarSpecific | Antic | Target(_) | HasTarget | Hybrid => true,
        _ => false,
    }
}

/// A playable card on the first turn, before any card is in the Ring and
/// with nothing played before it.
fn opener(def: &CardDef) -> bool {
    def.fortitude == 0
        && (is_maneuver(def) || def.kinds.contains(&CardKind::Action))
        && def.script.play_after.is_none()
        && def.script.requires.is_none()
}

pub fn analyze(db: &CardDb, deck: &DeckList) -> Analysis {
    let cards: Vec<&CardDef> = deck.arsenal.iter().filter_map(|id| db.card_by_id(id)).map(|i| db.card(i)).collect();
    let mut a = Analysis::default();
    for c in &cards {
        a.curve[usize::from(c.fortitude).min(13)] += 1;
    }
    let total = cards.len().max(1) as f32;
    let maneuvers: Vec<&&CardDef> = cards.iter().filter(|c| is_maneuver(c)).collect();

    let damage = maneuvers.iter().map(|c| f32::from(c.damage)).sum::<f32>() / maneuvers.len().max(1) as f32;
    a.focus[0] = if maneuvers.is_empty() { 0.0 } else { share(damage - 3.0, 5.0) };
    a.why[0] = format!("Daño promedio de las maniobras: {damage:.1}.");

    let big = maneuvers.iter().filter(|c| c.damage >= 7 || is_finisher(c)).count();
    a.focus[1] = share(big as f32, 24.0);
    a.why[1] = format!("{big} maniobras de 7 de daño o más, o Finishers.");

    let fortitude = cards.iter().map(|c| f32::from(c.fortitude)).sum::<f32>() / total;
    a.focus[2] = if cards.is_empty() { 0.0 } else { (10.0 * (1.0 - fortitude / 10.0)).clamp(0.0, 10.0) };
    a.why[2] = format!("Fortitude promedio: {fortitude:.1}.");

    let reversals: Vec<&&CardDef> = cards.iter().filter(|c| c.is_reversal()).collect();
    let covers = |t: ManeuverType| {
        let probe = CardDef { kinds: vec![CardKind::Maneuver(t)], fortitude: 4, damage: 4, ..CardDef::default() };
        reversals.iter().any(|r| r.script.reverses.as_ref().is_some_and(|s| may_match(&probe, &s.targets)))
    };
    let covered: Vec<&str> = PROBES.iter().filter(|t| covers(**t)).map(|t| crate::text::maneuver(*t)).collect();
    let action = CardDef { kinds: vec![CardKind::Action], ..CardDef::default() };
    let stops_actions =
        reversals.iter().any(|r| r.script.reverses.as_ref().is_some_and(|s| may_match(&action, &s.targets)));
    let coverage = (covered.len() + usize::from(stops_actions)) as f32 / (PROBES.len() + 1) as f32;
    a.focus[3] = 0.6 * share(reversals.len() as f32, 24.0) + 4.0 * coverage;
    a.why[3] = format!(
        "{} Reversals; cubren {}{}.",
        reversals.len(),
        if covered.is_empty() { "ninguna maniobra".to_string() } else { covered.join(", ") },
        if stops_actions { " y Acciones" } else { "" }
    );

    let control = cards.iter().filter(|c| c.kinds.contains(&CardKind::Action) || controls(c)).count();
    a.focus[4] = share(control as f32, 30.0);
    a.why[4] = format!("{control} Acciones o cartas que roban, descartan o buscan.");

    let stun = cards.iter().map(|c| f32::from(c.stun_value)).sum::<f32>() / total;
    a.focus[5] = share(stun, 0.6);
    a.why[5] = format!("Stun Value promedio: {stun:.2}.");

    let hand_size = db.superstar_by_id(&deck.superstar).map_or(0, |s| usize::from(db.superstar(s).hand_size));
    a.stats = hand_stats(&cards, hand_size);
    a
}

/// Deals [`HANDS`] opening hands, always with the same shuffles.
pub fn hand_stats(cards: &[&CardDef], hand_size: usize) -> HandStats {
    let mut stats = HandStats { hand_size, ..HandStats::default() };
    if cards.is_empty() || hand_size == 0 {
        return stats;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let mut order: Vec<&CardDef> = cards.to_vec();
    for _ in 0..HANDS {
        order.shuffle(&mut rng);
        let hand = &order[..hand_size.min(order.len())];
        stats.playable += hand.iter().filter(|c| opener(c)).count() as f32;
        stats.with_reversal += f32::from(u8::from(hand.iter().any(|c| c.is_reversal())));
        let best = hand.iter().filter(|c| opener(c) && is_maneuver(c)).map(|c| c.damage).max();
        stats.with_opener += f32::from(u8::from(best.is_some()));
        stats.opener_damage += f32::from(best.unwrap_or(0));
    }
    let n = HANDS as f32;
    stats.playable /= n;
    stats.with_reversal /= n;
    stats.opener_damage /= stats.with_opener.max(1.0);
    stats.with_opener /= n;
    stats
}

/// Is `def` an Action or maneuver playable at 0 Fortitude?
pub fn playable_first(def: &CardDef) -> bool {
    opener(def)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decks_have_a_shape() {
        let db = rawdeal_cards::test_db();
        let deck = rawdeal_cards::test_deck(&db.superstars()[0].id);
        let a = analyze(&db, &deck);
        assert_eq!(a.curve.iter().sum::<usize>(), deck.arsenal.len());
        assert!(a.focus.iter().all(|v| (0.0..=10.0).contains(v)), "{:?}", a.focus);
        assert!(a.focus[2] > 0.0 && a.focus[3] > 0.0, "{:?}", a.why);
        assert!(a.stats.hand_size > 0);
        assert!((0.0..=1.0).contains(&a.stats.with_reversal));
        // Same shuffles every time.
        assert_eq!(analyze(&db, &deck).stats, a.stats);
    }

    #[test]
    fn empty_deck_is_all_zero() {
        let db = rawdeal_cards::test_db();
        let deck = DeckList { superstar: db.superstars()[0].id.clone(), ..DeckList::default() };
        let a = analyze(&db, &deck);
        assert_eq!(a.focus, [0.0; 6]);
        assert_eq!(a.stats.playable, 0.0);
    }
}
