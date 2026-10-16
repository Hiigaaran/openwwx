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
    /// More Superstars whose logo is printed on the card; any of them may pack it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_superstars: Vec<String>,
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

    /// Superstar logos printed on the card.
    pub fn logos(&self) -> impl Iterator<Item = &str> {
        self.superstar.iter().chain(&self.also_superstars).map(String::as_str)
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
    /// Printed Superstar traits ("Diva", "Tag Team", "Legend", ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub abilities: Vec<Ability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

impl SuperstarDef {
    /// Does `group` describe this Superstar? It is their name, "Female",
    /// "Male", or one of their printed traits ("Diva", "Legend").
    pub fn is(&self, group: &str) -> bool {
        self.name.eq_ignore_ascii_case(group)
            || (group.eq_ignore_ascii_case("female") && self.female)
            || (group.eq_ignore_ascii_case("male") && !self.female)
            || self.traits.iter().any(|t| t.eq_ignore_ascii_case(group))
    }
}

/// Superstar abilities. Optional abilities wrap their effects in
/// [`Effect::Optional`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ability {
    /// Resolved at the start of your turn, before the Draw Segment.
    StartOfTurn(Vec<Effect>),
    /// Once during your turn, in your Main Segment: pay `cost`, then apply
    /// `effects`. Using it does not break a follow-up chain.
    Activated {
        cost: Vec<Effect>,
        effects: Vec<Effect>,
        /// Must hold for the ability to be used.
        #[serde(default)]
        requires: Option<Condition>,
        /// Only once per game instead of once per turn.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        once_per_game: bool,
    },
    /// You count as these Superstars for card effects and may pack cards with
    /// their logos.
    CountsAs(Vec<String>),
    /// You may pack these Superstars' logo cards, but you do not count as them.
    PacksLogos(Vec<String>),
    /// Effects at the end of the Pre-match phase.
    EndOfPrematch(Vec<Effect>),
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
    /// Your cards matching `filter` need `delta` more (or less) Fortitude.
    FortitudeFor { filter: CardFilter, delta: i16 },
    /// Your first maneuver each turn is +1D, the second +2D, and so on.
    EscalatingManeuverDamage,
    /// The first card matching this you play each turn cannot be reversed.
    FirstOfTurnIrreversible(CardFilter),
    /// Your cards matching `played` cannot be reversed by cards matching `reversal`.
    CannotBeReversedBy { played: CardFilter, reversal: CardFilter },
    /// You may reverse a maneuver from hand by discarding a maneuver of the
    /// same type; it deals no damage.
    ReverseByDiscardingSameType,
    /// After drawing in your Draw Segment, discard this many cards.
    DrawSegmentDiscard(u8),
    /// You may pack both Face and Heel cards.
    PackBothAlignments,
    /// In your Draw Segment you may apply these effects instead of drawing.
    DrawSegmentOption(Vec<Effect>),
    /// Extra effects when you successfully play a card matching `filter`.
    OnPlayMatching { filter: CardFilter, effects: Vec<Effect> },
    /// Hand Size is `times` your opponent's Superstar Value, at least `min`.
    HandSizeFromOpponentValue { times: u8, min: u8 },
    /// Card-drawing effects on cards your opponent plays do nothing (Stun
    /// Value draws still happen).
    BlankOpponentCardDraws,
    /// You may reverse a card matching this from hand by discarding any 1
    /// card; it deals no damage.
    ReverseByDiscardingAny(CardFilter),
    /// You cannot put cards with this trait in your decks.
    CannotPackTrait(Trait),
    /// You cannot put cards matching this (judged on the printed card) in your decks.
    CannotPackMatching(CardFilter),
    /// Discard this many cards to play a card matching `filter`.
    PlayCost { filter: CardFilter, discard: u8 },
    /// Your opponent's cards matching `filter` deal at most `max` damage.
    OpponentDamageCap { filter: CardFilter, max: u8 },
    /// Your cards matching `filter` deal `delta` more (or less) damage.
    DamageFor { filter: CardFilter, delta: i16 },
    /// You cannot play cards matching this.
    CannotPlayMatching(CardFilter),
    /// Extra effects for you when your opponent successfully plays a card
    /// matching `filter`.
    OnOpponentPlayMatching { filter: CardFilter, effects: Vec<Effect> },
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
    /// Cannot be reversed while this holds for the player playing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irreversible_when: Option<Condition>,
    /// Cannot be reversed from the opponent's hand.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_hand_reversal: bool,
    /// Can only be reversed by Reversals that name this card's title.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub only_reversed_by_title: bool,
    /// Effects for the player of this card when it is reversed from the
    /// opponent's hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_reversed_from_hand: Vec<Effect>,
    /// While this holds, `play_after` does not apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_after_unless: Option<Condition>,
    /// Something you may do with this card while it is in your Ring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_ability: Option<RingAbility>,
    /// While in your Ring: draw this many more cards in your Draw Segment.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub extra_draw: u8,
    /// While in your Ring: your opponent plays with their hand revealed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reveals_opponent_hand: bool,
    /// While in your Ring: boosts to every player's cards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub while_in_ring_all: Vec<Boost>,
    /// While in your Ring: boosts to your opponent's cards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub while_in_ring_opponent: Vec<Boost>,
    /// While in your Ring: your opponent cannot play cards matching this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restricts_opponent: Option<Restriction>,
    /// The card's Ring effects stop once you successfully play a maneuver.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lapses_after_own_maneuver: bool,
    /// While in your Ring: effects at the start of your opponent's turn,
    /// before their Draw Segment, controlled by that opponent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_ring_opponent_start_of_turn: Vec<Effect>,
    /// Effects after the card's damage, if it was not reversed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_damage: Vec<Effect>,
    /// Effects when the card is played, before the opponent may reverse it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before_reversal: Vec<Effect>,
    /// Reversals matching this cannot reverse the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_reversed_by: Option<CardFilter>,
    /// Cannot be reversed by cards overturned from the Arsenal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_arsenal_reversal: bool,
    /// While in your Ring: you may put it in your Ringside pile to reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverses_from_ring: Option<ReverseSpec>,
    /// Traits the importer could not read from the printed traits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_traits: Vec<Trait>,
    /// Only these Superstars may pack the card. Entries can also be
    /// "Female", "Male" or a Superstar trait (see [`SuperstarDef::is`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_packed_by: Vec<String>,
    /// These Superstars cannot pack the card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_packed_by: Vec<String>,
    /// While in your Ring: your opponent's Superstar ability is blank.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blanks_opponent_ability: bool,
    /// `blanks_opponent_ability` only while this holds for you.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blanks_when: Option<Condition>,
    /// While in your Ring: you cannot play cards matching this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restricts_self: Option<Restriction>,
    /// An ability used, like a Ring card's, while the card is in your Ringside
    /// pile (once per turn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ringside_ability: Option<RingAbility>,
    /// Effects when an effect your opponent controls takes this card out of your Ring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_removed_by_opponent: Vec<Effect>,
    /// The card cannot be reversed by cards from the Backlash deck.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_backlash_reversal: bool,
    /// "You may discard (or overturn) up to `max` cards when you play this:
    /// it is -`per`F for each." The engine pays just enough to play it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fortitude_discount: Option<FortitudeDiscount>,
    /// The card can also be played from your Ringside pile while this holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ringside: Option<Condition>,
    /// While in your Ring: your Superstar Value changes by this much.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub value_bonus: i8,
    /// While in your Ring: your opponent's Superstar Value changes by this much.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub opponent_value_bonus: i8,
    /// While in your Ring: you may have this many more (or fewer) Pre-match
    /// cards in your Ring.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub pre_match_limit: i8,
    /// Same, for your opponent's Pre-match cards.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub opponent_pre_match_limit: i8,
    /// While in your Ring: you may have this many more (or fewer) Mid-match
    /// cards in your Ring.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub mid_match_limit: i8,
    /// Same, for your opponent's Mid-match cards.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub opponent_mid_match_limit: i8,
    /// While in your Ring: your opponent's Hand Size changes by this much.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub opponent_hand_size_bonus: i8,
    /// While in your Ring: your Hand Size is this much larger.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hand_size_bonus: u8,
    /// While in your Ring and your opponent's Fortitude is lower than yours:
    /// the text of the maneuvers they play is blank.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blanks_opponent_maneuvers: bool,
    /// Cards with these titles cannot share a deck with this card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_with: Vec<String>,
    /// While in your Ring: effects when a matching card is successfully played.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_ring_on_play: Vec<RingTrigger>,
}

/// Effects of a card in the Ring when someone successfully plays a card.
/// The player who played the card controls the effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTrigger {
    /// Whose cards trigger it, relative to the owner of the Ring card; `None` is anyone's.
    #[serde(default)]
    pub whose: Option<Who>,
    pub filter: CardFilter,
    pub effects: Vec<Effect>,
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// Something a player may do with a card in their Ring, in their Main Segment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RingAbility {
    #[serde(default)]
    pub requires: Option<Condition>,
    #[serde(default)]
    pub cost: Vec<Effect>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// The card goes to its owner's Ringside pile when used.
    #[serde(default)]
    pub sacrifice: bool,
    /// Not on the turn the card was played.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_turn_played: bool,
}

/// See [`CardScript::fortitude_discount`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FortitudeDiscount {
    pub per: u8,
    pub max: u8,
    /// Overturn the top cards of your Arsenal instead of discarding.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overturn: bool,
}

/// What a card in the Ring keeps the opponent from playing, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Restriction {
    /// Judged for the owner of the restricting card.
    #[serde(default)]
    pub when: Condition,
    pub filter: CardFilter,
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
    /// Multiply the bonus by the number of other cards in your hand.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub per_hand: bool,
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
    All(Vec<Condition>),
    /// Your Fortitude is at most your opponent's.
    FortitudeAtMostOpponent,
    /// Your Fortitude is at least your opponent's.
    FortitudeAtLeastOpponent,
    /// Your Superstar has this name.
    Superstar(String),
    /// Your Superstar is this name, "Female", "Male" or has this Superstar
    /// trait ("Diva", "Legend").
    YouAre(String),
    /// Same, for your opponent's Superstar.
    OpponentIs(String),
    /// Your Superstar Value is lower than your opponent's.
    SuperstarValueBelowOpponent,
    /// The card being played came from its owner's Ringside pile.
    PlayedFromRingside,
    /// The count is at least `n`.
    CountAtLeast {
        count: Count,
        n: u8,
    },
    /// The count is at most `n`.
    CountAtMost {
        count: Count,
        n: u8,
    },
    /// Your opponent has at least `n` cards matching `filter` in their Ring.
    OpponentRingHas {
        filter: CardFilter,
        n: u8,
    },
    /// A card in your hand matches.
    HandHas(CardFilter),
    /// Your opponent has more cards in hand than you.
    OpponentHandLarger,
    /// Your opponent has at least this many cards in hand.
    OpponentHandAtLeast(u8),
    /// You have not played a card yet this turn.
    FirstCardThisTurn,
    /// You ended your opponent's last turn by reversing a card matching this.
    ReversedLastTurn(CardFilter),
    /// Your opponent's Superstar has this name.
    OpponentSuperstar(String),
    SuperstarValueAtMost(u8),
    SuperstarValueAtLeast(u8),
    OpponentSuperstarValueAtMost(u8),
    OpponentSuperstarValueAtLeast(u8),
    /// Your Superstar Value is at most your opponent's.
    SuperstarValueAtMostOpponent,
    /// The Stun Value in your Ring is greater than in your opponent's.
    RingStunValueGreater,
    /// Your last `n` cards this turn all match.
    LastPlayedThisTurn {
        filter: CardFilter,
        n: u8,
    },
    /// A card in your opponent's hand matches.
    OpponentHandHas(CardFilter),
    /// The first turn of the match.
    FirstTurnOfGame,
    OpponentArsenalAtMost(u8),
    OpponentArsenalAtLeast(u8),
    /// You have at least this many more cards in hand than your opponent.
    HandLargerBy(u8),
    /// You used the Ring ability of a card matching this this turn.
    UsedRingCardThisTurn(CardFilter),
    FortitudeAboveOpponent,
    /// Your Fortitude is at least this much greater than your opponent's.
    FortitudeLeadAtLeast(u8),
    /// The Stun Value in your Ring is at least this much greater than in your opponent's.
    RingStunLeadAtLeast(u8),
    FortitudeAtLeast(u8),
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
    /// Every matching card until the end of the owner's `n`th turn from now
    /// (counting the current one if it is theirs).
    Turns(u8),
    /// Your next `n` cards played this turn.
    NextCards(u8),
    /// Your first `n` cards played next turn.
    NextTurnCards(u8),
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
    #[serde(default)]
    pub stun_value: i16,
    /// The card may ignore its "can only be played after …" text.
    #[serde(default)]
    pub ignore_play_after: bool,
    /// The card may be played as if you had your opponent's Fortitude.
    #[serde(default)]
    pub use_opponent_fortitude: bool,
    /// The card's printed F is doubled (for the card and for Reversals).
    #[serde(default)]
    pub double_fortitude: bool,
    /// The card does twice its damage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub double_damage: bool,
    /// The player's turn ends after the card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end_turn: bool,
    /// The card cannot be reversed from hand (nor from the Backlash deck).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_hand_reversal: bool,
    /// The card cannot be reversed from the Backlash deck.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_backlash_reversal: bool,
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
    /// More cards it can reverse when played from hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_targets: Option<CardFilter>,
    /// Only works when overturned while taking damage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overturned_only: bool,
    /// Must hold for the reversing player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Condition>,
    /// Cards to discard (chosen) to reverse from hand.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub discard_cost: u8,
    /// The Fortitude needed is the damage of the reversed card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fortitude_is_damage: bool,
    /// Works even on cards that cannot be reversed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignores_irreversible: bool,
    /// The reversed card is shuffled into its owner's Arsenal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed_to_arsenal: bool,
    /// While the condition holds, this Fortitude is needed instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fortitude_when: Option<(Condition, u8)>,
}

impl ReverseSpec {
    pub fn new(targets: CardFilter, damage: ReversalDamage) -> Self {
        ReverseSpec {
            targets,
            damage,
            on_reverse: vec![],
            effects: vec![],
            from_hand_only: false,
            fortitude: None,
            hand_targets: None,
            overturned_only: false,
            requires: None,
            discard_cost: 0,
            fortitude_is_damage: false,
            ignores_irreversible: false,
            reversed_to_arsenal: false,
            fortitude_when: None,
        }
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
    /// Printed F at most this.
    FortitudeAtMost(u8),
    /// Printed F at least this.
    FortitudeAtLeast(u8),
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
    /// A printed trait the engine has no rules for, such as "Run-in" or
    /// "Foreign Object", ignoring case.
    Keyword(String),
    /// The player already played at least `n` cards matching `filter` earlier this turn.
    EarlierThisTurn {
        filter: Box<CardFilter>,
        n: u8,
    },
    /// The card is played by this Superstar (for printed cards: the effect's controller is).
    PlayedBy(String),
    StunValueAtLeast(u8),
    /// The card's effects let its player draw.
    Draws,
    /// The card's effects make the opponent discard.
    ForcesDiscard,
    /// The player already has a card with the same title in their Ring.
    SameTitleInRing,
    /// The card has a Superstar logo.
    SuperstarSpecific,
    /// The condition holds for the card's player (for printed cards: the effect's controller).
    Holds(Box<Condition>),
}

/// A number worked out when an effect resolves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Count {
    /// Cards in your hand plus this many.
    HandPlus(u8),
    /// Your opponent's Superstar Value times this.
    OpponentValueTimes(u8),
    /// Half your opponent's Fortitude, rounded up.
    HalfOpponentFortitude,
    /// Your Superstar Value.
    SuperstarValue,
    YourFortitude,
    /// Cards in your hand.
    HandSize,
    /// Total Stun Value in your Ring.
    YourRingStun,
    OpponentRingStun,
    YourRingMatching(CardFilter),
    OpponentRingMatching(CardFilter),
    /// Printed D of the top card of your Arsenal, at most 15.
    TopArsenalDamage,
    /// How many more cards your opponent has in hand than you.
    OpponentHandExcess,
    /// Printed D of the card in play.
    InPlayDamage,
    /// Total printed F of the cards you last discarded by choice.
    LastDiscardedFortitude,
    /// Half your Fortitude, rounded up.
    HalfYourFortitude,
    /// Cards in your opponent's hand.
    OpponentHandSize,
    /// Cards in your Ringside pile.
    YourRingsideSize,
    YourRingsideMatching(CardFilter),
    OpponentRingsideMatching(CardFilter),
    YourHandMatching(CardFilter),
    /// Cards in your Arsenal.
    YourArsenalSize,
    OpponentArsenalSize,
    /// Cards in your Backlash deck.
    YourBacklashSize,
    /// Cards in your Ring.
    YourRingSize,
    /// Cards in your opponent's Ringside pile.
    OpponentRingsideSize,
    /// Cards you have removed from the game.
    YourRemoved,
    /// Cards your opponent has removed from the game.
    OpponentRemoved,
}

/// What happens after burying a hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuryThen {
    /// Draw as many cards as were buried.
    Draw,
    /// Search the Arsenal for half as many, rounded down.
    SearchHalf,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<CardFilter>,
    },
    /// Shuffle cards of the player's choice from Ringside into their Arsenal.
    RingsideToArsenal {
        who: Who,
        n: u8,
        #[serde(default)]
        up_to: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<CardFilter>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<CardFilter>,
    },
    /// The controller searches their Arsenal (and Ringside pile, if
    /// `ringside`) for a card matching `filter`, reveals it, puts it in hand
    /// and shuffles the Arsenal.
    Search {
        filter: CardFilter,
        #[serde(default)]
        ringside: bool,
        /// How many cards (default 1).
        #[serde(default = "one", skip_serializing_if = "is_one")]
        n: u8,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        up_to: bool,
        /// Afterwards, this many of the found cards, picked at random, are
        /// shuffled back into the Arsenal.
        #[serde(default, skip_serializing_if = "is_zero")]
        random_back: u8,
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
        /// Stop after this many such Reversals (default 1).
        #[serde(default = "one", skip_serializing_if = "is_one")]
        n: u8,
        /// Stop when only this many cards are left in the Arsenal.
        #[serde(default, skip_serializing_if = "is_zero")]
        keep: u8,
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
    /// The card being played is removed from the game instead of going to the Ring.
    RemoveThis,
    /// Move `n` of the player's cards between any two of their zones (for
    /// example from the removed pile back to the hand, or from the Ringside
    /// pile into the Ring). From the Arsenal the top cards go; otherwise
    /// `pick` says who chooses. Cards moved into the Arsenal are shuffled in;
    /// cards put into the Ring this way are not played.
    MoveCards {
        who: Who,
        from: crate::state::Zone,
        to: crate::state::Zone,
        n: u8,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        up_to: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<CardFilter>,
        #[serde(default = "chosen", skip_serializing_if = "is_chosen")]
        pick: Pick,
    },
    /// The card whose effect this is goes on the top (or bottom) of its
    /// owner's Arsenal, from wherever it is.
    ThisToArsenal {
        #[serde(default)]
        bottom: bool,
    },
    /// The card whose effect this is moves to its owner's hand, Backlash deck,
    /// Arsenal (shuffled) or out of the game, from wherever it is.
    ReturnThis {
        to: crate::state::Zone,
    },
    /// Remove `n` cards of the player's from the game. From the Arsenal the
    /// top cards go; from the hand, Ring or Ringside pile `pick` says who
    /// chooses (or random).
    RemoveFromGame {
        who: Who,
        from: crate::state::Zone,
        n: u8,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        up_to: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<CardFilter>,
        #[serde(default = "chosen", skip_serializing_if = "is_chosen")]
        pick: Pick,
    },
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
    /// Effects that only happen if `condition` holds for the controller.
    If {
        condition: Condition,
        effects: Vec<Effect>,
    },
    /// The player shows their hand to the other player.
    RevealHand {
        who: Who,
    },
    /// Discard cards matching `filter`.
    DiscardMatching {
        who: Who,
        n: u8,
        filter: CardFilter,
        pick: Pick,
    },
    /// The player discards one card for each card in their Ring matching `filter`.
    DiscardPerRingCard {
        who: Who,
        filter: CardFilter,
    },
    /// The controller discards up to `max` cards; the opponent then discards as many.
    DiscardMirror {
        max: u8,
    },
    /// The player puts cards of their choice from hand on top of their Arsenal.
    HandToArsenalTop {
        who: Who,
        n: u8,
        /// The cards are picked at random instead of by the player.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        random: bool,
    },
    /// The player shuffles their hand into their Arsenal and draws as many cards.
    ShuffleHandAndRedraw {
        who: Who,
    },
    /// Look at the top `n` cards of your Arsenal, put `take` of them in your
    /// hand and the rest on the bottom.
    Dig {
        n: u8,
        take: u8,
    },
    /// The player overturns as many cards as their Superstar Value.
    DamageSuperstarValue {
        who: Who,
    },
    /// The controller moves cards matching `filter` from the player's Ring to their hand.
    RingToHand {
        who: Who,
        n: u8,
        filter: CardFilter,
        #[serde(default)]
        up_to: bool,
    },
    /// Every card in the player's Ring goes to their Ringside pile.
    ClearRing {
        who: Who,
    },
    /// Reveal cards from the top of your Arsenal until one matches; it goes to
    /// your hand and the rest are shuffled back.
    RevealUntil {
        filter: CardFilter,
    },
    /// The player cannot play cards matching `filter` during their next `turns` turns.
    CannotPlay {
        who: Who,
        filter: CardFilter,
        turns: u8,
    },
    /// A boost to your opponent's cards (it belongs to them).
    BoostOpponent(Boost),
    /// The controller wins by pinfall.
    WinByPin,
    /// Discard any number of cards: the card in play is +`per`D for each.
    DiscardForDamage {
        per: u8,
    },
    /// Only a card matching this may be your next card this turn.
    NextCardMustBe(CardFilter),
    /// Only one of the cards just chosen may be your next card this turn.
    NextCardMustBeChosen,
    /// The on-play effects of your previous card this turn happen again.
    RepeatPreviousEffects,
    /// The controller puts every card matching `filter` in the player's Arsenal
    /// into their Ringside pile, then shuffles it.
    MillAll {
        who: Who,
        filter: CardFilter,
    },
    /// The card this effect belongs to goes from the Ring to the Ringside pile.
    SacrificeThis,
    /// The next Pre-match card the player plays has no effect and goes to the
    /// Ringside pile at the end of the Pre-match phase.
    BlankNextPreMatch {
        who: Who,
    },
    /// The player puts Mid-match cards from their Ringside pile into their Backlash deck.
    RingsideToBacklash {
        who: Who,
        n: u8,
    },
    /// The controller puts cards from the player's Backlash deck into their Ringside pile.
    BacklashToRingside {
        who: Who,
        n: u8,
        /// The cards are picked at random instead of by the controller.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        random: bool,
    },
    /// The opponent picks `n` different lists, one at a time; the effects
    /// still belong to the controller.
    OpponentChooses {
        n: u8,
        options: Vec<Vec<Effect>>,
    },
    /// Effects controlled by the controller's opponent (used by
    /// `OpponentChooses` to hand the effects back to the card's controller).
    AsOpponent(Vec<Effect>),
    /// Shuffle up to `n` cards from hand into the Arsenal and draw as many.
    Mulligan {
        n: u8,
    },
    /// Draw until you have as many cards in hand as your opponent.
    DrawToMatchOpponent,
    /// Shuffle up to `count` cards from Ringside into the Arsenal.
    RingsideToArsenalCount {
        who: Who,
        count: Count,
    },
    /// Put the whole hand on the bottom of the Arsenal, then draw or search.
    BuryHand {
        who: Who,
        then: BuryThen,
    },
    /// Discard your hand and draw as many cards, at least `min`.
    DiscardHandAndDraw {
        min: u8,
    },
    /// The controller puts every copy of the card they just reversed from the
    /// opponent's hand and Arsenal into the opponent's Ringside pile.
    PurgeReversedTitle,
    ShuffleRingsideIntoArsenal {
        who: Who,
    },
    /// Your next card this turn matching `filter`: `on_success` if it is not
    /// reversed (after its damage), `on_reversed` if it is.
    NextPlayWatch {
        filter: CardFilter,
        #[serde(default)]
        on_success: Vec<Effect>,
        #[serde(default)]
        on_reversed: Vec<Effect>,
    },
    /// Draw a card for every card in both Rings, less `less`.
    DrawRingCards {
        less: u8,
    },
    /// `effect` with its amount (or, for a boost, each of its numbers
    /// multiplied) set to `count`.
    Counted {
        count: Count,
        effect: Box<Effect>,
    },
    /// The card being played cannot be reversed.
    IrreversibleThis,
    /// The player overturns cards until one matches `filter`.
    MillUntil {
        who: Who,
        filter: CardFilter,
    },
}

fn one() -> u8 {
    1
}

fn is_one(n: &u8) -> bool {
    *n == 1
}

fn is_zero_i8(n: &i8) -> bool {
    *n == 0
}

fn chosen() -> Pick {
    Pick::Chosen
}

fn is_chosen(p: &Pick) -> bool {
    *p == Pick::Chosen
}

/// Do two card titles name the same card? Case, spacing and punctuation are
/// ignored, so "Backlash" in a card's text matches the card "Backlash!".
pub fn same_title(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    norm(a) == norm(b)
}

/// Does a printed keyword trait match `word`? "Volley: 6" matches "Volley".
pub fn keyword_matches(trait_text: &str, word: &str) -> bool {
    trait_text.eq_ignore_ascii_case(word)
        || trait_text.split_once(':').is_some_and(|(head, _)| head.trim().eq_ignore_ascii_case(word))
}

#[cfg(test)]
mod tests {
    #[test]
    fn titles_match_without_case_or_punctuation() {
        assert!(super::same_title("Backlash!", "Backlash"));
        assert!(super::same_title("All the Answers ...", "all the answers..."));
        assert!(!super::same_title("Lock Up", "Lock-Down"));
    }

    #[test]
    fn keywords_match_their_numbered_forms() {
        assert!(super::keyword_matches("Volley: 6", "Volley"));
        assert!(super::keyword_matches("Heat", "heat"));
        assert!(!super::keyword_matches("Volleyball", "Volley"));
    }
}
