//! Static card definitions: what is printed on a card, plus the script the
//! engine interprets to apply its text.

use serde::{Deserialize, Serialize};

/// Maneuver subtypes printed on yellow cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ManeuverType {
    Strike,
    Grapple,
    Submission,
    HighRisk,
    TrademarkFinisher,
}

/// One function a card can be played as. A hybrid card lists several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CardKind {
    Maneuver(ManeuverType),
    Action,
    Reversal,
    /// Backlash card played only during the Pre-match phase.
    PreMatch(PreMatchType),
}

/// Pre-match cards. The first four are played in their own step, one each per
/// player; Events and Objects are played afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreMatchType {
    Venue,
    Feud,
    Stipulation,
    Manager,
    Event,
    Object,
}

impl CardKind {
    /// Kinds that can be played from hand during your own Main Segment.
    pub fn playable_on_own_turn(self) -> bool {
        !matches!(self, CardKind::Reversal | CardKind::PreMatch(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Trait {
    Face,
    Heel,
    Raw,
    SmackDown,
    FanFavorite,
    Cheater,
    /// No copy limit when building an Arsenal.
    SetUp,
    /// At most one copy in an Arsenal.
    Unique,
    /// Cannot be reversed.
    Irreversible,
    /// You cannot play it while a card with the same title is in your Ring.
    Active,
    /// Only one card with this title may be in all Ring areas at once.
    UniversallyUnique,
    /// Backlash card played during the match like a normal card.
    MidMatch,
    /// Must be reversed by two Reversals to be stopped. Female Superstars
    /// cannot play Multi maneuvers.
    Multi,
    /// Can only be reversed from hand or Backlash if the opponent discards 2
    /// cards; after it succeeds you may discard 2 to search for another Chain.
    Chain,
    /// Pay-Per-View Event: only one in your Ring.
    PayPerView,
}

/// A printed card that can go in an Arsenal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    /// Stable identifier such as `"premiere/042"`.
    pub id: String,
    pub title: String,
    pub kinds: Vec<CardKind>,
    pub fortitude: u8,
    pub damage: u8,
    #[serde(default)]
    pub stun_value: u8,
    #[serde(default)]
    pub traits: Vec<Trait>,
    /// Superstar this card is specific to, if any (e.g. a Trademark Finisher).
    #[serde(default)]
    pub superstar: Option<String>,
    /// Original card text, for display only.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub script: CardScript,
}

impl CardDef {
    pub fn has_trait(&self, t: Trait) -> bool {
        self.traits.contains(&t)
    }

    pub fn is_reversal(&self) -> bool {
        self.kinds.contains(&CardKind::Reversal)
    }

    /// Belongs in the Backlash deck rather than the Arsenal.
    pub fn is_backlash(&self) -> bool {
        self.is_pre_match() || self.has_trait(Trait::MidMatch)
    }

    pub fn is_pre_match(&self) -> bool {
        self.kinds.iter().any(|k| matches!(k, CardKind::PreMatch(_)))
    }
}

/// The Superstar card that sits in a player's Backstage area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuperstarDef {
    pub id: String,
    pub name: String,
    pub hand_size: u8,
    pub superstar_value: u8,
    #[serde(default)]
    pub female: bool,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub abilities: Vec<Ability>,
}

/// Superstar abilities. Optional abilities wrap their effects in
/// [`Effect::Optional`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ability {
    /// Resolved at the start of your turn, before the Draw Segment.
    StartOfTurn(Vec<Effect>),
    /// Once during your turn, in your Main Segment: pay `cost`, then apply
    /// `effects`. Using it does not break a follow-up chain.
    Activated { cost: Vec<Effect>, effects: Vec<Effect> },
    /// Draw this many cards in your Draw Segment instead of 1.
    DrawSegment(u8),
    /// Damage from cards your opponent plays is reduced by this much.
    ReduceDamage(u8),
}

/// Interpretable description of a card's text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CardScript {
    /// Effects resolved when the card is played successfully, before damage.
    #[serde(default)]
    pub on_play: Vec<Effect>,
    /// What this card can reverse, for cards that can be played as a Reversal.
    #[serde(default)]
    pub reverses: Option<ReverseSpec>,
    /// Follow-up: may only be played right after you successfully played a
    /// card matching this filter, with no other card played in between.
    #[serde(default)]
    pub play_after: Option<CardFilter>,
    /// Pre-match cards: effects resolved at the end of the Pre-match phase.
    #[serde(default)]
    pub end_of_prematch: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReverseSpec {
    /// Which played cards this Reversal can stop.
    pub targets: CardFilter,
    /// Damage dealt to the attacker when reversing from hand.
    #[serde(default)]
    pub damage: ReversalDamage,
    /// Effects applied when reversing from hand (not when overturned for damage).
    #[serde(default)]
    pub on_reverse: Vec<Effect>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReversalDamage {
    /// The Reversal's own printed D.
    #[default]
    Printed,
    None,
    Fixed(u8),
    /// As much damage as the reversed card would have dealt.
    SameAsReversed,
}

/// Predicate over a card being played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CardFilter {
    Any,
    Maneuver,
    ManeuverType(ManeuverType),
    Action,
    Title(String),
    DamageAtMost(u8),
    DamageAtLeast(u8),
    All(Vec<CardFilter>),
    AnyOf(Vec<CardFilter>),
    Not(Box<CardFilter>),
}

/// Who an effect applies to, relative to the card's controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Who {
    You,
    Opponent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pick {
    /// The affected player chooses.
    Chosen,
    /// Picked at random by the engine.
    Random,
}

/// Effect primitives. Mandatory effects do as much as possible and ignore the
/// rest, so amounts are always clamped to what is available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Draw {
        who: Who,
        n: u8,
    },
    Discard {
        who: Who,
        n: u8,
        pick: Pick,
    },
    /// Move cards of the player's choice from their Ringside pile to their hand.
    RingsideToHand {
        who: Who,
        n: u8,
    },
    /// Shuffle cards of the player's choice from Ringside into their Arsenal.
    RingsideToArsenal {
        who: Who,
        n: u8,
    },
    /// Put cards of the player's choice from their hand on the bottom of their Arsenal.
    HandToArsenalBottom {
        who: Who,
        n: u8,
    },
    /// Put cards of the player's choice from their Ringside pile on the bottom of their Arsenal.
    RingsideToArsenalBottom {
        who: Who,
        n: u8,
    },
    /// Extra damage outside the card's own D (does not add to Fortitude).
    Damage {
        who: Who,
        n: u8,
    },
    /// "You may …": the controller decides whether to apply the inner effects.
    Optional(Vec<Effect>),
}
