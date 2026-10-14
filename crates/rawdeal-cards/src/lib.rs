//! Card definitions for OpenWWX and loaders for card data files.
//!
//! Real card data is not distributed with OpenWWX. Players keep it in a
//! folder of their own with one RON file per set (see [`load_dir`]).

use std::path::{Path, PathBuf};

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
    Io(PathBuf, std::io::Error),
    Parse(Box<ron::error::SpannedError>),
    /// A parse error in a specific file.
    File(PathBuf, Box<ron::error::SpannedError>),
    Db(rawdeal_core::db::DbError),
    NoSets(PathBuf),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            LoadError::Parse(e) => write!(f, "card data: {e}"),
            LoadError::File(path, e) => write!(f, "{}: {e}", path.display()),
            LoadError::Db(e) => write!(f, "card data: {e}"),
            LoadError::NoSets(path) => write!(f, "{}: no .ron card sets found", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

pub fn parse_set(ron_text: &str) -> Result<CardSet, LoadError> {
    ron::from_str(ron_text).map_err(|e| LoadError::Parse(Box::new(e)))
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

/// Loads every `*.ron` card set in `dir`, in file name order.
pub fn load_dir(dir: &Path) -> Result<Vec<CardSet>, LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|e| LoadError::Io(dir.to_path_buf(), e))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(LoadError::NoSets(dir.to_path_buf()));
    }
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).map_err(|e| LoadError::Io(path.clone(), e))?;
            ron::from_str(&text).map_err(|e| LoadError::File(path, Box::new(e)))
        })
        .collect()
}

/// Reads a deck list (Superstar, Arsenal and optional Backlash) from a RON file.
pub fn load_deck(path: &Path) -> Result<DeckList, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Io(path.to_path_buf(), e))?;
    ron::from_str(&text).map_err(|e| LoadError::File(path.to_path_buf(), Box::new(e)))
}

const TEST_SET: &str = include_str!("../data/test-set.ron");

/// The invented cards used by tests and the text client.
pub fn test_set() -> CardSet {
    parse_set(TEST_SET).expect("bundled test set parses")
}

pub fn test_db() -> CardDb {
    build_db([test_set()]).expect("bundled test set is consistent")
}

/// A legal deck from the test set: two of every Arsenal card filled up to 60
/// with the unlimited Set-Up card, and one of every Backlash card.
pub fn test_deck(superstar: &str) -> DeckList {
    let set = test_set();
    let mut arsenal = Vec::new();
    let mut backlash = Vec::new();
    for card in set.cards.iter().filter(|c| c.id != "test/lock-up") {
        if card.is_backlash() {
            backlash.push(card.id.clone());
        } else {
            arsenal.extend([card.id.clone(), card.id.clone()]);
        }
    }
    assert!(arsenal.len() <= 60, "test set has too many Arsenal cards for two copies each");
    arsenal.resize(60, "test/lock-up".to_string());
    DeckList { superstar: superstar.to_string(), arsenal, backlash }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_folder_of_sets() {
        let dir = std::env::temp_dir().join(format!("openwwx-sets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a-test.ron"), TEST_SET).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let sets = load_dir(&dir).unwrap();
        assert_eq!(sets.len(), 1);
        assert!(build_db(sets).is_ok());

        std::fs::write(dir.join("b-broken.ron"), "(name: ").unwrap();
        assert!(matches!(load_dir(&dir), Err(LoadError::File(p, _)) if p.ends_with("b-broken.ron")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_set_loads_and_decks_are_legal() {
        let db = test_db();
        for s in db.superstars() {
            assert_eq!(rawdeal_core::validate_deck(&db, &test_deck(&s.id)), vec![]);
        }
    }
}
