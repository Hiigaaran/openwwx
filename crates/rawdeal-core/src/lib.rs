//! Deterministic rules engine for the Raw Deal trading card game.
//!
//! The engine is a pure library: no I/O, no UI, no networking. A match is a
//! [`GameState`] that only changes through [`Engine::apply`], which takes one
//! player's answer to the pending [`Decision`] and returns the [`Event`]s it
//! caused. All randomness comes from a seeded RNG stored in the state, so the
//! same seed and the same actions always replay the same match.

pub mod card;
pub mod db;
pub mod decision;
pub mod deck;
pub mod engine;
pub mod event;
pub mod state;
pub mod view;

pub use card::{
    Ability, AceReversal, ActionScript, AfterSuccess, Bonus, Boost, BuryThen, CardDef, CardFilter, CardKind,
    CardScript, Condition, Count, Duration, Effect, EnforcerDef, ManeuverType, Pick, PreMatchType, ReversalDamage,
    ReverseSpec, RingTrigger, SuccessTime, SuperstarDef, Trait, Who,
};
pub use db::{CardDb, DefIdx, SuperstarIdx};
pub use decision::{Action, ChoicePurpose, Decision, DecisionKind, YesNoPurpose};
pub use deck::{
    DeckList, DeckProblem, is_revolution, may_pack_backstage, may_pack_logo, printed_match, same_superstar,
    validate_deck, validate_deck_in,
};
pub use engine::{Engine, GameSetup, RuleError, SetupError};
pub use event::Event;
pub use state::{
    CardId, GameState, Mode, NextMust, Outcome, PlayBonus, Played, PlayerId, Source, Watch, WinMethod, Zone,
};
pub use view::PlayerView;
