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
    /// Printed traits the engine does not model yet (e.g. "Tag"), kept for display.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_traits: Vec<String>,
    /// Identifier of the card in the data source it was imported from (e.g. an
    /// OCTGN card GUID), used to match card art.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
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
    /// Extra effects when you successfully play the card with this title.
    OnPlay { title: String, effects: Vec<Effect> },
    /// Your cards with this title need `delta` more (or less) Fortitude.
    CardFortitude { title: String, delta: i16 },
    /// Your opponent cannot play cards with this title.
    OpponentCannotPlay(String),
    /// You cannot put cards with this title in your decks.
    CannotPack(String),
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
    /// Must hold for you to play the card in your Main Segment.
    #[serde(default)]
    pub requires: Option<Condition>,
    /// Replaces the rest of the script when a hybrid card is played as an Action.
    #[serde(default)]
    pub as_action: Option<ActionScript>,
    /// Changes to this card when played as a maneuver.
    #[serde(default)]
    pub bonuses: Vec<Bonus>,
    /// Cannot be reversed.
    #[serde(default)]
    pub irreversible: bool,
    /// Lasting boosts to your cards while this card is in your Ring.
    #[serde(default)]
    pub while_in_ring: Vec<Boost>,
    /// While this card is in your Ring: effects at the start of your turn,
    /// before your Draw Segment.
    #[serde(default)]
    pub in_ring_start_of_turn: Vec<Effect>,
    /// When your opponent makes you discard this card.
    #[serde(default)]
    pub on_forced_discard: Vec<Effect>,
}

/// How a hybrid card works when played as an Action.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionScript {
    /// Fortitude needed instead of the printed F.
    #[serde(default)]
    pub fortitude: Option<u8>,
    #[serde(default)]
    pub requires: Option<Condition>,
    #[serde(default)]
    pub on_play: Vec<Effect>,
}

/// A conditional change printed on a maneuver ("+2D when played after a Strike").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bonus {
    #[serde(default)]
    pub when: Condition,
    /// Multiply the bonus by the number of cards in your Ring matching this.
    #[serde(default)]
    pub per: Option<CardFilter>,
    #[serde(default)]
    pub damage: i16,
    #[serde(default)]
    pub fortitude: i16,
    /// Reversals to this card deal this much more damage.
    #[serde(default)]
    pub reversal_damage: i16,
    /// Reversals to this card need this much more Fortitude.
    #[serde(default)]
    pub reversal_fortitude: i16,
}

/// Something that must be true, judged from the card's controller.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    #[default]
    Always,
    /// Tag Team only cards, which the engine does not support yet.
    Never,
    /// The last card you successfully played this turn matches.
    After(CardFilter),
    /// A card in your Ring matches.
    InYourRing(CardFilter),
    /// Your Fortitude is lower than your opponent's.
    FortitudeBelowOpponent,
    /// You have at least this many cards in hand (counting the card being played).
    HandAtLeast(u8),
    OpponentFemale,
    Not(Box<Condition>),
    AnyOf(Vec<Condition>),
}

/// When a [`Boost`] applies and when it runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    /// Used up by your next card played this turn, whether it matches or not.
    NextCard,
    /// Used up by your next matching card played this turn.
    NextMatching,
    /// Every matching card for the rest of this turn.
    ThisTurn,
    /// Every matching card during your next turn.
    NextTurn,
    /// Your first card played next turn.
    NextTurnFirstCard,
}

/// Temporary change to the cards a player plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Boost {
    #[serde(default = "any_card")]
    pub applies_to: CardFilter,
    #[serde(default = "this_turn")]
    pub duration: Duration,
    #[serde(default)]
    pub damage: i16,
    #[serde(default)]
    pub fortitude: i16,
    #[serde(default)]
    pub reversal_damage: i16,
    #[serde(default)]
    pub reversal_fortitude: i16,
    #[serde(default)]
    pub irreversible: bool,
    /// The card cannot be reversed by cards overturned from the Arsenal.
    #[serde(default)]
    pub no_arsenal_reversal: bool,
}

fn any_card() -> CardFilter {
    CardFilter::Any
}

fn this_turn() -> Duration {
    Duration::ThisTurn
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReverseSpec {
    /// Which played cards this Reversal can stop.
    pub targets: CardFilter,
    /// Damage dealt to the attacker when reversing from hand.
    #[serde(default)]
    pub damage: ReversalDamage,
    /// Effects applied only when reversing from hand ("If played from your hand").
    #[serde(default)]
    pub on_reverse: Vec<Effect>,
    /// Effects applied whether reversing from hand or overturned while taking damage.
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Can only be played from hand, not overturned for damage.
    #[serde(default)]
    pub from_hand_only: bool,
    /// Fortitude needed instead of the printed F.
    #[serde(default)]
    pub fortitude: Option<u8>,
}

impl ReverseSpec {
    pub fn new(targets: CardFilter, damage: ReversalDamage) -> Self {
        ReverseSpec { targets, damage, on_reverse: vec![], effects: vec![], from_hand_only: false, fortitude: None }
    }
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

/// Predicate over a card. Filters about how a card was played (its kind,
/// boosted damage, previous card) judge the card being played; elsewhere, such
/// as when searching an Arsenal, they judge the printed card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CardFilter {
    Any,
    Maneuver,
    ManeuverType(ManeuverType),
    Action,
    Reversal,
    Title(String),
    /// The title contains this word, ignoring case.
    TitleContains(String),
    Trait(Trait),
    DamageAtMost(u8),
    DamageAtLeast(u8),
    /// Printed D at most the Fortitude of the effect's controller.
    DamageAtMostYourFortitude,
    /// Played right after a card matching this.
    After(Box<CardFilter>),
    /// The player who played the card has at least `n` cards matching `filter` in their Ring.
    PlayerRingHas {
        filter: Box<CardFilter>,
        n: u8,
    },
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
    /// The effect's controller looks at the cards and chooses.
    Controller,
}

/// Effect primitives. Mandatory effects do as much as possible and ignore the
/// rest, so amounts are always clamped to what is available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Draw {
        who: Who,
        n: u8,
    },
    /// The player draws between 0 and `n` cards.
    DrawUpTo {
        who: Who,
        n: u8,
    },
    Discard {
        who: Who,
        n: u8,
        pick: Pick,
    },
    /// Discard every card in hand matching `filter`.
    DiscardAll {
        who: Who,
        filter: CardFilter,
    },
    /// Move cards of the player's choice from their Ringside pile to their hand.
    RingsideToHand {
        who: Who,
        n: u8,
        #[serde(default)]
        up_to: bool,
    },
    /// Shuffle cards of the player's choice from Ringside into their Arsenal.
    RingsideToArsenal {
        who: Who,
        n: u8,
        #[serde(default)]
        up_to: bool,
    },
    /// Shuffle cards of the player's choice from their hand into their Arsenal.
    HandToArsenal {
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
    /// Put the top cards of the Arsenal into the Ringside pile. Unlike damage,
    /// an empty Arsenal is not a pinfall.
    Mill {
        who: Who,
        n: u8,
    },
    /// The controller looks through the player's Arsenal, puts `n` cards of
    /// their choice in that player's Ringside pile, and shuffles it.
    MillChosen {
        who: Who,
        n: u8,
    },
    /// The controller searches their Arsenal (and Ringside pile, if
    /// `ringside`) for a card matching `filter`, reveals it, puts it in hand
    /// and shuffles the Arsenal.
    Search {
        filter: CardFilter,
        #[serde(default)]
        ringside: bool,
    },
    /// The controller looks at the player's hand.
    LookAtHand {
        who: Who,
    },
    /// The controller looks at the top `n` cards of the player's Arsenal and
    /// puts them back in any order.
    Arrange {
        who: Who,
        n: u8,
    },
    Shuffle {
        who: Who,
    },
    /// The controller moves `n` cards matching `filter` from the player's Ring
    /// to their Ringside pile.
    RemoveFromRing {
        who: Who,
        n: u8,
        filter: CardFilter,
    },
    /// The player with the higher Fortitude removes maneuvers and Reversals
    /// from their Ring until it is not higher than the other's.
    LevelFortitude,
    /// The player overturns cards until one is a Reversal that could reverse
    /// the last card the controller played; an empty Arsenal is a pinfall.
    OverturnUntilReversal {
        who: Who,
    },
    Boost(Boost),
    /// The controller's turn ends after the current card.
    EndTurn,
    SkipNextTurn {
        who: Who,
    },
    /// Effects resolved at the end of this turn.
    AtEndOfTurn(Vec<Effect>),
    /// The card being played goes to the Ringside pile instead of the Ring.
    DiscardThis,
    /// The card being played is shuffled into its owner's Arsenal instead of
    /// going to the Ring.
    ShuffleThisIntoArsenal,
    /// The controller's first card next turn counts as played right after
    /// this card.
    NextTurnAfterThis,
    /// The controller wins the match by disqualification.
    WinGame,
    /// "You may …": the controller decides whether to apply the inner effects.
    Optional(Vec<Effect>),
    /// "Choose one": the controller picks one of the lists.
    Choice(Vec<Vec<Effect>>),
}
