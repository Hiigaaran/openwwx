//! Complete game state. Only the engine mutates it; players see a
//! [`crate::view::PlayerView`].

use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::card::CardKind;
use crate::db::{DefIdx, SuperstarIdx};
use crate::decision::Decision;
use crate::engine::Step;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerId(pub u8);

impl PlayerId {
    pub const ONE: PlayerId = PlayerId(0);
    pub const TWO: PlayerId = PlayerId(1);

    pub fn opponent(self) -> PlayerId {
        PlayerId(1 - self.0)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P{}", self.0 + 1)
    }
}

/// Physical card in a match, index into [`GameState::cards`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CardId(pub u16);

/// Where an effect comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    Card(CardId),
    Superstar(PlayerId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    Arsenal,
    Hand,
    Ring,
    Ringside,
    /// Being played: waiting for reversals and resolution.
    InPlay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardInstance {
    pub def: DefIdx,
    pub owner: PlayerId,
    pub zone: Zone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub superstar: SuperstarIdx,
    /// `last()` is the top of the Arsenal.
    pub arsenal: Vec<CardId>,
    pub hand: Vec<CardId>,
    /// In the order the cards were played.
    pub ring: Vec<CardId>,
    /// `last()` is the most recent card.
    pub ringside: Vec<CardId>,
    /// The Superstar's activated ability was used this turn.
    pub ability_used: bool,
}

impl PlayerState {
    pub fn zone(&self, zone: Zone) -> &[CardId] {
        match zone {
            Zone::Arsenal => &self.arsenal,
            Zone::Hand => &self.hand,
            Zone::Ring => &self.ring,
            Zone::Ringside => &self.ringside,
            Zone::InPlay => &[],
        }
    }

    pub(crate) fn zone_mut(&mut self, zone: Zone) -> Option<&mut Vec<CardId>> {
        match zone {
            Zone::Arsenal => Some(&mut self.arsenal),
            Zone::Hand => Some(&mut self.hand),
            Zone::Ring => Some(&mut self.ring),
            Zone::Ringside => Some(&mut self.ringside),
            Zone::InPlay => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinMethod {
    Pinfall,
    CountOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Win { winner: PlayerId, method: WinMethod },
    Draw,
}

/// The card currently being played and what it was played as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Played {
    pub card: CardId,
    pub player: PlayerId,
    pub kind: CardKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub cards: Vec<CardInstance>,
    pub players: [PlayerState; 2],
    pub active: PlayerId,
    pub turn: u32,
    pub outcome: Option<Outcome>,
    /// Card on the "stack", between being played and landing in the Ring.
    pub in_play: Option<Played>,
    /// Last card successfully played this turn, for follow-ups.
    pub last_played: Option<Played>,
    /// Players who refused a Count Out win and want to go for the pin.
    pub refused_count_out: [bool; 2],
    pub decision: Option<Decision>,
    pub(crate) pending: Vec<Step>,
    pub(crate) rng: ChaCha8Rng,
}

impl GameState {
    pub fn player(&self, p: PlayerId) -> &PlayerState {
        &self.players[p.index()]
    }

    pub(crate) fn player_mut(&mut self, p: PlayerId) -> &mut PlayerState {
        &mut self.players[p.index()]
    }

    pub fn card(&self, c: CardId) -> &CardInstance {
        &self.cards[c.0 as usize]
    }

    pub fn is_over(&self) -> bool {
        self.outcome.is_some()
    }

    /// Checks that every card is in exactly one zone and that its recorded zone
    /// matches. Intended for tests and debug builds.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut seen = vec![0u8; self.cards.len()];
        for (pi, p) in self.players.iter().enumerate() {
            for zone in [Zone::Arsenal, Zone::Hand, Zone::Ring, Zone::Ringside] {
                for &c in p.zone(zone) {
                    let inst = self.card(c);
                    if inst.zone != zone {
                        return Err(format!("{c:?} listed in {zone:?} but marked {:?}", inst.zone));
                    }
                    if inst.owner.index() != pi {
                        return Err(format!("{c:?} in wrong player's {zone:?}"));
                    }
                    seen[c.0 as usize] += 1;
                }
            }
        }
        if let Some(pl) = self.in_play {
            if self.card(pl.card).zone != Zone::InPlay {
                return Err(format!("{:?} in play but marked {:?}", pl.card, self.card(pl.card).zone));
            }
            seen[pl.card.0 as usize] += 1;
        }
        match seen.iter().position(|&n| n != 1) {
            Some(i) => Err(format!("CardId({i}) appears {} times", seen[i])),
            None => Ok(()),
        }
    }
}
