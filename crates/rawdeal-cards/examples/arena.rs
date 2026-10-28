//! Plays the computer levels against each other and prints how often each
//! wins: `cargo run --release -p rawdeal-cards --example arena -- [CARDS_DIR] [MATCHES]`.
//! Without a folder it uses the bundled test cards.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;
use rawdeal_ai::Difficulty;
use rawdeal_core::{Engine, GameSetup, Outcome, PlayerId, Seat};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = match args.first().filter(|a| a.parse::<usize>().is_err()) {
        Some(dir) => rawdeal_cards::build_db(rawdeal_cards::load_dir(Path::new(dir)).unwrap()).unwrap(),
        None => rawdeal_cards::test_db(),
    };
    let matches: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(40);
    let decks: Vec<_> = db.superstars().iter().filter_map(|s| rawdeal_cards::auto_deck(&db, &s.id)).collect();
    let engine = Engine::new(Arc::new(db));
    let pairs = [
        (Difficulty::Normal, Difficulty::Easy),
        (Difficulty::Hard, Difficulty::Easy),
        (Difficulty::Hard, Difficulty::Normal),
    ];
    for (a, b) in pairs {
        let mut wins = [0u32; 3];
        let mut thinking = [Duration::ZERO; 2];
        let mut decisions = [0u32; 2];
        let mut slowest = Duration::ZERO;
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut pick = [decks[0].clone(), decks[0].clone()];
        for seed in 0..matches {
            // Each pair of decks is played twice, the levels swapping seats.
            if seed % 2 == 0 {
                pick = [decks.choose(&mut rng).unwrap().clone(), decks.choose(&mut rng).unwrap().clone()];
            }
            let levels = if seed % 2 == 0 { [a, b] } else { [b, a] };
            let (mut state, _) = engine.new_game(&GameSetup::standard(pick.clone()), seed / 2).unwrap();
            let mut bots = [levels[0].bot(seed * 2 + 1), levels[1].bot(seed * 2 + 2)];
            let mut actions = 0;
            while state.outcome.is_none() && actions < 5000 {
                let p = state.decision.as_ref().unwrap().player;
                let t = Instant::now();
                let action = bots[p.index()].choose(&Seat::new(&engine, &state, p));
                let took = t.elapsed();
                let side = usize::from(levels[p.index()] != a);
                thinking[side] += took;
                decisions[side] += 1;
                if levels[p.index()] == Difficulty::Hard {
                    slowest = slowest.max(took);
                }
                engine.apply(&mut state, p, action).unwrap();
                actions += 1;
            }
            match state.outcome {
                Some(Outcome::Win { winner, .. }) => {
                    let first_won = winner == PlayerId::ONE;
                    wins[usize::from((levels[0] == a) != first_won)] += 1;
                }
                _ => wins[2] += 1,
            }
        }
        let avg = |i: usize| thinking[i].as_secs_f64() * 1000.0 / decisions[i].max(1) as f64;
        println!(
            "{a:?} vs {b:?}: {} - {} ({} draws or unfinished); ms per decision {:.2} / {:.2}; slowest Hard {:.0} ms",
            wins[0],
            wins[1],
            wins[2],
            avg(0),
            avg(1),
            slowest.as_secs_f64() * 1000.0
        );
    }
}
