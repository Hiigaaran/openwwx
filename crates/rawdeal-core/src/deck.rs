//! Arsenal construction rules.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::card::Trait;
use crate::db::CardDb;

pub const ARSENAL_SIZE: usize = 60;
pub const MAX_COPIES: usize = 3;

/// A player's deck: Superstar plus Arsenal, by card id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckList {
    pub superstar: String,
    pub arsenal: Vec<String>,
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
}

const CONFLICTS: [(Trait, Trait); 3] =
    [(Trait::Face, Trait::Heel), (Trait::Raw, Trait::SmackDown), (Trait::FanFavorite, Trait::Cheater)];

/// Returns every rule the deck breaks; empty means legal.
pub fn validate_deck(db: &CardDb, deck: &DeckList) -> Vec<DeckProblem> {
    let mut problems = Vec::new();
    if db.superstar_by_id(&deck.superstar).is_none() {
        problems.push(DeckProblem::UnknownSuperstar(deck.superstar.clone()));
    }
    if deck.arsenal.len() != ARSENAL_SIZE {
        problems.push(DeckProblem::WrongSize(deck.arsenal.len()));
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for id in &deck.arsenal {
        *counts.entry(id).or_default() += 1;
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
        let copies = counts[id];
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
            Some(s) if *s != deck.superstar => problems.push(DeckProblem::WrongSuperstar { card: id.to_string() }),
            Some(_) => {}
            None => present.extend(def.traits.iter().copied()),
        }
    }
    for (a, b) in CONFLICTS {
        if present.contains(&a) && present.contains(&b) {
            problems.push(DeckProblem::Conflict(a, b));
        }
    }
    problems
}
