//! Complete game state. Only the engine mutates it; players see a
//! [`crate::view::PlayerView`].

use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::card::{Boost, CardFilter, CardKind, Effect};
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
    Backlash,
    /// Removed from the game: the cards never come back.
    Removed,
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
    /// Backlash deck (Advanced matches). Hidden from the opponent.
    pub backlash: Vec<CardId>,
    /// Cards removed from the game, in the order they were removed.
    #[serde(default)]
    pub removed: Vec<CardId>,
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
            Zone::Backlash => &self.backlash,
            Zone::Removed => &self.removed,
            Zone::InPlay => &[],
        }
    }

    pub(crate) fn zone_mut(&mut self, zone: Zone) -> Option<&mut Vec<CardId>> {
        match zone {
            Zone::Arsenal => Some(&mut self.arsenal),
            Zone::Hand => Some(&mut self.hand),
            Zone::Ring => Some(&mut self.ring),
            Zone::Ringside => Some(&mut self.ringside),
            Zone::Backlash => Some(&mut self.backlash),
            Zone::Removed => Some(&mut self.removed),
            Zone::InPlay => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinMethod {
    Pinfall,
    CountOut,
    Disqualification,
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
    /// Reversals that already hit it (a Multi card needs two).
    pub reversals: u8,
    /// The card successfully played right before it this turn, if any.
    pub after: Option<CardId>,
    /// Boosts and bonuses applied to it when it was played.
    pub bonus: PlayBonus,
    /// It was played from its owner's Ringside pile.
    #[serde(default)]
    pub from_ringside: bool,
}

impl Played {
    pub fn new(card: CardId, player: PlayerId, kind: CardKind) -> Self {
        Played { card, player, kind, reversals: 0, after: None, bonus: PlayBonus::default(), from_ringside: false }
    }
}

/// Changes to a card being played, from its own bonuses and from boosts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayBonus {
    pub damage: i16,
    pub fortitude: i16,
    pub reversal_damage: i16,
    pub reversal_fortitude: i16,
    pub irreversible: bool,
    pub no_arsenal_reversal: bool,
    #[serde(default)]
    pub stun_value: i16,
    #[serde(default)]
    pub ignore_play_after: bool,
    #[serde(default)]
    pub opponent_fortitude: bool,
    #[serde(default)]
    pub double_fortitude: bool,
    #[serde(default)]
    pub no_hand_reversal: bool,
    #[serde(default)]
    pub double_damage: bool,
    #[serde(default)]
    pub no_backlash_reversal: bool,
}

/// What a player's next card this turn has to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NextMust {
    Filter(CardFilter),
    /// One of these cards.
    Cards(Vec<CardId>),
}

/// Effects waiting on how a player's next matching card turns out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watch {
    pub player: PlayerId,
    pub filter: CardFilter,
    pub on_success: Vec<Effect>,
    pub on_reversed: Vec<Effect>,
}

/// Which rulebook the match uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    QuickStart,
    /// Pre-match phase, Backlash decks and the Advanced count out.
    Advanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub cards: Vec<CardInstance>,
    pub players: [PlayerState; 2],
    pub active: PlayerId,
    pub turn: u32,
    pub mode: Mode,
    pub outcome: Option<Outcome>,
    /// Card on the "stack", between being played and landing in the Ring.
    pub in_play: Option<Played>,
    /// Last card successfully played this turn, for follow-ups.
    pub last_played: Option<Played>,
    /// Players who refused a Count Out win and want to go for the pin.
    pub refused_count_out: [bool; 2],
    pub decision: Option<Decision>,
    /// Active boosts and the player whose cards they change.
    pub boosts: Vec<(PlayerId, Boost)>,
    /// Effects waiting for the end of the turn, with their controller.
    pub end_of_turn: Vec<(PlayerId, Effect)>,
    /// Players who skip their next turn.
    pub skip_turn: [bool; 2],
    /// Card each player's first card next turn counts as played after.
    pub start_after: [Option<CardId>; 2],
    /// The active player's turn ends once the card in play is done.
    pub end_turn_after_play: bool,
    /// Where the card in play goes instead of the Ring.
    pub in_play_to: Option<Zone>,
    /// Cards the active player played this turn, in order.
    #[serde(default)]
    pub played_this_turn: Vec<Played>,
    /// The card each player reversed to end their opponent's last turn.
    #[serde(default)]
    pub reversed_last: [Option<Played>; 2],
    /// Players who cannot play matching cards, for this many more of their turns.
    #[serde(default)]
    pub restrictions: Vec<(PlayerId, CardFilter, u8)>,
    /// Ring cards whose ability was used this turn.
    #[serde(default)]
    pub ring_used: Vec<CardId>,
    /// Ring cards whose effects have stopped.
    #[serde(default)]
    pub lapsed: Vec<CardId>,
    /// Cards found by the search being resolved.
    #[serde(default)]
    pub last_chosen: Vec<CardId>,
    /// Cards the last chosen discard put in the Ringside pile.
    #[serde(default)]
    pub last_discarded: Vec<CardId>,
    /// Players who used a once-per-game Superstar ability.
    #[serde(default)]
    pub ability_spent: [bool; 2],
    /// The only cards a player may play next this turn.
    #[serde(default)]
    pub next_must: Option<(PlayerId, NextMust)>,
    /// Players whose next Pre-match card is blank.
    #[serde(default)]
    pub blank_prematch: [bool; 2],
    /// Waiting for a player's next matching card this turn.
    #[serde(default)]
    pub watches: Vec<Watch>,
    /// The watch the card in play triggered.
    #[serde(default)]
    pub in_play_watch: Option<Watch>,
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
            for zone in [Zone::Arsenal, Zone::Hand, Zone::Ring, Zone::Ringside, Zone::Backlash, Zone::Removed] {
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
