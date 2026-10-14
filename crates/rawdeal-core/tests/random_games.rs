//! Property tests: random bots play full matches with legal decks.

use std::sync::Arc;

use proptest::prelude::*;
use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::*;

const MAX_ACTIONS: usize = 20_000;

/// Plays a whole match and returns the outcome and every event.
fn play_out(seed: u64) -> (Outcome, Vec<Event>) {
    let engine = Engine::new(Arc::new(rawdeal_cards::test_db()));
    // Rotate through every Superstar pairing so all abilities get exercised.
    let ids: Vec<String> = engine.db().superstars().iter().map(|s| s.id.clone()).collect();
    let n = ids.len() as u64;
    let decks =
        [rawdeal_cards::test_deck(&ids[(seed % n) as usize]), rawdeal_cards::test_deck(&ids[(seed / n % n) as usize])];
    // Odd seeds play Advanced matches with Pre-match and Backlash decks.
    let setup = if seed % 2 == 1 { GameSetup::advanced(decks) } else { GameSetup::standard(decks) };
    let (mut state, mut log) = engine.new_game(&setup, seed).unwrap();
    let mut bots = [RandomBot::new(seed ^ 1), RandomBot::new(seed ^ 2)];
    for _ in 0..MAX_ACTIONS {
        if let Some(outcome) = state.outcome {
            return (outcome, log);
        }
        let player = state.decision.as_ref().expect("a live game always waits on someone").player;
        let legal = engine.legal_actions(&state, player);
        assert!(!legal.is_empty(), "{player} has a decision but no legal action");
        let view = engine.view(&state, player);
        let action = bots[player.index()].choose(&engine, &view, &legal);
        log.extend(engine.apply(&mut state, player, action).expect("bots only pick legal actions"));
        state.check_invariants().unwrap();
        assert!(state.cards.len() == 120 || state.cards.len() == 120 + 18);
    }
    panic!("match did not finish in {MAX_ACTIONS} actions");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn random_matches_finish_and_keep_invariants(seed in any::<u64>()) {
        play_out(seed);
    }
}

#[test]
fn same_seed_same_match() {
    for seed in [0, 7, 12345] {
        assert_eq!(play_out(seed), play_out(seed));
    }
}

#[test]
fn every_way_to_end_happens() {
    let outcomes: Vec<Outcome> = (0..300).map(|s| play_out(s).0).collect();
    let pins = outcomes.iter().filter(|o| matches!(o, Outcome::Win { method: WinMethod::Pinfall, .. })).count();
    let count_outs = outcomes.iter().filter(|o| matches!(o, Outcome::Win { method: WinMethod::CountOut, .. })).count();
    assert!(pins > 0 && count_outs > 0, "pins {pins}, count outs {count_outs}");
}
