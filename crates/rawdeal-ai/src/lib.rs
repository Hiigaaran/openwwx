//! Computer opponents. A bot gets a [`Seat`]: its own view, its legal
//! actions and guesses of the cards it cannot see; it never reads the real
//! hidden cards.

use rand::seq::{IndexedRandom, SliceRandom};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rawdeal_core::{
    Action, CardDef, CardId, CardKind, ChoicePurpose, DecisionKind, DefIdx, Engine, GameState, Outcome, PlayerId, Seat,
    YesNoPurpose,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub trait Bot {
    /// Picks one of the seat's legal actions, which are never empty.
    fn choose(&mut self, seat: &Seat) -> Action;
}

/// How well the computer plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    /// Random legal moves.
    Easy,
    /// Rules of thumb (strongest cards first, always reverse) checked by a
    /// few play-outs of each move: a light version of Hard.
    #[default]
    Normal,
    /// Tries each move against guesses of your hand and plays the match out
    /// to the end.
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn bot(self, seed: u64) -> Box<dyn Bot + Send> {
        match self {
            Difficulty::Easy => Box::new(RandomBot::new(seed)),
            Difficulty::Normal => Box::new(SearchBot::new(seed, SearchBot::NORMAL)),
            Difficulty::Hard => Box::new(SearchBot::new(seed, SearchBot::HARD)),
        }
    }
}

/// Easiest level: plays a random legal action, preferring to keep attacking
/// over ending its turn.
pub struct RandomBot {
    rng: ChaCha8Rng,
}

impl RandomBot {
    pub fn new(seed: u64) -> Self {
        RandomBot { rng: ChaCha8Rng::seed_from_u64(seed) }
    }

    /// The random choice among `legal`.
    pub fn pick(&mut self, legal: &[Action]) -> Action {
        let plays: Vec<&Action> = legal.iter().filter(|a| matches!(a, Action::PlayCard { .. })).collect();
        if !plays.is_empty() && self.rng.random_bool(0.85) {
            return (*plays.choose(&mut self.rng).expect("non-empty")).clone();
        }
        let others: Vec<&Action> = legal.iter().filter(|a| !matches!(a, Action::PlayCard { .. })).collect();
        (*others.choose(&mut self.rng).or_else(|| plays.choose(&mut self.rng)).expect("legal is non-empty")).clone()
    }
}

impl Bot for RandomBot {
    fn choose(&mut self, seat: &Seat) -> Action {
        self.pick(&seat.legal())
    }
}

/// Weights of [`greedy`]'s rules of thumb.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Random amount, up to this, added to every score.
    pub noise: f32,
    /// A maneuver scores `maneuver + damage * per_damage - fortitude * per_fortitude`.
    pub maneuver: f32,
    pub per_damage: f32,
    pub per_fortitude: f32,
    pub action: f32,
    pub ability: f32,
    pub end_turn: f32,
    /// How much a Reversal is worth keeping; other cards are worth
    /// `1 + damage * damage_worth`.
    pub reversal_worth: f32,
    pub damage_worth: f32,
    /// Chances of saying yes to a "you may" effect.
    pub optional_yes: f64,
    /// Which cards a choice counts as a loss for their owner: the 0.41
    /// list (only the chooser's discards) or every card that leaves its
    /// owner's hand or Ring, either player's.
    pub owner_aware: bool,
}

impl Style {
    pub const NORMAL: Style = Style {
        noise: 1.5,
        maneuver: 3.0,
        per_damage: 1.0,
        per_fortitude: 0.0,
        action: 2.5,
        ability: 3.0,
        end_turn: 1.0,
        reversal_worth: 6.0,
        damage_worth: 0.6,
        optional_yes: 0.8,
        owner_aware: true,
    };
    /// Normal up to version 0.42.
    pub const OLD: Style = Style { owner_aware: false, ..Style::NORMAL };

    fn worth(&self, def: &CardDef) -> f32 {
        if def.is_reversal() { self.reversal_worth } else { def.damage as f32 * self.damage_worth + 1.0 }
    }
}

/// Simple rules of thumb: reverse whenever possible, attack with the hardest
/// hitting maneuver, play Actions now and then, keep good cards and take
/// the opponent's.
#[allow(clippy::too_many_arguments)]
fn greedy(
    engine: &Engine,
    style: &Style,
    me: PlayerId,
    def: impl Fn(CardId) -> Option<DefIdx>,
    owner: impl Fn(CardId) -> PlayerId,
    decision: Option<&DecisionKind>,
    legal: &[Action],
    rng: &mut ChaCha8Rng,
) -> Action {
    let def_card = |c: CardId| def(c).map(|d| engine.db().card(d));
    let worth = |c: CardId| def_card(c).map_or(1.0, |d| style.worth(d));
    let noise = |rng: &mut ChaCha8Rng| if style.noise > 0.0 { rng.random_range(0.0..style.noise) } else { 0.0 };
    let best = |rng: &mut ChaCha8Rng, score: &dyn Fn(&Action) -> Option<f32>| {
        legal
            .iter()
            .filter_map(|a| score(a).map(|s| (a, s + noise(rng))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(a, _)| a.clone())
    };
    let pick = match decision {
        Some(DecisionKind::Reversal { .. }) | Some(DecisionKind::CounterReversal { .. }) => best(rng, &|a| match a {
            Action::Reverse { card } => Some(10.0 - worth(*card)),
            Action::ReverseByVolley => Some(5.0),
            _ => None,
        }),
        Some(DecisionKind::Main) => best(rng, &|a| match a {
            Action::PlayCard { card, kind } => {
                let d = def_card(*card);
                Some(match kind {
                    CardKind::Maneuver(_) => {
                        style.maneuver
                            + d.map_or(2.0, |d| {
                                d.damage as f32 * style.per_damage - d.fortitude as f32 * style.per_fortitude
                            })
                    }
                    CardKind::Reversal => -5.0,
                    _ => style.action,
                })
            }
            Action::UseAbility | Action::UseRingCard { .. } | Action::UseBackstageCard { .. } => Some(style.ability),
            Action::EndTurn => Some(style.end_turn),
            _ => Some(0.0),
        }),
        Some(DecisionKind::ChooseCards { purpose, .. }) if style.owner_aware => {
            // Positive: the chosen cards are gained by their owner.
            let gain = match gains(purpose) {
                Some(true) => 1.0,
                Some(false) => -1.0,
                None => 0.0,
            };
            best(rng, &|a| match a {
                Action::ChooseCards(cs) => {
                    Some(cs.iter().map(|&c| worth(c) * gain * if owner(c) == me { 1.0 } else { -1.0 }).sum::<f32>())
                }
                _ => None,
            })
        }
        Some(DecisionKind::ChooseCards { purpose, .. }) if is_loss(purpose) => best(rng, &|a| match a {
            Action::ChooseCards(cs) => Some(-cs.iter().map(|&c| worth(c)).sum::<f32>()),
            _ => None,
        }),
        Some(DecisionKind::ChooseCards { .. }) => best(rng, &|a| match a {
            Action::ChooseCards(cs) => Some(cs.iter().map(|&c| worth(c)).sum::<f32>()),
            _ => None,
        }),
        Some(DecisionKind::YesNo { purpose: YesNoPurpose::ClaimCountOut }) => Some(Action::Yes),
        Some(DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { .. } })
            if rng.random_bool(style.optional_yes) =>
        {
            legal.iter().find(|a| **a == Action::Yes).cloned()
        }
        Some(DecisionKind::StunDraw { .. }) => best(rng, &|a| match a {
            Action::DrawCards(n) => Some(*n as f32 * 2.0),
            _ => None,
        }),
        Some(DecisionKind::PreMatch { .. }) => best(rng, &|a| match a {
            Action::PlayCard { .. } => Some(2.0),
            _ => Some(0.0),
        }),
        _ => None,
    };
    pick.unwrap_or_else(|| legal.choose(rng).expect("legal is non-empty").clone())
}

/// Does choosing cards for `purpose` cost the chooser those cards? (0.41)
fn is_loss(purpose: &ChoicePurpose) -> bool {
    matches!(
        purpose,
        ChoicePurpose::Discard
            | ChoicePurpose::HandToArsenal
            | ChoicePurpose::HandToArsenalBottom
            | ChoicePurpose::HandToArsenalTop
            | ChoicePurpose::DiscardForDamage { .. }
            | ChoicePurpose::DiscardMirror
    )
}

/// Do the cards chosen for `purpose` go somewhere better for their owner
/// (`Some(true)`), worse (`Some(false)`), or is it unclear (`None`)?
fn gains(purpose: &ChoicePurpose) -> Option<bool> {
    use ChoicePurpose as P;
    match purpose {
        P::Discard
        | P::ForcedDiscard
        | P::HandToArsenal
        | P::HandToArsenalBottom
        | P::HandToArsenalTop
        | P::DiscardForDamage { .. }
        | P::DiscardMirror
        | P::RingToRingside
        | P::RemoveFromGame
        | P::RingToHand => Some(false),
        P::RingsideToHand
        | P::RingsideToArsenal
        | P::RingsideToArsenalBottom
        | P::SearchToHand
        | P::DigToHand
        | P::AfterSuccess(_)
        | P::RingsideToBacklash => Some(true),
        _ => None,
    }
}

/// [`greedy`] on what it can see, without looking ahead (Normal up to
/// version 0.42, and the play-outs of [`SearchBot`]).
pub struct GreedyBot {
    rng: ChaCha8Rng,
    style: Style,
}

impl GreedyBot {
    pub fn new(seed: u64) -> Self {
        GreedyBot::with_style(seed, Style::NORMAL)
    }

    pub fn with_style(seed: u64, style: Style) -> Self {
        GreedyBot { rng: ChaCha8Rng::seed_from_u64(seed), style }
    }
}

impl Bot for GreedyBot {
    fn choose(&mut self, seat: &Seat) -> Action {
        let view = seat.view();
        let legal = seat.legal();
        let def = |c: CardId| view.def_of(c);
        let owner = |c: CardId| seat.owner(c);
        let decision = view.decision.as_ref().map(|d| &d.kind);
        greedy(seat.engine(), &self.style, seat.me(), def, owner, decision, &legal, &mut self.rng)
    }
}

/// Hard level: Monte Carlo search. Each legal action is tried against
/// guesses of the hidden cards ([`Seat::sample`]); both players then follow
/// [`greedy`] to the end of the match (see [`Budget::turns`]) and the action
/// that wins most often is played. The play-outs are spent by successive
/// halving: every action gets a few, then the worse half is dropped and the
/// rest get more.
pub struct SearchBot {
    rng: ChaCha8Rng,
    budget: Budget,
}

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Play-outs per legal action, on average: the decision gets
    /// `playouts` times the number of actions tried.
    pub playouts: usize,
    /// At most this many actions are tried; the others are skipped.
    pub max_actions: usize,
    /// Actions in one play-out, at most; then the position is scored by
    /// [`evaluate`].
    pub max_steps: usize,
    /// Turns a play-out lasts (1: to the end of this turn), `None` to the
    /// end of the match.
    pub turns: Option<u32>,
    /// How positions are scored when a play-out stops before the end.
    pub weights: Weights,
    /// How both players play during a play-out.
    pub style: Style,
    /// Thinking time per decision, at most: past it the best action so far
    /// is played.
    pub max_time: Duration,
}

/// Weights of [`evaluate`].
#[derive(Debug, Clone, Copy)]
pub struct Weights {
    pub arsenal: f32,
    /// Below this many Arsenal cards each missing card costs `danger` more.
    pub danger_below: f32,
    pub danger: f32,
    pub hand: f32,
    /// On top of `hand`, for each Reversal in hand.
    pub reversal: f32,
    pub fortitude: f32,
}

impl Weights {
    pub const DEFAULT: Weights =
        Weights { arsenal: 1.0, danger_below: 10.0, danger: 1.5, hand: 0.7, reversal: 1.0, fortitude: 0.05 };
}

impl SearchBot {
    pub const HARD: Budget = Budget {
        playouts: 50,
        max_actions: 16,
        max_steps: 600,
        turns: None,
        weights: Weights::DEFAULT,
        style: Style::NORMAL,
        max_time: Duration::from_millis(400),
    };
    /// Few play-outs: about a tenth of Hard's thinking.
    pub const NORMAL: Budget = Budget { playouts: 4, max_time: Duration::from_millis(100), ..SearchBot::HARD };

    pub fn new(seed: u64, budget: Budget) -> Self {
        SearchBot { rng: ChaCha8Rng::seed_from_u64(seed), budget }
    }

    /// Plays `state` out and scores it for `me`, from -1 (lost) to 1 (won).
    fn playout(&self, engine: &Engine, state: &mut GameState, me: PlayerId, rng: &mut ChaCha8Rng) -> f32 {
        let horizon = self.budget.turns.map_or(u32::MAX, |t| state.turn.saturating_add(t));
        let mut steps = 0;
        for _ in 0..self.budget.max_steps {
            if state.is_over() || state.turn >= horizon {
                break;
            }
            let Some(decision) = state.decision.clone() else { break };
            let p = decision.player;
            let legal = engine.legal_actions(state, p);
            if legal.is_empty() {
                break;
            }
            let def = |c: CardId| Some(state.card(c).def);
            let owner = |c: CardId| state.card(c).owner;
            let action = greedy(engine, &self.budget.style, p, def, owner, Some(&decision.kind), &legal, rng);
            if engine.apply(state, p, action).is_err() {
                break;
            }
            steps += 1;
        }
        // A win sooner is better than a win later (and a loss later better
        // than a loss sooner), so a won match is closed rather than dragged on.
        let sooner = 1.0 - steps.min(500) as f32 * 0.001;
        match state.outcome {
            Some(Outcome::Win { winner, .. }) => {
                if winner == me {
                    sooner
                } else {
                    -sooner
                }
            }
            Some(Outcome::Draw) => 0.0,
            None => (evaluate(engine, state, me, &self.budget.weights) / 20.0).tanh(),
        }
    }
}

/// How good `state` is for `me`: the Arsenal left (the life total), cards in
/// hand (Reversals most) and Fortitude, against the opponent's.
fn evaluate(engine: &Engine, state: &GameState, me: PlayerId, w: &Weights) -> f32 {
    let side = |p: PlayerId| {
        let ps = state.player(p);
        let arsenal = ps.arsenal.len() as f32;
        // Running out of Arsenal is what loses the match.
        let danger = (w.danger_below - arsenal).max(0.0) * w.danger;
        let reversals = ps.hand.iter().filter(|&&c| engine.db().card(state.card(c).def).is_reversal()).count();
        arsenal * w.arsenal - danger
            + ps.hand.len() as f32 * w.hand
            + reversals as f32 * w.reversal
            + engine.fortitude(state, p) as f32 * w.fortitude
    };
    side(me) - side(me.opponent())
}

impl Bot for SearchBot {
    fn choose(&mut self, seat: &Seat) -> Action {
        let mut legal = seat.legal();
        if legal.len() == 1 {
            return legal.remove(0);
        }
        if legal.len() > self.budget.max_actions {
            // Keep the greedy choice and a random sample of the rest.
            let view = seat.view();
            let def = |c: CardId| view.def_of(c);
            let owner = |c: CardId| seat.owner(c);
            let decision = view.decision.as_ref().map(|d| &d.kind);
            let first =
                greedy(seat.engine(), &self.budget.style, seat.me(), def, owner, decision, &legal, &mut self.rng);
            let mut rest: Vec<Action> = legal.into_iter().filter(|a| *a != first).collect();
            rest.shuffle(&mut self.rng);
            rest.truncate(self.budget.max_actions - 1);
            legal = std::iter::once(first).chain(rest).collect();
        }
        let engine = seat.engine();
        let me = seat.me();
        let started = Instant::now();
        // (action, total score, play-outs)
        let mut arms: Vec<(Action, f32, usize)> = legal.into_iter().map(|a| (a, 0.0, 0)).collect();
        let rounds = (arms.len() as f32).log2().ceil().max(1.0) as usize;
        let mut budget = self.budget.playouts * arms.len();
        'search: for round in 0..rounds {
            let per_arm = (budget / (arms.len() * (rounds - round))).max(1);
            for _ in 0..per_arm {
                // The same guess and the same random choices for every
                // action, so they are compared fairly.
                let guess = seat.sample(&mut self.rng);
                let seed: u64 = self.rng.random();
                for (action, total, n) in arms.iter_mut() {
                    let mut state = guess.clone();
                    let mut rng = ChaCha8Rng::seed_from_u64(seed);
                    *total += match engine.apply(&mut state, me, action.clone()) {
                        Ok(_) => self.playout(engine, &mut state, me, &mut rng),
                        Err(_) => -10.0,
                    };
                    *n += 1;
                }
                if started.elapsed() > self.budget.max_time {
                    break 'search;
                }
            }
            budget = budget.saturating_sub(per_arm * arms.len());
            if round + 1 < rounds {
                arms.sort_by(|a, b| (b.1 / b.2 as f32).total_cmp(&(a.1 / a.2 as f32)));
                arms.truncate(arms.len().div_ceil(2));
            }
        }
        // The first of equally good actions: the legal order puts the
        // natural one first (claiming a Count Out, playing a card).
        let mean = |a: &(Action, f32, usize)| a.1 / a.2 as f32;
        let mut best = 0;
        for i in 1..arms.len() {
            if mean(&arms[i]) > mean(&arms[best]) {
                best = i;
            }
        }
        arms.swap_remove(best).0
    }
}
