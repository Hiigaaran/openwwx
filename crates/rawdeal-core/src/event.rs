//! Observable changes, emitted by [`crate::Engine::apply`] for UIs, logs and
//! the network.

use serde::{Deserialize, Serialize};

use crate::card::CardKind;
use crate::db::DefIdx;
use crate::state::{CardId, Outcome, PlayerId, Zone};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Advanced match: the Pre-match phase begins with `first`.
    PreMatchStarted {
        first: PlayerId,
    },
    PreMatchEnded,
    /// `player`'s Superstar now has the Enforcer `index` of the Enforcer card `card`.
    EnforcerChosen {
        player: PlayerId,
        card: DefIdx,
        index: u8,
    },
    /// The match proper begins.
    GameStarted {
        first: PlayerId,
    },
    TurnStarted {
        player: PlayerId,
        turn: u32,
    },
    /// `card` is `None` in the view of anyone but the drawing player.
    CardDrawn {
        player: PlayerId,
        card: Option<CardId>,
    },
    CardPlayed {
        player: PlayerId,
        card: CardId,
        kind: CardKind,
    },
    /// `player` reversed `reversed` with `reversal`, from hand or by
    /// overturning it while taking damage.
    Reversed {
        player: PlayerId,
        reversal: CardId,
        reversed: CardId,
        from_hand: bool,
    },
    /// `player` reversed the Volley card `card` by removing `removed` random
    /// cards of their Ringside pile from the game.
    VolleyReversed {
        player: PlayerId,
        card: CardId,
        removed: u8,
    },
    /// A Reversal hit a Multi card that still needs another one.
    PartialReversal {
        player: PlayerId,
        reversal: CardId,
        card: CardId,
    },
    /// `card` is shown to both players (e.g. found by a search).
    Revealed {
        player: PlayerId,
        card: CardId,
    },
    EffectsResolved {
        card: CardId,
    },
    Overturned {
        player: PlayerId,
        card: CardId,
    },
    /// A card changed zones (other than draws and overturns, which have their
    /// own events). `card` is `None` when the move is hidden from the viewer.
    CardMoved {
        player: PlayerId,
        card: Option<CardId>,
        from: Zone,
        to: Zone,
    },
    ArsenalShuffled {
        player: PlayerId,
    },
    /// `viewer` looked at `player`'s hand.
    HandShown {
        player: PlayerId,
        viewer: PlayerId,
        cards: Vec<CardId>,
    },
    TurnSkipped {
        player: PlayerId,
    },
    CountOutRefused {
        player: PlayerId,
    },
    TurnEnded {
        player: PlayerId,
    },
    /// The player's turn goes on although their card was reversed.
    TurnContinues {
        player: PlayerId,
    },
    GameOver(Outcome),
}

impl Event {
    /// The event as `viewer` is allowed to see it.
    pub fn redacted_for(&self, viewer: PlayerId) -> Event {
        match *self {
            Event::CardDrawn { player, card } if player != viewer && card.is_some() => {
                Event::CardDrawn { player, card: None }
            }
            Event::CardMoved { player, card: Some(_), from, to }
                if player != viewer && !is_public(from) && !is_public(to) =>
            {
                Event::CardMoved { player, card: None, from, to }
            }
            Event::HandShown { player, viewer: v, .. } if viewer != player && viewer != v => {
                Event::HandShown { player, viewer: v, cards: vec![] }
            }
            _ => self.clone(),
        }
    }
}

fn is_public(zone: Zone) -> bool {
    matches!(zone, Zone::Ring | Zone::Ringside | Zone::InPlay)
}
