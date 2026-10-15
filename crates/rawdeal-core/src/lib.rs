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
    Ability, ActionScript, Bonus, Boost, CardDef, CardFilter, CardKind, CardScript, Condition, Duration, Effect,
    ManeuverType, Pick, PreMatchType, ReversalDamage, ReverseSpec, SuperstarDef, Trait, Who,
};
pub use db::{CardDb, DefIdx, SuperstarIdx};
pub use decision::{Action, ChoicePurpose, Decision, DecisionKind, YesNoPurpose};
pub use deck::{DeckList, DeckProblem, same_superstar, validate_deck};
pub use engine::{Engine, GameSetup, RuleError, SetupError};
pub use event::Event;
pub use state::{CardId, GameState, Mode, Outcome, PlayBonus, Played, PlayerId, Source, WinMethod, Zone};
pub use view::PlayerView;
