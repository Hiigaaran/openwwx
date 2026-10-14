//! Card definitions for OpenWWX and loaders for card data files.

use rawdeal_core::{CardDb, CardDef, DeckList, SuperstarDef};
use serde::{Deserialize, Serialize};

/// A card data file: one set of Superstars and cards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardSet {
    pub name: String,
    pub superstars: Vec<SuperstarDef>,
    pub cards: Vec<CardDef>,
}

#[derive(Debug)]
pub enum LoadError {
    Parse(ron::error::SpannedError),
    Db(rawdeal_core::db::DbError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(e) => write!(f, "card data: {e}"),
            LoadError::Db(e) => write!(f, "card data: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

pub fn parse_set(ron_text: &str) -> Result<CardSet, LoadError> {
    ron::from_str(ron_text).map_err(LoadError::Parse)
}

/// Builds a database from one or more sets.
pub fn build_db(sets: impl IntoIterator<Item = CardSet>) -> Result<CardDb, LoadError> {
    let (mut cards, mut superstars) = (Vec::new(), Vec::new());
    for set in sets {
        cards.extend(set.cards);
        superstars.extend(set.superstars);
    }
    CardDb::new(cards, superstars).map_err(LoadError::Db)
}

const TEST_SET: &str = include_str!("../data/test-set.ron");

/// The invented cards used by tests and the text client.
pub fn test_set() -> CardSet {
    parse_set(TEST_SET).expect("bundled test set parses")
}

pub fn test_db() -> CardDb {
    build_db([test_set()]).expect("bundled test set is consistent")
}

/// A legal 60-card Arsenal from the test set: three of every card, filled up
/// with the unlimited Set-Up card.
pub fn test_deck(superstar: &str) -> DeckList {
    let set = test_set();
    let mut arsenal = Vec::new();
    for card in set.cards.iter().filter(|c| c.id != "test/lock-up") {
        for _ in 0..3 {
            arsenal.push(card.id.clone());
        }
    }
    while arsenal.len() < 60 {
        arsenal.push("test/lock-up".to_string());
    }
    DeckList { superstar: superstar.to_string(), arsenal }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_loads_and_decks_are_legal() {
        let db = test_db();
        for s in db.superstars() {
            assert_eq!(rawdeal_core::validate_deck(&db, &test_deck(&s.id)), vec![]);
        }
    }
}
