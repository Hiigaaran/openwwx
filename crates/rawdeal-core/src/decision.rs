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
    /// The opponent played `card`; you may reverse it from hand. With
    /// `volley` you may also reverse its Volley by removing cards from your
    /// Ringside pile.
    Reversal {
        card: CardId,
        options: Vec<CardId>,
        #[serde(default)]
        volley: bool,
    },
    /// Your opponent reversed your card with `reversal`; you may answer it
    /// with one of `options` (a Reversal that reverses Reversals, or an
    /// Action that cancels them).
    CounterReversal { reversal: CardId, options: Vec<CardId> },
    /// Pick between `min` and `max` cards from `options`.
    ChooseCards { options: Vec<CardId>, min: u8, max: u8, purpose: ChoicePurpose },
    /// Answer yes or no.
    YesNo { purpose: YesNoPurpose },
    /// Draw up to `max` cards (Stun Value after your card was reversed during
    /// damage, or a "draw up to" effect).
    StunDraw { max: u8 },
    /// Choose one of a card's options.
    ChooseOption { options: Vec<Vec<Effect>>, source: Source },
    /// Pre-match phase: play a Pre-match card from your Backlash deck or pass.
    /// Steps 0-3 are Venue, Feud, Stipulation and Manager; step 4 is open.
    PreMatch { step: u8, passes: u8, starter: PlayerId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChoicePurpose {
    Discard,
    /// Discard caused by the opponent (it can trigger the card).
    ForcedDiscard,
    HandToArsenal,
    /// Cards picked from an Arsenal go to its owner's Ringside pile.
    ArsenalToRingside,
    RingToRingside,
    /// Pick the next card, from the top down, to put back on the Arsenal.
    Arrange,
    RingsideToHand,
    RingsideToArsenal,
    /// Put under the card (`host`).
    PutUnder(crate::state::CardId),
    /// The picked cards are removed from the game.
    RemoveFromGame,
    /// The picked cards move to this zone of their owner's.
    MoveTo(crate::state::Zone),
    /// Put on the bottom of the owner's Arsenal, no shuffle.
    ToArsenalBottom,
    /// Put on top of the owner's Arsenal, no shuffle.
    ToArsenalTop,
    HandToArsenalBottom,
    RingsideToArsenalBottom,
    /// Search your Arsenal: the card is revealed and goes to your hand.
    SearchToHand,
    /// Discard any number up to the limit; your opponent discards as many.
    DiscardMirror,
    HandToArsenalTop,
    /// Cards from a Ring go back to their owner's hand.
    RingToHand,
    /// One of the top cards of your Arsenal goes to your hand.
    DigToHand,
    /// Discard any number: the card in play is +`per`D for each.
    DiscardForDamage {
        per: u8,
    },
    /// Mid-match cards from your Ringside pile go back to your Backlash deck.
    RingsideToBacklash,
    /// Shuffle these into your Arsenal and draw as many.
    Mulligan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum YesNoPurpose {
    /// "You may …" effect of a card.
    OptionalEffect { effects: Vec<Effect>, source: Source },
    /// Your opponent ended a turn with an empty Arsenal: take the Count Out win?
    ClaimCountOut,
    /// Your Chain card succeeded: discard 2 cards to search for another Chain?
    ChainSearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayCard {
        card: CardId,
        kind: CardKind,
    },
    UseAbility,
    /// Use the ability of a card in your Ring.
    UseRingCard {
        card: CardId,
    },
    EndTurn,
    Reverse {
        card: CardId,
    },
    DeclineReversal,
    /// Reverse a "Volley: #" card by removing # random cards of your Ringside
    /// pile from the game.
    ReverseByVolley,
    ChooseCards(Vec<CardId>),
    Yes,
    No,
    DrawCards(u8),
    ChooseOption(u8),
    /// Pre-match phase: play nothing this step.
    Pass,
}
