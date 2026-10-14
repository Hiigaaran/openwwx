//! What the engine asks of players, and what they answer.

use serde::{Deserialize, Serialize};

use crate::card::{CardKind, Effect};
use crate::state::{CardId, PlayerId, Source};

/// The engine is paused until `player` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub player: PlayerId,
    pub kind: DecisionKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionKind {
    /// Main Segment: play a card or end the turn.
    Main,
    /// The opponent played `card`; you may reverse it from hand.
    Reversal { card: CardId, options: Vec<CardId> },
    /// Pick between `min` and `max` cards from `options`.
    ChooseCards { options: Vec<CardId>, min: u8, max: u8, purpose: ChoicePurpose },
    /// Answer yes or no.
    YesNo { purpose: YesNoPurpose },
    /// Your card was reversed during damage: draw up to `max` cards.
    StunDraw { max: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChoicePurpose {
    Discard,
    RingsideToHand,
    RingsideToArsenal,
    HandToArsenalBottom,
    RingsideToArsenalBottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum YesNoPurpose {
    /// "You may …" effect of a card.
    OptionalEffect { effects: Vec<Effect>, source: Source },
    /// Your opponent ended a turn with an empty Arsenal: take the Count Out win?
    ClaimCountOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayCard { card: CardId, kind: CardKind },
    UseAbility,
    EndTurn,
    Reverse { card: CardId },
    DeclineReversal,
    ChooseCards(Vec<CardId>),
    Yes,
    No,
    DrawCards(u8),
}
