//! The rules engine: a resumable step machine that pauses whenever a player
//! has to decide something.

use std::sync::Arc;

use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::card::{
    Ability, Boost, BuryThen, CardDef, CardFilter, CardKind, Condition, Count, Duration, Effect, Pick, PreMatchType,
    ReversalDamage, ReverseSpec, Trait, Who,
};
use crate::db::{CardDb, DefIdx};
use crate::decision::{Action, ChoicePurpose, Decision, DecisionKind, YesNoPurpose};
use crate::deck::{DeckList, DeckProblem};
use crate::event::Event;
use crate::state::{
    CardId, CardInstance, GameState, Mode, NextMust, Outcome, PlayBonus, Played, PlayerId, PlayerState, Source, Watch,
    WinMethod, Zone,
};
use crate::view::PlayerView;

/// Everything needed to start a match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameSetup {
    pub decks: [DeckList; 2],
    /// Enforce Arsenal construction rules (60 cards, copy limits, …).
    /// Tests and puzzles may turn this off.
    pub enforce_deck_rules: bool,
    /// Shuffle the Arsenals at the start. When `false` each Arsenal keeps the
    /// deck list order with the first card on top, which tests and puzzles use
    /// to stack a deck.
    pub shuffle_arsenals: bool,
    #[serde(default)]
    pub mode: Mode,
}

impl GameSetup {
    /// A normal Quick Start match: legal decks, shuffled.
    pub fn standard(decks: [DeckList; 2]) -> Self {
        GameSetup { decks, enforce_deck_rules: true, shuffle_arsenals: true, mode: Mode::QuickStart }
    }

    /// A normal Advanced match, with Pre-match phase and Backlash decks.
    pub fn advanced(decks: [DeckList; 2]) -> Self {
        GameSetup { mode: Mode::Advanced, ..GameSetup::standard(decks) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetupError {
    UnknownCard(String),
    UnknownSuperstar(String),
    InvalidDeck { player: PlayerId, problems: Vec<DeckProblem> },
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::UnknownCard(id) => write!(f, "unknown card {id}"),
            SetupError::UnknownSuperstar(id) => write!(f, "unknown superstar {id}"),
            SetupError::InvalidDeck { player, problems } => {
                write!(f, "{player}'s deck is invalid: {problems:?}")
            }
        }
    }
}

impl std::error::Error for SetupError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    GameOver,
    NotYourDecision,
    Illegal(&'static str),
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleError::GameOver => write!(f, "the game is over"),
            RuleError::NotYourDecision => write!(f, "it is not this player's decision"),
            RuleError::Illegal(why) => write!(f, "illegal action: {why}"),
        }
    }
}

impl std::error::Error for RuleError {}

/// Internal unit of work. `GameState::pending` is a stack of these; the last
/// element runs next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum Step {
    /// Decides who goes first and starts the match or the Pre-match phase.
    Begin,
    /// `decider` chooses an Enforcer for `who` (a different one with `change`).
    PickEnforcer {
        who: PlayerId,
        decider: PlayerId,
        change: bool,
    },
    StartTurn,
    DrawSegment,
    Main,
    ReversalWindow,
    ResolveEffects,
    DealCardDamage,
    /// After the card in play dealt its damage without being reversed.
    AfterDamage,
    Effect {
        effect: Effect,
        controller: PlayerId,
        source: Source,
    },
    /// `player` overturns `remaining` cards. `source` is the played card
    /// causing the damage, if overturned Reversals can stop it.
    Overturn {
        player: PlayerId,
        remaining: u16,
        source: Option<CardId>,
    },
    ToRing,
    StunDraw {
        player: PlayerId,
        max: u8,
    },
    EndTurn,
    CountOutCheck,
    /// After a Chain card's damage: offer the Chain search.
    ChainSearch,
    /// Search `player`'s Arsenal for a Chain card.
    SearchChain {
        player: PlayerId,
    },
    /// Reverse the card in play from hand or Backlash (after any Chain cost).
    ReverseFromHand {
        defender: PlayerId,
        reversal: CardId,
    },
    /// Pre-match step `step` (see [`DecisionKind::PreMatch`]): `actor` may play.
    PreMatch {
        step: u8,
        actor: PlayerId,
        passes: u8,
        starter: PlayerId,
    },
    EndPreMatch {
        starter: PlayerId,
    },
    DecideFirst {
        starter: PlayerId,
    },
    /// `chooser` puts the top cards of `owner`'s Arsenal back in order: `done`
    /// from the top down, then whatever is left in `left`.
    Arrange {
        chooser: PlayerId,
        owner: PlayerId,
        left: Vec<CardId>,
        done: Vec<CardId>,
    },
    /// `player` overturns until a Reversal that could stop `target` shows up.
    OverturnUntil {
        player: PlayerId,
        target: Played,
        /// Reversals still to find.
        left: u8,
        /// Stop when the Arsenal is down to this many cards.
        keep: u8,
    },
    LevelFortitude,
    /// A large choice asked one card at a time: `remaining` more picks.
    AskMore {
        decider: PlayerId,
        options: Vec<CardId>,
        remaining: u8,
        up_to: bool,
        purpose: ChoicePurpose,
    },
    /// `n` of the cards just searched for, at random, go back into the Arsenal.
    RandomBack {
        player: PlayerId,
        n: u8,
    },
    /// The looked-at cards still on the Arsenal go to its bottom.
    /// The attacker may answer the Reversal the defender just played.
    CounterWindow {
        attacker: PlayerId,
        reversal: CardId,
    },
    DigRest {
        cards: Vec<CardId>,
    },
}

/// Pre-match card types for steps 0-4.
const PRE_MATCH_STEPS: [PreMatchType; 5] = [
    PreMatchType::Allegiance,
    PreMatchType::Venue,
    PreMatchType::Feud,
    PreMatchType::Stipulation,
    PreMatchType::Manager,
];
const OPEN_STEP: u8 = 5;
const RING_LIMIT: usize = 5;
const REVOLUTION_RING_LIMIT: usize = 3;
const VIRTUAL_RING_LIMIT: usize = 6;

#[derive(Debug, Clone)]
pub struct Engine {
    db: Arc<CardDb>,
}

impl Engine {
    pub fn new(db: Arc<CardDb>) -> Self {
        Engine { db }
    }

    pub fn db(&self) -> &CardDb {
        &self.db
    }

    /// Starts a match. Quick Start: shuffle, draw Hand Size, highest Superstar
    /// Value goes first (coin flip on a tie). Advanced: the Pre-match phase
    /// runs first and hands are drawn at its end.
    pub fn new_game(&self, setup: &GameSetup, seed: u64) -> Result<(GameState, Vec<Event>), SetupError> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = Vec::new();
        let mut players = Vec::with_capacity(2);
        for (pi, deck) in setup.decks.iter().enumerate() {
            let player = PlayerId(pi as u8);
            let superstar = self
                .db
                .superstar_by_id(&deck.superstar)
                .ok_or_else(|| SetupError::UnknownSuperstar(deck.superstar.clone()))?;
            if setup.enforce_deck_rules {
                let problems = crate::deck::validate_deck_in(&self.db, deck, setup.mode);
                if !problems.is_empty() {
                    return Err(SetupError::InvalidDeck { player, problems });
                }
            }
            let mut add = |ids: &[String], zone: Zone| -> Result<Vec<CardId>, SetupError> {
                ids.iter()
                    .map(|id| {
                        let def = self.db.card_by_id(id).ok_or_else(|| SetupError::UnknownCard(id.clone()))?;
                        cards.push(CardInstance { def, owner: player, zone, host: None });
                        Ok(CardId(cards.len() as u16 - 1))
                    })
                    .collect()
            };
            let mut arsenal = add(&deck.arsenal, Zone::Arsenal)?;
            let backstage = deck
                .backstage
                .iter()
                .map(|id| self.db.card_by_id(id).ok_or_else(|| SetupError::UnknownCard(id.clone())))
                .collect::<Result<Vec<_>, _>>()?;
            let backlash = match setup.mode {
                Mode::Advanced | Mode::Revolution | Mode::Virtual => add(&deck.backlash, Zone::Backlash)?,
                Mode::QuickStart => vec![],
            };
            if setup.shuffle_arsenals {
                arsenal.shuffle(&mut rng);
            } else {
                arsenal.reverse();
            }
            players.push(PlayerState {
                superstar,
                arsenal,
                hand: vec![],
                ring: vec![],
                ringside: vec![],
                backlash,
                removed: vec![],
                under: vec![],
                hidden: backstage.iter().copied().filter(|&c| self.db.card(c).script.starts_hidden).collect(),
                backstage,
                enforcer: None,
                ability_used: false,
            });
        }
        let players: [PlayerState; 2] = players.try_into().expect("two decks");

        let first = PlayerId::ONE;

        let mut state = GameState {
            cards,
            players,
            active: first,
            turn: 0,
            mode: setup.mode,
            outcome: None,
            in_play: None,
            last_played: None,
            refused_count_out: [false; 2],
            decision: None,
            boosts: vec![],
            end_of_turn: vec![],
            skip_turn: [false; 2],
            start_after: [None; 2],
            end_turn_after_play: false,
            continue_turn: false,
            in_play_to: None,
            in_play_bottom: None,
            played_this_turn: vec![],
            aggression_checked: false,
            backstage_used: vec![],
            reversed_last: [None; 2],
            reversed_from: [None; 2],
            heat_used: [false; 2],
            volley_lock: None,
            volley_turn: None,
            effect_ctx: None,
            restrictions: vec![],
            ring_used: vec![],
            lapsed: vec![],
            last_chosen: vec![],
            last_discarded: vec![],
            ability_spent: [false; 2],
            next_must: None,
            blank_prematch: [false; 2],
            watches: vec![],
            in_play_watch: None,
            pending: vec![],
            rng,
        };
        let mut events = Vec::new();
        if setup.shuffle_arsenals {
            events.extend([PlayerId::ONE, PlayerId::TWO].map(|player| Event::ArsenalShuffled { player }));
        }
        // Enforcers are chosen first: they change Hand Size and Superstar Value.
        state.pending.push(Step::Begin);
        for p in [PlayerId::TWO, PlayerId::ONE] {
            state.pending.push(Step::PickEnforcer { who: p, decider: p, change: false });
        }
        self.run(&mut state, &mut events);
        Ok((state, events))
    }

    /// What `player` sees of the game.
    pub fn view(&self, state: &GameState, player: PlayerId) -> PlayerView {
        PlayerView::new(self, state, player)
    }

    /// Fortitude Rating: total printed D of the cards in the player's Ring.
    pub fn fortitude(&self, state: &GameState, player: PlayerId) -> u16 {
        state.player(player).ring.iter().map(|&c| self.def(state, c).damage as u16).sum()
    }

    /// Damage `played` deals if it succeeds, before the defender's reductions.
    /// A hybrid card played as an Action deals none.
    pub fn card_damage(&self, state: &GameState, played: Played) -> u16 {
        let def = self.def(state, played.card);
        let is_maneuver = |k: &CardKind| matches!(k, CardKind::Maneuver(_));
        if !is_maneuver(&played.kind) && def.kinds.iter().any(is_maneuver) {
            return 0;
        }
        let damage = (def.damage as i16 + played.bonus.damage).max(0) as u16;
        if played.bonus.double_damage { damage * 2 } else { damage }
    }

    /// Hand Size, after Superstar abilities that change it.
    pub fn hand_size(&self, state: &GameState, p: PlayerId) -> u8 {
        let printed = self.db.superstar(state.player(p).superstar).hand_size;
        let printed = printed.saturating_add(self.enforcer(state, p).map_or(0, |e| e.hand_size));
        let opponent_value = self.superstar_value(state, p.opponent());
        let bonus: u8 = self.ring_cards(state, p).iter().map(|&c| self.def(state, c).script.hand_size_bonus).sum();
        let theirs: i16 = self
            .ring_cards(state, p.opponent())
            .iter()
            .map(|&c| self.def(state, c).script.opponent_hand_size_bonus as i16)
            .sum();
        self.abilities(state, p)
            .iter()
            .fold(printed, |n, a| match a {
                Ability::HandSizeFromOpponentValue { times, min } => opponent_value.saturating_mul(*times).max(*min),
                _ => n,
            })
            .saturating_add(bonus)
            .saturating_add_signed(theirs.clamp(i8::MIN as i16, i8::MAX as i16) as i8)
    }

    /// Every action `player` may take right now.
    pub fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        let Some(decision) = &state.decision else { return vec![] };
        if decision.player != player || state.is_over() {
            return vec![];
        }
        match &decision.kind {
            DecisionKind::Main => {
                let me = state.player(player);
                let mut actions: Vec<Action> = me
                    .hand
                    .iter()
                    .chain(&me.backlash)
                    .chain(me.ringside.iter().filter(|&&c| {
                        let def = self.def(state, c);
                        def.script.from_ringside.is_some() || def.has_keyword("Heat")
                    }))
                    .chain(&me.under)
                    .flat_map(|&card| {
                        self.def(state, card)
                            .kinds
                            .iter()
                            .filter(move |&&kind| self.can_play(state, player, card, kind))
                            .map(move |&kind| Action::PlayCard { card, kind })
                    })
                    .collect();
                if self.can_use_ability(state, player) {
                    actions.push(Action::UseAbility);
                }
                actions.extend(
                    me.ring
                        .iter()
                        .chain(&me.ringside)
                        .filter(|&&card| self.can_use_ring_card(state, player, card))
                        .map(|&card| Action::UseRingCard { card }),
                );
                actions.extend(
                    me.backstage
                        .iter()
                        .filter(|&&card| self.can_use_backstage_card(state, player, card))
                        .map(|&card| Action::UseBackstageCard { card }),
                );
                actions.push(Action::EndTurn);
                actions
            }
            DecisionKind::Reversal { options, volley, .. } => options
                .iter()
                .map(|&card| Action::Reverse { card })
                .chain(volley.then_some(Action::ReverseByVolley))
                .chain([Action::DeclineReversal])
                .collect(),
            DecisionKind::CounterReversal { options, .. } => {
                options.iter().map(|&card| Action::Reverse { card }).chain([Action::DeclineReversal]).collect()
            }
            DecisionKind::ChooseCards { options, min, max, .. } => {
                let mut out = Vec::new();
                for k in *min..=*max {
                    combinations(options, k as usize, &mut |c| out.push(Action::ChooseCards(c.to_vec())));
                }
                out
            }
            DecisionKind::YesNo { .. } => vec![Action::Yes, Action::No],
            DecisionKind::StunDraw { max } => (0..=*max).map(Action::DrawCards).collect(),
            DecisionKind::ChooseOption { options, .. } => (0..options.len() as u8).map(Action::ChooseOption).collect(),
            DecisionKind::ChooseEnforcer { options, .. } => {
                (0..options.len() as u8).map(Action::ChooseOption).collect()
            }
            DecisionKind::PreMatch { step, .. } => self
                .pre_match_options(state, player, *step)
                .into_iter()
                .map(|(card, kind)| Action::PlayCard { card, kind })
                .chain([Action::Pass])
                .collect(),
        }
    }

    /// Applies `player`'s answer to the pending decision and runs the game
    /// until the next decision or the end of the match. On error the state is
    /// left unchanged.
    pub fn apply(&self, state: &mut GameState, player: PlayerId, action: Action) -> Result<Vec<Event>, RuleError> {
        if state.is_over() {
            return Err(RuleError::GameOver);
        }
        let decision = state.decision.clone().ok_or(RuleError::Illegal("nothing to decide"))?;
        if decision.player != player {
            return Err(RuleError::NotYourDecision);
        }
        self.validate(state, &decision, &action)?;
        state.decision = None;
        let mut events = Vec::new();
        self.answer(state, decision, action, &mut events);
        self.run(state, &mut events);
        Ok(events)
    }

    fn validate(&self, state: &GameState, decision: &Decision, action: &Action) -> Result<(), RuleError> {
        let p = decision.player;
        let ok = match (&decision.kind, action) {
            (DecisionKind::Main, Action::EndTurn) => true,
            (DecisionKind::Main, Action::UseAbility) => self.can_use_ability(state, p),
            (DecisionKind::Main, Action::UseRingCard { card }) => {
                (card.0 as usize) < state.cards.len() && self.can_use_ring_card(state, p, *card)
            }
            (DecisionKind::Main, Action::UseBackstageCard { card }) => self.can_use_backstage_card(state, p, *card),
            (DecisionKind::Main, Action::PlayCard { card, kind }) => {
                if (card.0 as usize) >= state.cards.len() {
                    return Err(RuleError::Illegal("no such card"));
                }
                if !self.def(state, *card).kinds.contains(kind) {
                    return Err(RuleError::Illegal("card cannot be played as that kind"));
                }
                if !self.can_play(state, p, *card, *kind) {
                    return Err(RuleError::Illegal("card cannot be played now"));
                }
                true
            }
            (DecisionKind::Reversal { options, .. }, Action::Reverse { card }) => options.contains(card),
            (DecisionKind::Reversal { .. }, Action::DeclineReversal) => true,
            (DecisionKind::CounterReversal { options, .. }, Action::Reverse { card }) => options.contains(card),
            (DecisionKind::CounterReversal { .. }, Action::DeclineReversal) => true,
            (DecisionKind::Reversal { volley, .. }, Action::ReverseByVolley) => *volley,
            (DecisionKind::ChooseCards { options, min, max, .. }, Action::ChooseCards(chosen)) => {
                let mut dedup = chosen.clone();
                dedup.sort();
                dedup.dedup();
                dedup.len() == chosen.len()
                    && chosen.iter().all(|c| options.contains(c))
                    && (*min as usize..=*max as usize).contains(&chosen.len())
            }
            (DecisionKind::YesNo { .. }, Action::Yes | Action::No) => true,
            (DecisionKind::StunDraw { max }, Action::DrawCards(n)) => n <= max,
            (DecisionKind::ChooseOption { options, .. }, Action::ChooseOption(i)) => (*i as usize) < options.len(),
            (DecisionKind::ChooseEnforcer { options, .. }, Action::ChooseOption(i)) => (*i as usize) < options.len(),
            (DecisionKind::PreMatch { step, .. }, Action::PlayCard { card, kind }) => {
                self.pre_match_options(state, p, *step).contains(&(*card, *kind))
            }
            (DecisionKind::PreMatch { .. }, Action::Pass) => true,
            _ => false,
        };
        if ok { Ok(()) } else { Err(RuleError::Illegal("action does not answer the pending decision")) }
    }

    fn answer(&self, state: &mut GameState, decision: Decision, action: Action, events: &mut Vec<Event>) {
        let p = decision.player;
        match (decision.kind, action) {
            (DecisionKind::Main, Action::EndTurn) => state.pending.push(Step::EndTurn),
            (DecisionKind::Main, Action::UseAbility) => {
                let (cost, effects, once) = self.activated_ability(state, p).expect("validated");
                let all: Vec<Effect> = cost.iter().chain(effects).cloned().collect();
                state.player_mut(p).ability_used = true;
                if once {
                    state.ability_spent[p.index()] = true;
                }
                state.pending.push(Step::Main);
                schedule_effects(state, &all, p, Source::Superstar(p));
            }
            (DecisionKind::Main, Action::UseRingCard { card }) => {
                let script = &self.def(state, card).script;
                let ability = match state.card(card).zone {
                    Zone::Ringside => script.ringside_ability.clone(),
                    _ => script.ring_ability.clone(),
                }
                .expect("validated");
                state.ring_used.push(card);
                if ability.sacrifice {
                    self.move_card(state, card, Zone::Ringside, events);
                }
                state.pending.push(Step::Main);
                let all: Vec<Effect> = ability.cost.iter().chain(&ability.effects).cloned().collect();
                schedule_effects(state, &all, p, Source::Card(card));
            }
            (DecisionKind::Main, Action::UseBackstageCard { card }) => {
                let ability = self.db.card(card).script.backstage_ability.clone().expect("validated");
                state.backstage_used.push((p, card));
                if ability.sacrifice {
                    state.player_mut(p).hidden.push(card);
                    events.push(Event::BackstageHidden { player: p, card });
                }
                state.pending.push(Step::Main);
                let all: Vec<Effect> = ability.cost.iter().chain(&ability.effects).cloned().collect();
                schedule_effects(state, &all, p, Source::Superstar(p));
            }
            (DecisionKind::Main, Action::PlayCard { card, kind }) => {
                let pay_k = self.fortitude_payment(state, p, card, kind).unwrap_or(0);
                let (bonus, used, end_turn) = self.play_bonus(state, p, card, kind);
                for i in used.into_iter().rev() {
                    match &mut state.boosts[i].1.duration {
                        Duration::NextCards(n) if *n > 1 => *n -= 1,
                        _ => {
                            state.boosts.remove(i);
                        }
                    }
                }
                if state.next_must.as_ref().is_some_and(|(q, _)| *q == p) {
                    state.next_must = None;
                }
                let from_ringside = state.card(card).zone == Zone::Ringside;
                if from_ringside && !self.ringside_by_script(state, p, card) {
                    state.heat_used[p.index()] = true;
                }
                self.move_card(state, card, Zone::InPlay, events);
                let after = state.last_played.map(|l| l.card);
                let played = Played { after, bonus, from_ringside, ..Played::new(card, p, kind) };
                state.in_play = Some(played);
                state.played_this_turn.push(played);
                state.in_play_to = None;
                state.in_play_bottom = None;
                state.continue_turn = false;
                state.in_play_watch = state
                    .watches
                    .iter()
                    .position(|w| w.player == p && self.matches(state, &w.filter, played))
                    .map(|i| state.watches.remove(i));
                if end_turn {
                    state.end_turn_after_play = true;
                }
                events.push(Event::CardPlayed { player: p, card, kind });
                schedule(
                    state,
                    [
                        Step::ReversalWindow,
                        Step::ResolveEffects,
                        Step::DealCardDamage,
                        Step::AfterDamage,
                        Step::ChainSearch,
                        Step::ToRing,
                        Step::Main,
                    ],
                );
                let before = self.def(state, card).script.before_reversal.clone();
                schedule_effects(state, &before, p, Source::Card(card));
                if let Some(d) = self.def(state, card).script.fortitude_discount {
                    // Paid before the card moved: the card no longer counts in hand.
                    let k = pay_k;
                    if k > 0 {
                        let effect = if d.overturn {
                            Effect::Mill { who: Who::You, n: k }
                        } else {
                            Effect::Discard { who: Who::You, n: k, pick: Pick::Chosen }
                        };
                        schedule_effects(state, &[effect], p, Source::Card(card));
                    }
                }
                let cost = self.play_cost(state, p, played);
                if cost > 0 {
                    schedule_effects(
                        state,
                        &[Effect::Discard { who: Who::You, n: cost, pick: Pick::Chosen }],
                        p,
                        Source::Superstar(p),
                    );
                }
            }
            (DecisionKind::Reversal { .. }, Action::ReverseByVolley) => self.volley_reverse(state, p, events),
            (DecisionKind::Reversal { card: attacker, .. }, Action::Reverse { card }) => {
                if state.card(card).zone == Zone::Ringside && !self.ringside_by_script(state, p, card) {
                    state.heat_used[p.index()] = true;
                }
                state.pending.push(Step::ReverseFromHand { defender: p, reversal: card });
                if self.def(state, attacker).has_trait(Trait::Chain) {
                    // Reversing a Chain card from hand or Backlash costs 2 discards.
                    let options: Vec<CardId> = state.player(p).hand.iter().copied().filter(|&c| c != card).collect();
                    state.decision = Some(Decision {
                        player: p,
                        kind: DecisionKind::ChooseCards { options, min: 2, max: 2, purpose: ChoicePurpose::Discard },
                    });
                }
            }
            (DecisionKind::PreMatch { step, passes, starter }, action) => {
                let played = matches!(action, Action::PlayCard { .. });
                state.pending.push(next_pre_match(step, p, passes, starter, played));
                if let Action::PlayCard { card, kind } = action {
                    self.move_card(state, card, Zone::Ring, events);
                    events.push(Event::CardPlayed { player: p, card, kind });
                    if std::mem::take(&mut state.blank_prematch[p.index()]) {
                        state.lapsed.push(card);
                    } else {
                        let effects = self.def(state, card).script.on_play.clone();
                        schedule_effects(state, &effects, p, Source::Card(card));
                    }
                }
            }
            (DecisionKind::Reversal { .. }, _) => {}
            (DecisionKind::CounterReversal { reversal, .. }, Action::Reverse { card }) => {
                self.counter_reverse(state, p, card, reversal, events)
            }
            (DecisionKind::CounterReversal { .. }, _) => {}
            (DecisionKind::ChooseCards { purpose, .. }, Action::ChooseCards(chosen)) => {
                if let Some(Step::AskMore { options, remaining, .. }) = state.pending.last_mut() {
                    options.retain(|c| !chosen.contains(c));
                    if chosen.is_empty() {
                        *remaining = 0;
                    }
                }
                self.answer_choice(state, p, purpose, chosen, events);
            }
            (DecisionKind::YesNo { purpose }, yes_no) => {
                let yes = yes_no == Action::Yes;
                match purpose {
                    YesNoPurpose::OptionalEffect { effects, source } => {
                        if yes {
                            schedule_effects(state, &effects, p, source);
                        }
                    }
                    YesNoPurpose::ClaimCountOut => {
                        if yes {
                            self.finish(state, Outcome::Win { winner: p, method: WinMethod::CountOut }, events);
                        } else {
                            state.refused_count_out[p.index()] = true;
                            events.push(Event::CountOutRefused { player: p });
                        }
                    }
                    YesNoPurpose::ChainSearch => {
                        if yes {
                            state.pending.push(Step::SearchChain { player: p });
                            let cost = [Effect::Discard { who: Who::You, n: 2, pick: Pick::Chosen }];
                            schedule_effects(state, &cost, p, Source::Superstar(p));
                        }
                    }
                }
            }
            (DecisionKind::StunDraw { .. }, Action::DrawCards(n)) => self.draw(state, p, n, events),
            (DecisionKind::ChooseEnforcer { who, options }, Action::ChooseOption(i)) => {
                let (card, index) = options[i as usize];
                state.player_mut(who).enforcer = Some((card, index));
                events.push(Event::EnforcerChosen { player: who, card, index });
            }
            (DecisionKind::ChooseOption { options, source }, Action::ChooseOption(i)) => {
                schedule_effects(state, &options[i as usize], p, source);
            }
            _ => unreachable!("validated above"),
        }
    }

    fn answer_choice(
        &self,
        state: &mut GameState,
        p: PlayerId,
        purpose: ChoicePurpose,
        chosen: Vec<CardId>,
        events: &mut Vec<Event>,
    ) {
        match purpose {
            ChoicePurpose::Discard | ChoicePurpose::RingToRingside => {
                if purpose == ChoicePurpose::Discard {
                    state.last_discarded = chosen.clone();
                }
                for c in chosen {
                    let from_ring = state.card(c).zone == Zone::Ring;
                    self.move_card(state, c, Zone::Ringside, events);
                    if from_ring {
                        self.removed_from_ring(state, c, p);
                    }
                }
            }
            ChoicePurpose::ForcedDiscard => {
                for c in chosen {
                    self.forced_discard(state, c, events);
                }
            }
            ChoicePurpose::HandToArsenal => {
                for c in chosen {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, p, events);
            }
            ChoicePurpose::ArsenalToRingside => {
                let Some(owner) = chosen.first().map(|&c| state.card(c).owner) else { return };
                for c in chosen {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                self.shuffle_arsenal(state, owner, events);
            }
            ChoicePurpose::Arrange => {
                if let (Some(Step::Arrange { left, done, .. }), Some(&c)) = (state.pending.last_mut(), chosen.first()) {
                    left.retain(|&l| l != c);
                    done.push(c);
                }
            }
            ChoicePurpose::RingsideToHand => {
                for c in chosen {
                    self.move_card(state, c, Zone::Hand, events);
                    state.last_chosen.push(c);
                }
            }
            ChoicePurpose::DiscardForDamage { per } => {
                let n = chosen.len() as i16;
                for c in chosen {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                if let Some(played) = state.in_play.as_mut() {
                    played.bonus.damage += n * per as i16;
                }
            }
            ChoicePurpose::RingsideToBacklash => {
                for c in chosen {
                    self.move_card(state, c, Zone::Backlash, events);
                }
            }
            ChoicePurpose::Mulligan => {
                let n = chosen.len().min(u8::MAX as usize) as u8;
                for c in chosen {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, p, events);
                self.draw(state, p, n, events);
            }
            ChoicePurpose::HandToArsenalBottom | ChoicePurpose::RingsideToArsenalBottom => {
                for c in chosen {
                    self.move_card_to_bottom(state, c, events);
                }
            }
            ChoicePurpose::SearchToHand => {
                for c in chosen {
                    self.move_card(state, c, Zone::Hand, events);
                    events.push(Event::Revealed { player: p, card: c });
                    state.last_chosen.push(c);
                }
                self.shuffle_arsenal(state, p, events);
            }
            ChoicePurpose::DiscardMirror => {
                let n = chosen.len() as u8;
                for c in chosen {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                if n > 0 {
                    let effect = Effect::Discard { who: Who::Opponent, n, pick: Pick::Chosen };
                    state.pending.push(Step::Effect { effect, controller: p, source: Source::Superstar(p) });
                }
            }
            ChoicePurpose::HandToArsenalTop => {
                for c in chosen {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
            }
            ChoicePurpose::RingToHand | ChoicePurpose::DigToHand => {
                for c in chosen {
                    self.move_card(state, c, Zone::Hand, events);
                }
            }
            ChoicePurpose::MoveTo(to) => {
                let mut owners = vec![];
                for c in chosen {
                    let owner = state.card(c).owner;
                    let from = state.card(c).zone;
                    self.move_card(state, c, to, events);
                    if from == Zone::Ring {
                        self.removed_from_ring(state, c, p);
                    }
                    if (to == Zone::Arsenal || from == Zone::Arsenal) && !owners.contains(&owner) {
                        owners.push(owner);
                    }
                }
                for owner in owners {
                    self.shuffle_arsenal(state, owner, events);
                }
            }
            ChoicePurpose::ToArsenalTop => {
                for c in chosen {
                    let from_ring = state.card(c).zone == Zone::Ring;
                    self.move_card(state, c, Zone::Arsenal, events);
                    if from_ring {
                        self.removed_from_ring(state, c, p);
                    }
                }
            }
            ChoicePurpose::ToArsenalBottom => {
                for c in chosen {
                    let from_ring = state.card(c).zone == Zone::Ring;
                    self.move_card_to_bottom(state, c, events);
                    if from_ring {
                        self.removed_from_ring(state, c, p);
                    }
                }
            }
            ChoicePurpose::RemoveFromGame => {
                let searched: Vec<PlayerId> = chosen
                    .iter()
                    .filter(|&&c| state.card(c).zone == Zone::Arsenal)
                    .map(|&c| state.card(c).owner)
                    .collect();
                for c in chosen {
                    let from_ring = state.card(c).zone == Zone::Ring;
                    self.move_card(state, c, Zone::Removed, events);
                    if from_ring {
                        self.removed_from_ring(state, c, p);
                    }
                }
                if let Some(&owner) = searched.first() {
                    self.shuffle_arsenal(state, owner, events);
                }
            }
            ChoicePurpose::PutUnder(host) => {
                let mut searched = false;
                for c in chosen {
                    searched |= state.card(c).zone == Zone::Arsenal;
                    self.move_card(state, c, Zone::Under, events);
                    state.cards[c.0 as usize].host = Some(host);
                }
                if searched {
                    self.shuffle_arsenal(state, p, events);
                }
            }
            ChoicePurpose::RingsideToArsenal => {
                for c in chosen {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, p, events);
            }
        }
    }

    /// Runs pending steps until a decision is needed or the game ends.
    fn run(&self, state: &mut GameState, events: &mut Vec<Event>) {
        while state.decision.is_none() && !state.is_over() {
            let Some(step) = state.pending.pop() else {
                debug_assert!(false, "step stack ran dry");
                return;
            };
            self.step(state, step, events);
        }
    }

    fn step(&self, state: &mut GameState, step: Step, events: &mut Vec<Event>) {
        match &step {
            Step::Effect { controller, source, .. } => {
                let heat = matches!(*source, Source::Card(c) if self.def(state, c).has_keyword("Heat"));
                state.effect_ctx = Some((*controller, heat));
            }
            Step::AskMore { .. }
            | Step::RandomBack { .. }
            | Step::DigRest { .. }
            | Step::Arrange { .. }
            | Step::OverturnUntil { .. } => {}
            _ => state.effect_ctx = None,
        }
        match step {
            Step::Begin => {
                let values = [PlayerId::ONE, PlayerId::TWO].map(|p| self.superstar_value(state, p));
                let first = match values[0].cmp(&values[1]) {
                    std::cmp::Ordering::Greater => PlayerId::ONE,
                    std::cmp::Ordering::Less => PlayerId::TWO,
                    std::cmp::Ordering::Equal => PlayerId(state.rng.random_range(0..2u8)),
                };
                state.active = first;
                match state.mode {
                    Mode::QuickStart => {
                        events.push(Event::GameStarted { first });
                        for p in [first, first.opponent()] {
                            let n = self.hand_size(state, p);
                            self.draw(state, p, n, events);
                        }
                        state.pending.push(Step::StartTurn);
                    }
                    Mode::Advanced | Mode::Revolution | Mode::Virtual => {
                        if state.mode == Mode::Revolution {
                            for p in [first, first.opponent()] {
                                let n = self.hand_size(state, p);
                                self.draw(state, p, n, events);
                            }
                        }
                        events.push(Event::PreMatchStarted { first });
                        state.pending.push(Step::PreMatch { step: 0, actor: first, passes: 0, starter: first });
                    }
                }
            }
            Step::PickEnforcer { who, decider, change } => {
                let options = self.enforcer_options(state, who, change);
                if !options.is_empty() {
                    state.decision =
                        Some(Decision { player: decider, kind: DecisionKind::ChooseEnforcer { who, options } });
                }
            }
            Step::StartTurn => {
                state.turn += 1;
                let p = state.active;
                events.push(Event::TurnStarted { player: p, turn: state.turn });
                state.player_mut(p).ability_used = false;
                state.played_this_turn.clear();
                state.aggression_checked = false;
                // A refused count out is offered again at the end of the next turn, so a
                // match where nobody can pin still ends.
                state.refused_count_out = [false; 2];
                state.backstage_used.clear();
                state.ring_used.clear();
                state.heat_used = [false; 2];
                state.volley_turn = state.volley_lock.take().filter(|&q| q == p);
                state.last_played = state.start_after[p.index()].take().map(|c| Played::new(c, p, CardKind::Action));
                for (owner, boost) in &mut state.boosts {
                    if *owner == p {
                        boost.duration = match boost.duration {
                            Duration::NextTurn => Duration::ThisTurn,
                            Duration::NextTurnFirstCard => Duration::NextCard,
                            Duration::NextTurnCards(n) => Duration::NextCards(n),
                            d => d,
                        };
                    }
                }
                schedule(state, [Step::DrawSegment, Step::Main]);
                for ability in self.abilities(state, p).iter().rev() {
                    if let Ability::StartOfTurn(effects) = ability {
                        schedule_effects(state, effects, p, Source::Superstar(p));
                    }
                }
                for c in self.ring_cards(state, p).into_iter().rev() {
                    let effects = self.def(state, c).script.in_ring_start_of_turn.clone();
                    schedule_effects(state, &effects, p, Source::Card(c));
                }
                for c in self.ring_cards(state, p.opponent()).into_iter().rev() {
                    let effects = self.def(state, c).script.in_ring_opponent_start_of_turn.clone();
                    schedule_effects(state, &effects, p, Source::Card(c));
                }
            }
            Step::DrawSegment => {
                let p = state.active;
                let n = self
                    .abilities(state, p)
                    .iter()
                    .filter_map(|a| if let Ability::DrawSegment(n) = a { Some(*n) } else { None })
                    .max()
                    .unwrap_or(1);
                let extra: u8 = self.ring_cards(state, p).iter().map(|&c| self.def(state, c).script.extra_draw).sum();
                let option = self.abilities(state, p).iter().find_map(|a| match a {
                    Ability::DrawSegmentOption(effects) => Some(effects.clone()),
                    _ => None,
                });
                match option {
                    Some(effects) => {
                        let effect = Effect::Choice(vec![vec![Effect::Draw { who: Who::You, n: n + extra }], effects]);
                        state.pending.push(Step::Effect { effect, controller: p, source: Source::Superstar(p) });
                    }
                    None => self.draw(state, p, n + extra, events),
                }
                for ability in self.abilities(state, p) {
                    if let Ability::DrawSegmentDiscard(k) = ability {
                        let effect = Effect::Discard { who: Who::You, n: *k, pick: Pick::Chosen };
                        state.pending.push(Step::Effect { effect, controller: p, source: Source::Superstar(p) });
                    }
                }
            }
            Step::Main => {
                if std::mem::take(&mut state.end_turn_after_play) {
                    state.pending.push(Step::EndTurn);
                } else {
                    state.decision = Some(Decision { player: state.active, kind: DecisionKind::Main });
                }
            }
            Step::ReversalWindow => {
                let played = state.in_play.expect("card in play");
                let defender = played.player.opponent();
                let me = state.player(defender);
                let chain = self.def(state, played.card).has_trait(Trait::Chain);
                let ring = self.ring_cards(state, defender);
                let from_ringside = |c: &CardId| {
                    self.ringside_by_script(state, defender, *c) || self.heat_playable(state, defender, *c)
                };
                let options: Vec<CardId> = me
                    .hand
                    .iter()
                    .chain(me.backlash.iter().filter(|&&c| self.def(state, c).has_trait(Trait::MidMatch)))
                    .chain(me.ringside.iter().filter(|c| from_ringside(c)))
                    .chain(me.under.iter().filter(|&&c| self.playable_from_under(state, defender, c)))
                    .copied()
                    .filter(|&r| {
                        self.can_reverse(state, defender, r, played, true)
                            || self.can_discard_reverse(state, defender, r, played)
                    })
                    .chain(ring.into_iter().filter(|&r| self.can_ring_reverse(state, defender, r, played)))
                    .filter(|&r| !chain || me.hand.iter().filter(|&&c| c != r).count() >= 2)
                    .collect();
                let pdef = self.def(state, played.card);
                let not_heat = me.ringside.iter().filter(|&&c| !self.def(state, c).has_keyword("Heat")).count();
                let volley = pdef.volley().is_some_and(|n| n == 0 || not_heat >= n as usize)
                    && !pdef.has_trait(Trait::Irreversible)
                    && !played.bonus.irreversible;
                if !options.is_empty() || volley {
                    state.decision = Some(Decision {
                        player: defender,
                        kind: DecisionKind::Reversal { card: played.card, options, volley },
                    });
                }
            }
            Step::ResolveEffects => {
                let played = state.in_play.expect("card in play");
                events.push(Event::EffectsResolved { card: played.card });
                let def = self.def(state, played.card);
                let mut effects = match (&def.script.as_action, played.kind) {
                    (Some(action), CardKind::Action | CardKind::Antic) => action.on_play.clone(),
                    _ => def.script.on_play.clone(),
                };
                if matches!(played.kind, CardKind::Maneuver(_)) && self.maneuvers_blanked(state, played.player) {
                    effects.clear();
                }
                for ability in self.abilities(state, played.player) {
                    match ability {
                        Ability::OnPlay { title, effects: extra } if title.eq_ignore_ascii_case(&def.title) => {
                            effects.extend(extra.iter().cloned())
                        }
                        Ability::OnPlayMatching { filter, effects: extra } if self.matches(state, filter, played) => {
                            effects.extend(extra.iter().cloned())
                        }
                        _ => {}
                    }
                }
                let mut triggers = Vec::new();
                let opponent = played.player.opponent();
                let mut theirs = Vec::new();
                for ability in self.abilities(state, opponent) {
                    if let Ability::OnOpponentPlayMatching { filter, effects: extra } = ability
                        && self.matches(state, filter, played)
                    {
                        theirs.extend(extra.iter().cloned());
                    }
                }
                schedule_effects(state, &theirs, opponent, Source::Superstar(opponent));
                for owner in [played.player, played.player.opponent()] {
                    for c in self.ring_cards(state, owner) {
                        for t in &self.def(state, c).script.in_ring_on_play {
                            let whose_ok = match t.whose {
                                None => true,
                                Some(Who::You) => owner == played.player,
                                Some(Who::Opponent) => owner != played.player,
                            };
                            if whose_ok && self.matches(state, &t.filter, played) {
                                triggers.push((c, t.effects.clone()));
                            }
                        }
                    }
                }
                for (c, extra) in triggers.into_iter().rev() {
                    schedule_effects(state, &extra, played.player, Source::Card(c));
                }
                schedule_effects(state, &effects, played.player, Source::Card(played.card));
            }
            Step::DealCardDamage => {
                let played = state.in_play.expect("card in play");
                if matches!(played.kind, CardKind::Maneuver(_)) {
                    for c in state.player(played.player).ring.clone() {
                        if self.def(state, c).script.lapses_after_own_maneuver && !state.lapsed.contains(&c) {
                            state.lapsed.push(c);
                        }
                    }
                }
                let defender = played.player.opponent();
                let reduction: u16 = self
                    .abilities(state, defender)
                    .iter()
                    .map(|a| if let Ability::ReduceDamage(n) = a { *n as u16 } else { 0 })
                    .sum();
                let cap = self
                    .abilities(state, defender)
                    .iter()
                    .filter_map(|a| match a {
                        Ability::OpponentDamageCap { filter, max } if self.matches(state, filter, played) => {
                            Some(*max as u16)
                        }
                        _ => None,
                    })
                    .min()
                    .unwrap_or(u16::MAX);
                let damage = self.card_damage(state, played).min(cap).saturating_sub(reduction);
                if damage > 0 {
                    state.pending.push(Step::Overturn {
                        player: played.player.opponent(),
                        remaining: damage,
                        source: Some(played.card),
                    });
                }
            }
            Step::AfterDamage => {
                let played = state.in_play.expect("card in play");
                let mut effects = self.def(state, played.card).script.after_damage.clone();
                if matches!(played.kind, CardKind::Maneuver(_)) && self.maneuvers_blanked(state, played.player) {
                    effects.clear();
                }
                if let Some(watch) = state.in_play_watch.take() {
                    effects.extend(watch.on_success);
                }
                schedule_effects(state, &effects, played.player, Source::Card(played.card));
            }
            Step::Effect { effect, controller, source } => self.effect(state, effect, controller, source, events),
            Step::Overturn { player, remaining, source } => self.overturn(state, player, remaining, source, events),
            Step::ToRing => {
                let played = state.in_play.take().expect("card in play");
                match state.in_play_to.take() {
                    Some(Zone::Arsenal) if state.in_play_bottom.is_some() => {
                        if state.in_play_bottom.take() == Some(true) {
                            self.move_card_to_bottom(state, played.card, events);
                        } else {
                            self.move_card(state, played.card, Zone::Arsenal, events);
                        }
                    }
                    Some(Zone::Arsenal) => {
                        self.move_card(state, played.card, Zone::Arsenal, events);
                        self.shuffle_arsenal(state, played.player, events);
                    }
                    Some(zone) => self.move_card(state, played.card, zone, events),
                    None => self.move_card(state, played.card, Zone::Ring, events),
                }
                state.last_played = Some(played);
            }
            Step::StunDraw { player, max } => {
                let max = max.min(state.player(player).arsenal.len().min(u8::MAX as usize) as u8);
                if max > 0 {
                    state.decision = Some(Decision { player, kind: DecisionKind::StunDraw { max } });
                }
            }
            Step::EndTurn => {
                if std::mem::take(&mut state.continue_turn) {
                    events.push(Event::TurnContinues { player: state.active });
                    state.pending.push(Step::Main);
                    return;
                }
                let p = state.active;
                let (now, later): (Vec<_>, Vec<_>) =
                    std::mem::take(&mut state.end_of_turn).into_iter().partition(|(c, _)| *c == p);
                state.end_of_turn = later;
                if !now.is_empty() {
                    state.pending.push(Step::EndTurn);
                    for (controller, effect) in now.into_iter().rev() {
                        state.pending.push(Step::Effect { effect, controller, source: Source::Superstar(controller) });
                    }
                    return;
                }
                if state.mode == Mode::Revolution && !std::mem::replace(&mut state.aggression_checked, true) {
                    let tried = state
                        .played_this_turn
                        .iter()
                        .any(|pl| pl.player == p && matches!(pl.kind, CardKind::Maneuver(_)));
                    if !tried {
                        events.push(Event::Aggression { player: p });
                        state.pending.push(Step::EndTurn);
                        let effect = Effect::Damage { who: Who::You, n: 6 };
                        state.pending.push(Step::Effect { effect, controller: p, source: Source::Superstar(p) });
                        return;
                    }
                }
                if state.volley_turn.take() == Some(p) {
                    // Volley: no maneuver played under the restriction lets the
                    // Volley player take a Volley card back.
                    let opponent = p.opponent();
                    let maneuver = state.played_this_turn.iter().any(|pl| matches!(pl.kind, CardKind::Maneuver(_)));
                    let volley = CardFilter::Keyword("Volley".into());
                    if !maneuver
                        && state.player(opponent).ringside.iter().any(|&c| self.def(state, c).has_keyword("Volley"))
                    {
                        state.pending.push(Step::EndTurn);
                        let back = Effect::MoveCards {
                            who: Who::You,
                            from: Zone::Ringside,
                            to: Zone::Hand,
                            n: 1,
                            up_to: false,
                            filter: Some(volley),
                            pick: Pick::Chosen,
                            bottom: false,
                            top: false,
                        };
                        let effect = Effect::Optional(vec![back]);
                        state.pending.push(Step::Effect {
                            effect,
                            controller: opponent,
                            source: Source::Superstar(opponent),
                        });
                        return;
                    }
                }
                state.end_turn_after_play = false;
                state.in_play_to = None;
                state.in_play_bottom = None;
                for (owner, boost) in &mut state.boosts {
                    if let (true, Duration::Turns(n)) = (*owner == p, &mut boost.duration) {
                        *n = n.saturating_sub(1);
                    }
                }
                state.boosts.retain(|(owner, b)| {
                    *owner != p
                        || matches!(
                            b.duration,
                            Duration::NextTurn | Duration::NextTurnFirstCard | Duration::NextTurnCards(_)
                        )
                        || matches!(b.duration, Duration::Turns(n) if n > 0)
                });
                for (who, _, turns) in &mut state.restrictions {
                    if *who == p {
                        *turns = turns.saturating_sub(1);
                    }
                }
                state.restrictions.retain(|(_, _, turns)| *turns > 0);
                state.reversed_last[p.index()] = None;
                state.reversed_from[p.index()] = None;
                state.next_must = None;
                state.watches.retain(|w| w.player != p);
                events.push(Event::TurnEnded { player: p });
                let next = p.opponent();
                if std::mem::take(&mut state.skip_turn[next.index()]) {
                    events.push(Event::TurnSkipped { player: next });
                } else {
                    state.active = next;
                }
                schedule(state, [Step::CountOutCheck, Step::StartTurn]);
            }
            Step::CountOutCheck => {
                let mut empty = [PlayerId::ONE, PlayerId::TWO].map(|p| state.player(p).arsenal.is_empty());
                // Advanced: only the player who just ended a turn can be counted out.
                let ended = state.active.opponent();
                if state.mode.is_advanced() && !empty[ended.index()] {
                    empty = [false; 2];
                }
                match empty {
                    [true, true] => self.finish(state, Outcome::Draw, events),
                    [false, false] => {}
                    _ => {
                        let winner = if empty[0] { PlayerId::TWO } else { PlayerId::ONE };
                        if !state.refused_count_out[winner.index()] {
                            state.decision = Some(Decision {
                                player: winner,
                                kind: DecisionKind::YesNo { purpose: YesNoPurpose::ClaimCountOut },
                            });
                        }
                    }
                }
            }
            Step::ChainSearch => {
                let played = state.in_play.expect("card in play");
                if self.def(state, played.card).has_trait(Trait::Chain) && state.player(played.player).hand.len() >= 2 {
                    state.decision = Some(Decision {
                        player: played.player,
                        kind: DecisionKind::YesNo { purpose: YesNoPurpose::ChainSearch },
                    });
                }
            }
            Step::SearchChain { player } => {
                let options: Vec<CardId> = state
                    .player(player)
                    .arsenal
                    .iter()
                    .copied()
                    .filter(|&c| self.def(state, c).has_trait(Trait::Chain))
                    .collect();
                if options.is_empty() {
                    self.shuffle_arsenal(state, player, events);
                } else {
                    state.decision = Some(Decision {
                        player,
                        kind: DecisionKind::ChooseCards {
                            options,
                            min: 1,
                            max: 1,
                            purpose: ChoicePurpose::SearchToHand,
                        },
                    });
                }
            }
            Step::ReverseFromHand { defender, reversal } => self.reverse_from_hand(state, defender, reversal, events),
            Step::CounterWindow { attacker, reversal } => {
                let options = self.counter_options(state, attacker, reversal);
                if !options.is_empty() {
                    state.decision =
                        Some(Decision { player: attacker, kind: DecisionKind::CounterReversal { reversal, options } });
                }
            }
            Step::Arrange { chooser, owner, left, mut done } => {
                if left.len() <= 1 {
                    done.extend(left);
                    let arsenal = &mut state.player_mut(owner).arsenal;
                    arsenal.retain(|c| !done.contains(c));
                    arsenal.extend(done.iter().rev());
                } else {
                    let options = left.clone();
                    state.pending.push(Step::Arrange { chooser, owner, left, done });
                    state.decision = Some(Decision {
                        player: chooser,
                        kind: DecisionKind::ChooseCards { options, min: 1, max: 1, purpose: ChoicePurpose::Arrange },
                    });
                }
            }
            Step::OverturnUntil { player, target, left, keep } => {
                if keep > 0 && state.player(player).arsenal.len() <= keep as usize {
                    return;
                }
                let Some(&card) = state.player(player).arsenal.last() else {
                    self.finish(state, Outcome::Win { winner: player.opponent(), method: WinMethod::Pinfall }, events);
                    return;
                };
                self.move_card(state, card, Zone::Ringside, &mut Vec::new());
                events.push(Event::Overturned { player, card });
                let found = self.def(state, card).script.reverses.as_ref().is_some_and(|spec| {
                    !spec.from_hand_only
                        && self.def(state, card).is_reversal()
                        && self.matches(state, &spec.targets, target)
                });
                let left = if found { left.saturating_sub(1) } else { left };
                if left > 0 {
                    state.pending.push(Step::OverturnUntil { player, target, left, keep });
                }
            }
            Step::AskMore { decider, options, remaining, up_to, purpose } => {
                if remaining > 0 && !options.is_empty() {
                    let min = if up_to { 0 } else { 1 };
                    let ask_options = options.clone();
                    state.pending.push(Step::AskMore { decider, options, remaining: remaining - 1, up_to, purpose });
                    state.decision = Some(Decision {
                        player: decider,
                        kind: DecisionKind::ChooseCards { options: ask_options, min, max: 1, purpose },
                    });
                }
            }
            Step::RandomBack { player, n } => {
                let mut found: Vec<CardId> = std::mem::take(&mut state.last_chosen)
                    .into_iter()
                    .filter(|&c| state.card(c).zone == Zone::Hand)
                    .collect();
                found.shuffle(&mut state.rng);
                for c in found.into_iter().take(n as usize) {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, player, events);
            }
            Step::DigRest { cards } => {
                for c in cards {
                    if state.card(c).zone == Zone::Arsenal {
                        let owner = state.card(c).owner;
                        let arsenal = &mut state.player_mut(owner).arsenal;
                        arsenal.retain(|&a| a != c);
                        arsenal.insert(0, c);
                    }
                }
            }
            Step::LevelFortitude => {
                let f = [PlayerId::ONE, PlayerId::TWO].map(|p| self.fortitude(state, p));
                if f[0] == f[1] {
                    return;
                }
                let high = if f[0] > f[1] { PlayerId::ONE } else { PlayerId::TWO };
                let options: Vec<CardId> = state
                    .player(high)
                    .ring
                    .iter()
                    .copied()
                    .filter(|&c| {
                        let def = self.def(state, c);
                        def.damage > 0
                            && (def.is_reversal() || def.kinds.iter().any(|k| matches!(k, CardKind::Maneuver(_))))
                    })
                    .collect();
                if !options.is_empty() {
                    state.pending.push(Step::LevelFortitude);
                    state.decision = Some(Decision {
                        player: high,
                        kind: DecisionKind::ChooseCards {
                            options,
                            min: 1,
                            max: 1,
                            purpose: ChoicePurpose::RingToRingside,
                        },
                    });
                }
            }
            Step::PreMatch { step, actor, passes, starter } => {
                if step >= OPEN_STEP && passes >= 2 {
                    state.pending.push(Step::EndPreMatch { starter });
                } else if self.pre_match_options(state, actor, step).is_empty() {
                    state.pending.push(next_pre_match(step, actor, passes, starter, false));
                } else {
                    state.decision =
                        Some(Decision { player: actor, kind: DecisionKind::PreMatch { step, passes, starter } });
                }
            }
            Step::EndPreMatch { starter } => {
                events.push(Event::PreMatchEnded);
                if state.mode != Mode::Revolution {
                    for p in [starter, starter.opponent()] {
                        let n = self.hand_size(state, p);
                        self.draw(state, p, n, events);
                    }
                }
                state.pending.push(Step::DecideFirst { starter });
                // The starter's end-of-Pre-match effects resolve first.
                for p in [starter.opponent(), starter] {
                    for ability in self.abilities(state, p).iter().rev() {
                        if let Ability::EndOfPrematch(effects) = ability {
                            schedule_effects(state, effects, p, Source::Superstar(p));
                        }
                    }
                    for c in state.player(p).ring.clone().into_iter().rev() {
                        let effects = if state.lapsed.contains(&c) {
                            vec![Effect::SacrificeThis]
                        } else {
                            self.def(state, c).script.end_of_prematch.clone()
                        };
                        schedule_effects(state, &effects, p, Source::Card(c));
                    }
                }
            }
            Step::DecideFirst { starter } => {
                let value = |p: PlayerId| self.superstar_value(state, p);
                let first = match value(PlayerId::ONE).cmp(&value(PlayerId::TWO)) {
                    std::cmp::Ordering::Greater => PlayerId::ONE,
                    std::cmp::Ordering::Less => PlayerId::TWO,
                    // A tie goes to the starter.
                    std::cmp::Ordering::Equal => starter,
                };
                state.active = first;
                events.push(Event::GameStarted { first });
                state.pending.push(Step::StartTurn);
            }
        }
    }

    fn overturn(
        &self,
        state: &mut GameState,
        player: PlayerId,
        remaining: u16,
        source: Option<CardId>,
        events: &mut Vec<Event>,
    ) {
        if remaining == 0 {
            return;
        }
        let Some(&card) = state.player(player).arsenal.last() else {
            self.finish(state, Outcome::Win { winner: player.opponent(), method: WinMethod::Pinfall }, events);
            return;
        };
        self.move_card(state, card, Zone::Ringside, &mut Vec::new());
        events.push(Event::Overturned { player, card });

        let stopping = source
            .and_then(|src| state.in_play.filter(|p| p.card == src))
            .filter(|played| !played.bonus.no_arsenal_reversal)
            .filter(|&played| self.can_reverse(state, player, card, played, false));
        if let Some(played) = stopping.filter(|&p| self.needs_more_reversals(state, p)) {
            events.push(Event::PartialReversal { player, reversal: card, card: played.card });
            state.in_play = Some(Played { reversals: played.reversals + 1, ..played });
            if remaining > 1 {
                state.pending.push(Step::Overturn { player, remaining: remaining - 1, source });
            }
        } else if let Some(played) = stopping {
            // The damage stops, the attacker's turn ends, and the reversed card
            // still lands in the Ring. The attacker may draw up to its SV.
            events.push(Event::Reversed { player, reversal: card, reversed: played.card, from_hand: false });
            state.pending.clear();
            state.reversed_last[player.index()] = Some(played);
            state.reversed_from[player.index()] = Some(Zone::Arsenal);
            let max = (self.def(state, played.card).stun_value as i16 + played.bonus.stun_value).max(0) as u8;
            state.pending.push(Step::EndTurn);
            if let Some(watch) = state.in_play_watch.take() {
                schedule_effects(state, &watch.on_reversed, played.player, Source::Card(played.card));
            }
            self.schedule_reversed_triggers(state, played.player);
            if let Some(spec) = self.def(state, card).script.reverses.clone() {
                if spec.reversed_removed {
                    state.in_play_to = Some(Zone::Removed);
                }
                if spec.removed_after {
                    self.move_card(state, card, Zone::Removed, events);
                }
                schedule_effects(state, &spec.effects, player, Source::Card(card));
            }
            schedule(state, [Step::ToRing, Step::StunDraw { player: played.player, max }]);
        } else if remaining > 1 {
            state.pending.push(Step::Overturn { player, remaining: remaining - 1, source });
        }
    }

    /// A Multi card that has not been reversed yet needs another Reversal.
    fn needs_more_reversals(&self, state: &GameState, played: Played) -> bool {
        self.def(state, played.card).has_trait(Trait::Multi) && played.reversals == 0
    }

    fn reverse_from_hand(&self, state: &mut GameState, defender: PlayerId, reversal: CardId, events: &mut Vec<Event>) {
        let played = state.in_play.expect("card in play");
        if self.needs_more_reversals(state, played) {
            // The first Reversal lands in the Ring without effect or damage;
            // the defender gets another chance to reverse.
            events.push(Event::PartialReversal { player: defender, reversal, card: played.card });
            self.move_card(state, reversal, Zone::Ring, events);
            state.in_play = Some(Played { reversals: 1, ..played });
            state.pending.push(Step::ReversalWindow);
            return;
        }
        let from_ring = state.card(reversal).zone == Zone::Ring;
        let real = from_ring || self.can_reverse(state, defender, reversal, played, true);
        state.in_play = None;
        state.reversed_last[defender.index()] = Some(played);
        state.reversed_from[defender.index()] = Some(state.card(reversal).zone);
        let mut on_reversed = self.def(state, played.card).script.on_reversed_from_hand.clone();
        if let Some(watch) = state.in_play_watch.take() {
            on_reversed.extend(watch.on_reversed);
        }
        if !real {
            // Reversing by discarding a maneuver of the same type: no damage.
            events.push(Event::Reversed { player: defender, reversal, reversed: played.card, from_hand: true });
            self.move_card(state, played.card, Zone::Ringside, events);
            self.move_card(state, reversal, Zone::Ringside, events);
            state.pending.clear();
            schedule(state, [Step::EndTurn]);
            schedule_effects(state, &on_reversed, played.player, Source::Card(played.card));
            return;
        }
        if !from_ring && self.def(state, played.card).volley().is_some() {
            state.volley_lock = Some(defender);
        }
        let script = &self.def(state, reversal).script;
        let spec = if from_ring { &script.reverses_from_ring } else { &script.reverses };
        let spec = spec.clone().expect("validated reversal");
        let base = match spec.damage {
            ReversalDamage::Printed => self.def(state, reversal).damage as u16,
            ReversalDamage::None => 0,
            ReversalDamage::Fixed(n) => n as u16,
            ReversalDamage::SameAsReversed => self.card_damage(state, played),
        };
        let damage = (base as i16 + played.bonus.reversal_damage).max(0) as u16;
        events.push(Event::Reversed { player: defender, reversal, reversed: played.card, from_hand: true });
        if spec.reversed_removed {
            self.move_card(state, played.card, Zone::Removed, events);
        } else if spec.reversed_to_arsenal {
            self.move_card(state, played.card, Zone::Arsenal, events);
            self.shuffle_arsenal(state, played.player, events);
        } else {
            self.move_card(state, played.card, Zone::Ringside, events);
        }
        let reversal_to = match (spec.removed_after, from_ring) {
            (true, _) => Zone::Removed,
            (false, true) => Zone::Ringside,
            (false, false) => Zone::Ring,
        };
        self.move_card(state, reversal, reversal_to, events);

        state.pending.clear();
        schedule(state, [Step::EndTurn]);
        self.schedule_reversed_triggers(state, played.player);
        if damage > 0 {
            state.pending.push(Step::Overturn { player: played.player, remaining: damage, source: None });
        }
        schedule_effects(state, &on_reversed, played.player, Source::Card(played.card));
        let effects: Vec<Effect> = spec.effects.iter().chain(&spec.on_reverse).cloned().collect();
        schedule_effects(state, &effects, defender, Source::Card(reversal));
        if spec.discard_cost > 0 {
            let cost = [Effect::Discard { who: Who::You, n: spec.discard_cost, pick: Pick::Chosen }];
            schedule_effects(state, &cost, defender, Source::Card(reversal));
        }
        if !from_ring {
            state.pending.push(Step::CounterWindow { attacker: played.player, reversal });
        }
    }

    /// Cards in `attacker`'s hand that answer the Reversal `reversal` just
    /// played against their card.
    fn counter_options(&self, state: &GameState, attacker: PlayerId, reversal: CardId) -> Vec<CardId> {
        let defender = attacker.opponent();
        if state.card(reversal).zone != Zone::Ring || state.active != attacker {
            return vec![];
        }
        let from_hand = state.reversed_from[defender.index()] == Some(Zone::Hand);
        let as_played = Played::new(reversal, defender, CardKind::Reversal);
        state
            .player(attacker)
            .hand
            .iter()
            .copied()
            .filter(|&c| {
                let script = &self.def(state, c).script;
                let cancels = from_hand
                    && script.answers_reversal.as_ref().is_some_and(|f| self.matches_def(state, f, reversal, attacker));
                let reverses = script.reverses.as_ref().is_some_and(|spec| {
                    spec.reverses_reversals
                        && self.def(state, c).is_reversal()
                        && self.can_reverse_with(state, attacker, c, spec, as_played, true)
                });
                cancels || reverses
            })
            .collect()
    }

    /// `attacker` answers the Reversal against their card with `card`: the
    /// reversal's damage and effects are dropped and the turn goes on.
    fn counter_reverse(
        &self,
        state: &mut GameState,
        attacker: PlayerId,
        card: CardId,
        reversal: CardId,
        events: &mut Vec<Event>,
    ) {
        let defender = attacker.opponent();
        state.pending.clear();
        state.reversed_last[defender.index()] = None;
        state.reversed_from[defender.index()] = None;
        state.continue_turn = false;
        state.end_turn_after_play = false;
        state.volley_lock = None;
        state.pending.push(Step::Main);
        let def = self.def(state, card);
        if def.script.answers_reversal.is_some() {
            let effects = def.script.on_play.clone();
            self.move_card(state, card, Zone::Ring, events);
            events.push(Event::CardPlayed { player: attacker, card, kind: CardKind::Action });
            schedule_effects(state, &effects, attacker, Source::Card(card));
            return;
        }
        let spec = def.script.reverses.clone().expect("validated reversal");
        let as_played = Played::new(reversal, defender, CardKind::Reversal);
        let damage = match spec.damage {
            ReversalDamage::Printed => def.damage as u16,
            ReversalDamage::None => 0,
            ReversalDamage::Fixed(n) => n as u16,
            ReversalDamage::SameAsReversed => self.card_damage(state, as_played),
        };
        events.push(Event::Reversed { player: attacker, reversal: card, reversed: reversal, from_hand: true });
        self.move_card(state, reversal, Zone::Ringside, events);
        self.move_card(state, card, Zone::Ring, events);
        if damage > 0 {
            state.pending.push(Step::Overturn { player: defender, remaining: damage, source: None });
        }
        let effects: Vec<Effect> = spec.effects.iter().chain(&spec.on_reverse).cloned().collect();
        schedule_effects(state, &effects, attacker, Source::Card(card));
        if spec.discard_cost > 0 {
            let cost = [Effect::Discard { who: Who::You, n: spec.discard_cost, pick: Pick::Chosen }];
            schedule_effects(state, &cost, attacker, Source::Card(card));
        }
    }

    fn effect(
        &self,
        state: &mut GameState,
        effect: Effect,
        controller: PlayerId,
        source: Source,
        events: &mut Vec<Event>,
    ) {
        let target = |who: Who| match who {
            Who::You => controller,
            Who::Opponent => controller.opponent(),
        };
        let recoverable = |state: &GameState, p: PlayerId| -> Vec<CardId> {
            state.player(p).ringside.iter().copied().filter(|&c| !self.def(state, c).is_backlash()).collect()
        };
        // Christian: cards played by his opponent cannot make their player draw.
        if matches!(source, Source::Card(_))
            && self.abilities(state, controller.opponent()).contains(&&Ability::BlankOpponentCardDraws)
            && matches!(
                effect,
                Effect::Draw { who: Who::You, .. }
                    | Effect::DrawUpTo { who: Who::You, .. }
                    | Effect::DrawToMatchOpponent
                    | Effect::DrawRingCards { .. }
            )
        {
            return;
        }
        match effect {
            Effect::Draw { who, n } => self.draw(state, target(who), n, events),
            Effect::DrawUpTo { who, n } => state.pending.push(Step::StunDraw { player: target(who), max: n }),
            Effect::Discard { who, n, pick } => {
                let p = target(who);
                let forced = p != controller;
                match pick {
                    Pick::Random => {
                        let k = (n as usize).min(state.player(p).hand.len());
                        for _ in 0..k {
                            let i = state.rng.random_range(0..state.player(p).hand.len());
                            let c = state.player(p).hand[i];
                            if forced {
                                self.forced_discard(state, c, events);
                            } else {
                                self.move_card(state, c, Zone::Ringside, events);
                            }
                        }
                    }
                    Pick::Chosen | Pick::Controller => {
                        let decider = if pick == Pick::Chosen { p } else { controller };
                        let purpose = if forced { ChoicePurpose::ForcedDiscard } else { ChoicePurpose::Discard };
                        let options = state.player(p).hand.clone();
                        ask(state, decider, options, n, false, purpose);
                    }
                }
            }
            Effect::DiscardAll { who, filter } => {
                let p = target(who);
                let cards: Vec<CardId> = state
                    .player(p)
                    .hand
                    .iter()
                    .copied()
                    .filter(|&c| self.matches_def(state, &filter, c, controller))
                    .collect();
                for c in cards {
                    if p != controller {
                        self.forced_discard(state, c, events);
                    } else {
                        self.move_card(state, c, Zone::Ringside, events);
                    }
                }
            }
            Effect::RingsideToHand { who, n, up_to, filter } => {
                let p = target(who);
                state.last_chosen.clear();
                let mut options = recoverable(state, p);
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                ask(state, p, options, n, up_to, ChoicePurpose::RingsideToHand);
            }
            Effect::RingsideToArsenal { who, n, up_to, filter } => {
                let p = target(who);
                let mut options = recoverable(state, p);
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                ask(state, p, options, n, up_to, ChoicePurpose::RingsideToArsenal);
            }
            Effect::HandToArsenal { who, n } => {
                let p = target(who);
                let options = state.player(p).hand.clone();
                ask(state, p, options, n, false, ChoicePurpose::HandToArsenal);
            }
            Effect::HandToArsenalBottom { who, n } => {
                let p = target(who);
                let options = state.player(p).hand.clone();
                ask(state, p, options, n, false, ChoicePurpose::HandToArsenalBottom);
            }
            Effect::RingsideToArsenalBottom { who, n } => {
                let p = target(who);
                let options = recoverable(state, p);
                ask(state, p, options, n, false, ChoicePurpose::RingsideToArsenalBottom);
            }
            Effect::Damage { who, n } => {
                if n > 0 {
                    state.pending.push(Step::Overturn { player: target(who), remaining: n as u16, source: None });
                }
            }
            Effect::Mill { who, n } => {
                let p = target(who);
                for _ in 0..n {
                    let Some(&c) = state.player(p).arsenal.last() else { break };
                    self.move_card(state, c, Zone::Ringside, events);
                }
            }
            Effect::MillChosen { who, n, filter } => {
                let p = target(who);
                let mut options = state.player(p).arsenal.clone();
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                ask(state, controller, options, n, false, ChoicePurpose::ArsenalToRingside);
            }
            Effect::Search { filter, ringside, n, up_to, random_back } => {
                let me = state.player(controller);
                let options: Vec<CardId> = me
                    .arsenal
                    .iter()
                    .chain(if ringside { me.ringside.as_slice() } else { &[] })
                    .copied()
                    .filter(|&c| !self.def(state, c).is_backlash() && self.matches_def(state, &filter, c, controller))
                    .collect();
                state.last_chosen.clear();
                if options.is_empty() {
                    self.shuffle_arsenal(state, controller, events);
                } else {
                    if random_back > 0 {
                        state.pending.push(Step::RandomBack { player: controller, n: random_back });
                    }
                    ask(state, controller, options, n, up_to, ChoicePurpose::SearchToHand);
                }
            }
            Effect::LookAtHand { who } => {
                let p = target(who);
                events.push(Event::HandShown { player: p, viewer: controller, cards: state.player(p).hand.clone() });
            }
            Effect::Arrange { who, n } => {
                let owner = target(who);
                let left: Vec<CardId> = state.player(owner).arsenal.iter().rev().take(n as usize).copied().collect();
                if left.len() > 1 {
                    state.pending.push(Step::Arrange { chooser: controller, owner, left, done: vec![] });
                }
            }
            Effect::Shuffle { who } => self.shuffle_arsenal(state, target(who), events),
            Effect::RemoveFromRing { who, n, filter } => {
                let p = target(who);
                let options: Vec<CardId> = state
                    .player(p)
                    .ring
                    .iter()
                    .copied()
                    .filter(|&c| self.matches_def(state, &filter, c, controller))
                    .filter(|&c| self.leaves_ring(state, c, source, Some(&filter)))
                    .collect();
                ask(state, controller, options, n, false, ChoicePurpose::RingToRingside);
            }
            Effect::LevelFortitude => state.pending.push(Step::LevelFortitude),
            Effect::OverturnUntilReversal { who, n, keep } => {
                if let Some(target_card) = state.last_played.filter(|l| l.player == controller) {
                    state.pending.push(Step::OverturnUntil { player: target(who), target: target_card, left: n, keep });
                }
            }
            Effect::Boost(boost) => state.boosts.push((controller, boost)),
            Effect::EndTurn => state.end_turn_after_play = true,
            Effect::ContinueTurn => state.continue_turn = true,
            Effect::ChooseEnforcer { who } => {
                state.pending.push(Step::PickEnforcer { who: target(who), decider: controller, change: true })
            }
            Effect::SkipNextTurn { who } => state.skip_turn[target(who).index()] = true,
            Effect::AtEndOfTurn(effects) => state.end_of_turn.extend(effects.into_iter().map(|e| (controller, e))),
            Effect::DiscardThis => state.in_play_to = Some(Zone::Ringside),
            Effect::RemoveThis => state.in_play_to = Some(Zone::Removed),
            Effect::ThisToArsenal { bottom } => {
                if let Source::Card(c) = source
                    && state.in_play.is_some_and(|p| p.card == c)
                {
                    state.in_play_to = Some(Zone::Arsenal);
                    state.in_play_bottom = Some(bottom);
                } else if let Source::Card(c) = source
                    && !matches!(state.card(c).zone, Zone::InPlay | Zone::Removed)
                {
                    if bottom {
                        self.move_card_to_bottom(state, c, events);
                    } else {
                        self.move_card(state, c, Zone::Arsenal, events);
                    }
                }
            }
            Effect::ReturnThis { to } => {
                if let Source::Card(c) = source
                    && !matches!(state.card(c).zone, Zone::InPlay | Zone::Removed)
                {
                    let owner = state.card(c).owner;
                    self.move_card(state, c, to, events);
                    if to == Zone::Arsenal {
                        self.shuffle_arsenal(state, owner, events);
                    }
                }
            }
            Effect::MoveCards { who, from, to, n, up_to, filter, pick, bottom, top } => {
                if matches!(to, Zone::InPlay) || matches!(from, Zone::InPlay) {
                    return;
                }
                let p = target(who);
                let purpose = match (to, bottom, top) {
                    (Zone::Arsenal, true, _) => ChoicePurpose::ToArsenalBottom,
                    (Zone::Arsenal, _, true) => ChoicePurpose::ToArsenalTop,
                    _ => ChoicePurpose::MoveTo(to),
                };
                if from == Zone::Arsenal && filter.is_none() {
                    for _ in 0..n {
                        let Some(&c) = state.player(p).arsenal.last() else { break };
                        match purpose {
                            ChoicePurpose::ToArsenalBottom => self.move_card_to_bottom(state, c, events),
                            ChoicePurpose::ToArsenalTop => {}
                            _ => self.move_card(state, c, to, events),
                        }
                    }
                    return;
                }
                let mut options = state.player(p).zone(from).to_vec();
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                if from == Zone::Ring {
                    options.retain(|&c| self.leaves_ring(state, c, source, filter.as_ref()));
                }
                match pick {
                    Pick::Random => {
                        let mut chosen = vec![];
                        for _ in 0..(n as usize).min(options.len()) {
                            let i = state.rng.random_range(0..options.len());
                            chosen.push(options.swap_remove(i));
                        }
                        self.answer_choice(state, p, purpose, chosen, events);
                    }
                    Pick::Chosen | Pick::Controller => {
                        let decider = if pick == Pick::Chosen { p } else { controller };
                        ask(state, decider, options, n, up_to, purpose);
                    }
                }
            }
            Effect::RemoveFromGame { who, from, n, up_to, filter, pick } => {
                let p = target(who);
                if from == Zone::Arsenal && filter.is_none() {
                    for _ in 0..n {
                        let Some(&c) = state.player(p).arsenal.last() else { break };
                        self.move_card(state, c, Zone::Removed, events);
                    }
                    return;
                }
                let mut options = state.player(p).zone(from).to_vec();
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                if from == Zone::Ring {
                    options.retain(|&c| self.leaves_ring(state, c, source, filter.as_ref()));
                }
                match pick {
                    Pick::Random => {
                        for _ in 0..(n as usize).min(options.len()) {
                            let i = state.rng.random_range(0..options.len());
                            let c = options.swap_remove(i);
                            self.move_card(state, c, Zone::Removed, events);
                        }
                    }
                    Pick::Chosen | Pick::Controller => {
                        let decider = if pick == Pick::Chosen { p } else { controller };
                        ask(state, decider, options, n, up_to, ChoicePurpose::RemoveFromGame);
                    }
                }
            }
            Effect::ShuffleThisIntoArsenal => state.in_play_to = Some(Zone::Arsenal),
            Effect::NextTurnAfterThis => {
                if let Source::Card(c) = source {
                    state.start_after[controller.index()] = Some(c);
                }
            }
            Effect::WinGame => {
                self.finish(state, Outcome::Win { winner: controller, method: WinMethod::Disqualification }, events)
            }
            Effect::Optional(effects) => {
                if self.payable(state, controller, &effects) {
                    state.decision = Some(Decision {
                        player: controller,
                        kind: DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { effects, source } },
                    });
                }
            }
            Effect::Choice(options) => {
                let options: Vec<Vec<Effect>> =
                    options.into_iter().filter(|o| self.payable(state, controller, o)).collect();
                match options.len() {
                    0 => {}
                    1 => schedule_effects(state, &options[0], controller, source),
                    _ => {
                        state.decision =
                            Some(Decision { player: controller, kind: DecisionKind::ChooseOption { options, source } })
                    }
                }
            }
            Effect::If { condition, effects } => {
                if self.holds(state, &condition, controller) {
                    schedule_effects(state, &effects, controller, source);
                }
            }
            Effect::RevealHand { who } => {
                let p = target(who);
                events.push(Event::HandShown { player: p, viewer: p.opponent(), cards: state.player(p).hand.clone() });
            }
            Effect::DiscardMatching { who, n, filter, pick } => {
                let p = target(who);
                let forced = p != controller;
                let mut options: Vec<CardId> = state
                    .player(p)
                    .hand
                    .iter()
                    .copied()
                    .filter(|&c| self.matches_def(state, &filter, c, controller))
                    .collect();
                match pick {
                    Pick::Random => {
                        options.shuffle(&mut state.rng);
                        for c in options.into_iter().take(n as usize) {
                            if forced {
                                self.forced_discard(state, c, events);
                            } else {
                                self.move_card(state, c, Zone::Ringside, events);
                            }
                        }
                    }
                    Pick::Chosen | Pick::Controller => {
                        let decider = if pick == Pick::Chosen { p } else { controller };
                        let purpose = if forced { ChoicePurpose::ForcedDiscard } else { ChoicePurpose::Discard };
                        ask(state, decider, options, n, false, purpose);
                    }
                }
            }
            Effect::DiscardPerRingCard { who, filter } => {
                let p = target(who);
                let n = self.count_in_ring_matching(state, p, &filter).min(u8::MAX as usize) as u8;
                if n > 0 {
                    let effect = Effect::Discard { who, n, pick: Pick::Chosen };
                    state.pending.push(Step::Effect { effect, controller, source });
                }
            }
            Effect::DiscardMirror { max } => {
                let options = state.player(controller).hand.clone();
                ask(state, controller, options, max, true, ChoicePurpose::DiscardMirror);
            }
            Effect::HandToArsenalTop { who, n, random } => {
                let p = target(who);
                if random {
                    for _ in 0..n {
                        let len = state.player(p).hand.len();
                        if len == 0 {
                            break;
                        }
                        let i = state.rng.random_range(0..len);
                        let c = state.player(p).hand[i];
                        self.move_card(state, c, Zone::Arsenal, events);
                    }
                } else {
                    let options = state.player(p).hand.clone();
                    ask(state, p, options, n, false, ChoicePurpose::HandToArsenalTop);
                }
            }
            Effect::Counted { count, effect } => {
                let n = self.count(state, controller, count);
                let mut effect = *effect;
                set_amount(&mut effect, n);
                self.effect(state, effect, controller, source, events);
            }
            Effect::IrreversibleThis => {
                if let Some(played) = state.in_play.as_mut() {
                    played.bonus.irreversible = true;
                }
            }
            Effect::MillUntil { who, filter } => {
                let p = target(who);
                while let Some(&c) = state.player(p).arsenal.last() {
                    self.move_card(state, c, Zone::Ringside, events);
                    events.push(Event::Overturned { player: p, card: c });
                    if self.matches_def(state, &filter, c, controller) {
                        break;
                    }
                }
            }
            Effect::ShuffleHandAndRedraw { who } => {
                let p = target(who);
                let hand = state.player(p).hand.clone();
                for &c in &hand {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, p, events);
                self.draw(state, p, hand.len().min(u8::MAX as usize) as u8, events);
            }
            Effect::Dig { n, take } => {
                let top: Vec<CardId> =
                    state.player(controller).arsenal.iter().rev().take(n as usize).copied().collect();
                if !top.is_empty() {
                    state.pending.push(Step::DigRest { cards: top.clone() });
                    ask(state, controller, top, take, false, ChoicePurpose::DigToHand);
                }
            }
            Effect::DamageSuperstarValue { who } => {
                let p = target(who);
                let n = self.superstar_value(state, p) as u16;
                if n > 0 {
                    state.pending.push(Step::Overturn { player: p, remaining: n, source: None });
                }
            }
            Effect::RingToHand { who, n, filter, up_to } => {
                let p = target(who);
                let options: Vec<CardId> = state
                    .player(p)
                    .ring
                    .iter()
                    .copied()
                    .filter(|&c| self.matches_def(state, &filter, c, controller))
                    .filter(|&c| self.leaves_ring(state, c, source, Some(&filter)))
                    .collect();
                ask(state, controller, options, n, up_to, ChoicePurpose::RingToHand);
            }
            Effect::ClearRing { who } => {
                for c in state.player(target(who)).ring.clone() {
                    if self.leaves_ring(state, c, source, None) {
                        self.move_card(state, c, Zone::Ringside, events);
                    }
                }
            }
            Effect::RevealUntil { filter } => {
                let arsenal = state.player(controller).arsenal.clone();
                for &c in arsenal.iter().rev() {
                    events.push(Event::Revealed { player: controller, card: c });
                    if self.matches_def(state, &filter, c, controller) {
                        self.move_card(state, c, Zone::Hand, events);
                        break;
                    }
                }
                self.shuffle_arsenal(state, controller, events);
            }
            Effect::CannotPlay { who, filter, turns } => {
                if turns > 0 {
                    state.restrictions.push((target(who), filter, turns));
                }
            }
            Effect::BoostOpponent(boost) => state.boosts.push((controller.opponent(), boost)),
            Effect::WinByPin => {
                self.finish(state, Outcome::Win { winner: controller, method: WinMethod::Pinfall }, events)
            }
            Effect::DiscardForDamage { per } => {
                let options = state.player(controller).hand.clone();
                let n = options.len().min(u8::MAX as usize) as u8;
                ask(state, controller, options, n, true, ChoicePurpose::DiscardForDamage { per });
            }
            Effect::NextCardMustBe(filter) => state.next_must = Some((controller, NextMust::Filter(filter))),
            Effect::NextCardMustBeChosen => {
                let cards: Vec<CardId> =
                    state.last_chosen.iter().copied().filter(|&c| state.card(c).zone == Zone::Hand).collect();
                if !cards.is_empty() {
                    state.next_must = Some((controller, NextMust::Cards(cards)));
                }
            }
            Effect::RepeatPreviousEffects => {
                if let Some(prev) = state.last_played.filter(|l| l.player == controller) {
                    // Copying another copy of the same text would never end.
                    let effects: Vec<Effect> = self
                        .def(state, prev.card)
                        .script
                        .on_play
                        .iter()
                        .filter(|e| **e != Effect::RepeatPreviousEffects)
                        .cloned()
                        .collect();
                    schedule_effects(state, &effects, controller, Source::Card(prev.card));
                }
            }
            Effect::MillAll { who, filter } => {
                let p = target(who);
                let cards: Vec<CardId> = state
                    .player(p)
                    .arsenal
                    .iter()
                    .copied()
                    .filter(|&c| self.matches_def(state, &filter, c, controller))
                    .collect();
                for c in cards {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                self.shuffle_arsenal(state, p, events);
            }
            Effect::SacrificeThis => {
                if let Source::Card(c) = source
                    && state.card(c).zone == Zone::Ring
                {
                    self.move_card(state, c, Zone::Ringside, events);
                }
            }
            Effect::BlankNextPreMatch { who } => state.blank_prematch[target(who).index()] = true,
            Effect::RingsideToBacklash { who, n } => {
                let p = target(who);
                let options: Vec<CardId> = state
                    .player(p)
                    .ringside
                    .iter()
                    .copied()
                    .filter(|&c| self.def(state, c).has_trait(Trait::MidMatch))
                    .collect();
                ask(state, p, options, n, false, ChoicePurpose::RingsideToBacklash);
            }
            Effect::BacklashToRingside { who, n, random } => {
                let p = target(who);
                if random {
                    for _ in 0..n {
                        let len = state.player(p).backlash.len();
                        if len == 0 {
                            break;
                        }
                        let i = state.rng.random_range(0..len);
                        let c = state.player(p).backlash[i];
                        self.move_card(state, c, Zone::Ringside, events);
                    }
                } else {
                    let options = state.player(p).backlash.clone();
                    ask(state, controller, options, n, false, ChoicePurpose::RingToRingside);
                }
            }
            Effect::OpponentChooses { n, options } => {
                if n == 0 || options.is_empty() {
                    return;
                }
                let lists: Vec<Vec<Effect>> = (0..options.len())
                    .map(|i| {
                        let mut rest = options.clone();
                        let mut picked = rest.remove(i);
                        picked.push(Effect::OpponentChooses { n: n - 1, options: rest });
                        vec![Effect::AsOpponent(picked)]
                    })
                    .collect();
                state.decision = Some(Decision {
                    player: controller.opponent(),
                    kind: DecisionKind::ChooseOption { options: lists, source },
                });
            }
            Effect::AsOpponent(effects) => schedule_effects(state, &effects, controller.opponent(), source),
            Effect::Mulligan { n } => {
                let options = state.player(controller).hand.clone();
                ask(state, controller, options, n, true, ChoicePurpose::Mulligan);
            }
            Effect::DrawToMatchOpponent => {
                let gap =
                    state.player(controller.opponent()).hand.len().saturating_sub(state.player(controller).hand.len());
                self.draw(state, controller, gap.min(u8::MAX as usize) as u8, events);
            }
            Effect::RingsideToArsenalCount { who, count } => {
                let n = self.count(state, controller, count);
                if n > 0 {
                    let effect = Effect::RingsideToArsenal { who, n, up_to: true, filter: None };
                    state.pending.push(Step::Effect { effect, controller, source });
                }
            }
            Effect::BuryHand { who, then } => {
                let p = target(who);
                let hand = state.player(p).hand.clone();
                for &c in &hand {
                    self.move_card_to_bottom(state, c, events);
                }
                let k = hand.len().min(u8::MAX as usize) as u8;
                match then {
                    BuryThen::Draw => self.draw(state, p, k, events),
                    BuryThen::SearchHalf if k >= 2 => {
                        let effect = Effect::Search {
                            filter: CardFilter::Any,
                            ringside: false,
                            n: k / 2,
                            up_to: false,
                            random_back: 0,
                        };
                        state.pending.push(Step::Effect { effect, controller: p, source });
                    }
                    BuryThen::SearchHalf => {}
                }
            }
            Effect::DiscardHandAndDraw { min } => {
                let hand = state.player(controller).hand.clone();
                for &c in &hand {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                let n = (hand.len().min(u8::MAX as usize) as u8).max(min);
                self.draw(state, controller, n, events);
            }
            Effect::PutUnder { from, n, up_to, filter } => {
                let Source::Card(host) = source else { return };
                let p = controller;
                if from == Zone::Arsenal && filter.is_none() {
                    let top: Vec<CardId> = state.player(p).arsenal.iter().rev().take(n as usize).copied().collect();
                    self.answer_choice(state, p, ChoicePurpose::PutUnder(host), top, events);
                    return;
                }
                let mut options = state.player(p).zone(from).to_vec();
                options.retain(|&c| c != host);
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                ask(state, p, options, n, up_to, ChoicePurpose::PutUnder(host));
            }
            Effect::TakeFromUnder { to, n, up_to, filter } => {
                let Source::Card(host) = source else { return };
                let mut options = self.under(state, host);
                if let Some(f) = &filter {
                    options.retain(|&c| self.matches_def(state, f, c, controller));
                }
                ask(state, controller, options, n, up_to, ChoicePurpose::MoveTo(to));
            }
            Effect::AllFromUnder { to } => {
                let Source::Card(host) = source else { return };
                let cards = self.under(state, host);
                self.answer_choice(state, controller, ChoicePurpose::MoveTo(to), cards, events);
            }
            Effect::ReversedCardTo { to, bottom } => {
                let Some(reversed) = state.reversed_last[controller.opponent().index()] else { return };
                let c = reversed.card;
                if state.card(c).owner != controller || matches!(state.card(c).zone, Zone::InPlay | Zone::Removed) {
                    return;
                }
                if to == Zone::Arsenal && bottom {
                    self.move_card_to_bottom(state, c, events);
                } else {
                    self.move_card(state, c, to, events);
                    if to == Zone::Arsenal {
                        self.shuffle_arsenal(state, controller, events);
                    }
                }
            }
            Effect::PurgeReversedTitle => {
                let opponent = controller.opponent();
                let Some(reversed) = state.reversed_last[controller.index()] else { return };
                let title = self.def(state, reversed.card).title.clone();
                let me = state.player(opponent);
                let copies: Vec<CardId> = me
                    .hand
                    .iter()
                    .chain(&me.arsenal)
                    .copied()
                    .filter(|&c| self.def(state, c).title.eq_ignore_ascii_case(&title))
                    .collect();
                for c in copies {
                    self.move_card(state, c, Zone::Ringside, events);
                }
                self.shuffle_arsenal(state, opponent, events);
            }
            Effect::ShuffleRingsideIntoArsenal { who } => {
                let p = target(who);
                for c in recoverable(state, p) {
                    self.move_card(state, c, Zone::Arsenal, events);
                }
                self.shuffle_arsenal(state, p, events);
            }
            Effect::NextPlayWatch { filter, on_success, on_reversed } => {
                state.watches.push(Watch { player: controller, filter, on_success, on_reversed })
            }
            Effect::DrawRingCards { less } => {
                let total = state.player(controller).ring.len() + state.player(controller.opponent()).ring.len();
                let n = total.saturating_sub(less as usize).min(u8::MAX as usize) as u8;
                self.draw(state, controller, n, events);
            }
        }
    }

    fn count(&self, state: &GameState, p: PlayerId, count: Count) -> u8 {
        let value = |q: PlayerId| self.superstar_value(state, q);
        let stun = |q: PlayerId| -> usize {
            state.player(q).ring.iter().map(|&c| self.def(state, c).stun_value as usize).sum()
        };
        let n = match count {
            Count::YourFortitude => self.fortitude(state, p) as usize,
            Count::HandSize => state.player(p).hand.len(),
            Count::YourRingStun => stun(p),
            Count::OpponentRingStun => stun(p.opponent()),
            Count::YourRingMatching(f) => self.count_in_ring_matching(state, p, &f),
            Count::OpponentRingMatching(f) => self.count_in_ring_matching(state, p.opponent(), &f),
            Count::TopArsenalDamage => {
                state.player(p).arsenal.last().map_or(0, |&c| self.def(state, c).damage as usize).min(15)
            }
            Count::OpponentHandExcess => {
                state.player(p.opponent()).hand.len().saturating_sub(state.player(p).hand.len())
            }
            Count::InPlayDamage => state.in_play.map_or(0, |pl| self.def(state, pl.card).damage as usize),
            Count::LastDiscardedFortitude => {
                state.last_discarded.iter().map(|&c| self.def(state, c).fortitude as usize).sum()
            }
            Count::HalfYourFortitude => (self.fortitude(state, p) as usize).div_ceil(2),
            Count::OpponentHandSize => state.player(p.opponent()).hand.len(),
            Count::YourRingsideSize => state.player(p).ringside.len(),
            Count::YourRingsideMatching(f) => self.count_matching(state, p, p, Zone::Ringside, &f),
            Count::OpponentRingsideMatching(f) => self.count_matching(state, p.opponent(), p, Zone::Ringside, &f),
            Count::YourHandMatching(f) => self.count_matching(state, p, p, Zone::Hand, &f),
            Count::YourArsenalSize => state.player(p).arsenal.len(),
            Count::OpponentArsenalSize => state.player(p.opponent()).arsenal.len(),
            Count::YourBacklashSize => state.player(p).backlash.len(),
            Count::YourRingSize => state.player(p).ring.len(),
            Count::OpponentRingsideSize => state.player(p.opponent()).ringside.len(),
            Count::YourRemoved => state.player(p).removed.len(),
            Count::OpponentRemoved => state.player(p.opponent()).removed.len(),
            Count::YourRemovedMatching(f) => self.count_matching(state, p, p, Zone::Removed, &f),
            Count::OpponentRemovedMatching(f) => self.count_matching(state, p.opponent(), p, Zone::Removed, &f),
            Count::BothRemoved => state.player(p).removed.len() + state.player(p.opponent()).removed.len(),
            Count::OpponentFortitude => self.fortitude(state, p.opponent()) as usize,
            Count::YourRingAndRingsideMatching(f) => {
                self.count_matching(state, p, p, Zone::Ring, &f) + self.count_matching(state, p, p, Zone::Ringside, &f)
            }
            Count::HalfYourHand => state.player(p).hand.len() / 2,
            Count::HalfOpponentHand => state.player(p.opponent()).hand.len() / 2,
            Count::Minus(a, b) => self.count(state, p, *a).saturating_sub(self.count(state, p, *b)) as usize,
            Count::Half(c) => self.count(state, p, *c) as usize / 2,
            Count::AtMost(c, max) => self.count(state, p, *c).min(max) as usize,
            Count::Plus(a, b) => self.count(state, p, *a) as usize + self.count(state, p, *b) as usize,
            Count::Const(n) => n as usize,
            Count::YourUnderSize => state.player(p).under.len(),
            Count::YourRingTargets => {
                let mut seen: Vec<&str> = vec![];
                for &c in &state.player(p).ring {
                    for t in &self.def(state, c).targets {
                        if !seen.contains(&t.as_str()) {
                            seen.push(t);
                        }
                    }
                }
                seen.len()
            }
            Count::OpponentReversedFortitude => {
                state.reversed_last[p.opponent().index()].map_or(0, |r| self.def(state, r.card).fortitude as usize)
            }
            Count::HandPlus(k) => state.player(p).hand.len() + k as usize,
            Count::OpponentValueTimes(k) => value(p.opponent()) as usize * k as usize,
            Count::HalfOpponentFortitude => (self.fortitude(state, p.opponent()) as usize).div_ceil(2),
            Count::SuperstarValue => value(p) as usize,
        };
        n.min(u8::MAX as usize) as u8
    }

    /// Can `p` pay every "discard N cards" in `effects` in full?
    fn payable(&self, state: &GameState, p: PlayerId, effects: &[Effect]) -> bool {
        let discards: usize = effects
            .iter()
            .map(|e| match e {
                Effect::Discard { who: Who::You, n, pick: Pick::Chosen } => *n as usize,
                _ => 0,
            })
            .sum();
        let matching = effects.iter().all(|e| match e {
            Effect::DiscardMatching { who: Who::You, n, filter, .. } => {
                state.player(p).hand.iter().filter(|&&c| self.matches_def(state, filter, c, p)).count() >= *n as usize
            }
            Effect::RemoveFromGame { who: Who::You, from, n, up_to: false, filter, .. } => {
                let f = filter.clone().unwrap_or(CardFilter::Any);
                self.count_matching(state, p, p, *from, &f) >= *n as usize
            }
            _ => true,
        });
        state.player(p).hand.len() >= discards && matching
    }

    /// Discards `card` because its owner's opponent made them.
    fn forced_discard(&self, state: &mut GameState, card: CardId, events: &mut Vec<Event>) {
        self.move_card(state, card, Zone::Ringside, events);
        let effects = self.def(state, card).script.on_forced_discard.clone();
        schedule_effects(state, &effects, state.card(card).owner, Source::Card(card));
    }

    /// Can `player` play `card` as `kind` in their Main Segment right now?
    pub fn can_play(&self, state: &GameState, player: PlayerId, card: CardId, kind: CardKind) -> bool {
        let def = self.def(state, card);
        let inst = state.card(card);
        let from_backlash = inst.zone == Zone::Backlash;
        let location_ok = inst.owner == player
            && match inst.zone {
                Zone::Hand => true,
                Zone::Backlash => def.has_trait(Trait::MidMatch),
                Zone::Ringside => {
                    self.ringside_by_script(state, player, card) || self.heat_playable(state, player, card)
                }
                Zone::Under => self.playable_from_under(state, player, card),
                _ => false,
            };
        let as_action = def.script.as_action.as_ref().filter(|_| matches!(kind, CardKind::Action | CardKind::Antic));
        let (bonus, _, _) = self.play_bonus(state, player, card, kind);
        let follows_up = match (&def.script.play_after, as_action) {
            (Some(filter), None) => {
                bonus.ignore_play_after
                    || def.script.play_after_unless.as_ref().is_some_and(|c| self.holds(state, c, player))
                    || state.last_played.is_some_and(|last| self.matches(state, filter, last))
            }
            _ => true,
        };
        let after = state.last_played.map(|l| l.card);
        let hypothetical = Played { after, bonus, ..Played::new(card, player, kind) };
        let requires = match as_action {
            Some(a) => a.requires.as_ref(),
            None => def.script.requires.as_ref(),
        };
        let female = self.db.superstar(state.player(player).superstar).female;
        if !(state.active == player
            && state.decision.as_ref().is_none_or(|d| d.kind == DecisionKind::Main)
            && location_ok
            && kind.playable_on_own_turn()
            && def.kinds.contains(&kind)
            && follows_up
            && requires.is_none_or(|c| self.holds(state, c, player))
            && !self.banned_by_opponent(state, player, def)
            && !self.restricted(state, player, hypothetical)
            && !self.abilities(state, player).iter().any(|a| match a {
                Ability::CannotPlayMatching(f) => self.matches(state, f, hypothetical),
                _ => false,
            })
            && state.player(player).hand.iter().filter(|&&c| c != card).count()
                >= self.play_cost(state, player, hypothetical) as usize
            && self.allowed_next(state, player, hypothetical)
            && (state.volley_turn != Some(player)
                || matches!((kind, inst.zone), (CardKind::Maneuver(_), Zone::Hand))
                || (kind == CardKind::Action && def.has_trait(Trait::SetUp)))
            // Women cannot play Multi maneuvers, except their own logo cards (Virtual sets print some).
            && !(female
                && def.has_trait(Trait::Multi)
                && matches!(kind, CardKind::Maneuver(_))
                && !(def.superstar.is_some()
                    && def.logos().any(|l| crate::deck::may_pack_logo(self.db.superstar(state.player(player).superstar), l))))
            && !(from_backlash
                && self.count_in_ring(state, player, |d| d.has_trait(Trait::MidMatch))
                    >= self.ring_limit(state, player, true)))
        {
            return false;
        }
        self.fortitude_payment(state, player, card, kind).is_some()
    }

    /// How many cards `player` must discard (or overturn) for the card's
    /// Fortitude discount to play `card` as `kind`, or `None` if they cannot.
    fn fortitude_payment(&self, state: &GameState, player: PlayerId, card: CardId, kind: CardKind) -> Option<u8> {
        let def = self.def(state, card);
        let as_action = def.script.as_action.as_ref().filter(|_| matches!(kind, CardKind::Action | CardKind::Antic));
        let (bonus, _, _) = self.play_bonus(state, player, card, kind);
        let mut printed = as_action.and_then(|a| a.fortitude).unwrap_or(def.fortitude) as i32;
        if bonus.double_fortitude {
            printed *= 2;
        }
        let mut available = self.fortitude(state, player) as i32;
        if bonus.opponent_fortitude {
            available = available.max(self.fortitude(state, player.opponent()) as i32);
        }
        let needed = printed + bonus.fortitude as i32;
        let mut k = 0;
        if let Some(d) = def.script.fortitude_discount.filter(|d| d.per > 0 && needed > available) {
            k = (needed - available + d.per as i32 - 1) / d.per as i32;
            let after = state.last_played.map(|l| l.card);
            let hypothetical = Played { after, bonus, ..Played::new(card, player, kind) };
            let spare = if d.overturn {
                state.player(player).arsenal.len()
            } else {
                let hand = state.player(player).hand.iter().filter(|&&c| c != card).count();
                hand.saturating_sub(self.play_cost(state, player, hypothetical) as usize)
            };
            if k > d.max as i32 || k as usize > spare {
                return None;
            }
        }
        let discount = k * def.script.fortitude_discount.map_or(0, |d| d.per as i32);
        self.ring_allows(state, player, def, available, needed - discount).then_some(k as u8)
    }

    /// Cards matching a restriction on `player` (from an effect or from a card
    /// in the opponent's Ring) cannot be played.
    fn restricted(&self, state: &GameState, player: PlayerId, played: Played) -> bool {
        let opponent = player.opponent();
        let own = self.ring_cards(state, player).into_iter().any(|c| {
            self.def(state, c)
                .script
                .restricts_self
                .as_ref()
                .is_some_and(|r| self.holds(state, &r.when, player) && self.matches(state, &r.filter, played))
        });
        own || state.restrictions.iter().any(|(p, f, _)| *p == player && self.matches(state, f, played))
            || self.ring_cards(state, opponent).into_iter().any(|c| {
                self.def(state, c)
                    .script
                    .restricts_opponent
                    .as_ref()
                    .is_some_and(|r| self.holds(state, &r.when, opponent) && self.matches(state, &r.filter, played))
            })
    }

    /// Cards `player` must discard to play `played`.
    fn play_cost(&self, state: &GameState, player: PlayerId, played: Played) -> u8 {
        self.abilities(state, player)
            .iter()
            .map(|a| match a {
                Ability::PlayCost { filter, discard } if self.matches(state, filter, played) => *discard,
                _ => 0,
            })
            .sum()
    }

    /// Does `played` satisfy a "your next card must be …" effect on `player`?
    fn allowed_next(&self, state: &GameState, player: PlayerId, played: Played) -> bool {
        match &state.next_must {
            Some((p, NextMust::Filter(f))) if *p == player => self.matches(state, f, played),
            Some((p, NextMust::Cards(cards))) if *p == player => cards.contains(&played.card),
            _ => true,
        }
    }

    /// Cards in `p`'s Ring whose effects still apply.
    /// Ring cards of `attacker` that react to the opponent reversing.
    fn schedule_reversed_triggers(&self, state: &mut GameState, attacker: PlayerId) {
        for c in self.ring_cards(state, attacker).into_iter().rev() {
            let effects = self.def(state, c).script.on_opponent_reverses.clone();
            schedule_effects(state, &effects, attacker, Source::Card(c));
        }
        for c in state.player(attacker).ringside.clone().into_iter().rev() {
            let effects = self.def(state, c).script.ringside_on_opponent_reverses.clone();
            schedule_effects(state, &effects, attacker, Source::Card(c));
        }
        let own: Vec<Effect> = self
            .abilities(state, attacker)
            .iter()
            .filter_map(|a| match a {
                Ability::OnOpponentReverses(effects) => Some(effects.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        schedule_effects(state, &own, attacker, Source::Superstar(attacker));
        let defender = attacker.opponent();
        for c in self.ring_cards(state, defender).into_iter().rev() {
            let effects = self.def(state, c).script.on_you_reverse.clone();
            schedule_effects(state, &effects, defender, Source::Card(c));
        }
        let own: Vec<Effect> = self
            .abilities(state, defender)
            .iter()
            .filter_map(|a| match a {
                Ability::OnYouReverse(effects) => Some(effects.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        schedule_effects(state, &own, defender, Source::Superstar(defender));
    }

    fn ring_cards(&self, state: &GameState, p: PlayerId) -> Vec<CardId> {
        state.player(p).ring.iter().copied().filter(|c| !state.lapsed.contains(c)).collect()
    }

    /// Can `player` use the ability of `card` in their Ring now?
    pub fn can_use_ring_card(&self, state: &GameState, player: PlayerId, card: CardId) -> bool {
        let inst = state.card(card);
        let script = &self.def(state, card).script;
        let ability = match inst.zone {
            Zone::Ring => script.ring_ability.as_ref(),
            Zone::Ringside => script.ringside_ability.as_ref(),
            _ => None,
        };
        let Some(ability) = ability else { return false };
        state.active == player
            && inst.owner == player
            && !state.ring_used.contains(&card)
            && !state.lapsed.contains(&card)
            && !(ability.not_turn_played && state.played_this_turn.iter().any(|p| p.card == card))
            && ability.requires.as_ref().is_none_or(|c| self.holds(state, c, player))
            && self.payable(state, player, &ability.cost)
    }

    /// Can `player` use the ability of the revealed Backstage card `card` now?
    pub fn can_use_backstage_card(&self, state: &GameState, player: PlayerId, card: DefIdx) -> bool {
        let me = state.player(player);
        let Some(ability) = self.db.card(card).script.backstage_ability.as_ref() else { return false };
        state.active == player
            && me.backstage.contains(&card)
            && !me.hidden.contains(&card)
            && !state.backstage_used.contains(&(player, card))
            && ability.requires.as_ref().is_none_or(|c| self.holds(state, c, player))
            && self.payable(state, player, &ability.cost)
    }

    /// `p`'s revealed Backstage cards.
    fn revealed_backstage<'a>(&'a self, state: &'a GameState, p: PlayerId) -> impl Iterator<Item = DefIdx> + 'a {
        let me = state.player(p);
        me.backstage.iter().copied().filter(|c| !me.hidden.contains(c))
    }

    /// Does a card in `viewer`'s Ring make their opponent play with the hand revealed?
    pub fn hand_revealed_to(&self, state: &GameState, viewer: PlayerId) -> bool {
        self.ring_cards(state, viewer).into_iter().any(|c| self.def(state, c).script.reveals_opponent_hand)
    }

    /// Requirements shared by every way of playing a card: Fortitude, Active,
    /// Universally Unique.
    fn ring_allows(&self, state: &GameState, player: PlayerId, def: &CardDef, available: i32, fortitude: i32) -> bool {
        let in_ring = |p: PlayerId| state.player(p).ring.iter().any(|&c| self.def(state, c).title == def.title);
        available >= fortitude
            && !(def.has_trait(Trait::Active) && in_ring(player))
            && !((def.has_trait(Trait::UniversallyUnique) || def.is_title_belt())
                && (in_ring(player) || in_ring(player.opponent())))
    }

    /// The opponent's Superstar forbids `player` to play this card.
    fn banned_by_opponent(&self, state: &GameState, player: PlayerId, def: &CardDef) -> bool {
        self.abilities(state, player.opponent())
            .iter()
            .any(|a| matches!(a, Ability::OpponentCannotPlay(title) if title.eq_ignore_ascii_case(&def.title)))
    }

    /// Bonuses `card` would get if `player` played it as `kind` now, and the
    /// indices of the boosts in [`GameState::boosts`] it would use up.
    fn play_bonus(
        &self,
        state: &GameState,
        player: PlayerId,
        card: CardId,
        kind: CardKind,
    ) -> (PlayBonus, Vec<usize>, bool) {
        let def = self.def(state, card);
        let after = state.last_played.map(|l| l.card);
        let mut played = Played { after, ..Played::new(card, player, kind) };
        let b = &mut played.bonus;
        b.irreversible = def.script.irreversible;
        let pure_action = !def.kinds.iter().any(|k| matches!(k, CardKind::Maneuver(_)));
        if matches!(kind, CardKind::Maneuver(_)) || pure_action {
            for bonus in &def.script.bonuses {
                if !self.holds(state, &bonus.when, player) {
                    continue;
                }
                let mut times = match &bonus.per {
                    Some(f) => self.count_in_ring_matching(state, player, f) as i16,
                    None => 1,
                };
                if bonus.per_hand {
                    times *= state.player(player).hand.iter().filter(|&&c| c != card).count() as i16;
                }
                b.damage += bonus.damage * times;
                b.fortitude += bonus.fortitude * times;
                b.reversal_damage += bonus.reversal_damage * times;
                b.reversal_fortitude += bonus.reversal_fortitude * times;
            }
        }
        b.no_hand_reversal = def.script.no_hand_reversal;
        let earlier = |f: &CardFilter| {
            state.played_this_turn.iter().filter(|e| e.player == player && self.matches(state, f, **e)).count()
        };
        let snapshot = played;
        let mut extra = PlayBonus::default();
        for ability in self.abilities(state, player) {
            match ability {
                Ability::CardFortitude { title, delta } if title.eq_ignore_ascii_case(&def.title) => {
                    extra.fortitude += delta
                }
                Ability::FortitudeFor { filter, delta } if self.matches(state, filter, snapshot) => {
                    extra.fortitude += delta
                }
                Ability::DamageFor { filter, delta } if self.matches(state, filter, snapshot) => extra.damage += delta,
                Ability::EscalatingManeuverDamage if matches!(kind, CardKind::Maneuver(_)) => {
                    extra.damage += 1 + earlier(&CardFilter::Maneuver) as i16
                }
                Ability::FirstOfTurnIrreversible(f) if self.matches(state, f, snapshot) && earlier(f) == 0 => {
                    extra.irreversible = true
                }
                _ => {}
            }
        }
        if def.script.irreversible_when.as_ref().is_some_and(|c| self.holds(state, c, player)) {
            extra.irreversible = true;
        }
        let opponent = player.opponent();
        let mut auras: Vec<&Boost> = Vec::new();
        for c in self.ring_cards(state, player) {
            let script = &self.def(state, c).script;
            auras.extend(script.while_in_ring.iter().chain(&script.while_in_ring_all));
        }
        for c in self.ring_cards(state, opponent) {
            let script = &self.def(state, c).script;
            auras.extend(script.while_in_ring_all.iter().chain(&script.while_in_ring_opponent));
        }
        let b = &mut played.bonus;
        b.fortitude += extra.fortitude;
        b.damage += extra.damage;
        b.irreversible |= extra.irreversible;
        for boost in auras {
            if self.matches(state, &boost.applies_to, played) {
                add_boost(&mut played.bonus, boost);
            }
        }
        let mut used = Vec::new();
        let mut end_turn = false;
        for (i, (owner, boost)) in state.boosts.iter().enumerate() {
            if *owner != player {
                continue;
            }
            let applies = self.matches(state, &boost.applies_to, played);
            match boost.duration {
                Duration::NextTurn | Duration::NextTurnFirstCard | Duration::NextTurnCards(_) => continue,
                Duration::NextCard | Duration::NextCards(_) => used.push(i),
                Duration::NextMatching if applies => used.push(i),
                _ => {}
            }
            if applies {
                add_boost(&mut played.bonus, boost);
                end_turn |= boost.end_turn;
            }
        }
        (played.bonus, used, end_turn)
    }

    /// Is `cond` true for `player`?
    fn holds(&self, state: &GameState, cond: &Condition, player: PlayerId) -> bool {
        match cond {
            Condition::Always => true,
            Condition::Never => false,
            Condition::HasEnforcer(who) => {
                let p = if *who == Who::You { player } else { player.opponent() };
                state.player(p).enforcer.is_some()
            }
            Condition::BackstageRevealed { who, filter } => {
                let p = if *who == Who::You { player } else { player.opponent() };
                self.revealed_backstage(state, p).any(|c| crate::deck::printed_match(self.db.card(c), filter))
            }
            Condition::After(f) => state.last_played.is_some_and(|last| self.matches(state, f, last)),
            Condition::InYourRing(f) => self.count_in_ring_matching(state, player, f) > 0,
            Condition::FortitudeBelowOpponent => {
                self.fortitude(state, player) < self.fortitude(state, player.opponent())
            }
            Condition::HandAtLeast(n) => state.player(player).hand.len() >= *n as usize,
            Condition::OpponentFemale => self.db.superstar(state.player(player.opponent()).superstar).female,
            Condition::Not(c) => !self.holds(state, c, player),
            Condition::AnyOf(cs) => cs.iter().any(|c| self.holds(state, c, player)),
            Condition::All(cs) => cs.iter().all(|c| self.holds(state, c, player)),
            Condition::FortitudeAtMostOpponent => {
                self.fortitude(state, player) <= self.fortitude(state, player.opponent())
            }
            Condition::FortitudeAtLeastOpponent => {
                self.fortitude(state, player) >= self.fortitude(state, player.opponent())
            }
            Condition::Superstar(name) => self.is_superstar(state, player, name),
            Condition::PlayedFromRingside => state.in_play.is_some_and(|pl| pl.from_ringside),
            Condition::CountAtLeast { count, n } => self.count(state, player, count.clone()) >= *n,
            Condition::CountBelow { a, b } => {
                self.count(state, player, a.clone()) < self.count(state, player, b.clone())
            }
            Condition::CountAtMost { count, n } => self.count(state, player, count.clone()) <= *n,
            Condition::SuperstarValueBelowOpponent => {
                self.superstar_value(state, player) < self.superstar_value(state, player.opponent())
            }
            Condition::YouAre(group) => self.db.superstar(state.player(player).superstar).is(group),
            Condition::OpponentIs(group) => self.db.superstar(state.player(player.opponent()).superstar).is(group),
            Condition::OpponentRingHas { filter, n } => {
                self.count_in_ring_matching(state, player.opponent(), filter) >= *n as usize
            }
            Condition::HandHas(f) => state.player(player).hand.iter().any(|&c| self.matches_def(state, f, c, player)),
            Condition::OpponentHandLarger => {
                state.player(player.opponent()).hand.len() > state.player(player).hand.len()
            }
            Condition::OpponentHandAtLeast(n) => state.player(player.opponent()).hand.len() >= *n as usize,
            Condition::FirstCardThisTurn => !state.played_this_turn.iter().any(|p| p.player == player),
            Condition::OpponentReversed(f) => {
                state.reversed_last[player.opponent().index()].is_some_and(|r| self.matches(state, f, r))
            }
            Condition::YouReversedFrom(zones) => {
                state.reversed_from[player.index()].is_some_and(|z| zones.contains(&z))
            }
            Condition::OpponentReversedFrom(zones) => {
                state.reversed_from[player.opponent().index()].is_some_and(|z| zones.contains(&z))
            }
            Condition::ReversedLastTurn(f) => {
                state.reversed_last[player.index()].is_some_and(|r| self.matches(state, f, r))
            }
            Condition::OpponentSuperstar(name) => self.is_superstar(state, player.opponent(), name),
            Condition::SuperstarValueAtMost(n) => self.superstar_value(state, player) <= *n,
            Condition::OpponentSuperstarValueAtMost(n) => self.superstar_value(state, player.opponent()) <= *n,
            Condition::OpponentSuperstarValueAtLeast(n) => self.superstar_value(state, player.opponent()) >= *n,
            Condition::SuperstarValueAtLeast(n) => self.superstar_value(state, player) >= *n,
            Condition::SuperstarValueAtMostOpponent => {
                self.superstar_value(state, player) <= self.superstar_value(state, player.opponent())
            }
            Condition::FortitudeLeadAtLeast(n) => {
                self.fortitude(state, player) >= self.fortitude(state, player.opponent()) + *n as u16
            }
            Condition::FortitudeAtLeast(n) => self.fortitude(state, player) >= *n as u16,
            Condition::RingStunLeadAtLeast(n) => {
                let sv = |p: PlayerId| -> u32 {
                    state.player(p).ring.iter().map(|&c| self.def(state, c).stun_value as u32).sum()
                };
                sv(player) >= sv(player.opponent()) + *n as u32
            }
            Condition::RingStunValueGreater => {
                let sv = |p: PlayerId| -> u32 {
                    state.player(p).ring.iter().map(|&c| self.def(state, c).stun_value as u32).sum()
                };
                sv(player) > sv(player.opponent())
            }
            Condition::LastPlayedThisTurn { filter, n } => {
                let mine: Vec<Played> = state.played_this_turn.iter().copied().filter(|p| p.player == player).collect();
                mine.len() >= *n as usize
                    && mine[mine.len() - *n as usize..].iter().all(|&p| self.matches(state, filter, p))
            }
            Condition::OpponentHandHas(f) => {
                state.player(player.opponent()).hand.iter().any(|&c| self.matches_def(state, f, c, player))
            }
            Condition::FirstTurnOfGame => state.turn == 1,
            Condition::OpponentArsenalAtMost(n) => state.player(player.opponent()).arsenal.len() <= *n as usize,
            Condition::OpponentArsenalAtLeast(n) => state.player(player.opponent()).arsenal.len() >= *n as usize,
            Condition::HandLargerBy(n) => {
                state.player(player).hand.len() >= state.player(player.opponent()).hand.len() + *n as usize
            }
            Condition::UsedRingCardThisTurn(f) => {
                state.ring_used.iter().any(|&c| state.card(c).owner == player && self.matches_def(state, f, c, player))
            }
            Condition::FortitudeAboveOpponent => {
                self.fortitude(state, player) > self.fortitude(state, player.opponent())
            }
        }
    }

    /// Superstar Value, after Ring cards that change it.
    pub fn superstar_value(&self, state: &GameState, p: PlayerId) -> u8 {
        let printed = self.db.superstar(state.player(p).superstar).superstar_value as i16
            + self.enforcer(state, p).map_or(0, |e| e.superstar_value as i16);
        let mine: i16 = self.ring_cards(state, p).iter().map(|&c| self.def(state, c).script.value_bonus as i16).sum();
        let theirs: i16 = self
            .ring_cards(state, p.opponent())
            .iter()
            .map(|&c| self.def(state, c).script.opponent_value_bonus as i16)
            .sum();
        (printed + mine + theirs).clamp(0, u8::MAX as i16) as u8
    }

    /// How many Pre-match (or Mid-match) cards `p` may have in their Ring.
    fn ring_limit(&self, state: &GameState, p: PlayerId, mid_match: bool) -> usize {
        let pick = |d: &CardDef, own: bool| match (mid_match, own) {
            (false, true) => d.script.pre_match_limit,
            (false, false) => d.script.opponent_pre_match_limit,
            (true, true) => d.script.mid_match_limit,
            (true, false) => d.script.opponent_mid_match_limit,
        } as i16;
        let mine: i16 = self.ring_cards(state, p).iter().map(|&c| pick(self.def(state, c), true)).sum();
        let theirs: i16 = self.ring_cards(state, p.opponent()).iter().map(|&c| pick(self.def(state, c), false)).sum();
        let base = match state.mode {
            Mode::Revolution => REVOLUTION_RING_LIMIT,
            Mode::Virtual => VIRTUAL_RING_LIMIT,
            _ => RING_LIMIT,
        };
        (base as i16 + mine + theirs).max(0) as usize
    }

    fn count_matching(
        &self,
        state: &GameState,
        owner: PlayerId,
        viewer: PlayerId,
        zone: Zone,
        f: &CardFilter,
    ) -> usize {
        state.player(owner).zone(zone).iter().filter(|&&c| self.matches_def(state, f, c, viewer)).count()
    }

    fn count_in_ring_matching(&self, state: &GameState, p: PlayerId, f: &CardFilter) -> usize {
        state.player(p).ring.iter().filter(|&&c| self.matches_def(state, f, c, p)).count()
    }

    fn count_in_ring(&self, state: &GameState, p: PlayerId, f: impl Fn(&CardDef) -> bool) -> usize {
        state.player(p).ring.iter().filter(|&&c| f(self.def(state, c))).count()
    }

    /// Pre-match cards `p` may play from their Backlash deck in `step`.
    pub fn pre_match_options(&self, state: &GameState, p: PlayerId, step: u8) -> Vec<(CardId, CardKind)> {
        if self.count_in_ring(state, p, CardDef::is_pre_match) >= self.ring_limit(state, p, false) {
            return vec![];
        }
        let venue_in_play = [PlayerId::ONE, PlayerId::TWO]
            .into_iter()
            .any(|q| self.count_in_ring(state, q, |d| d.kinds.contains(&CardKind::PreMatch(PreMatchType::Venue))) > 0);
        let ppv_in_ring = self.count_in_ring(state, p, |d| d.has_trait(Trait::PayPerView)) > 0;
        let mut out = Vec::new();
        for &card in &state.player(p).backlash {
            let def = self.def(state, card);
            for &kind in &def.kinds {
                let CardKind::PreMatch(t) = kind else { continue };
                let step_ok = match PRE_MATCH_STEPS.get(step as usize) {
                    Some(&wanted) => t == wanted,
                    None => matches!(
                        t,
                        PreMatchType::Event
                            | PreMatchType::Object
                            | PreMatchType::Corner
                            | PreMatchType::Faction
                            | PreMatchType::Storyline
                    ),
                };
                if step_ok
                    && def.script.requires.as_ref().is_none_or(|c| self.holds(state, c, p))
                    && !(t == PreMatchType::Venue && venue_in_play)
                    && !(def.has_trait(Trait::PayPerView) && ppv_in_ring)
                    && self.ring_allows(state, p, def, self.fortitude(state, p) as i32, def.fortitude as i32)
                {
                    out.push((card, kind));
                }
            }
        }
        out
    }

    fn abilities<'a>(&'a self, state: &GameState, p: PlayerId) -> Vec<&'a Ability> {
        // A Storyline in your Ring replaces your Superstar Ability, even a blanked one.
        let storyline = state.player(p).ring.iter().rev().find_map(|&c| {
            let def = self.db.card(state.card(c).def);
            (def.kinds.contains(&CardKind::PreMatch(PreMatchType::Storyline)) && !def.script.storyline.is_empty())
                .then_some(&def.script.storyline)
        });
        let backstage: Vec<&Ability> =
            self.revealed_backstage(state, p).flat_map(|c| &self.db.card(c).script.backstage_abilities).collect();
        if let Some(abilities) = storyline {
            return abilities.iter().chain(backstage).collect();
        }
        let blanked = self.ring_cards(state, p.opponent()).iter().any(|&c| {
            let script = &self.def(state, c).script;
            script.blanks_opponent_ability
                && script.blanks_when.as_ref().is_none_or(|w| self.holds(state, w, p.opponent()))
        });
        if blanked {
            return backstage;
        }
        let enforcer = self.enforcer(state, p).map(|e| e.abilities.as_slice()).unwrap_or_default();
        self.db.superstar(state.player(p).superstar).abilities.iter().chain(enforcer).chain(backstage).collect()
    }

    /// `p`'s chosen Enforcer.
    pub fn enforcer<'a>(&'a self, state: &GameState, p: PlayerId) -> Option<&'a crate::card::EnforcerDef> {
        let (card, index) = state.player(p).enforcer?;
        self.db.card(card).enforcers.get(index as usize)
    }

    /// Enforcers `p` may choose: those on the Enforcer cards in their
    /// Backstage Area their Superstar may reveal (not the current one with `change`).
    fn enforcer_options(&self, state: &GameState, p: PlayerId, change: bool) -> Vec<(DefIdx, u8)> {
        let star = self.db.superstar(state.player(p).superstar);
        let filters: Vec<&CardFilter> = star
            .abilities
            .iter()
            .filter_map(|a| if let Ability::RevealEnforcers(f) = a { Some(f) } else { None })
            .collect();
        let current = state.player(p).enforcer;
        if filters.is_empty() || (change && current.is_none()) {
            return vec![];
        }
        let mut out = vec![];
        for &card in &state.player(p).backstage {
            let def = self.db.card(card);
            if !filters.iter().any(|f| crate::deck::printed_match(def, f)) {
                continue;
            }
            for i in 0..def.enforcers.len() as u8 {
                if !(change && current == Some((card, i))) {
                    out.push((card, i));
                }
            }
        }
        out
    }

    /// Is `p`'s Superstar `name`, or does it count as `name`?
    fn is_superstar(&self, state: &GameState, p: PlayerId, name: &str) -> bool {
        // Counting as another Superstar is read from the printed abilities, even
        // blanked ones: blanking conditions can name Superstars, so reading
        // `abilities` here would recurse.
        let star = self.db.superstar(state.player(p).superstar);
        let backstage = self.revealed_backstage(state, p).flat_map(|c| &self.db.card(c).script.backstage_abilities);
        star.name.eq_ignore_ascii_case(name)
            || star.abilities.iter().chain(backstage).any(|a| match a {
                Ability::CountsAs(names) => names.iter().any(|n| same_name(n, name)),
                _ => false,
            })
    }

    /// Is the text of the maneuvers `p` plays blank?
    fn maneuvers_blanked(&self, state: &GameState, p: PlayerId) -> bool {
        self.ring_cards(state, p.opponent()).iter().any(|&c| self.def(state, c).script.blanks_opponent_maneuvers)
            && self.fortitude(state, p) < self.fortitude(state, p.opponent())
    }

    fn activated_ability<'a>(&'a self, state: &GameState, p: PlayerId) -> Option<(&'a [Effect], &'a [Effect], bool)> {
        self.abilities(state, p).iter().find_map(|a| match a {
            Ability::Activated { cost, effects, requires, once_per_game }
                if requires.as_ref().is_none_or(|c| self.holds(state, c, p))
                    && !(*once_per_game && state.ability_spent[p.index()]) =>
            {
                Some((cost.as_slice(), effects.as_slice(), *once_per_game))
            }
            _ => None,
        })
    }

    /// Can `p` use their Superstar's activated ability now? Its cost must be
    /// payable in full.
    pub fn can_use_ability(&self, state: &GameState, p: PlayerId) -> bool {
        let Some((cost, _, _)) = self.activated_ability(state, p) else { return false };
        let me = state.player(p);
        let (mut from_hand, mut from_ringside) = (0usize, 0usize);
        for c in cost {
            match c {
                Effect::Discard { who: Who::You, n, .. } | Effect::HandToArsenalBottom { who: Who::You, n } => {
                    from_hand += *n as usize
                }
                Effect::RingsideToHand { who: Who::You, n, .. }
                | Effect::RingsideToArsenal { who: Who::You, n, .. }
                | Effect::RingsideToArsenalBottom { who: Who::You, n } => from_ringside += *n as usize,
                _ => {}
            }
        }
        let recoverable = me.ringside.iter().filter(|&&c| !self.def(state, c).is_backlash()).count();
        state.active == p
            && !me.ability_used
            && me.hand.len() >= from_hand
            && recoverable >= from_ringside
            && self.payable(state, p, cost)
    }

    /// Can `player` use `reversal` to stop `played` (from hand or overturned)?
    pub fn can_reverse(
        &self,
        state: &GameState,
        player: PlayerId,
        reversal: CardId,
        played: Played,
        from_hand: bool,
    ) -> bool {
        let rdef = self.def(state, reversal);
        let Some(spec) = &rdef.script.reverses else { return false };
        rdef.is_reversal() && self.can_reverse_with(state, player, reversal, spec, played, from_hand)
    }

    /// Can `player` put `card` from their Ring into the Ringside pile to stop `played`?
    fn can_ring_reverse(&self, state: &GameState, player: PlayerId, card: CardId, played: Played) -> bool {
        let Some(spec) = &self.def(state, card).script.reverses_from_ring else { return false };
        state.card(card).zone == Zone::Ring
            && state.card(card).owner == player
            && self.can_reverse_with(state, player, card, spec, played, true)
    }

    fn can_reverse_with(
        &self,
        state: &GameState,
        player: PlayerId,
        reversal: CardId,
        spec: &ReverseSpec,
        played: Played,
        from_hand: bool,
    ) -> bool {
        let rdef = self.def(state, reversal);
        let pdef = self.def(state, played.card);
        let mut base = if spec.fortitude_is_damage {
            self.card_damage(state, played) as i32
        } else {
            match &spec.fortitude_when {
                Some((cond, f)) if self.holds(state, cond, player) => *f as i32,
                _ => spec.fortitude.unwrap_or(rdef.fortitude) as i32,
            }
        };
        if self.doubled_for(state, player, reversal) {
            base *= 2;
        }
        let needed = base + played.bonus.reversal_fortitude as i32;
        let hand_targets = spec.hand_targets.as_ref().filter(|_| from_hand);
        let targeted =
            self.matches(state, &spec.targets, played) || hand_targets.is_some_and(|f| self.matches(state, f, played));
        let names_title = |f: &CardFilter| names_title(f, &pdef.title);
        let blocked_by_ability = self.abilities(state, played.player).iter().any(|a| {
            matches!(a, Ability::CannotBeReversedBy { played: pf, reversal: rf }
                if self.matches(state, pf, played) && self.matches_def(state, rf, reversal, player))
        });
        let hand_left = state.player(player).hand.iter().filter(|&&c| c != reversal).count();
        let irreversible = pdef.has_trait(Trait::Irreversible) || played.bonus.irreversible;
        let reverses_momentum = |f: &CardFilter| mentions_trait(f, Trait::Momentum);
        played.player != player
            && !(self.has_momentum(state, played)
                && !reverses_momentum(&spec.targets)
                && !hand_targets.is_some_and(reverses_momentum))
            && !(irreversible && !spec.ignores_irreversible)
            && !(from_hand && played.bonus.no_hand_reversal)
            && !((played.bonus.no_backlash_reversal || pdef.script.no_backlash_reversal)
                && state.card(reversal).zone == Zone::Backlash)
            && !(!from_hand && pdef.script.no_arsenal_reversal)
            && pdef.script.not_reversed_by.as_ref().is_none_or(|f| !self.matches_def(state, f, reversal, player))
            && !(from_hand && spec.overturned_only)
            && !(!from_hand && spec.from_hand_only)
            && !(spec.ringside_only && !(from_hand && state.card(reversal).zone == Zone::Ringside))
            && !(from_hand && hand_left < spec.discard_cost as usize)
            && !self.banned_by_opponent(state, player, rdef)
            && !blocked_by_ability
            && spec.requires.as_ref().is_none_or(|c| self.holds(state, c, player))
            && self.fortitude(state, player) as i32 >= needed
            && targeted
            && (!pdef.script.only_reversed_by_title
                || names_title(&spec.targets)
                || hand_targets.is_some_and(names_title))
    }

    /// Momentum cards can only be reversed by cards that name Momentum.
    fn has_momentum(&self, state: &GameState, played: Played) -> bool {
        let def = self.def(state, played.card);
        def.has_trait(Trait::Momentum)
            || played.bonus.momentum
            || def.script.momentum_when.as_ref().is_some_and(|c| self.holds(state, c, played.player))
    }

    /// Chyna-style reversal: discard a maneuver of the same type from hand.
    fn can_discard_reverse(&self, state: &GameState, player: PlayerId, card: CardId, played: Played) -> bool {
        let same_type = match played.kind {
            CardKind::Maneuver(t) => {
                self.abilities(state, player).contains(&&Ability::ReverseByDiscardingSameType)
                    && self.def(state, card).kinds.contains(&CardKind::Maneuver(t))
            }
            _ => false,
        };
        let any = self.abilities(state, player).iter().any(|a| match a {
            Ability::ReverseByDiscardingAny(f) => self.matches(state, f, played),
            _ => false,
        });
        let pdef = self.def(state, played.card);
        state.card(card).zone == Zone::Hand
            && played.player != player
            && !self.has_momentum(state, played)
            && (same_type || any)
            && !pdef.has_trait(Trait::Irreversible)
            && !pdef.script.only_reversed_by_title
            && !played.bonus.irreversible
            && !played.bonus.no_hand_reversal
    }

    /// A boost owned by `player` doubles the printed F of `card`.
    fn doubled_for(&self, state: &GameState, player: PlayerId, card: CardId) -> bool {
        state.boosts.iter().any(|(owner, b)| {
            *owner == player
                && b.double_fortitude
                && !matches!(b.duration, Duration::NextTurn | Duration::NextTurnFirstCard)
                && self.matches_def(state, &b.applies_to, card, player)
        })
    }

    /// Does the card being played match `filter`?
    fn matches(&self, state: &GameState, filter: &CardFilter, played: Played) -> bool {
        let def = self.def(state, played.card);
        match filter {
            CardFilter::SuperstarSpecific => def.superstar.is_some(),
            CardFilter::Holds(c) => self.holds(state, c, played.player),
            CardFilter::Any => true,
            CardFilter::Maneuver => matches!(played.kind, CardKind::Maneuver(_)),
            CardFilter::ManeuverType(t) => played.kind == CardKind::Maneuver(*t),
            CardFilter::Action => played.kind == CardKind::Action,
            CardFilter::Reversal => played.kind == CardKind::Reversal,
            CardFilter::DamageAtMost(n) => self.card_damage(state, played) <= *n as u16,
            CardFilter::DamageAtLeast(n) => self.card_damage(state, played) >= *n as u16,
            CardFilter::FortitudeAtMost(n) => def.fortitude <= *n,
            CardFilter::FortitudeAtLeast(n) => def.fortitude >= *n,
            CardFilter::After(f) => played.after.is_some_and(|a| self.matches_def(state, f, a, played.player)),
            CardFilter::PlayerRingHas { filter, n } => {
                self.count_in_ring_matching(state, played.player, filter) >= *n as usize
            }
            CardFilter::All(fs) => fs.iter().all(|f| self.matches(state, f, played)),
            CardFilter::AnyOf(fs) => fs.iter().any(|f| self.matches(state, f, played)),
            CardFilter::Not(f) => !self.matches(state, f, played),
            CardFilter::EarlierThisTurn { filter, n } => {
                let list = &state.played_this_turn;
                let before = list.iter().position(|e| e.card == played.card).unwrap_or(list.len());
                let count = list[..before]
                    .iter()
                    .filter(|e| e.player == played.player && self.matches(state, filter, **e))
                    .count();
                count >= *n as usize
            }
            CardFilter::Antic => played.kind == CardKind::Antic,
            CardFilter::Trait(Trait::Momentum) => self.has_momentum(state, played),
            CardFilter::Keyword(_)
            | CardFilter::TitleBelt
            | CardFilter::Target(_)
            | CardFilter::HasTarget
            | CardFilter::Hybrid => self.matches_printed(def, filter, || 0),
            CardFilter::PlayedBy(name) => {
                self.db.superstar(state.player(played.player).superstar).name.eq_ignore_ascii_case(name)
            }
            CardFilter::StunValueAtLeast(n) => def.stun_value as i16 + played.bonus.stun_value >= *n as i16,
            CardFilter::Draws => effects_any(played_effects(def, played.kind), &draws),
            CardFilter::ForcesDiscard => effects_any(played_effects(def, played.kind), &forces_discard),
            CardFilter::SameTitleInRing => self.title_in_ring(state, played.player, &def.title),
            CardFilter::Title(_)
            | CardFilter::TitleContains(_)
            | CardFilter::Trait(_)
            | CardFilter::DamageAtMostYourFortitude => {
                self.matches_printed(def, filter, || self.fortitude(state, played.player))
            }
        }
    }

    fn title_in_ring(&self, state: &GameState, p: PlayerId, title: &str) -> bool {
        state.player(p).ring.iter().any(|&c| self.def(state, c).title.eq_ignore_ascii_case(title))
    }

    /// Does the printed card `card` match `filter`? Filters about how a card is
    /// played never match.
    fn matches_def(&self, state: &GameState, filter: &CardFilter, card: CardId, controller: PlayerId) -> bool {
        let def = self.def(state, card);
        let is_maneuver = |k: &CardKind| matches!(k, CardKind::Maneuver(_));
        match filter {
            CardFilter::Any => true,
            CardFilter::Maneuver => def.kinds.iter().any(is_maneuver),
            CardFilter::ManeuverType(t) => def.kinds.contains(&CardKind::Maneuver(*t)),
            CardFilter::Action => def.kinds.contains(&CardKind::Action),
            CardFilter::Reversal => def.is_reversal(),
            CardFilter::DamageAtMost(n) => def.damage <= *n,
            CardFilter::DamageAtLeast(n) => def.damage >= *n,
            CardFilter::FortitudeAtMost(n) => def.fortitude <= *n,
            CardFilter::FortitudeAtLeast(n) => def.fortitude >= *n,
            CardFilter::After(_) | CardFilter::PlayerRingHas { .. } | CardFilter::EarlierThisTurn { .. } => false,
            CardFilter::Antic => def.kinds.contains(&CardKind::Antic),
            CardFilter::Keyword(_)
            | CardFilter::TitleBelt
            | CardFilter::Target(_)
            | CardFilter::HasTarget
            | CardFilter::Hybrid => self.matches_printed(def, filter, || 0),
            CardFilter::PlayedBy(name) => self.is_superstar(state, controller, name),
            CardFilter::StunValueAtLeast(n) => def.stun_value >= *n,
            CardFilter::Draws => {
                effects_any(&def.script.on_play, &draws)
                    || def.script.as_action.as_ref().is_some_and(|a| effects_any(&a.on_play, &draws))
            }
            CardFilter::ForcesDiscard => effects_any(&def.script.on_play, &forces_discard),
            CardFilter::SameTitleInRing => self.title_in_ring(state, controller, &def.title),
            CardFilter::SuperstarSpecific => def.superstar.is_some(),
            CardFilter::Holds(c) => self.holds(state, c, controller),
            CardFilter::All(fs) => fs.iter().all(|f| self.matches_def(state, f, card, controller)),
            CardFilter::AnyOf(fs) => fs.iter().any(|f| self.matches_def(state, f, card, controller)),
            CardFilter::Not(f) => !self.matches_def(state, f, card, controller),
            CardFilter::Title(_)
            | CardFilter::TitleContains(_)
            | CardFilter::Trait(_)
            | CardFilter::DamageAtMostYourFortitude => {
                self.matches_printed(def, filter, || self.fortitude(state, controller))
            }
        }
    }

    fn matches_printed(&self, def: &CardDef, filter: &CardFilter, fortitude: impl Fn() -> u16) -> bool {
        match filter {
            CardFilter::Title(title) => crate::card::same_title(&def.title, title),
            CardFilter::TitleContains(word) => def.title.to_lowercase().contains(&word.to_lowercase()),
            CardFilter::Trait(t) => def.has_trait(*t),
            CardFilter::DamageAtMostYourFortitude => def.damage as u16 <= fortitude(),
            CardFilter::Keyword(word) => def.other_traits.iter().any(|t| crate::card::keyword_matches(t, word)),
            CardFilter::TitleBelt => def.is_title_belt(),
            CardFilter::Target(t) => def.targets.iter().any(|x| x.eq_ignore_ascii_case(t)),
            CardFilter::HasTarget => !def.targets.is_empty(),
            CardFilter::Hybrid => def.kinds.len() > 1,
            _ => unreachable!("only printed filters"),
        }
    }

    fn draw(&self, state: &mut GameState, p: PlayerId, n: u8, events: &mut Vec<Event>) {
        for _ in 0..n {
            let Some(card) = state.player_mut(p).arsenal.pop() else { return };
            state.player_mut(p).hand.push(card);
            state.cards[card.0 as usize].zone = Zone::Hand;
            events.push(Event::CardDrawn { player: p, card: Some(card) });
        }
    }

    fn shuffle_arsenal(&self, state: &mut GameState, p: PlayerId, events: &mut Vec<Event>) {
        let GameState { players, rng, .. } = state;
        players[p.index()].arsenal.shuffle(rng);
        events.push(Event::ArsenalShuffled { player: p });
    }

    /// Moves `card` to the top of `to` in its owner's areas.
    /// The cards under `host`, in the order they were put there.
    /// The card's own text lets `player` play it from the Ringside pile now.
    fn ringside_by_script(&self, state: &GameState, player: PlayerId, card: CardId) -> bool {
        self.def(state, card).script.from_ringside.as_ref().is_some_and(|c| self.holds(state, c, player))
    }

    /// Heat rule: with 6 or more Heat cards in your Ringside pile, once each
    /// turn you may play 1 Heat card from it.
    fn heat_playable(&self, state: &GameState, player: PlayerId, card: CardId) -> bool {
        let heat = |c: CardId| self.def(state, c).has_keyword("Heat");
        !state.heat_used[player.index()]
            && heat(card)
            && state.card(card).zone == Zone::Ringside
            && state.player(player).ringside.iter().filter(|&&c| heat(c)).count() >= 6
    }

    /// Can an effect from `source` take `card` out of its Ring? Permanent
    /// cards (and Stipulations) only leave by their own text, by an effect
    /// that names them, or (Stipulations) by another Stipulation.
    fn leaves_ring(&self, state: &GameState, card: CardId, source: Source, filter: Option<&CardFilter>) -> bool {
        let stipulation = |c: CardId| self.def(state, c).kinds.contains(&CardKind::PreMatch(PreMatchType::Stipulation));
        let def = self.def(state, card);
        !(def.has_keyword("Permanent") || stipulation(card))
            || source == Source::Card(card)
            || filter.is_some_and(|f| names_title(f, &def.title))
            || (stipulation(card) && matches!(source, Source::Card(s) if stipulation(s)))
    }

    /// Volley: the defender reverses the card in play by removing its # random
    /// cards of their Ringside pile from the game; the turn ends.
    fn volley_reverse(&self, state: &mut GameState, defender: PlayerId, events: &mut Vec<Event>) {
        let played = state.in_play.take().expect("card in play");
        let n = self.def(state, played.card).volley().unwrap_or(0);
        if n > 0 {
            let heat: Vec<CardId> = state
                .player(defender)
                .ringside
                .iter()
                .copied()
                .filter(|&c| self.def(state, c).has_keyword("Heat"))
                .collect();
            for c in heat {
                self.move_card(state, c, Zone::Removed, events);
            }
        }
        for _ in 0..n {
            let len = state.player(defender).ringside.len();
            if len == 0 {
                break;
            }
            let i = state.rng.random_range(0..len);
            let c = state.player(defender).ringside[i];
            self.move_card(state, c, Zone::Removed, events);
        }
        events.push(Event::VolleyReversed { player: defender, card: played.card, removed: n });
        state.reversed_last[defender.index()] = Some(played);
        state.reversed_from[defender.index()] = Some(Zone::Removed);
        state.in_play_watch = None;
        self.move_card(state, played.card, Zone::Ringside, events);
        state.pending.clear();
        schedule(state, [Step::EndTurn]);
        self.schedule_reversed_triggers(state, played.player);
    }

    fn under(&self, state: &GameState, host: CardId) -> Vec<CardId> {
        let owner = state.card(host).owner;
        state.player(owner).under.iter().copied().filter(|&c| state.card(c).host == Some(host)).collect()
    }

    /// Can `card`, under one of `player`'s cards, be played as if in hand?
    fn playable_from_under(&self, state: &GameState, player: PlayerId, card: CardId) -> bool {
        let Some(host) = state.card(card).host else { return false };
        state.card(host).zone == Zone::Ring
            && state.card(host).owner == player
            && self
                .def(state, host)
                .script
                .play_from_under
                .as_ref()
                .is_some_and(|f| self.matches_def(state, f, card, player))
    }

    fn move_card(&self, state: &mut GameState, card: CardId, to: Zone, events: &mut Vec<Event>) {
        let CardInstance { owner, zone: from, .. } = *state.card(card);
        // Heat rule: an effect you control (not a Heat card's) moving a card
        // out of your Ringside pile first removes your Heat cards there.
        if from == Zone::Ringside
            && !matches!(to, Zone::Ringside | Zone::InPlay)
            && state.effect_ctx == Some((owner, false))
        {
            state.effect_ctx = None;
            let heat: Vec<CardId> = state
                .player(owner)
                .ringside
                .iter()
                .copied()
                .filter(|&c| c != card && self.def(state, c).has_keyword("Heat"))
                .collect();
            for c in heat {
                self.move_card(state, c, Zone::Removed, events);
            }
            state.effect_ctx = Some((owner, false));
            if self.def(state, card).has_keyword("Heat") {
                // The Heat card removes itself before it can be moved.
                return self.move_card_raw(state, card, Zone::Removed, events);
            }
        }
        self.move_card_raw(state, card, to, events)
    }

    fn move_card_raw(&self, state: &mut GameState, card: CardId, to: Zone, events: &mut Vec<Event>) {
        let CardInstance { owner, zone: from, .. } = *state.card(card);
        // InPlay has no list: it is tracked by `GameState::in_play`.
        if let Some(list) = state.player_mut(owner).zone_mut(from) {
            let i = list.iter().position(|&c| c == card).expect("card is in its zone");
            list.remove(i);
        }
        if let Some(list) = state.player_mut(owner).zone_mut(to) {
            list.push(card);
        }
        state.cards[card.0 as usize].zone = to;
        state.cards[card.0 as usize].host = None;
        if from == Zone::Ring {
            state.lapsed.retain(|&c| c != card);
        }
        // Cards under a card that leaves play go with it (default: Ringside).
        if matches!(from, Zone::Ring | Zone::InPlay) && !matches!(to, Zone::Ring | Zone::InPlay) {
            let under = self.under(state, card);
            if !under.is_empty() {
                let dest = self.def(state, card).script.under_to.unwrap_or(Zone::Ringside);
                for c in under {
                    self.move_card(state, c, dest, events);
                }
                if dest == Zone::Arsenal {
                    self.shuffle_arsenal(state, owner, events);
                }
            }
        }
        if to != Zone::InPlay {
            events.push(Event::CardMoved { player: owner, card: Some(card), from, to });
        }
    }

    /// `card` left its owner's Ring because of an effect `controller` controls.
    fn removed_from_ring(&self, state: &mut GameState, card: CardId, controller: PlayerId) {
        let owner = state.card(card).owner;
        if owner != controller {
            let effects = self.def(state, card).script.on_removed_by_opponent.clone();
            schedule_effects(state, &effects, owner, Source::Card(card));
        }
    }

    /// Moves `card` from its owner's hand or Ringside to the bottom of their Arsenal.
    fn move_card_to_bottom(&self, state: &mut GameState, card: CardId, events: &mut Vec<Event>) {
        self.move_card(state, card, Zone::Arsenal, events);
        let arsenal = &mut state.player_mut(state.card(card).owner).arsenal;
        let top = arsenal.pop().expect("just added");
        arsenal.insert(0, top);
    }

    fn finish(&self, state: &mut GameState, outcome: Outcome, events: &mut Vec<Event>) {
        state.outcome = Some(outcome);
        state.decision = None;
        state.pending.clear();
        events.push(Event::GameOver(outcome));
    }

    pub(crate) fn def<'a>(&'a self, state: &GameState, card: CardId) -> &'a crate::card::CardDef {
        self.db.card(state.card(card).def)
    }
}

/// The Pre-match step after `actor` played (or passed) in `step`. Steps 0-4
/// give each player one turn, starter first; the open step alternates until
/// both players pass in a row.
fn next_pre_match(step: u8, actor: PlayerId, passes: u8, starter: PlayerId, played: bool) -> Step {
    if step < OPEN_STEP {
        if actor == starter {
            Step::PreMatch { step, actor: actor.opponent(), passes: 0, starter }
        } else {
            Step::PreMatch { step: step + 1, actor: starter, passes: 0, starter }
        }
    } else {
        let passes = if played { 0 } else { passes + 1 };
        Step::PreMatch { step, actor: actor.opponent(), passes, starter }
    }
}

/// Pushes steps so that the first one runs next.
fn schedule<const N: usize>(state: &mut GameState, steps: [Step; N]) {
    state.pending.extend(steps.into_iter().rev());
}

fn schedule_effects(state: &mut GameState, effects: &[Effect], controller: PlayerId, source: Source) {
    state.pending.extend(effects.iter().rev().map(|e| Step::Effect { effect: e.clone(), controller, source }));
}

/// Calls `f` with every `k`-element combination of `items`, in order.
fn combinations(items: &[CardId], k: usize, f: &mut impl FnMut(&[CardId])) {
    fn go(items: &[CardId], k: usize, start: usize, acc: &mut Vec<CardId>, f: &mut impl FnMut(&[CardId])) {
        if acc.len() == k {
            f(acc);
            return;
        }
        for i in start..items.len() {
            if items.len() - i < k - acc.len() {
                break;
            }
            acc.push(items[i]);
            go(items, k, i + 1, acc, f);
            acc.pop();
        }
    }
    go(items, k, 0, &mut Vec::with_capacity(k), f);
}

/// Asks `decider` to choose `n` of `options` (fewer if there are not enough;
/// any number up to that if `up_to`). Choices with too many combinations are
/// asked one card at a time.
fn ask(state: &mut GameState, decider: PlayerId, options: Vec<CardId>, n: u8, up_to: bool, purpose: ChoicePurpose) {
    let k = n.min(options.len().min(u8::MAX as usize) as u8);
    if k == 0 {
        return;
    }
    let min = if up_to { 0 } else { k };
    let choices: usize = (min..=k).map(|j| binomial(options.len(), j as usize)).fold(0, usize::saturating_add);
    if k > 1 && choices > 200 {
        state.pending.push(Step::AskMore { decider, options, remaining: k, up_to, purpose });
        return;
    }
    state.decision =
        Some(Decision { player: decider, kind: DecisionKind::ChooseCards { options, min, max: k, purpose } });
}

fn binomial(n: usize, k: usize) -> usize {
    (0..k).fold(1usize, |acc, i| acc.saturating_mul(n - i) / (i + 1))
}

/// Does `f` name the trait `t` anywhere (not negated)?
fn mentions_trait(f: &CardFilter, t: Trait) -> bool {
    match f {
        CardFilter::Trait(x) => *x == t,
        CardFilter::All(fs) | CardFilter::AnyOf(fs) => fs.iter().any(|f| mentions_trait(f, t)),
        _ => false,
    }
}

fn add_boost(bonus: &mut PlayBonus, boost: &Boost) {
    bonus.damage += boost.damage;
    bonus.fortitude += boost.fortitude;
    bonus.reversal_damage += boost.reversal_damage;
    bonus.reversal_fortitude += boost.reversal_fortitude;
    bonus.irreversible |= boost.irreversible;
    bonus.no_arsenal_reversal |= boost.no_arsenal_reversal;
    bonus.no_hand_reversal |= boost.no_hand_reversal;
    bonus.no_backlash_reversal |= boost.no_backlash_reversal;
    bonus.stun_value += boost.stun_value;
    bonus.ignore_play_after |= boost.ignore_play_after;
    bonus.opponent_fortitude |= boost.use_opponent_fortitude;
    bonus.double_fortitude |= boost.double_fortitude;
    bonus.double_damage |= boost.double_damage;
    bonus.momentum |= boost.momentum;
}

/// The on-play effects of `def` when played as `kind`.
fn played_effects(def: &CardDef, kind: CardKind) -> &[Effect] {
    match (&def.script.as_action, kind) {
        (Some(action), CardKind::Action | CardKind::Antic) => &action.on_play,
        _ => &def.script.on_play,
    }
}

/// Does any effect in `effects` (looking inside optional and conditional ones) satisfy `f`?
fn effects_any(effects: &[Effect], f: &dyn Fn(&Effect) -> bool) -> bool {
    effects.iter().any(|e| {
        f(e) || match e {
            Effect::Optional(inner) | Effect::AtEndOfTurn(inner) | Effect::If { effects: inner, .. } => {
                effects_any(inner, f)
            }
            Effect::Choice(options) | Effect::OpponentChooses { options, .. } => {
                options.iter().any(|o| effects_any(o, f))
            }
            _ => false,
        }
    })
}

fn draws(e: &Effect) -> bool {
    matches!(
        e,
        Effect::Draw { who: Who::You, .. }
            | Effect::DrawUpTo { who: Who::You, .. }
            | Effect::DrawToMatchOpponent
            | Effect::DrawRingCards { .. }
            | Effect::ShuffleHandAndRedraw { who: Who::You }
    )
}

fn forces_discard(e: &Effect) -> bool {
    matches!(
        e,
        Effect::Discard { who: Who::Opponent, .. }
            | Effect::DiscardAll { who: Who::Opponent, .. }
            | Effect::DiscardMatching { who: Who::Opponent, .. }
            | Effect::DiscardPerRingCard { who: Who::Opponent, .. }
            | Effect::DiscardMirror { .. }
    )
}

/// Does `filter` mention the card titled `title` by name?
fn names_title(filter: &CardFilter, title: &str) -> bool {
    match filter {
        CardFilter::Title(t) => crate::card::same_title(t, title),
        CardFilter::All(fs) | CardFilter::AnyOf(fs) => fs.iter().any(|f| names_title(f, title)),
        _ => false,
    }
}

/// Names match ignoring case and punctuation ("Deadman Inc." is "Deadman Inc").
fn same_name(a: &str, b: &str) -> bool {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    key(a) == key(b)
}

/// Sets the amount of `effect` to `n`; a boost has each of its numbers multiplied by `n`.
fn set_amount(effect: &mut Effect, n: u8) {
    match effect {
        Effect::Draw { n: x, .. }
        | Effect::DrawUpTo { n: x, .. }
        | Effect::Discard { n: x, .. }
        | Effect::RingsideToHand { n: x, .. }
        | Effect::RingsideToArsenal { n: x, .. }
        | Effect::RingsideToArsenalBottom { n: x, .. }
        | Effect::HandToArsenal { n: x, .. }
        | Effect::Damage { n: x, .. }
        | Effect::Mill { n: x, .. }
        | Effect::DiscardMatching { n: x, .. }
        | Effect::RemoveFromRing { n: x, .. }
        | Effect::RemoveFromGame { n: x, .. }
        | Effect::MoveCards { n: x, .. } => *x = n,
        Effect::Boost(b) | Effect::BoostOpponent(b) => {
            let k = n as i16;
            b.damage *= k;
            b.fortitude *= k;
            b.reversal_damage *= k;
            b.reversal_fortitude *= k;
        }
        Effect::Optional(inner) => inner.iter_mut().for_each(|e| set_amount(e, n)),
        _ => {}
    }
}
