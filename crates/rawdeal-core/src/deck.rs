//! Arsenal construction rules.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::card::{Ability, CardDef, CardFilter, CardKind, SuperstarDef, Trait};
use crate::db::CardDb;

pub const ARSENAL_SIZE: usize = 60;
pub const MAX_COPIES: usize = 3;
pub const BACKLASH_MAX: usize = 20;
pub const BACKLASH_MAX_PER_KIND: usize = 10;

/// A player's deck: Superstar plus Arsenal, by card id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckList {
    pub superstar: String,
    pub arsenal: Vec<String>,
    /// Optional Backlash deck for Advanced matches.
    #[serde(default)]
    pub backlash: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeckProblem {
    WrongSize(usize),
    UnknownCard(String),
    UnknownSuperstar(String),
    TooManyCopies {
        card: String,
        copies: usize,
        max: usize,
    },
    /// Two traits that cannot share an Arsenal (Face/Heel, Raw/SmackDown,
    /// Fan Favorite/Cheater).
    Conflict(Trait, Trait),
    /// A Superstar-specific card packed by a different Superstar.
    WrongSuperstar {
        card: String,
    },
    /// Pre-match and Mid-match cards go in the Backlash deck, not the Arsenal.
    BacklashCardInArsenal {
        card: String,
    },
    /// Only Pre-match and Mid-match cards may go in the Backlash deck.
    NotABacklashCard {
        card: String,
    },
    BacklashTooLarge(usize),
    TooManyPreMatch(usize),
    TooManyMidMatch(usize),
    /// The Superstar cannot pack this card.
    Forbidden {
        card: String,
    },
    /// Two cards that cannot share a deck.
    Exclusive {
        card: String,
        other: String,
    },
}

const CONFLICTS: [(Trait, Trait); 3] =
    [(Trait::Face, Trait::Heel), (Trait::Raw, Trait::SmackDown), (Trait::FanFavorite, Trait::Cheater)];

/// Returns every rule the deck breaks; empty means legal.
pub fn validate_deck(db: &CardDb, deck: &DeckList) -> Vec<DeckProblem> {
    let mut problems = Vec::new();
    let superstar = db.superstar_by_id(&deck.superstar).map(|i| db.superstar(i));
    if superstar.is_none() {
        problems.push(DeckProblem::UnknownSuperstar(deck.superstar.clone()));
    }
    if deck.arsenal.len() != ARSENAL_SIZE {
        problems.push(DeckProblem::WrongSize(deck.arsenal.len()));
    }

    if deck.backlash.len() > BACKLASH_MAX {
        problems.push(DeckProblem::BacklashTooLarge(deck.backlash.len()));
    }
    let (mut pre, mut mid) = (0, 0);
    for id in &deck.backlash {
        match db.card_by_id(id).map(|i| db.card(i)) {
            Some(def) if def.is_pre_match() => pre += 1,
            Some(def) if def.is_backlash() => mid += 1,
            Some(_) => problems.push(DeckProblem::NotABacklashCard { card: id.clone() }),
            None => {}
        }
    }
    if pre > BACKLASH_MAX_PER_KIND {
        problems.push(DeckProblem::TooManyPreMatch(pre));
    }
    if mid > BACKLASH_MAX_PER_KIND {
        problems.push(DeckProblem::TooManyMidMatch(mid));
    }
    for id in &deck.arsenal {
        if db.card_by_id(id).is_some_and(|i| db.card(i).is_backlash()) {
            problems.push(DeckProblem::BacklashCardInArsenal { card: id.clone() });
        }
    }

    // Copy limits and trait conflicts cover both decks. Reprints of a card in
    // other sets share its limit.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut per_title: HashMap<String, usize> = HashMap::new();
    for id in deck.arsenal.iter().chain(&deck.backlash) {
        *counts.entry(id).or_default() += 1;
        if let Some(i) = db.card_by_id(id) {
            *per_title.entry(db.card(i).title.to_lowercase()).or_default() += 1;
        }
    }
    let mut ids: Vec<&str> = counts.keys().copied().collect();
    ids.sort();

    let mut present = Vec::new();
    for id in ids {
        let Some(idx) = db.card_by_id(id) else {
            problems.push(DeckProblem::UnknownCard(id.to_string()));
            continue;
        };
        let def = db.card(idx);
        let forbidden = superstar.is_some_and(|s| {
            let named = |names: &[String]| names.iter().any(|n| n.eq_ignore_ascii_case(&s.name));
            s.abilities.iter().any(|a| match a {
                Ability::CannotPack(t) => t.eq_ignore_ascii_case(&def.title),
                Ability::CannotPackTrait(t) => def.has_trait(*t),
                Ability::CannotPackMatching(f) => printed_match(def, f),
                _ => false,
            }) || (!def.script.only_packed_by.is_empty() && !named(&def.script.only_packed_by))
                || named(&def.script.not_packed_by)
        });
        if forbidden {
            problems.push(DeckProblem::Forbidden { card: id.to_string() });
        }
        let copies = per_title[&def.title.to_lowercase()];
        let max = if def.has_trait(Trait::SetUp) {
            usize::MAX
        } else if def.has_trait(Trait::Unique) {
            1
        } else {
            MAX_COPIES
        };
        if copies > max {
            problems.push(DeckProblem::TooManyCopies { card: id.to_string(), copies, max });
        }
        match &def.superstar {
            // Superstar-specific cards ignore trait conflicts.
            Some(_) if !superstar.is_some_and(|star| def.logos().any(|s| may_pack_logo(star, s))) => {
                problems.push(DeckProblem::WrongSuperstar { card: id.to_string() })
            }
            Some(_) => {}
            None => present.extend(def.traits.iter().copied()),
        }
    }
    for id in deck.arsenal.iter().chain(&deck.backlash) {
        let Some(def) = db.card_by_id(id).map(|i| db.card(i)) else { continue };
        for other in &def.script.not_with {
            if per_title.contains_key(&other.to_lowercase()) {
                let problem = DeckProblem::Exclusive { card: def.title.clone(), other: other.clone() };
                if !problems.contains(&problem) {
                    problems.push(problem);
                }
            }
        }
    }
    let both = superstar.is_some_and(|s| s.abilities.contains(&Ability::PackBothAlignments));
    for (a, b) in CONFLICTS {
        if both && (a, b) == (Trait::Face, Trait::Heel) {
            continue;
        }
        if present.contains(&a) && present.contains(&b) {
            problems.push(DeckProblem::Conflict(a, b));
        }
    }
    problems
}

/// Does the printed card match `filter`? Filters about how a card is played
/// never match.
pub fn printed_match(def: &CardDef, filter: &CardFilter) -> bool {
    match filter {
        CardFilter::Any => true,
        CardFilter::Maneuver => def.kinds.iter().any(|k| matches!(k, CardKind::Maneuver(_))),
        CardFilter::ManeuverType(t) => def.kinds.contains(&CardKind::Maneuver(*t)),
        CardFilter::Action => def.kinds.contains(&CardKind::Action),
        CardFilter::Reversal => def.is_reversal(),
        CardFilter::Title(t) => crate::card::same_title(&def.title, t),
        CardFilter::TitleContains(w) => def.title.to_lowercase().contains(&w.to_lowercase()),
        CardFilter::Trait(t) => def.has_trait(*t),
        CardFilter::DamageAtMost(n) => def.damage <= *n,
        CardFilter::DamageAtLeast(n) => def.damage >= *n,
        CardFilter::Keyword(w) => def.other_traits.iter().any(|t| t.eq_ignore_ascii_case(w)),
        CardFilter::StunValueAtLeast(n) => def.stun_value >= *n,
        CardFilter::SuperstarSpecific => def.superstar.is_some(),
        CardFilter::All(fs) => fs.iter().all(|f| printed_match(def, f)),
        CardFilter::AnyOf(fs) => fs.iter().any(|f| printed_match(def, f)),
        CardFilter::Not(f) => !printed_match(def, f),
        _ => false,
    }
}

/// May `star` pack cards with this logo, as itself or as a Superstar it
/// counts as?
pub fn may_pack_logo(star: &SuperstarDef, logo: &str) -> bool {
    let slug = |name: &str| {
        let s: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
    };
    same_superstar(logo, &star.id)
        || star.abilities.iter().any(|a| match a {
            Ability::CountsAs(names) => names.iter().any(|n| same_superstar(logo, &slug(n))),
            _ => false,
        })
}

/// Does a card's Superstar logo (`"set/slug"`) name this Superstar? Logos
/// printed in one set name Superstars from other sets by the same slug, or by
/// a shorter name ("billy-gunn" for "b-a-billy-gunn").
pub fn same_superstar(logo: &str, superstar: &str) -> bool {
    let slug = |id: &str| id.rsplit('/').next().unwrap_or(id).to_string();
    let (logo, star) = (slug(logo), slug(superstar));
    logo == star || star.ends_with(&format!("-{logo}"))
}
