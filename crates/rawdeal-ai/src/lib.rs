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

pub trait Bot {
    /// Picks one of the seat's legal actions, which are never empty.
    fn choose(&mut self, seat: &Seat) -> Action;
}

/// How well the computer plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    /// Random legal moves.
    Easy,
    /// Plays its strongest cards and always reverses, without looking ahead.
    #[default]
    Normal,
    /// Tries each move against guesses of your hand and plays out the rest of
    /// the turn.
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn bot(self, seed: u64) -> Box<dyn Bot + Send> {
        match self {
            Difficulty::Easy => Box::new(RandomBot::new(seed)),
            Difficulty::Normal => Box::new(GreedyBot::new(seed)),
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

/// Simple rules of thumb: reverse whenever possible, attack with the hardest
/// hitting maneuver, play Actions now and then, and keep good cards when
/// discarding.
fn greedy(
    engine: &Engine,
    def: impl Fn(CardId) -> Option<DefIdx>,
    decision: Option<&DecisionKind>,
    legal: &[Action],
    rng: &mut ChaCha8Rng,
) -> Action {
    let def_card = |c: CardId| def(c).map(|d| engine.db().card(d));
    let worth = |c: CardId| def_card(c).map_or(1.0, card_worth);
    let noise = |rng: &mut ChaCha8Rng| rng.random_range(0.0..1.5);
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
                    CardKind::Maneuver(_) => 3.0 + d.map_or(2.0, |d| d.damage as f32),
                    CardKind::Reversal => -5.0,
                    _ => 2.5,
                })
            }
            Action::UseAbility | Action::UseRingCard { .. } | Action::UseBackstageCard { .. } => Some(3.0),
            Action::EndTurn => Some(1.0),
            _ => Some(0.0),
        }),
        Some(DecisionKind::ChooseCards { purpose, .. }) if is_loss(purpose) => best(rng, &|a| match a {
            Action::ChooseCards(cs) => Some(-cs.iter().map(|&c| worth(c)).sum::<f32>()),
            _ => None,
        }),
        Some(DecisionKind::ChooseCards { .. }) => best(rng, &|a| match a {
            Action::ChooseCards(cs) => Some(cs.iter().map(|&c| worth(c)).sum::<f32>()),
            _ => None,
        }),
        Some(DecisionKind::YesNo { purpose: YesNoPurpose::ClaimCountOut }) => Some(Action::Yes),
        Some(DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { .. } }) if rng.random_bool(0.8) => {
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

/// Does choosing cards for `purpose` cost the chooser those cards?
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

/// How much a card is worth keeping: Reversals and big maneuvers most.
fn card_worth(def: &CardDef) -> f32 {
    if def.is_reversal() { 6.0 } else { def.damage as f32 * 0.6 + 1.0 }
}

/// Normal level: [`greedy`] on what it can see.
pub struct GreedyBot {
    rng: ChaCha8Rng,
}

impl GreedyBot {
    pub fn new(seed: u64) -> Self {
        GreedyBot { rng: ChaCha8Rng::seed_from_u64(seed) }
    }
}

impl Bot for GreedyBot {
    fn choose(&mut self, seat: &Seat) -> Action {
        let view = seat.view();
        let legal = seat.legal();
        let def = |c: CardId| view.def_of(c);
        greedy(seat.engine(), def, view.decision.as_ref().map(|d| &d.kind), &legal, &mut self.rng)
    }
}

/// Hard level: flat Monte Carlo search. Each legal action is tried against
/// several guesses of the hidden cards ([`Seat::sample`]); both players then
/// follow [`greedy`] for the rest of the turn (see [`Budget::turns`]), and
/// the action with the best average position wins.
pub struct SearchBot {
    rng: ChaCha8Rng,
    budget: Budget,
}

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Play-outs per legal action.
    pub playouts: usize,
    /// At most this many actions are tried; the others are skipped.
    pub max_actions: usize,
    /// Actions in one play-out, at most.
    pub max_steps: usize,
    /// Turns a play-out lasts: 1 to the end of this turn, 2 also through
    /// the opponent's next turn.
    pub turns: u32,
}

impl SearchBot {
    pub const HARD: Budget = Budget { playouts: 12, max_actions: 16, max_steps: 400, turns: 1 };

    pub fn new(seed: u64, budget: Budget) -> Self {
        SearchBot { rng: ChaCha8Rng::seed_from_u64(seed), budget }
    }

    fn playout(&self, engine: &Engine, state: &mut GameState, me: PlayerId, rng: &mut ChaCha8Rng) -> f32 {
        // To the end of this turn, or also through the opponent's next one.
        let start = state.turn;
        let horizon = start + self.budget.turns - u32::from(state.active != me && self.budget.turns > 1);
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
            let action = greedy(engine, def, Some(&decision.kind), &legal, rng);
            if engine.apply(state, p, action).is_err() {
                break;
            }
        }
        evaluate(engine, state, me)
    }
}

/// How good `state` is for `me`: winning or losing outweighs everything,
/// then the Arsenal left (the life total), cards in hand (Reversals most) and
/// Fortitude.
fn evaluate(engine: &Engine, state: &GameState, me: PlayerId) -> f32 {
    match state.outcome {
        Some(Outcome::Win { winner, .. }) => return if winner == me { 1000.0 } else { -1000.0 },
        Some(Outcome::Draw) => return 0.0,
        None => {}
    }
    let side = |p: PlayerId| {
        let ps = state.player(p);
        let arsenal = ps.arsenal.len() as f32;
        // Running out of Arsenal is what loses the match.
        let danger = if arsenal < 10.0 { (10.0 - arsenal) * 1.5 } else { 0.0 };
        let reversals = ps.hand.iter().filter(|&&c| engine.db().card(state.card(c).def).is_reversal()).count();
        // Reversals in hand are worth more than other cards: they are the defence.
        arsenal - danger + ps.hand.len() as f32 * 0.7 + reversals as f32 + engine.fortitude(state, p) as f32 * 0.05
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
            let first = greedy(seat.engine(), def, view.decision.as_ref().map(|d| &d.kind), &legal, &mut self.rng);
            let mut rest: Vec<Action> = legal.into_iter().filter(|a| *a != first).collect();
            rest.shuffle(&mut self.rng);
            rest.truncate(self.budget.max_actions - 1);
            legal = std::iter::once(first).chain(rest).collect();
        }
        let engine = seat.engine();
        let me = seat.me();
        let mut totals = vec![0.0f32; legal.len()];
        for _ in 0..self.budget.playouts {
            // The same guess for every action, so they are compared fairly.
            // The same guess and the same random choices for every action, so
            // they are compared fairly.
            let guess = seat.sample(&mut self.rng);
            let seed: u64 = self.rng.random();
            for (i, action) in legal.iter().enumerate() {
                let mut state = guess.clone();
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                totals[i] += match engine.apply(&mut state, me, action.clone()) {
                    Ok(_) => self.playout(engine, &mut state, me, &mut rng),
                    Err(_) => -10_000.0,
                };
            }
        }
        let best = totals.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
        legal.swap_remove(best)
    }
}
