//! A player's seat at the table: what a computer opponent may use to pick
//! its action. It sees its own [`PlayerView`] and legal actions and can play
//! out guesses of the hidden cards, but never reads the real hidden cards.

use rand::seq::SliceRandom;
use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::decision::Action;
use crate::engine::Engine;
use crate::state::{GameState, PlayerId, Zone};
use crate::view::PlayerView;

pub struct Seat<'a> {
    engine: &'a Engine,
    state: &'a GameState,
    me: PlayerId,
}

impl<'a> Seat<'a> {
    pub fn new(engine: &'a Engine, state: &'a GameState, me: PlayerId) -> Self {
        Seat { engine, state, me }
    }

    pub fn engine(&self) -> &'a Engine {
        self.engine
    }

    pub fn me(&self) -> PlayerId {
        self.me
    }

    pub fn view(&self) -> PlayerView {
        self.engine.view(self.state, self.me)
    }

    pub fn legal(&self) -> Vec<Action> {
        self.engine.legal_actions(self.state, self.me)
    }

    /// A full game state consistent with what this player knows: the cards
    /// they cannot see (their own Arsenal order, the opponent's hand and
    /// Arsenal) are dealt again at random from the same cards, and the
    /// game's random numbers are reseeded.
    pub fn sample(&self, rng: &mut impl Rng) -> GameState {
        let view = self.view();
        let mut state = self.state.clone();
        let opponent = self.me.opponent();
        let pools = [
            (self.me, &[Zone::Arsenal][..]),
            (opponent, if view.opponent_hand.is_some() { &[Zone::Arsenal][..] } else { &[Zone::Hand, Zone::Arsenal] }),
        ];
        for (p, zones) in pools {
            let ids: Vec<_> = zones
                .iter()
                .flat_map(|&z| state.player(p).zone(z).to_vec())
                .filter(|&c| view.def_of(c).is_none() && state.card(c).face_down.is_none())
                .collect();
            let mut defs: Vec<_> = ids.iter().map(|&c| state.card(c).def).collect();
            defs.shuffle(rng);
            for (c, def) in ids.into_iter().zip(defs) {
                state.cards[c.0 as usize].def = def;
            }
        }
        state.rng = ChaCha8Rng::seed_from_u64(rng.random());
        state
    }
}
