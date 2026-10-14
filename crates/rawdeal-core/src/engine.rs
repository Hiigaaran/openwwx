//! The rules engine: a resumable step machine that pauses whenever a player
//! has to decide something.

use std::sync::Arc;

use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::card::{
    Ability, Boost, CardDef, CardFilter, CardKind, Condition, Duration, Effect, Pick, PreMatchType, ReversalDamage,
    Trait, Who,
};
use crate::db::CardDb;
use crate::decision::{Action, ChoicePurpose, Decision, DecisionKind, YesNoPurpose};
use crate::deck::{DeckList, DeckProblem, validate_deck};
use crate::event::Event;
use crate::state::{
    CardId, CardInstance, GameState, Mode, Outcome, PlayBonus, Played, PlayerId, PlayerState, Source, WinMethod, Zone,
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
    StartTurn,
    DrawSegment,
    Main,
    ReversalWindow,
    ResolveEffects,
    DealCardDamage,
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
}

/// Pre-match card types for steps 0-3.
const PRE_MATCH_STEPS: [PreMatchType; 4] =
    [PreMatchType::Venue, PreMatchType::Feud, PreMatchType::Stipulation, PreMatchType::Manager];
const OPEN_STEP: u8 = 4;
const RING_LIMIT: usize = 5;

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
                let problems = validate_deck(&self.db, deck);
                if !problems.is_empty() {
                    return Err(SetupError::InvalidDeck { player, problems });
                }
            }
            let mut add = |ids: &[String], zone: Zone| -> Result<Vec<CardId>, SetupError> {
                ids.iter()
                    .map(|id| {
                        let def = self.db.card_by_id(id).ok_or_else(|| SetupError::UnknownCard(id.clone()))?;
                        cards.push(CardInstance { def, owner: player, zone });
                        Ok(CardId(cards.len() as u16 - 1))
                    })
                    .collect()
            };
            let mut arsenal = add(&deck.arsenal, Zone::Arsenal)?;
            let backlash = match setup.mode {
                Mode::Advanced => add(&deck.backlash, Zone::Backlash)?,
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
                ability_used: false,
            });
        }
        let players: [PlayerState; 2] = players.try_into().expect("two decks");

        let values = players.each_ref().map(|p| self.db.superstar(p.superstar).superstar_value);
        let first = match values[0].cmp(&values[1]) {
            std::cmp::Ordering::Greater => PlayerId::ONE,
            std::cmp::Ordering::Less => PlayerId::TWO,
            std::cmp::Ordering::Equal => PlayerId(rng.random_range(0..2u8)),
        };

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
            in_play_to: None,
            pending: vec![],
            rng,
        };
        let mut events = Vec::new();
        if setup.shuffle_arsenals {
            events.extend([PlayerId::ONE, PlayerId::TWO].map(|player| Event::ArsenalShuffled { player }));
        }
        match setup.mode {
            Mode::QuickStart => {
                events.push(Event::GameStarted { first });
                for p in [first, first.opponent()] {
                    let n = self.db.superstar(state.player(p).superstar).hand_size;
                    self.draw(&mut state, p, n, &mut events);
                }
                state.pending.push(Step::StartTurn);
            }
            Mode::Advanced => {
                events.push(Event::PreMatchStarted { first });
                state.pending.push(Step::PreMatch { step: 0, actor: first, passes: 0, starter: first });
            }
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
        (def.damage as i16 + played.bonus.damage).max(0) as u16
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
                actions.push(Action::EndTurn);
                actions
            }
            DecisionKind::Reversal { options, .. } => {
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
                let (cost, effects) = self.activated_ability(state, p).expect("validated");
                let all: Vec<Effect> = cost.iter().chain(effects).cloned().collect();
                state.player_mut(p).ability_used = true;
                state.pending.push(Step::Main);
                schedule_effects(state, &all, p, Source::Superstar(p));
            }
            (DecisionKind::Main, Action::PlayCard { card, kind }) => {
                let (bonus, used) = self.play_bonus(state, p, card, kind);
                for i in used.into_iter().rev() {
                    state.boosts.remove(i);
                }
                self.move_card(state, card, Zone::InPlay, events);
                let after = state.last_played.map(|l| l.card);
                state.in_play = Some(Played { after, bonus, ..Played::new(card, p, kind) });
                state.in_play_to = None;
                events.push(Event::CardPlayed { player: p, card, kind });
                schedule(
                    state,
                    [
                        Step::ReversalWindow,
                        Step::ResolveEffects,
                        Step::DealCardDamage,
                        Step::ChainSearch,
                        Step::ToRing,
                        Step::Main,
                    ],
                );
            }
            (DecisionKind::Reversal { card: attacker, .. }, Action::Reverse { card }) => {
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
                    let effects = self.def(state, card).script.on_play.clone();
                    schedule_effects(state, &effects, p, Source::Card(card));
                }
            }
            (DecisionKind::Reversal { .. }, _) => {}
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
                for c in chosen {
                    self.move_card(state, c, Zone::Ringside, events);
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
                }
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
                }
                self.shuffle_arsenal(state, p, events);
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
        match step {
            Step::StartTurn => {
                state.turn += 1;
                let p = state.active;
                events.push(Event::TurnStarted { player: p, turn: state.turn });
                state.player_mut(p).ability_used = false;
                state.last_played = state.start_after[p.index()].take().map(|c| Played::new(c, p, CardKind::Action));
                for (owner, boost) in &mut state.boosts {
                    if *owner == p {
                        boost.duration = match boost.duration {
                            Duration::NextTurn => Duration::ThisTurn,
                            Duration::NextTurnFirstCard => Duration::NextCard,
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
                for c in state.player(p).ring.clone().into_iter().rev() {
                    let effects = self.def(state, c).script.in_ring_start_of_turn.clone();
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
                self.draw(state, p, n, events);
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
                let options: Vec<CardId> = me
                    .hand
                    .iter()
                    .chain(me.backlash.iter().filter(|&&c| self.def(state, c).has_trait(Trait::MidMatch)))
                    .copied()
                    .filter(|&r| self.can_reverse(state, defender, r, played))
                    .filter(|&r| !chain || me.hand.iter().filter(|&&c| c != r).count() >= 2)
                    .collect();
                if !options.is_empty() {
                    state.decision = Some(Decision {
                        player: defender,
                        kind: DecisionKind::Reversal { card: played.card, options },
                    });
                }
            }
            Step::ResolveEffects => {
                let played = state.in_play.expect("card in play");
                events.push(Event::EffectsResolved { card: played.card });
                let def = self.def(state, played.card);
                let mut effects = match (&def.script.as_action, played.kind) {
                    (Some(action), CardKind::Action) => action.on_play.clone(),
                    _ => def.script.on_play.clone(),
                };
                for ability in self.abilities(state, played.player) {
                    if let Ability::OnPlay { title, effects: extra } = ability
                        && title.eq_ignore_ascii_case(&def.title)
                    {
                        effects.extend(extra.iter().cloned());
                    }
                }
                schedule_effects(state, &effects, played.player, Source::Card(played.card));
            }
            Step::DealCardDamage => {
                let played = state.in_play.expect("card in play");
                let defender = played.player.opponent();
                let reduction: u16 = self
                    .abilities(state, defender)
                    .iter()
                    .map(|a| if let Ability::ReduceDamage(n) = a { *n as u16 } else { 0 })
                    .sum();
                let damage = self.card_damage(state, played).saturating_sub(reduction);
                if damage > 0 {
                    state.pending.push(Step::Overturn {
                        player: played.player.opponent(),
                        remaining: damage,
                        source: Some(played.card),
                    });
                }
            }
            Step::Effect { effect, controller, source } => self.effect(state, effect, controller, source, events),
            Step::Overturn { player, remaining, source } => self.overturn(state, player, remaining, source, events),
            Step::ToRing => {
                let played = state.in_play.take().expect("card in play");
                match state.in_play_to.take() {
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
                state.end_turn_after_play = false;
                state.in_play_to = None;
                state.boosts.retain(|(owner, b)| {
                    *owner != p || matches!(b.duration, Duration::NextTurn | Duration::NextTurnFirstCard)
                });
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
                if state.mode == Mode::Advanced && !empty[ended.index()] {
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
            Step::OverturnUntil { player, target } => {
                let Some(&card) = state.player(player).arsenal.last() else {
                    self.finish(state, Outcome::Win { winner: player.opponent(), method: WinMethod::Pinfall }, events);
                    return;
                };
                self.move_card(state, card, Zone::Ringside, &mut Vec::new());
                events.push(Event::Overturned { player, card });
                let found = self.def(state, card).script.reverses.as_ref().is_some_and(|spec| {
                    self.def(state, card).is_reversal() && self.matches(state, &spec.targets, target)
                });
                if !found {
                    state.pending.push(Step::OverturnUntil { player, target });
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
                for p in [starter, starter.opponent()] {
                    let n = self.db.superstar(state.player(p).superstar).hand_size;
                    self.draw(state, p, n, events);
                }
                state.pending.push(Step::DecideFirst { starter });
                // The starter's end-of-Pre-match effects resolve first.
                for p in [starter.opponent(), starter] {
                    for c in state.player(p).ring.clone().into_iter().rev() {
                        let effects = self.def(state, c).script.end_of_prematch.clone();
                        schedule_effects(state, &effects, p, Source::Card(c));
                    }
                }
            }
            Step::DecideFirst { starter } => {
                let value = |p: PlayerId| self.db.superstar(state.player(p).superstar).superstar_value;
                let first = match value(PlayerId::ONE).cmp(&value(PlayerId::TWO)) {
                    std::cmp::Ordering::Greater => PlayerId::ONE,
                    std::cmp::Ordering::Less => PlayerId::TWO,
                    // Values cannot change during Pre-match yet, so a tie goes to the starter.
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
            .filter(|&played| self.can_reverse(state, player, card, played))
            .filter(|_| !self.def(state, card).script.reverses.as_ref().is_some_and(|s| s.from_hand_only));
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
            let max = self.def(state, played.card).stun_value;
            state.pending.push(Step::EndTurn);
            let effects = self.def(state, card).script.reverses.as_ref().map(|s| s.effects.clone()).unwrap_or_default();
            schedule_effects(state, &effects, player, Source::Card(card));
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
        state.in_play = None;
        let spec = self.def(state, reversal).script.reverses.clone().expect("validated reversal");
        let base = match spec.damage {
            ReversalDamage::Printed => self.def(state, reversal).damage as u16,
            ReversalDamage::None => 0,
            ReversalDamage::Fixed(n) => n as u16,
            ReversalDamage::SameAsReversed => self.card_damage(state, played),
        };
        let damage = (base as i16 + played.bonus.reversal_damage).max(0) as u16;
        events.push(Event::Reversed { player: defender, reversal, reversed: played.card, from_hand: true });
        self.move_card(state, played.card, Zone::Ringside, events);
        self.move_card(state, reversal, Zone::Ring, events);

        state.pending.clear();
        schedule(state, [Step::EndTurn]);
        if damage > 0 {
            state.pending.push(Step::Overturn { player: played.player, remaining: damage, source: None });
        }
        let effects: Vec<Effect> = spec.effects.iter().chain(&spec.on_reverse).cloned().collect();
        schedule_effects(state, &effects, defender, Source::Card(reversal));
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
            Effect::RingsideToHand { who, n, up_to } => {
                let p = target(who);
                let options = recoverable(state, p);
                ask(state, p, options, n, up_to, ChoicePurpose::RingsideToHand);
            }
            Effect::RingsideToArsenal { who, n, up_to } => {
                let p = target(who);
                let options = recoverable(state, p);
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
            Effect::MillChosen { who, n } => {
                let p = target(who);
                let options = state.player(p).arsenal.clone();
                ask(state, controller, options, n, false, ChoicePurpose::ArsenalToRingside);
            }
            Effect::Search { filter, ringside } => {
                let me = state.player(controller);
                let options: Vec<CardId> = me
                    .arsenal
                    .iter()
                    .chain(if ringside { me.ringside.as_slice() } else { &[] })
                    .copied()
                    .filter(|&c| !self.def(state, c).is_backlash() && self.matches_def(state, &filter, c, controller))
                    .collect();
                if options.is_empty() {
                    self.shuffle_arsenal(state, controller, events);
                } else {
                    ask(state, controller, options, 1, false, ChoicePurpose::SearchToHand);
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
                    .collect();
                ask(state, controller, options, n, false, ChoicePurpose::RingToRingside);
            }
            Effect::LevelFortitude => state.pending.push(Step::LevelFortitude),
            Effect::OverturnUntilReversal { who } => {
                if let Some(target_card) = state.last_played.filter(|l| l.player == controller) {
                    state.pending.push(Step::OverturnUntil { player: target(who), target: target_card });
                }
            }
            Effect::Boost(boost) => state.boosts.push((controller, boost)),
            Effect::EndTurn => state.end_turn_after_play = true,
            Effect::SkipNextTurn { who } => state.skip_turn[target(who).index()] = true,
            Effect::AtEndOfTurn(effects) => state.end_of_turn.extend(effects.into_iter().map(|e| (controller, e))),
            Effect::DiscardThis => state.in_play_to = Some(Zone::Ringside),
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
        }
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
        state.player(p).hand.len() >= discards
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
                _ => false,
            };
        let as_action = def.script.as_action.as_ref().filter(|_| kind == CardKind::Action);
        let follows_up = match (&def.script.play_after, as_action) {
            (Some(filter), None) => state.last_played.is_some_and(|last| self.matches(state, filter, last)),
            _ => true,
        };
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
            && !(female && def.has_trait(Trait::Multi) && matches!(kind, CardKind::Maneuver(_)))
            && !(from_backlash && self.count_in_ring(state, player, |d| d.has_trait(Trait::MidMatch)) >= RING_LIMIT))
        {
            return false;
        }
        let printed = as_action.and_then(|a| a.fortitude).unwrap_or(def.fortitude);
        let (bonus, _) = self.play_bonus(state, player, card, kind);
        self.ring_allows(state, player, def, printed as i32 + bonus.fortitude as i32)
    }

    /// Requirements shared by every way of playing a card: Fortitude, Active,
    /// Universally Unique.
    fn ring_allows(&self, state: &GameState, player: PlayerId, def: &CardDef, fortitude: i32) -> bool {
        let in_ring = |p: PlayerId| state.player(p).ring.iter().any(|&c| self.def(state, c).title == def.title);
        self.fortitude(state, player) as i32 >= fortitude
            && !(def.has_trait(Trait::Active) && in_ring(player))
            && !(def.has_trait(Trait::UniversallyUnique) && (in_ring(player) || in_ring(player.opponent())))
    }

    /// The opponent's Superstar forbids `player` to play this card.
    fn banned_by_opponent(&self, state: &GameState, player: PlayerId, def: &CardDef) -> bool {
        self.abilities(state, player.opponent())
            .iter()
            .any(|a| matches!(a, Ability::OpponentCannotPlay(title) if title.eq_ignore_ascii_case(&def.title)))
    }

    /// Bonuses `card` would get if `player` played it as `kind` now, and the
    /// indices of the boosts in [`GameState::boosts`] it would use up.
    fn play_bonus(&self, state: &GameState, player: PlayerId, card: CardId, kind: CardKind) -> (PlayBonus, Vec<usize>) {
        let def = self.def(state, card);
        let after = state.last_played.map(|l| l.card);
        let mut played = Played { after, ..Played::new(card, player, kind) };
        let b = &mut played.bonus;
        b.irreversible = def.script.irreversible;
        if matches!(kind, CardKind::Maneuver(_)) {
            for bonus in &def.script.bonuses {
                if !self.holds(state, &bonus.when, player) {
                    continue;
                }
                let times = match &bonus.per {
                    Some(f) => self.count_in_ring_matching(state, player, f) as i16,
                    None => 1,
                };
                b.damage += bonus.damage * times;
                b.fortitude += bonus.fortitude * times;
                b.reversal_damage += bonus.reversal_damage * times;
                b.reversal_fortitude += bonus.reversal_fortitude * times;
            }
        }
        for ability in self.abilities(state, player) {
            if let Ability::CardFortitude { title, delta } = ability
                && title.eq_ignore_ascii_case(&def.title)
            {
                b.fortitude += delta;
            }
        }
        let auras: Vec<&Boost> =
            state.player(player).ring.iter().flat_map(|&c| &self.def(state, c).script.while_in_ring).collect();
        for boost in auras {
            if self.matches(state, &boost.applies_to, played) {
                add_boost(&mut played.bonus, boost);
            }
        }
        let mut used = Vec::new();
        for (i, (owner, boost)) in state.boosts.iter().enumerate() {
            if *owner != player {
                continue;
            }
            let applies = self.matches(state, &boost.applies_to, played);
            match boost.duration {
                Duration::NextTurn | Duration::NextTurnFirstCard => continue,
                Duration::NextCard => used.push(i),
                Duration::NextMatching if applies => used.push(i),
                _ => {}
            }
            if applies {
                add_boost(&mut played.bonus, boost);
            }
        }
        (played.bonus, used)
    }

    /// Is `cond` true for `player`?
    fn holds(&self, state: &GameState, cond: &Condition, player: PlayerId) -> bool {
        match cond {
            Condition::Always => true,
            Condition::Never => false,
            Condition::After(f) => state.last_played.is_some_and(|last| self.matches(state, f, last)),
            Condition::InYourRing(f) => self.count_in_ring_matching(state, player, f) > 0,
            Condition::FortitudeBelowOpponent => {
                self.fortitude(state, player) < self.fortitude(state, player.opponent())
            }
            Condition::HandAtLeast(n) => state.player(player).hand.len() >= *n as usize,
            Condition::OpponentFemale => self.db.superstar(state.player(player.opponent()).superstar).female,
            Condition::Not(c) => !self.holds(state, c, player),
            Condition::AnyOf(cs) => cs.iter().any(|c| self.holds(state, c, player)),
        }
    }

    fn count_in_ring_matching(&self, state: &GameState, p: PlayerId, f: &CardFilter) -> usize {
        state.player(p).ring.iter().filter(|&&c| self.matches_def(state, f, c, p)).count()
    }

    fn count_in_ring(&self, state: &GameState, p: PlayerId, f: impl Fn(&CardDef) -> bool) -> usize {
        state.player(p).ring.iter().filter(|&&c| f(self.def(state, c))).count()
    }

    /// Pre-match cards `p` may play from their Backlash deck in `step`.
    pub fn pre_match_options(&self, state: &GameState, p: PlayerId, step: u8) -> Vec<(CardId, CardKind)> {
        if self.count_in_ring(state, p, CardDef::is_pre_match) >= RING_LIMIT {
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
                    None => matches!(t, PreMatchType::Event | PreMatchType::Object),
                };
                if step_ok
                    && !(t == PreMatchType::Venue && venue_in_play)
                    && !(def.has_trait(Trait::PayPerView) && ppv_in_ring)
                    && self.ring_allows(state, p, def, def.fortitude as i32)
                {
                    out.push((card, kind));
                }
            }
        }
        out
    }

    fn abilities<'a>(&'a self, state: &GameState, p: PlayerId) -> &'a [Ability] {
        &self.db.superstar(state.player(p).superstar).abilities
    }

    fn activated_ability<'a>(&'a self, state: &GameState, p: PlayerId) -> Option<(&'a [Effect], &'a [Effect])> {
        self.abilities(state, p).iter().find_map(|a| match a {
            Ability::Activated { cost, effects } => Some((cost.as_slice(), effects.as_slice())),
            _ => None,
        })
    }

    /// Can `p` use their Superstar's activated ability now? Its cost must be
    /// payable in full.
    pub fn can_use_ability(&self, state: &GameState, p: PlayerId) -> bool {
        let Some((cost, _)) = self.activated_ability(state, p) else { return false };
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
        state.active == p && !me.ability_used && me.hand.len() >= from_hand && recoverable >= from_ringside
    }

    /// Can `player` use `reversal` to stop `played` (from hand or overturned)?
    pub fn can_reverse(&self, state: &GameState, player: PlayerId, reversal: CardId, played: Played) -> bool {
        let rdef = self.def(state, reversal);
        let Some(spec) = &rdef.script.reverses else { return false };
        let needed = spec.fortitude.unwrap_or(rdef.fortitude) as i32 + played.bonus.reversal_fortitude as i32;
        played.player != player
            && rdef.is_reversal()
            && !self.def(state, played.card).has_trait(Trait::Irreversible)
            && !played.bonus.irreversible
            && !self.banned_by_opponent(state, player, rdef)
            && self.fortitude(state, player) as i32 >= needed
            && self.matches(state, &spec.targets, played)
    }

    /// Does the card being played match `filter`?
    fn matches(&self, state: &GameState, filter: &CardFilter, played: Played) -> bool {
        let def = self.def(state, played.card);
        match filter {
            CardFilter::Any => true,
            CardFilter::Maneuver => matches!(played.kind, CardKind::Maneuver(_)),
            CardFilter::ManeuverType(t) => played.kind == CardKind::Maneuver(*t),
            CardFilter::Action => played.kind == CardKind::Action,
            CardFilter::Reversal => played.kind == CardKind::Reversal,
            CardFilter::DamageAtMost(n) => self.card_damage(state, played) <= *n as u16,
            CardFilter::DamageAtLeast(n) => self.card_damage(state, played) >= *n as u16,
            CardFilter::After(f) => played.after.is_some_and(|a| self.matches_def(state, f, a, played.player)),
            CardFilter::PlayerRingHas { filter, n } => {
                self.count_in_ring_matching(state, played.player, filter) >= *n as usize
            }
            CardFilter::All(fs) => fs.iter().all(|f| self.matches(state, f, played)),
            CardFilter::AnyOf(fs) => fs.iter().any(|f| self.matches(state, f, played)),
            CardFilter::Not(f) => !self.matches(state, f, played),
            CardFilter::Title(_)
            | CardFilter::TitleContains(_)
            | CardFilter::Trait(_)
            | CardFilter::DamageAtMostYourFortitude => {
                self.matches_printed(def, filter, || self.fortitude(state, played.player))
            }
        }
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
            CardFilter::After(_) | CardFilter::PlayerRingHas { .. } => false,
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
            CardFilter::Title(title) => def.title.eq_ignore_ascii_case(title),
            CardFilter::TitleContains(word) => def.title.to_lowercase().contains(&word.to_lowercase()),
            CardFilter::Trait(t) => def.has_trait(*t),
            CardFilter::DamageAtMostYourFortitude => def.damage as u16 <= fortitude(),
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
    fn move_card(&self, state: &mut GameState, card: CardId, to: Zone, events: &mut Vec<Event>) {
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
        if to != Zone::InPlay {
            events.push(Event::CardMoved { player: owner, card: Some(card), from, to });
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

/// The Pre-match step after `actor` played (or passed) in `step`. Steps 0-3
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
    if k > 1 && binomial(options.len(), k as usize) > 200 {
        state.pending.push(Step::AskMore { decider, options, remaining: k, up_to, purpose });
        return;
    }
    let min = if up_to { 0 } else { k };
    state.decision =
        Some(Decision { player: decider, kind: DecisionKind::ChooseCards { options, min, max: k, purpose } });
}

fn binomial(n: usize, k: usize) -> usize {
    (0..k).fold(1usize, |acc, i| acc.saturating_mul(n - i) / (i + 1))
}

fn add_boost(bonus: &mut PlayBonus, boost: &Boost) {
    bonus.damage += boost.damage;
    bonus.fortitude += boost.fortitude;
    bonus.reversal_damage += boost.reversal_damage;
    bonus.reversal_fortitude += boost.reversal_fortitude;
    bonus.irreversible |= boost.irreversible;
    bonus.no_arsenal_reversal |= boost.no_arsenal_reversal;
}
