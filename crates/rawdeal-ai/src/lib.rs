//! Computer opponents. A bot only ever sees its own [`PlayerView`] and the
//! list of legal actions; it never gets the full game state.

use rand::seq::IndexedRandom;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rawdeal_core::{Action, Engine, PlayerView};

pub trait Bot {
    /// Picks one of `legal`, which is never empty.
    fn choose(&mut self, engine: &Engine, view: &PlayerView, legal: &[Action]) -> Action;
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
}

impl Bot for RandomBot {
    fn choose(&mut self, _engine: &Engine, _view: &PlayerView, legal: &[Action]) -> Action {
        let plays: Vec<&Action> = legal.iter().filter(|a| matches!(a, Action::PlayCard { .. })).collect();
        if !plays.is_empty() && self.rng.random_bool(0.85) {
            return (*plays.choose(&mut self.rng).expect("non-empty")).clone();
        }
        let others: Vec<&Action> = legal.iter().filter(|a| !matches!(a, Action::PlayCard { .. })).collect();
        (*others.choose(&mut self.rng).or_else(|| plays.choose(&mut self.rng)).expect("legal is non-empty")).clone()
    }
}
