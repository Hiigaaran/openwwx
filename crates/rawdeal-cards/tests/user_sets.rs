//! Plays random matches with the card sets in `$OPENWWX_CARDS`, if set.
//! Real card data is not in the repository, so this is skipped in CI:
//! `OPENWWX_CARDS=~/raw-deal/sets cargo test -p rawdeal-cards --test user_sets`

use std::path::PathBuf;
use std::sync::Arc;

use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::{Engine, GameSetup};

#[test]
fn user_sets_play_random_matches() {
    let Some(dir) = std::env::var_os("OPENWWX_CARDS").map(PathBuf::from) else {
        eprintln!("OPENWWX_CARDS not set; skipping");
        return;
    };
    let db = rawdeal_cards::build_db(rawdeal_cards::load_dir(&dir).unwrap()).unwrap();
    let decks: Vec<_> = db.superstars().iter().filter_map(|s| rawdeal_cards::auto_deck(&db, &s.id)).collect();
    assert_eq!(decks.len(), db.superstars().len(), "every Superstar gets a legal deck");
    let engine = Engine::new(Arc::new(db));
    let mut games = 0;
    for (i, a) in decks.iter().enumerate() {
        for b in &decks[i..] {
            for seed in 0..10u64 {
                let setup = GameSetup::standard([a.clone(), b.clone()]);
                let (mut state, _) = engine.new_game(&setup, seed).unwrap();
                let mut bots = [RandomBot::new(seed), RandomBot::new(seed + 1)];
                let mut actions = 0;
                while state.outcome.is_none() {
                    let p = state.decision.as_ref().unwrap().player;
                    let legal = engine.legal_actions(&state, p);
                    let action = bots[p.index()].choose(&engine, &engine.view(&state, p), &legal);
                    engine.apply(&mut state, p, action).unwrap();
                    state.check_invariants().unwrap();
                    actions += 1;
                    assert!(actions < 20_000, "{} vs {} seed {seed} did not finish", a.superstar, b.superstar);
                }
                games += 1;
            }
        }
    }
    eprintln!("{games} matches finished");
}
