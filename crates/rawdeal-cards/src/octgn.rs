//! Import card sets from OCTGN `set.xml` files.
//!
//! The importer copies what is printed on each card (type, F, D, SV, traits,
//! Superstar logo, text). Reversal targets are inferred from the card type and
//! a few common text patterns; Superstar abilities come from a hand-written
//! [`Overlay`]. Everything else in the card text is kept for display only.

use std::collections::BTreeMap;

use rawdeal_core::card::ReversalDamage;
use rawdeal_core::{
    Ability, CardDef, CardFilter, CardKind, CardScript, ManeuverType, ReverseSpec, SuperstarDef, Trait,
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
        let title = card.attribute("name").unwrap_or_default().trim().to_string();
        let source_id = card.attribute("id").map(str::to_string);
        let prop = |key: &str| {
            card.children()
                .find(|p| p.has_tag_name("property") && p.attribute("name") == Some(key))
                .and_then(|p| p.attribute("value"))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let number = |key: &str| prop(key).parse::<u8>().unwrap_or(0);
        let card_type = prop("CardType");
        let text = prop("Card Text");
        let id = format!("{prefix}/{}", slug(&title));

        if card_type == "Superstar Card" {
            let abilities = overlay.abilities.get(&title).cloned().unwrap_or_default();
            if overlay.abilities.contains_key(&title) {
                used.push(overlay.abilities.get_key_value(&title).unwrap().0);
            }
            set.superstars.push(SuperstarDef {
                id,
                name: title,
                hand_size: number("Hand Size"),
                superstar_value: number("SSV"),
                female: false,
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
        for part in card_type.split('/').map(str::trim) {
            match parse_kind(part) {
                Some((kind, rev)) => {
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                    reversal_type = reversal_type.or(rev);
                }
                // "Reversal: Strike/High Risk: Special": the rest of a Reversal's type.
                None if kinds.last() == Some(&CardKind::Reversal) => reversal_type = Some(ReversalType::Special),
                None => unknown = Some(part.to_string()),
            }
        }
        if let Some(part) = unknown {
            report.skipped.push((title, part));
            continue;
        }

        let (traits, other_traits) = parse_traits(&prop("Traits"));
        let superstar = prop("SS Logos")
            .split(',')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("{prefix}/{}", slug(s)));

        let mut script = overlay.scripts.get(&title).cloned().unwrap_or_default();
        if let Some((key, _)) = overlay.scripts.get_key_value(&title) {
            used.push(key);
        }
        if kinds.contains(&CardKind::Reversal) && script.reverses.is_none() {
            script.reverses = infer_reversal(reversal_type, &text);
            match script.reverses {
                Some(ref spec) => report.reversals_inferred.push((title.clone(), format!("{:?}", spec.targets))),
                None => report.reversals_unscripted.push(title.clone()),
            }
        }

        set.cards.push(CardDef {
            id,
            title,
            kinds,
            fortitude: number("Fortitude"),
            damage: number("Damage"),
            stun_value: number("Stun"),
            traits,
            superstar,
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
        if let Some(logo) = &mut card.superstar
            && !star_ids.contains(logo)
        {
            let short = logo.rsplit('/').next().unwrap_or_default().to_string();
            if let Some(full) = star_ids.iter().find(|id| id.ends_with(&format!("-{short}"))) {
                *logo = full.clone();
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

/// What a Reversal's printed type says it reverses.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReversalType {
    Maneuver(ManeuverType),
    Action,
    /// "Special": the card text says what it reverses.
    Special,
}

fn parse_kind(part: &str) -> Option<(CardKind, Option<ReversalType>)> {
    let maneuver = |t| Some((CardKind::Maneuver(t), None));
    match part {
        "Strike" => maneuver(ManeuverType::Strike),
        "Grapple" => maneuver(ManeuverType::Grapple),
        "Submission" => maneuver(ManeuverType::Submission),
        "High Risk" => maneuver(ManeuverType::HighRisk),
        "Trademark Finisher" => maneuver(ManeuverType::TrademarkFinisher),
        "Action" => Some((CardKind::Action, None)),
        _ => {
            let rest = part.strip_prefix("Reversal")?;
            let rev = if rest.contains("Special") {
                ReversalType::Special
            } else {
                match rest.trim_start_matches(':').trim() {
                    "Strike" => ReversalType::Maneuver(ManeuverType::Strike),
                    "Grapple" => ReversalType::Maneuver(ManeuverType::Grapple),
                    "Submission" => ReversalType::Maneuver(ManeuverType::Submission),
                    "High Risk" => ReversalType::Maneuver(ManeuverType::HighRisk),
                    "Action" => ReversalType::Action,
                    _ => ReversalType::Special,
                }
            };
            Some((CardKind::Reversal, Some(rev)))
        }
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
<card id="g4" name="Weird"><property name="CardType" value="Pre-match Venue" /></card>
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
        assert_eq!(report.skipped, vec![("Weird".to_string(), "Pre-match Venue".to_string())]);
        assert_eq!(report.unused_overlay, vec!["Nobody".to_string()]);
    }

    #[test]
    fn reversal_text_patterns() {
        let special = Some(ReversalType::Special);
        let targets = |t: &str| infer_reversal(special, t).map(|s| s.targets);
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
}
