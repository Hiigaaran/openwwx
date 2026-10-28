//! The set of card definitions a game is played with.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::card::{CardDef, CardScript, SuperstarDef};

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

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Debug, Clone, Default)]
pub struct CardDb {
    cards: Vec<CardDef>,
    superstars: Vec<SuperstarDef>,
    card_index: HashMap<String, DefIdx>,
    superstar_index: HashMap<String, SuperstarIdx>,
}

impl CardDb {
    pub fn new(mut cards: Vec<CardDef>, superstars: Vec<SuperstarDef>) -> Result<Self, DbError> {
        if cards.len() >= u16::MAX as usize || superstars.len() > u16::MAX as usize {
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
        cards.push(face_down_card());
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

    /// The printed cards (without the face-down card).
    pub fn cards(&self) -> &[CardDef] {
        &self.cards[..self.cards.len().saturating_sub(1)]
    }

    /// The blank card a face-down Ring card is: no type, traits or text, F: 0 D: 0.
    pub fn face_down(&self) -> DefIdx {
        DefIdx((self.cards.len() - 1) as u16)
    }

    pub fn superstars(&self) -> &[SuperstarDef] {
        &self.superstars
    }

    /// A fingerprint of every set loaded, by id prefix (`"premiere"` for
    /// `"premiere/042"`), in load order. Two players with equal lists have the
    /// same cards at the same indexes. Stable only within one app version.
    pub fn fingerprints(&self) -> Vec<(String, u64)> {
        let mut sets: Vec<(String, u64)> = Vec::new();
        let items = self
            .cards
            .iter()
            .map(|c| (c.id.as_str(), format!("{c:?}")))
            .chain(self.superstars.iter().map(|s| (s.id.as_str(), format!("{s:?}"))));
        for (id, text) in items {
            let set = id.split('/').next().unwrap_or(id);
            let i = match sets.iter().position(|(s, _)| s == set) {
                Some(i) => i,
                None => {
                    sets.push((set.to_string(), FNV_OFFSET));
                    sets.len() - 1
                }
            };
            let hash = &mut sets[i].1;
            for b in text.bytes() {
                *hash = (*hash ^ b as u64).wrapping_mul(FNV_PRIME);
            }
        }
        sets
    }
}

pub const FACE_DOWN_ID: &str = "openwwx/face-down";

fn face_down_card() -> CardDef {
    CardDef {
        id: FACE_DOWN_ID.into(),
        title: "Face-down card".into(),
        kinds: vec![],
        fortitude: 0,
        damage: 0,
        stun_value: 0,
        traits: vec![],
        superstar: None,
        also_superstars: vec![],
        text: String::new(),
        other_traits: vec![],
        source_id: None,
        script: CardScript::default(),
        enforcers: vec![],
        targets: vec![],
    }
}
