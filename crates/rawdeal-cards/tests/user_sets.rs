//! Plays random matches with the card sets in `$OPENWWX_CARDS`, if set.
//! Real card data is not in the repository, so this is skipped in CI:
//! `OPENWWX_CARDS=~/raw-deal/sets cargo test -p rawdeal-cards --test user_sets`

use std::path::PathBuf;
use std::sync::Arc;

use rawdeal_ai::RandomBot;
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
                    let action = bots[p.index()].pick(&legal);
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

/// Revolution format matches between the Revolution Superstars, with legal decks.
#[test]
fn user_sets_revolution_format() {
    use rawdeal_core::Mode;
    let Some(dir) = std::env::var_os("OPENWWX_CARDS").map(PathBuf::from) else {
        eprintln!("OPENWWX_CARDS not set; skipping");
        return;
    };
    let db = rawdeal_cards::build_db(rawdeal_cards::load_dir(&dir).unwrap()).unwrap();
    let stars: Vec<_> = db.superstars().iter().filter(|s| rawdeal_core::is_revolution(&s.name)).collect();
    let decks: Vec<_> =
        stars.iter().filter_map(|s| rawdeal_cards::auto_deck_in(&db, &s.id, Mode::Revolution)).collect();
    assert_eq!(decks.len(), stars.len(), "every Revolution Superstar gets a legal Revolution deck");
    let engine = Engine::new(Arc::new(db));
    let mut games = 0;
    for (i, a) in decks.iter().enumerate() {
        for b in &decks[i..] {
            for seed in 0..10u64 {
                let setup = GameSetup { mode: Mode::Revolution, ..GameSetup::standard([a.clone(), b.clone()]) };
                let (mut state, _) = engine.new_game(&setup, seed).unwrap();
                let mut bots = [RandomBot::new(seed), RandomBot::new(seed + 1)];
                let mut actions = 0;
                while state.outcome.is_none() {
                    let p = state.decision.as_ref().unwrap().player;
                    let legal = engine.legal_actions(&state, p);
                    let action = bots[p.index()].pick(&legal);
                    engine.apply(&mut state, p, action).unwrap();
                    state.check_invariants().unwrap();
                    actions += 1;
                    assert!(actions < 20_000, "{} vs {} seed {seed} did not finish", a.superstar, b.superstar);
                }
                games += 1;
            }
        }
    }
    eprintln!("{games} Revolution matches finished");
}

/// Random 60-card piles (deck rules off) so that every scripted card gets
/// played, in both Quick Start and Advanced matches.
#[test]
fn user_sets_every_card_in_random_piles() {
    use rand::SeedableRng;
    use rand::seq::IndexedRandom;
    use rawdeal_core::{DeckList, Event, Mode};

    let Some(dir) = std::env::var_os("OPENWWX_CARDS").map(PathBuf::from) else {
        eprintln!("OPENWWX_CARDS not set; skipping");
        return;
    };
    let db = rawdeal_cards::build_db(rawdeal_cards::load_dir(&dir).unwrap()).unwrap();
    let stars: Vec<String> = db.superstars().iter().map(|s| s.id.clone()).collect();
    let engine = Engine::new(Arc::new(db));
    let db = engine.db();
    let pile = |star: &str, rng: &mut rand_chacha::ChaCha8Rng| {
        let pool = |f: &dyn Fn(&rawdeal_core::CardDef) -> bool| -> Vec<String> {
            db.cards()
                .iter()
                .filter(|c| {
                    let def = db.superstar(db.superstar_by_id(star).unwrap());
                    c.superstar.is_none() || c.logos().any(|s| rawdeal_core::may_pack_logo(def, s))
                })
                .filter(|c| f(c))
                .map(|c| c.id.clone())
                .collect()
        };
        let main = pool(&|c| !c.is_backlash() && !c.is_backstage());
        let pre = pool(&|c| c.is_pre_match());
        let mid = pool(&|c| c.is_backlash() && !c.is_pre_match());
        let arsenal = (0..60).map(|_| main.choose(rng).unwrap().clone()).collect();
        let mut backlash = Vec::new();
        for list in [&pre, &mid] {
            if !list.is_empty() {
                backlash.extend((0..10).map(|_| list.choose(rng).unwrap().clone()));
            }
        }
        let mut backstage = rawdeal_cards::backstage(db, db.superstar(db.superstar_by_id(star).unwrap()));
        let extra = pool(&|c| c.kinds.contains(&rawdeal_core::CardKind::Backstage));
        if !extra.is_empty() {
            for _ in 0..3 {
                let card = extra.choose(rng).unwrap().clone();
                if !backstage.contains(&card) {
                    backstage.push(card);
                }
            }
        }
        DeckList { superstar: star.to_string(), arsenal, backlash, backstage }
    };
    let mut played = std::collections::BTreeSet::new();
    let seeds = std::env::var("OPENWWX_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(5000u64);
    let from = std::env::var("OPENWWX_SEED_FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(0u64);
    for seed in from..seeds {
        if std::env::var_os("OPENWWX_TRACE").is_some() {
            eprintln!("seed {seed}");
        }
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let a = stars.choose(&mut rng).unwrap().clone();
        let b = stars.choose(&mut rng).unwrap().clone();
        let mode = match seed % 4 {
            0 => Mode::QuickStart,
            1 => Mode::Advanced,
            2 => Mode::Revolution,
            _ => Mode::Virtual,
        };
        let setup = GameSetup {
            decks: [pile(&a, &mut rng), pile(&b, &mut rng)],
            enforce_deck_rules: false,
            shuffle_arsenals: true,
            mode,
            revolution: None,
        };
        let (mut state, _) = engine.new_game(&setup, seed).unwrap();
        let mut bots = [RandomBot::new(seed), RandomBot::new(seed + 1)];
        let mut actions = 0;
        while state.outcome.is_none() {
            let p = state.decision.as_ref().unwrap().player;
            let legal = engine.legal_actions(&state, p);
            assert!(legal.len() < 5_000, "{} legal actions", legal.len());
            let action = bots[p.index()].pick(&legal);
            for ev in engine.apply(&mut state, p, action).unwrap() {
                // A Superstar ability can reverse by discarding any card: only count real Reversals.
                let card = match ev {
                    Event::CardPlayed { card, .. } => Some(card),
                    Event::Reversed { reversal, .. } if db.card(state.card(reversal).def).is_reversal() => {
                        Some(reversal)
                    }
                    _ => None,
                };
                if let Some(card) = card {
                    played.insert(db.card(state.card(card).def).title.clone());
                }
            }
            state.check_invariants().unwrap();
            actions += 1;
            assert!(actions < 20_000, "{a} vs {b} seed {seed} did not finish");
        }
    }
    let never: Vec<&str> = db.cards().iter().map(|c| c.title.as_str()).filter(|t| !played.contains(*t)).collect();
    eprintln!("{} card titles played; never played: {never:?}", played.len());
}
