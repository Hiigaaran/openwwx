//! The set of card definitions a game is played with.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::card::{CardDef, SuperstarDef};

/// Index of a [`CardDef`] inside a [`CardDb`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DefIdx(pub u16);

/// Index of a [`SuperstarDef`] inside a [`CardDb`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SuperstarIdx(pub u16);

#[derive(Debug, Clone, PartialEq)]
pub enum DbError {
    DuplicateId(String),
    TooManyCards,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::DuplicateId(id) => write!(f, "duplicate card id {id}"),
            DbError::TooManyCards => write!(f, "too many card definitions"),
        }
    }
}

impl std::error::Error for DbError {}

#[derive(Debug, Clone, Default)]
pub struct CardDb {
    cards: Vec<CardDef>,
    superstars: Vec<SuperstarDef>,
    card_index: HashMap<String, DefIdx>,
    superstar_index: HashMap<String, SuperstarIdx>,
}

impl CardDb {
    pub fn new(cards: Vec<CardDef>, superstars: Vec<SuperstarDef>) -> Result<Self, DbError> {
        if cards.len() > u16::MAX as usize || superstars.len() > u16::MAX as usize {
            return Err(DbError::TooManyCards);
        }
        let mut card_index = HashMap::new();
        for (i, c) in cards.iter().enumerate() {
            if card_index.insert(c.id.clone(), DefIdx(i as u16)).is_some() {
                return Err(DbError::DuplicateId(c.id.clone()));
            }
        }
        let mut superstar_index = HashMap::new();
        for (i, s) in superstars.iter().enumerate() {
            if superstar_index.insert(s.id.clone(), SuperstarIdx(i as u16)).is_some() {
                return Err(DbError::DuplicateId(s.id.clone()));
            }
        }
        Ok(CardDb { cards, superstars, card_index, superstar_index })
    }

    pub fn card(&self, idx: DefIdx) -> &CardDef {
        &self.cards[idx.0 as usize]
    }

    pub fn superstar(&self, idx: SuperstarIdx) -> &SuperstarDef {
        &self.superstars[idx.0 as usize]
    }

    pub fn card_by_id(&self, id: &str) -> Option<DefIdx> {
        self.card_index.get(id).copied()
    }

    pub fn superstar_by_id(&self, id: &str) -> Option<SuperstarIdx> {
        self.superstar_index.get(id).copied()
    }

    pub fn cards(&self) -> &[CardDef] {
        &self.cards
    }

    pub fn superstars(&self) -> &[SuperstarDef] {
        &self.superstars
    }
}
