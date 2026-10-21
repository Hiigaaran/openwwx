//! Import card sets from OCTGN `set.xml` files.
//!
//! The importer copies what is printed on each card (type, F, D, SV, traits,
//! Superstar logo, text). Reversal targets are inferred from the card type and
//! a few common text patterns; Superstar abilities come from a hand-written
//! [`Overlay`]. Everything else in the card text is kept for display only.

use std::collections::BTreeMap;

use rawdeal_core::card::ReversalDamage;
use rawdeal_core::{
    Ability, CardDef, CardFilter, CardKind, CardScript, EnforcerDef, ManeuverType, PreMatchType, ReverseSpec,
    SuperstarDef, Trait,
};
use serde::{Deserialize, Serialize};

use crate::CardSet;

/// Scripts written by hand for cards whose text the importer cannot infer,
/// keyed by card title.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    #[serde(default)]
    pub abilities: BTreeMap<String, Vec<Ability>>,
    #[serde(default)]
    pub scripts: BTreeMap<String, CardScript>,
}

/// What the import did, for review.
#[derive(Debug, Default)]
pub struct Report {
    pub cards: usize,
    pub superstars: usize,
    /// Reversals whose targets were inferred, with what they reverse.
    pub reversals_inferred: Vec<(String, String)>,
    /// Reversals left without targets: they cannot reverse anything yet.
    pub reversals_unscripted: Vec<String>,
    /// Card types the importer does not understand; these cards were skipped.
    pub skipped: Vec<(String, String)>,
    /// Overlay entries that matched no card.
    pub unused_overlay: Vec<String>,
}

/// Lowercase ASCII slug: "Stone Cold Steve Austin" -> "stone-cold-steve-austin".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Converts one OCTGN set. Card ids are `"{prefix}/{slug of title}"`.
pub fn import_set(xml: &str, name: &str, prefix: &str, overlay: &Overlay) -> Result<(CardSet, Report), String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut report = Report::default();
    let mut set = CardSet { name: name.to_string(), superstars: vec![], cards: vec![] };
    let mut used: Vec<&str> = Vec::new();

    for card in doc.descendants().filter(|n| n.has_tag_name("card")) {
        let mut title = card.attribute("name").unwrap_or_default().trim().to_string();
        let source_id = card.attribute("id").map(str::to_string);
        let prop = |key: &str| {
            card.children()
                .find(|p| p.has_tag_name("property") && p.attribute("name") == Some(key))
                .and_then(|p| p.attribute("value"))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        // "1+" (a variable Hand Size) reads as its printed number.
        let number = |key: &str| {
            let v = prop(key);
            v.trim_end_matches('+').trim().parse::<u8>().unwrap_or(0)
        };
        let card_type = prop("CardType");
        let text = prop("Card Text");
        // Two different cards with one title (V5's "Dream Street"): the later one
        // is told apart by its last logo, "Dream Street (The Legacy)".
        if card_type != "Superstar Card" && set.cards.iter().any(|c| c.title.eq_ignore_ascii_case(&title)) {
            let logos = prop("SS Logos");
            match logos.rsplit(',').map(str::trim).find(|l| !l.is_empty()) {
                Some(logo) => title = format!("{title} ({logo})"),
                None => title = format!("{title} (2)"),
            }
        }
        let id = format!("{prefix}/{}", slug(&title));

        if card_type == "Superstar Card" {
            let abilities = overlay.abilities.get(&title).cloned().unwrap_or_default();
            if overlay.abilities.contains_key(&title) {
                used.push(overlay.abilities.get_key_value(&title).unwrap().0);
            }
            set.superstars.push(SuperstarDef {
                id,
                female: is_female(&title) || traits_of(&prop("Traits")).iter().any(|t| t.eq_ignore_ascii_case("diva")),
                traits: traits_of(&prop("Traits")),
                name: title,
                hand_size: number("Hand Size"),
                superstar_value: number("SSV"),
                text,
                abilities,
                source_id,
            });
            report.superstars += 1;
            continue;
        }

        let mut kinds = Vec::new();
        let mut reversal_type = None;
        let mut unknown = None;
        // Backlash cards: "Pre-match Event", "Mid-match Reversal: Special".
        let mut mid_match = false;
        // Revolution: "Throw: Mean: Follow-up: Card", "Antic: Momentum".
        let mut type_traits = Vec::new();
        let mut follow_up = None;
        // "Strike + Grapple" can be played as either type.
        let mut type_words = Vec::new();
        for part in card_type.split(" / ").flat_map(|p| p.split(" + ")).map(str::trim) {
            // Virtual sets write "Pre-match: Event" and "Mid-match: Antic".
            let part = part.replacen("Pre-match:", "Pre-match", 1).replacen("Mid-match:", "Mid-match", 1);
            let part = part.trim();
            let part = match part.strip_prefix("Mid-match ") {
                Some(rest) => {
                    mid_match = true;
                    rest
                }
                None => part,
            };
            match parse_kind(part) {
                Some(parsed) => {
                    if !kinds.contains(&parsed.kind) {
                        kinds.push(parsed.kind);
                    }
                    reversal_type = reversal_type.or(parsed.reversal);
                    for t in parsed.traits {
                        if !type_traits.contains(&t) {
                            type_traits.push(t);
                        }
                    }
                    follow_up = follow_up.or(parsed.follow_up);
                    type_words.extend(parsed.words);
                }
                None => unknown = Some(part.to_string()),
            }
        }
        if let Some(part) = unknown {
            report.skipped.push((title, part));
            continue;
        }

        let (mut traits, mut other_traits) = parse_traits(&prop("Traits"));
        // Traits written on the type line: "Strike: Heel", "Action: Movie".
        let (known, other) = parse_traits(&type_words.join(","));
        for t in known {
            if !traits.contains(&t) {
                traits.push(t);
            }
        }
        for t in other {
            if !other_traits.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
                other_traits.push(t);
            }
        }
        if mid_match && !traits.contains(&Trait::MidMatch) {
            traits.push(Trait::MidMatch);
        }
        for t in type_traits {
            if !traits.contains(&t) {
                traits.push(t);
            }
        }
        let mut logos: Vec<String> = prop("SS Logos")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("{prefix}/{}", slug(s)))
            .collect();
        let superstar = (!logos.is_empty()).then(|| logos.remove(0));

        let mut script = overlay.scripts.get(&title).cloned().unwrap_or_default();
        if let Some((key, _)) = overlay.scripts.get_key_value(&title) {
            used.push(key);
        }
        for t in std::mem::take(&mut script.extra_traits) {
            if !traits.contains(&t) {
                traits.push(t);
            }
        }
        if script.play_after.is_none() {
            script.play_after = follow_up;
        }
        if script.requires.is_none() && text.contains("Requirement: Extremist") {
            script.requires = Some(rawdeal_core::Condition::YouAre("Extremist".into()));
        }
        if kinds.contains(&CardKind::Reversal) && script.reverses.is_none() {
            script.reverses = infer_reversal(reversal_type, &text);
            match script.reverses {
                Some(ref spec) => report.reversals_inferred.push((title.clone(), format!("{:?}", spec.targets))),
                None => report.reversals_unscripted.push(title.clone()),
            }
        }

        let enforcers = if kinds.contains(&CardKind::Enforcer) {
            parse_enforcers(&title, &text, std::mem::take(&mut script.enforcer_abilities))
        } else {
            vec![]
        };
        let targets = prop("CardRef-Target").split_whitespace().map(str::to_lowercase).collect();
        set.cards.push(CardDef {
            targets,
            enforcers,
            id,
            title,
            kinds,
            fortitude: number("Fortitude"),
            damage: number("Damage"),
            stun_value: number("Stun"),
            traits,
            superstar,
            also_superstars: logos,
            text,
            other_traits,
            source_id,
            script,
        });
        report.cards += 1;
    }

    // Logos may shorten a Superstar's name ("Billy Gunn" for "B.A. Billy Gunn").
    let star_ids: Vec<String> = set.superstars.iter().map(|s| s.id.clone()).collect();
    for card in &mut set.cards {
        for logo in card.superstar.iter_mut().chain(&mut card.also_superstars) {
            if !star_ids.contains(logo) {
                let short = logo.rsplit('/').next().unwrap_or_default().to_string();
                if let Some(full) = star_ids.iter().find(|id| id.ends_with(&format!("-{short}"))) {
                    *logo = full.clone();
                }
            }
        }
    }

    report.unused_overlay = overlay
        .abilities
        .keys()
        .chain(overlay.scripts.keys())
        .filter(|k| !used.contains(&k.as_str()))
        .cloned()
        .collect();
    Ok((set, report))
}

/// The Enforcers printed on an Enforcer card. The title names them ("Stone
/// Cold, Triple H, or ... Christian?") and the text has one "Hand Size: +n
/// Superstar Value: +n Superstar Ability: +text" block each.
fn parse_enforcers(title: &str, text: &str, mut abilities: Vec<Vec<Ability>>) -> Vec<EnforcerDef> {
    let names: Vec<String> = title
        .trim_end_matches('?')
        .replace(", or", ",")
        .replace(" or ", ",")
        .split(',')
        .map(|n| n.trim().trim_start_matches('.').trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let blocks: Vec<&str> = text.split("Hand Size:").skip(1).collect();
    let number = |s: &str| -> u8 {
        let digits: String = s.trim().trim_start_matches('+').chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().unwrap_or(0)
    };
    blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            let block = block.trim().trim_end_matches('≥').trim();
            let (hand, rest) = block.split_once("Superstar Value:").unwrap_or((block, ""));
            let (value, ability) = rest.split_once("Superstar Ability:").unwrap_or((rest, ""));
            let name = if names.len() == blocks.len() { names[i].clone() } else { format!("Enforcer {}", i + 1) };
            EnforcerDef {
                name,
                hand_size: number(hand),
                superstar_value: number(value),
                text: ability.trim().trim_start_matches('+').trim().trim_end_matches('≥').trim().to_string(),
                abilities: if i < abilities.len() { std::mem::take(&mut abilities[i]) } else { vec![] },
            }
        })
        .collect()
}

/// What a Reversal's printed type says it reverses.
#[derive(Debug, Clone, PartialEq)]
enum ReversalType {
    Maneuver(ManeuverType),
    Action,
    /// Revolution types: "Reversal: Mean or Colossal", "Reversal: non-Unique Maneuver".
    Filter(CardFilter),
    /// "Special": the card text says what it reverses.
    Special,
}

/// One function read from a card type, with what the type line adds to it.
struct ParsedKind {
    kind: CardKind,
    reversal: Option<ReversalType>,
    /// Revolution: Core, Mean, Colossal, Momentum, Follow-up.
    traits: Vec<Trait>,
    /// "Follow-up: X": what must have been played right before.
    follow_up: Option<CardFilter>,
    /// Printed traits written after the type ("Strike: Heel", "Action: Movie").
    words: Vec<String>,
}

fn parse_kind(part: &str) -> Option<ParsedKind> {
    let simple = |kind| Some(ParsedKind { kind, reversal: None, traits: vec![], follow_up: None, words: vec![] });
    let pre = |t| simple(CardKind::PreMatch(t));
    // Virtual types.
    match part {
        "TMF" => return simple(CardKind::Maneuver(ManeuverType::TrademarkFinisher)),
        "Backstage Card" => return simple(CardKind::Backstage),
        "Storyline" => return pre(PreMatchType::Storyline),
        "Pre-match Pay-Per-View Event" => {
            let mut p = pre(PreMatchType::Event)?;
            p.words.push("Pay-Per-View".into());
            return Some(p);
        }
        _ => {}
    }
    if part.starts_with("Pre-match Allegiance") || part.starts_with("Pre-match Allegience") {
        return pre(PreMatchType::Allegiance);
    }
    // A classic type followed by printed traits: "Strike: Heel", "Action: Movie",
    // "Backstage Card: PRO", "Pre-match Event: Heat".
    if let Some((head, rest)) = part.split_once(':')
        && !head.starts_with("Reversal")
        && !matches!(head.trim(), "Throw" | "Hold" | "Assault" | "Trademark" | "Extreme" | "Antic" | "Maneuver")
        && let Some(mut p) = parse_kind(head.trim())
        && p.reversal.is_none()
    {
        p.words.extend(rest.split(':').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()));
        return Some(p);
    }
    match part {
        "Strike" => return simple(CardKind::Maneuver(ManeuverType::Strike)),
        "Grapple" => return simple(CardKind::Maneuver(ManeuverType::Grapple)),
        "Submission" => return simple(CardKind::Maneuver(ManeuverType::Submission)),
        "High Risk" => return simple(CardKind::Maneuver(ManeuverType::HighRisk)),
        "Trademark Finisher" => return simple(CardKind::Maneuver(ManeuverType::TrademarkFinisher)),
        "Action" => return simple(CardKind::Action),
        "Enforcer" => return simple(CardKind::Enforcer),
        "Pre-match Venue" => return pre(PreMatchType::Venue),
        "Pre-match Feud" => return pre(PreMatchType::Feud),
        "Pre-match Stipulation" => return pre(PreMatchType::Stipulation),
        "Pre-match Manager" => return pre(PreMatchType::Manager),
        "Pre-match Event" => return pre(PreMatchType::Event),
        "Pre-match Object" => return pre(PreMatchType::Object),
        "Pre-match Corner" => return pre(PreMatchType::Corner),
        "Pre-match Faction" => return pre(PreMatchType::Faction),
        "Pre-match Storyline" => return pre(PreMatchType::Storyline),
        _ => {}
    }
    if let Some(rest) = part.strip_prefix("Reversal") {
        let rest = rest.trim_start_matches(':').trim();
        let rev = if rest.contains("Special") || rest.contains('/') {
            ReversalType::Special
        } else {
            match rest {
                "Strike" => ReversalType::Maneuver(ManeuverType::Strike),
                "Grapple" => ReversalType::Maneuver(ManeuverType::Grapple),
                "Submission" => ReversalType::Maneuver(ManeuverType::Submission),
                "High Risk" => ReversalType::Maneuver(ManeuverType::HighRisk),
                "Action" => ReversalType::Action,
                _ => parse_card_expr(rest).map_or(ReversalType::Special, ReversalType::Filter),
            }
        };
        return Some(ParsedKind {
            kind: CardKind::Reversal,
            reversal: Some(rev),
            traits: vec![],
            follow_up: None,
            words: vec![],
        });
    }

    // Revolution: "Trademark: Mean: Follow-up: Maneuver", "Antic: Momentum".
    let mut segments = part.split(':').map(str::trim);
    let kind = match segments.next()? {
        "Throw" => CardKind::Maneuver(ManeuverType::Throw),
        "Hold" => CardKind::Maneuver(ManeuverType::Hold),
        "Assault" => CardKind::Maneuver(ManeuverType::Assault),
        "Trademark" => CardKind::Maneuver(ManeuverType::Trademark),
        "Extreme" => CardKind::Maneuver(ManeuverType::Extreme),
        "Antic" => CardKind::Antic,
        _ => return None,
    };
    let (mut traits, mut follow_up) = (vec![], None);
    while let Some(seg) = segments.next() {
        match seg.to_ascii_lowercase().as_str() {
            "core" => traits.push(Trait::Core),
            "mean" => traits.push(Trait::Mean),
            "colossal" => traits.push(Trait::Colossal),
            "momentum" => traits.push(Trait::Momentum),
            "follow-up" | "followup" => {
                traits.push(Trait::FollowUp);
                follow_up = Some(parse_card_expr(segments.next()?)?);
            }
            _ => return None,
        }
    }
    Some(ParsedKind { kind, reversal: None, traits, follow_up, words: vec![] })
}

/// Reads a Revolution card description such as "Mean or Colossal",
/// "non-Core non-Follow-up Throw", "printed 3D∙7D Assault" or "Hold: Mean".
/// Returns `None` for descriptions it cannot express ("Maneuver with Target 'head'").
fn parse_card_expr(expr: &str) -> Option<CardFilter> {
    let expr = expr.trim();
    if let Some(rest) = expr.strip_prefix("printed ") {
        let (range, rest) = rest.split_once(' ')?;
        let numbers: Vec<u8> = range.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
        let [low, high] = numbers[..] else { return None };
        let what = parse_card_expr(rest)?;
        return Some(CardFilter::All(vec![CardFilter::DamageAtLeast(low), CardFilter::DamageAtMost(high), what]));
    }
    let alternatives: Vec<&str> = expr
        .split(',')
        .flat_map(|p| p.split(" or "))
        .map(|p| p.trim().trim_start_matches("or ").trim())
        .filter(|p| !p.is_empty())
        .collect();
    let mut any = Vec::new();
    for alt in alternatives {
        let alt = alt.replace("High Risk", "HighRisk").replace("Trademark Finisher", "TrademarkFinisher");
        let mut all = Vec::new();
        for token in alt.split(':').flat_map(str::split_whitespace) {
            let (negated, word) = match token.strip_prefix("non-") {
                Some(w) => (true, w),
                None => (false, token),
            };
            let f = match word.to_ascii_lowercase().as_str() {
                "strike" => CardFilter::ManeuverType(ManeuverType::Strike),
                "grapple" => CardFilter::ManeuverType(ManeuverType::Grapple),
                "submission" => CardFilter::ManeuverType(ManeuverType::Submission),
                "highrisk" => CardFilter::ManeuverType(ManeuverType::HighRisk),
                "trademarkfinisher" => CardFilter::ManeuverType(ManeuverType::TrademarkFinisher),
                "throw" | "throws" => CardFilter::ManeuverType(ManeuverType::Throw),
                "hold" | "holds" => CardFilter::ManeuverType(ManeuverType::Hold),
                "assault" | "assaults" => CardFilter::ManeuverType(ManeuverType::Assault),
                "trademark" | "trademarks" => CardFilter::ManeuverType(ManeuverType::Trademark),
                "extreme" | "extremes" => CardFilter::ManeuverType(ManeuverType::Extreme),
                "maneuver" | "maneuvers" => CardFilter::Maneuver,
                "card" | "cards" => CardFilter::Any,
                "action" | "actions" => CardFilter::Action,
                "antic" | "antics" => CardFilter::Antic,
                "reversal" | "reversals" => CardFilter::Reversal,
                "core" | "cores" => CardFilter::Trait(Trait::Core),
                "mean" | "means" => CardFilter::Trait(Trait::Mean),
                "colossal" | "colossals" => CardFilter::Trait(Trait::Colossal),
                "momentum" => CardFilter::Trait(Trait::Momentum),
                "unique" => CardFilter::Trait(Trait::Unique),
                "follow-up" | "followup" => CardFilter::Trait(Trait::FollowUp),
                "mid-match" => CardFilter::Trait(Trait::MidMatch),
                "hybrid" => CardFilter::Hybrid,
                _ => return None,
            };
            all.push(if negated { CardFilter::Not(Box::new(f)) } else { f });
        }
        any.push(match all.len() {
            0 => return None,
            1 => all.remove(0),
            _ => CardFilter::All(all),
        });
    }
    match any.len() {
        0 => None,
        1 => Some(any.remove(0)),
        _ => Some(CardFilter::AnyOf(any)),
    }
}

fn parse_traits(traits: &str) -> (Vec<Trait>, Vec<String>) {
    let (mut known, mut other) = (Vec::new(), Vec::new());
    for t in traits.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match t.to_ascii_lowercase().as_str() {
            "unique" => known.push(Trait::Unique),
            "face" => known.push(Trait::Face),
            "heel" => known.push(Trait::Heel),
            "set-up" | "setup" => known.push(Trait::SetUp),
            "raw" => known.push(Trait::Raw),
            "smackdown" | "smackdown!" => known.push(Trait::SmackDown),
            "multi" => known.push(Trait::Multi),
            "chain" => known.push(Trait::Chain),
            "irreversible" => known.push(Trait::Irreversible),
            "active" => known.push(Trait::Active),
            "pay-per-view" | "pay-per-view event" => {
                known.push(Trait::PayPerView);
                other.push(t.to_string());
            }
            // Also kept as text, so `Keyword("Fan Favorite")` filters still match.
            "fan favorite" | "cheater" | "universally unique" => {
                known.push(match t.to_ascii_lowercase().as_str() {
                    "fan favorite" => Trait::FanFavorite,
                    "cheater" => Trait::Cheater,
                    _ => Trait::UniversallyUnique,
                });
                other.push(t.to_string());
            }
            _ => other.push(t.to_string()),
        }
    }
    (known, other)
}

/// Infers what a Reversal stops. Returns `None` when the text has conditions
/// the engine cannot express yet (e.g. "played after Irish Whip", tag
/// partners), so such cards never reverse by mistake.
fn infer_reversal(rev: Option<ReversalType>, text: &str) -> Option<ReverseSpec> {
    let spec = |targets| Some(ReverseSpec::new(targets, ReversalDamage::Printed));
    match rev? {
        ReversalType::Maneuver(t) => return spec(CardFilter::ManeuverType(t)),
        ReversalType::Action => return spec(CardFilter::Action),
        ReversalType::Filter(f) => return spec(f),
        ReversalType::Special => {}
    }
    let lower = text.to_ascii_lowercase();
    const UNSUPPORTED: [&str; 6] = ["irish whip", "inactive partner", "heel", "set-up", "if that opponent", "-10f"];
    if UNSUPPORTED.iter().any(|u| lower.contains(u)) {
        return None;
    }

    for phrase in ["may only reverse the maneuver titled ", "reverse the card titled "] {
        if let Some(rest) = lower.split(phrase).nth(1) {
            let start = lower.len() - rest.len();
            let title = text[start..].split(" and end").next().unwrap_or_default();
            return spec(CardFilter::Title(title.trim_end_matches('.').trim().to_string()));
        }
    }

    let after = lower.split("reverse").nth(1)?.trim_start_matches('s').trim_start();
    let clause = after.strip_prefix("any ")?;
    let clause = clause.split(['.', ';']).next().unwrap_or_default();
    let clause = clause.split(" and end").next().unwrap_or_default().trim();

    let (subject, limit) = match clause.split_once(" of ") {
        Some((subject, limit)) => (subject, Some(limit)),
        None => (clause, None),
    };
    let mut filters = Vec::new();
    let types: Vec<ManeuverType> = subject
        .trim_end_matches("maneuver")
        .split([',', ' '])
        .filter_map(|w| match w.trim() {
            "strike" => Some(ManeuverType::Strike),
            "grapple" => Some(ManeuverType::Grapple),
            "submission" => Some(ManeuverType::Submission),
            "high" => Some(ManeuverType::HighRisk),
            _ => None,
        })
        .collect();
    match (subject.trim(), types.len()) {
        ("maneuver", _) => filters.push(CardFilter::Maneuver),
        (_, 0) => return None,
        (_, 1) => filters.push(CardFilter::ManeuverType(types[0])),
        _ => filters.push(CardFilter::AnyOf(types.into_iter().map(CardFilter::ManeuverType).collect())),
    }
    if let Some(limit) = limit {
        let n: u8 = limit.split('d').next()?.trim().parse().ok()?;
        if !limit.contains("or less") {
            return None;
        }
        filters.push(CardFilter::DamageAtMost(n));
    }
    let targets = if filters.len() == 1 { filters.remove(0) } else { CardFilter::All(filters) };
    let damage = if lower.contains("# is equal to the reversed maneuver's damage") {
        ReversalDamage::SameAsReversed
    } else {
        ReversalDamage::Printed
    };
    Some(ReverseSpec::new(targets, damage))
}

fn traits_of(traits: &str) -> Vec<String> {
    traits.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// OCTGN has no gender field, so women Superstars (and all-women teams) are listed by name.
const FEMALE_SUPERSTARS: &[&str] = &[
    "A.J. Lee",
    "Alexa Bliss",
    "Asuka",
    "Babe of the Year",
    "Bayley",
    "Becky Lynch",
    "Beth Phoenix",
    "Bianca Belair",
    "Brie Bella",
    "Carmella",
    "Charlotte",
    "Christy",
    "Chyna",
    "Chyna Doll",
    "Crimson Goddess",
    "Dakota Kai",
    "Damage CTRL",
    "Ember Moon",
    "Emma",
    "Eve",
    "Gail Kim",
    "Gail Kim and Molly Holly",
    "IYO SKY",
    "Jacqueline",
    "Kairi Sane",
    "Lacey Evans",
    "Lay-Cool",
    "Lita",
    "Liv Morgan",
    "Mae Young",
    "Mandy Rose",
    "Maryse",
    "Melina",
    "Mickie James",
    "Molly Holly",
    "Naomi",
    "Natalya",
    "Nia Jax",
    "Nidia",
    "Nikki Bella",
    "Paige",
    "Rhea Ripley",
    "Ronda Rousey",
    "Sable",
    "Sasha Banks",
    "Shayna Baszler",
    "Sherri Martel",
    "Stacy Keibler",
    "Stephanie McMahon",
    "Summer Rae",
    "The Bella Twins",
    "The Fabulous Moolah",
    "The Iiconics",
    "The Legit Boss",
    "The Man",
    "The Role Model",
    "Tiffany Stratton",
    "Torrie Wilson",
    "Toxic Attraction",
    "Trish Stratus",
    "Vickie Guerrero",
    "Victoria",
    "X-treme Diva",
];

fn is_female(name: &str) -> bool {
    FEMALE_SUPERSTARS.iter().any(|n| n.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<set name="01. Test" id="x"><cards>
<card id="g1" name="Big Hit"><property name="Fortitude" value="2" /><property name="Damage" value="4" />
  <property name="CardType" value="Strike" /><property name="Stun" value="1" /><property name="Traits" value="" />
  <property name="SS Logos" value="" /><property name="Card Text" value="" /></card>
<card id="g2" name="Duck Under"><property name="Fortitude" value="1" /><property name="Damage" value="" />
  <property name="CardType" value="Reversal: Strike: Special" /><property name="Traits" value="Unique, Tag" />
  <property name="SS Logos" value="Some Guy, Other Name" />
  <property name="Card Text" value="Reverse any Strike maneuver of 7D or less and end your opponent's turn. # is equal to the reversed maneuver's Damage." /></card>
<card id="g3" name="Some Guy"><property name="Hand Size" value="6" /><property name="SSV" value="3" />
  <property name="CardType" value="Superstar Card" /><property name="Card Text" value="Ability." /></card>
<card id="g4" name="Weird"><property name="CardType" value="Pre-match Banana" /></card>
</cards></set>"#;

    #[test]
    fn imports_cards_superstars_and_reversals() {
        let mut overlay = Overlay::default();
        overlay.abilities.insert("Some Guy".into(), vec![Ability::DrawSegment(2)]);
        overlay.scripts.insert("Nobody".into(), CardScript::default());
        let (set, report) = import_set(XML, "Test", "t", &overlay).unwrap();

        assert_eq!(set.cards.len(), 2);
        let hit = &set.cards[0];
        assert_eq!((hit.id.as_str(), hit.fortitude, hit.damage, hit.stun_value), ("t/big-hit", 2, 4, 1));
        assert_eq!(hit.source_id.as_deref(), Some("g1"));

        let duck = &set.cards[1];
        assert_eq!(duck.kinds, vec![CardKind::Reversal]);
        assert_eq!(duck.traits, vec![Trait::Unique]);
        assert_eq!(duck.other_traits, vec!["Tag".to_string()]);
        assert_eq!(duck.superstar.as_deref(), Some("t/some-guy"));
        let rev = duck.script.reverses.as_ref().unwrap();
        assert_eq!(
            rev.targets,
            CardFilter::All(vec![CardFilter::ManeuverType(ManeuverType::Strike), CardFilter::DamageAtMost(7)])
        );
        assert_eq!(rev.damage, ReversalDamage::SameAsReversed);

        assert_eq!(set.superstars[0].hand_size, 6);
        assert_eq!(set.superstars[0].abilities, vec![Ability::DrawSegment(2)]);
        assert_eq!(report.skipped, vec![("Weird".to_string(), "Pre-match Banana".to_string())]);
        assert_eq!(report.unused_overlay, vec!["Nobody".to_string()]);
    }

    #[test]
    fn reversal_text_patterns() {
        let special = Some(ReversalType::Special);
        let targets = |t: &str| infer_reversal(special.clone(), t).map(|s| s.targets);
        assert_eq!(
            targets("Reverse any maneuver and end your opponent's turn. Draw 1 card."),
            Some(CardFilter::Maneuver)
        );
        assert_eq!(targets("May only reverse the maneuver titled Kick."), Some(CardFilter::Title("Kick".into())));
        assert_eq!(
            targets("Reverse any Strike, Grapple, or Submission maneuver and end your opponent's turn."),
            Some(CardFilter::AnyOf(
                [ManeuverType::Strike, ManeuverType::Grapple, ManeuverType::Submission]
                    .map(CardFilter::ManeuverType)
                    .to_vec()
            ))
        );
        assert_eq!(targets("Reverses any maneuver and ends your opponent's turn."), Some(CardFilter::Maneuver));
        assert_eq!(targets("May only reverse a maneuver played after the card titled Irish Whip."), None);
        assert_eq!(
            targets("As a reversal, reverse the card titled Back Body Drop and end your opponent's turn."),
            Some(CardFilter::Title("Back Body Drop".into()))
        );
        assert_eq!(targets("When played from your hand by an inactive partner, reverse any maneuver."), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Don't You Never ... EVER!"), "don-t-you-never-ever");
        assert_eq!(slug("Stone Cold Steve Austin"), "stone-cold-steve-austin");
    }

    #[test]
    fn revolution_types() {
        use CardFilter as F;
        let p = parse_kind("Trademark: Mean: Follow-up: Maneuver").unwrap();
        assert_eq!(p.kind, CardKind::Maneuver(ManeuverType::Trademark));
        assert_eq!(p.traits, vec![Trait::Mean, Trait::FollowUp]);
        assert_eq!(p.follow_up, Some(F::Maneuver));
        let p = parse_kind("Antic: Momentum").unwrap();
        assert_eq!((p.kind, p.traits), (CardKind::Antic, vec![Trait::Momentum]));
        assert!(parse_kind("Pre-match Storyline").is_some());
        let rev = |t: &str| match parse_kind(t).unwrap().reversal {
            Some(ReversalType::Filter(f)) => Some(f),
            _ => None,
        };
        assert_eq!(
            rev("Reversal: Mean or Colossal"),
            Some(F::AnyOf(vec![F::Trait(Trait::Mean), F::Trait(Trait::Colossal)]))
        );
        assert_eq!(
            rev("Reversal: non-Core non-Follow-up Throw"),
            Some(F::All(vec![
                F::Not(Box::new(F::Trait(Trait::Core))),
                F::Not(Box::new(F::Trait(Trait::FollowUp))),
                F::ManeuverType(ManeuverType::Throw)
            ]))
        );
        assert_eq!(
            rev("Reversal: printed 3D\u{2219}7D Assault"),
            Some(F::All(vec![F::DamageAtLeast(3), F::DamageAtMost(7), F::ManeuverType(ManeuverType::Assault)]))
        );
        assert_eq!(
            rev("Reversal: Hold: Mean"),
            Some(F::All(vec![F::ManeuverType(ManeuverType::Hold), F::Trait(Trait::Mean)]))
        );
        assert_eq!(rev("Reversal: Maneuver with Target 'head' or 'leg'"), None);
    }

    #[test]
    fn a_repeated_title_is_told_apart_by_its_logo() {
        let card = |id: &str, logos: &str| {
            format!(
                r#"<card id="{id}" name="Dream Street"><property name="CardType" value="Action" />
  <property name="SS Logos" value="{logos}" /><property name="Card Text" value="" /></card>"#
            )
        };
        let xml = format!(
            r#"<set name="31. V5" id="x"><cards>{}{}</cards></set>"#,
            card("a", "Ted DiBiase"),
            card("b", "Priceless, The Legacy")
        );
        let (set, _) = import_set(&xml, "V5", "v", &Overlay::default()).unwrap();
        let titles: Vec<&str> = set.cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Dream Street", "Dream Street (The Legacy)"]);
        assert_eq!(set.cards[1].id, "v/dream-street-the-legacy");
    }

    #[test]
    fn virtual_types() {
        let p = parse_kind("Strike: Heel").unwrap();
        assert_eq!((p.kind, p.words), (CardKind::Maneuver(ManeuverType::Strike), vec!["Heel".to_string()]));
        assert_eq!(parse_kind("TMF").unwrap().kind, CardKind::Maneuver(ManeuverType::TrademarkFinisher));
        assert_eq!(parse_kind("Backstage Card").unwrap().kind, CardKind::Backstage);
        let p = parse_kind("Backstage Card: PRO").unwrap();
        assert_eq!((p.kind, p.words), (CardKind::Backstage, vec!["PRO".to_string()]));
        assert_eq!(parse_kind("Pre-match Allegiance: nWo").unwrap().kind, CardKind::PreMatch(PreMatchType::Allegiance));
        let p = parse_kind("Pre-match Pay-Per-View Event").unwrap();
        assert_eq!(p.kind, CardKind::PreMatch(PreMatchType::Event));
        assert!(parse_kind("Maneuver: Special").is_none());
    }
}
