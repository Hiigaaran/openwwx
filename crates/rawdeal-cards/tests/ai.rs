//! The computer levels play legal, finished matches, and guessing the hidden
//! cards keeps what the player can see.

use std::sync::Arc;

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rawdeal_ai::Difficulty;
use rawdeal_core::{Engine, GameSetup, PlayerId, Seat};

fn engine() -> Engine {
    Engine::new(Arc::new(rawdeal_cards::test_db()))
}

fn decks() -> [rawdeal_core::DeckList; 2] {
    [rawdeal_cards::test_deck("test/technician"), rawdeal_cards::test_deck("test/brawler")]
}

#[test]
fn every_level_finishes_matches() {
    let e = engine();
    for level in Difficulty::ALL {
        for seed in 0..4 {
            let setup = if seed % 2 == 0 { GameSetup::standard(decks()) } else { GameSetup::advanced(decks()) };
            let (mut state, _) = e.new_game(&setup, seed).unwrap();
            let mut bots = [level.bot(seed), Difficulty::Easy.bot(seed + 1)];
            let mut actions = 0;
            while state.outcome.is_none() {
                let p = state.decision.as_ref().unwrap().player;
                let action = bots[p.index()].choose(&Seat::new(&e, &state, p));
                e.apply(&mut state, p, action).expect("bots pick legal actions");
                actions += 1;
                assert!(actions < 5000, "{level:?} seed {seed} did not finish");
            }
        }
    }
}

#[test]
fn a_guess_keeps_what_the_player_sees() {
    let e = engine();
    let (state, _) = e.new_game(&GameSetup::standard(decks()), 3).unwrap();
    let me = PlayerId::ONE;
    let seat = Seat::new(&e, &state, me);
    let view = seat.view();
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let mut changed = false;
    for _ in 0..20 {
        let guess = seat.sample(&mut rng);
        guess.check_invariants().unwrap();
        assert_eq!(e.view(&guess, me), view, "the guess looks the same from my seat");
        let opponent = |s: &rawdeal_core::GameState| {
            s.player(me.opponent()).hand.iter().map(|&c| s.card(c).def).collect::<Vec<_>>()
        };
        changed |= opponent(&guess) != opponent(&state);
    }
    assert!(changed, "the opponent's hand is guessed, not copied");
}
