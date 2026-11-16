//! Pits two computer players against each other on all CPU cores, to tune
//! the search: `cargo run --release -p rawdeal-cards --example tune -- CARDS_DIR MATCHES A B`.
//! A player is `easy`, `normal`, `hard`, or `s` (the Hard search) followed
//! by changes such as `s,p=25,full` or `s,t=1,p=12`: `p` play-outs per
//! action, `a` actions tried, `m` steps per play-out, `t` turns per play-out
//! (`full`: to the end of the match), and the weights `ar db d h r f`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rand::SeedableRng;
use rand::seq::IndexedRandom;
use rand_chacha::ChaCha8Rng;
use rawdeal_ai::{Bot, Difficulty, GreedyBot, SearchBot, Style, Weights};
use rawdeal_core::{Engine, GameSetup, Outcome, PlayerId, Seat};

fn bot(name: &str, seed: u64) -> Box<dyn Bot + Send> {
    let mut b = SearchBot::HARD;
    let mut w = Weights::DEFAULT;
    let mut st = Style::NORMAL;
    let mut greedy = false;
    for part in name.split(',') {
        let (k, v) = part.split_once('=').unwrap_or((part, "0"));
        let f: f32 = v.parse().unwrap_or(0.0);
        match k {
            "easy" => return Difficulty::Easy.bot(seed),
            "normal" => return Difficulty::Normal.bot(seed),
            "hard" => return Difficulty::Hard.bot(seed),
            "s" => {}
            "n" => b = SearchBot::NORMAL,
            "g" => greedy = true,
            "old" => st = Style::OLD,
            "gn" => st.noise = f,
            "gm" => st.maneuver = f,
            "gd" => st.per_damage = f,
            "gf" => st.per_fortitude = f,
            "ga" => st.action = f,
            "gab" => st.ability = f,
            "ge" => st.end_turn = f,
            "grw" => st.reversal_worth = f,
            "gdw" => st.damage_worth = f,
            "gy" => st.optional_yes = f as f64,
            "gown" => st.owner_aware = f > 0.0,
            "p" => b.playouts = f as usize,
            "a" => b.max_actions = f as usize,
            "t" => b.turns = Some(f as u32),
            "full" => b.turns = None,
            "m" => b.max_steps = f as usize,
            "ar" => w.arsenal = f,
            "db" => w.danger_below = f,
            "d" => w.danger = f,
            "h" => w.hand = f,
            "r" => w.reversal = f,
            "f" => w.fortitude = f,
            _ => panic!("unknown {k}"),
        }
    }
    if greedy {
        return Box::new(GreedyBot::with_style(seed, st));
    }
    b.weights = w;
    b.style = st;
    Box::new(SearchBot::new(seed, b))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = rawdeal_cards::build_db(rawdeal_cards::load_dir(Path::new(&args[0])).unwrap()).unwrap();
    let matches: u64 = args[1].parse().unwrap();
    let (a, b) = (args[2].clone(), args[3].clone());
    let decks: Vec<_> = db.superstars().iter().filter_map(|s| rawdeal_cards::auto_deck(&db, &s.id)).collect();
    let engine = Engine::new(Arc::new(db));
    let next = AtomicU64::new(0);
    let wins = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    let think = [AtomicU64::new(0), AtomicU64::new(0)];
    let count = [AtomicU64::new(0), AtomicU64::new(0)];
    let slowest = AtomicU64::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= matches {
                        break;
                    }
                    let pair = seed / 2;
                    let mut rng = ChaCha8Rng::seed_from_u64(1000 + pair);
                    let pick = [decks.choose(&mut rng).unwrap().clone(), decks.choose(&mut rng).unwrap().clone()];
                    let swap = seed % 2 == 1;
                    let names = if swap { [&b, &a] } else { [&a, &b] };
                    let (mut state, _) = engine.new_game(&GameSetup::standard(pick), pair).unwrap();
                    let mut bots = [bot(names[0], seed * 2 + 1), bot(names[1], seed * 2 + 2)];
                    let mut actions = 0;
                    while state.outcome.is_none() && actions < 5000 {
                        let p = state.decision.as_ref().unwrap().player;
                        let t = Instant::now();
                        let action = bots[p.index()].choose(&Seat::new(&engine, &state, p));
                        let took = t.elapsed().as_micros() as u64;
                        let side = usize::from((p == PlayerId::ONE) == swap);
                        think[side].fetch_add(took, Ordering::Relaxed);
                        count[side].fetch_add(1, Ordering::Relaxed);
                        slowest.fetch_max(took, Ordering::Relaxed);
                        engine.apply(&mut state, p, action).unwrap();
                        actions += 1;
                    }
                    let i = match state.outcome {
                        Some(Outcome::Win { winner, .. }) => usize::from((winner == PlayerId::ONE) == swap),
                        _ => 2,
                    };
                    wins[i].fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    let w = |i: usize| wins[i].load(Ordering::Relaxed);
    let ms =
        |i: usize| think[i].load(Ordering::Relaxed) as f64 / 1000.0 / count[i].load(Ordering::Relaxed).max(1) as f64;
    let decided = (w(0) + w(1)).max(1) as f64;
    println!(
        "{a} vs {b}: {} - {} ({} other) = {:.1}%; ms/decision {:.2} / {:.2}; slowest {:?}",
        w(0),
        w(1),
        w(2),
        w(0) as f64 * 100.0 / decided,
        ms(0),
        ms(1),
        Duration::from_micros(slowest.load(Ordering::Relaxed))
    );
}
