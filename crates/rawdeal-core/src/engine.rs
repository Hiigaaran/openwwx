//! The rules engine: a resumable step machine that pauses whenever a player
//! has to decide something.

use std::sync::Arc;

use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::card::{CardFilter, CardKind, Effect, Pick, ReversalDamage, Trait, Who};
use crate::db::CardDb;
use crate::decision::{Action, ChoicePurpose, Decision, DecisionKind, YesNoPurpose};
use crate::deck::{DeckList, DeckProblem, validate_deck};
use crate::event::Event;
use crate::state::{CardId, CardInstance, GameState, Outcome, Played, PlayerId, PlayerState, WinMethod, Zone};
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
}

impl GameSetup {
    /// A normal match: legal decks, shuffled.
    pub fn standard(decks: [DeckList; 2]) -> Self {
        GameSetup { decks, enforce_deck_rules: true, shuffle_arsenals: true }
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
        source: CardId,
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
}

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

    /// Quick Start setup: reveal Superstars, shuffle, draw Hand Size, highest
    /// Superstar Value goes first (coin flip on a tie).
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
            let mut arsenal = Vec::with_capacity(deck.arsenal.len());
            for id in &deck.arsenal {
                let def = self.db.card_by_id(id).ok_or_else(|| SetupError::UnknownCard(id.clone()))?;
                arsenal.push(CardId(cards.len() as u16));
                cards.push(CardInstance { def, owner: player, zone: Zone::Arsenal });
            }
            if setup.shuffle_arsenals {
                arsenal.shuffle(&mut rng);
            } else {
                arsenal.reverse();
            }
            players.push(PlayerState { superstar, arsenal, hand: vec![], ring: vec![], ringside: vec![] });
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
            outcome: None,
            in_play: None,
            refused_count_out: [false; 2],
            decision: None,
            pending: vec![Step::StartTurn],
            rng,
        };
        let mut events = Vec::new();
        if setup.shuffle_arsenals {
            events.extend([PlayerId::ONE, PlayerId::TWO].map(|player| Event::ArsenalShuffled { player }));
        }
        events.push(Event::GameStarted { first });
        for p in [first, first.opponent()] {
            let n = self.db.superstar(state.player(p).superstar).hand_size;
            self.draw(&mut state, p, n, &mut events);
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

    /// Damage `card` deals if played successfully now.
    pub fn card_damage(&self, state: &GameState, card: CardId) -> u16 {
        self.def(state, card).damage as u16
    }

    /// Every action `player` may take right now.
    pub fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        let Some(decision) = &state.decision else { return vec![] };
        if decision.player != player || state.is_over() {
            return vec![];
        }
        match &decision.kind {
            DecisionKind::Main => {
                let mut actions: Vec<Action> = state
                    .player(player)
                    .hand
                    .iter()
                    .flat_map(|&card| {
                        self.def(state, card)
                            .kinds
                            .iter()
                            .filter(move |&&kind| self.can_play(state, player, card, kind))
                            .map(move |&kind| Action::PlayCard { card, kind })
                    })
                    .collect();
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
            (DecisionKind::Main, Action::PlayCard { card, kind }) => {
                if !state.player(p).hand.contains(card) {
                    return Err(RuleError::Illegal("card is not in your hand"));
                }
                if !self.def(state, *card).kinds.contains(kind) {
                    return Err(RuleError::Illegal("card cannot be played as that kind"));
                }
                if !self.can_play(state, p, *card, *kind) {
                    return Err(RuleError::Illegal("not enough Fortitude or wrong turn"));
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
            _ => false,
        };
        if ok { Ok(()) } else { Err(RuleError::Illegal("action does not answer the pending decision")) }
    }

    fn answer(&self, state: &mut GameState, decision: Decision, action: Action, events: &mut Vec<Event>) {
        let p = decision.player;
        match (decision.kind, action) {
            (DecisionKind::Main, Action::EndTurn) => state.pending.push(Step::EndTurn),
            (DecisionKind::Main, Action::PlayCard { card, kind }) => {
                self.move_card(state, card, Zone::InPlay, events);
                state.in_play = Some(Played { card, player: p, kind });
                events.push(Event::CardPlayed { player: p, card, kind });
                schedule(
                    state,
                    [Step::ReversalWindow, Step::ResolveEffects, Step::DealCardDamage, Step::ToRing, Step::Main],
                );
            }
            (DecisionKind::Reversal { .. }, Action::Reverse { card }) => self.reverse_from_hand(state, p, card, events),
            (DecisionKind::Reversal { .. }, _) => {}
            (DecisionKind::ChooseCards { purpose, .. }, Action::ChooseCards(chosen)) => match purpose {
                ChoicePurpose::Discard => {
                    for c in chosen {
                        self.move_card(state, c, Zone::Ringside, events);
                    }
                }
                ChoicePurpose::RingsideToHand => {
                    for c in chosen {
                        self.move_card(state, c, Zone::Hand, events);
                    }
                }
                ChoicePurpose::RingsideToArsenal => {
                    for c in chosen {
                        self.move_card(state, c, Zone::Arsenal, events);
                    }
                    self.shuffle_arsenal(state, p, events);
                }
            },
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
                }
            }
            (DecisionKind::StunDraw { .. }, Action::DrawCards(n)) => self.draw(state, p, n, events),
            _ => unreachable!("validated above"),
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
                events.push(Event::TurnStarted { player: state.active, turn: state.turn });
                schedule(state, [Step::DrawSegment, Step::Main]);
            }
            Step::DrawSegment => self.draw(state, state.active, 1, events),
            Step::Main => state.decision = Some(Decision { player: state.active, kind: DecisionKind::Main }),
            Step::ReversalWindow => {
                let played = state.in_play.expect("card in play");
                let defender = played.player.opponent();
                let options: Vec<CardId> = state
                    .player(defender)
                    .hand
                    .iter()
                    .copied()
                    .filter(|&r| self.can_reverse(state, defender, r, played))
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
                let effects = self.def(state, played.card).script.on_play.clone();
                schedule_effects(state, &effects, played.player, played.card);
            }
            Step::DealCardDamage => {
                let played = state.in_play.expect("card in play");
                let damage = self.card_damage(state, played.card);
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
                self.move_card(state, played.card, Zone::Ring, events);
            }
            Step::StunDraw { player, max } => {
                let max = max.min(state.player(player).arsenal.len().min(u8::MAX as usize) as u8);
                if max > 0 {
                    state.decision = Some(Decision { player, kind: DecisionKind::StunDraw { max } });
                }
            }
            Step::EndTurn => {
                events.push(Event::TurnEnded { player: state.active });
                state.active = state.active.opponent();
                schedule(state, [Step::CountOutCheck, Step::StartTurn]);
            }
            Step::CountOutCheck => {
                let empty = [PlayerId::ONE, PlayerId::TWO].map(|p| state.player(p).arsenal.is_empty());
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
            .filter(|&played| self.can_reverse(state, player, card, played));
        if let Some(played) = stopping {
            // The damage stops, the attacker's turn ends, and the reversed card
            // still lands in the Ring. The attacker may draw up to its SV.
            events.push(Event::Reversed { player, reversal: card, reversed: played.card, from_hand: false });
            state.pending.clear();
            let max = self.def(state, played.card).stun_value;
            schedule(state, [Step::ToRing, Step::StunDraw { player: played.player, max }, Step::EndTurn]);
        } else if remaining > 1 {
            state.pending.push(Step::Overturn { player, remaining: remaining - 1, source });
        }
    }

    fn reverse_from_hand(&self, state: &mut GameState, defender: PlayerId, reversal: CardId, events: &mut Vec<Event>) {
        let played = state.in_play.take().expect("card in play");
        let spec = self.def(state, reversal).script.reverses.clone().expect("validated reversal");
        let damage = match spec.damage {
            ReversalDamage::Printed => self.def(state, reversal).damage as u16,
            ReversalDamage::None => 0,
            ReversalDamage::Fixed(n) => n as u16,
            ReversalDamage::SameAsReversed => self.card_damage(state, played.card),
        };
        events.push(Event::Reversed { player: defender, reversal, reversed: played.card, from_hand: true });
        self.move_card(state, played.card, Zone::Ringside, events);
        self.move_card(state, reversal, Zone::Ring, events);

        state.pending.clear();
        schedule(state, [Step::EndTurn]);
        if damage > 0 {
            state.pending.push(Step::Overturn { player: played.player, remaining: damage, source: None });
        }
        schedule_effects(state, &spec.on_reverse, defender, reversal);
    }

    fn effect(
        &self,
        state: &mut GameState,
        effect: Effect,
        controller: PlayerId,
        source: CardId,
        events: &mut Vec<Event>,
    ) {
        let target = |who: Who| match who {
            Who::You => controller,
            Who::Opponent => controller.opponent(),
        };
        match effect {
            Effect::Draw { who, n } => self.draw(state, target(who), n, events),
            Effect::Discard { who, n, pick } => {
                let p = target(who);
                let k = (n as usize).min(state.player(p).hand.len());
                match pick {
                    Pick::Random => {
                        for _ in 0..k {
                            let i = state.rng.random_range(0..state.player(p).hand.len());
                            let c = state.player(p).hand[i];
                            self.move_card(state, c, Zone::Ringside, events);
                        }
                    }
                    Pick::Chosen => self.ask_cards(state, p, Zone::Hand, k, ChoicePurpose::Discard),
                }
            }
            Effect::RingsideToHand { who, n } => {
                let p = target(who);
                let k = (n as usize).min(state.player(p).ringside.len());
                self.ask_cards(state, p, Zone::Ringside, k, ChoicePurpose::RingsideToHand);
            }
            Effect::RingsideToArsenal { who, n } => {
                let p = target(who);
                let k = (n as usize).min(state.player(p).ringside.len());
                self.ask_cards(state, p, Zone::Ringside, k, ChoicePurpose::RingsideToArsenal);
            }
            Effect::Damage { who, n } => {
                if n > 0 {
                    state.pending.push(Step::Overturn { player: target(who), remaining: n as u16, source: None });
                }
            }
            Effect::Optional(effects) => {
                state.decision = Some(Decision {
                    player: controller,
                    kind: DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { effects, source } },
                });
            }
        }
    }

    fn ask_cards(&self, state: &mut GameState, p: PlayerId, from: Zone, k: usize, purpose: ChoicePurpose) {
        if k == 0 {
            return;
        }
        let options = state.player(p).zone(from).to_vec();
        let k = k as u8;
        state.decision =
            Some(Decision { player: p, kind: DecisionKind::ChooseCards { options, min: k, max: k, purpose } });
    }

    /// Can `player` play `card` as `kind` in their Main Segment right now?
    pub fn can_play(&self, state: &GameState, player: PlayerId, card: CardId, kind: CardKind) -> bool {
        let def = self.def(state, card);
        state.active == player
            && kind.playable_on_own_turn()
            && def.kinds.contains(&kind)
            && self.fortitude(state, player) >= def.fortitude as u16
    }

    /// Can `player` use `reversal` to stop `played` (from hand or overturned)?
    pub fn can_reverse(&self, state: &GameState, player: PlayerId, reversal: CardId, played: Played) -> bool {
        let rdef = self.def(state, reversal);
        let Some(spec) = &rdef.script.reverses else { return false };
        played.player != player
            && rdef.is_reversal()
            && !self.def(state, played.card).has_trait(Trait::Irreversible)
            && self.fortitude(state, player) >= rdef.fortitude as u16
            && self.matches(state, &spec.targets, played)
    }

    fn matches(&self, state: &GameState, filter: &CardFilter, played: Played) -> bool {
        match filter {
            CardFilter::Any => true,
            CardFilter::Maneuver => matches!(played.kind, CardKind::Maneuver(_)),
            CardFilter::ManeuverType(t) => played.kind == CardKind::Maneuver(*t),
            CardFilter::Action => played.kind == CardKind::Action,
            CardFilter::Title(title) => self.def(state, played.card).title.eq_ignore_ascii_case(title),
            CardFilter::DamageAtMost(n) => self.card_damage(state, played.card) <= *n as u16,
            CardFilter::DamageAtLeast(n) => self.card_damage(state, played.card) >= *n as u16,
            CardFilter::All(fs) => fs.iter().all(|f| self.matches(state, f, played)),
            CardFilter::AnyOf(fs) => fs.iter().any(|f| self.matches(state, f, played)),
            CardFilter::Not(f) => !self.matches(state, f, played),
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

/// Pushes steps so that the first one runs next.
fn schedule<const N: usize>(state: &mut GameState, steps: [Step; N]) {
    state.pending.extend(steps.into_iter().rev());
}

fn schedule_effects(state: &mut GameState, effects: &[Effect], controller: PlayerId, source: CardId) {
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
