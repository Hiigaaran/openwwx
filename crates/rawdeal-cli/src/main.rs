//! Text-mode client: play against the random bot.
//!
//! Usage: `rawdeal-cli [--seed N] [--watch] [--advanced | --revolution | --virtual] [--cards DIR [--deck1 FILE | --superstar1 ID] [--deck2 FILE | --superstar2 ID]] [--check]`.
//!
//! - `--advanced`: Pre-match phase and Backlash decks.
//! - `--revolution`: Revolution format (Revolution cards only, Advanced rules).
//! - `--virtual`: Advanced rules with the Virtual Backlash and Ring limits.
//! - `--watch`: two bots play each other and the match log is printed.
//! - `--cards DIR`: load card sets from your own folder of `.ron` files instead
//!   of the bundled test set; `--deck1`/`--deck2` are deck list files, or
//!   `--superstar1`/`--superstar2` build a quick deck for that Superstar id.
//! - `--check`: load the cards and decks, report problems, and exit.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::*;

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let seed = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    let watch = args.iter().any(|a| a == "--watch");
    let mode = if args.iter().any(|a| a == "--revolution") {
        Mode::Revolution
    } else if args.iter().any(|a| a == "--virtual") {
        Mode::Virtual
    } else if args.iter().any(|a| a == "--advanced") {
        Mode::Advanced
    } else {
        Mode::QuickStart
    };

    let (db, decks) = match arg(&args, "--cards") {
        None => (
            rawdeal_cards::test_db(),
            [rawdeal_cards::test_deck("test/technician"), rawdeal_cards::test_deck("test/brawler")],
        ),
        Some(dir) => {
            let db = match rawdeal_cards::load_dir(Path::new(dir)).and_then(rawdeal_cards::build_db) {
                Ok(db) => db,
                Err(e) => return fail(e),
            };
            println!("Loaded {} cards and {} Superstars from {dir}.", db.cards().len(), db.superstars().len());
            let mut decks = Vec::new();
            for (flag, star) in [("--deck1", "--superstar1"), ("--deck2", "--superstar2")] {
                if let Some(file) = arg(&args, flag) {
                    match rawdeal_cards::load_deck(Path::new(file)) {
                        Ok(d) => decks.push(d),
                        Err(e) => return fail(e),
                    }
                } else if let Some(id) = arg(&args, star) {
                    match rawdeal_cards::auto_deck_in(&db, id, mode) {
                        Some(d) => decks.push(d),
                        None => return fail(format!("cannot build a deck for {id}")),
                    }
                } else if args.iter().any(|a| a == "--check") {
                    for s in db.superstars() {
                        let ok = rawdeal_cards::auto_deck_in(&db, &s.id, mode).is_some();
                        println!("  {} ({}): {}", s.name, s.id, if ok { "auto deck OK" } else { "no legal auto deck" });
                    }
                    return ExitCode::SUCCESS;
                } else {
                    return fail(format!("{flag} or {star} is required with --cards"));
                }
            }
            (db, decks.try_into().expect("two decks"))
        }
    };
    if args.iter().any(|a| a == "--check") {
        let mut ok = true;
        for (i, deck) in decks.iter().enumerate() {
            let mut problems = validate_deck_in(&db, deck, mode);
            if !mode.is_advanced() {
                problems.retain(|p| !matches!(p, DeckProblem::NotABacklashCard { .. }));
            }
            ok &= problems.is_empty();
            println!(
                "Deck {}: {}",
                i + 1,
                if problems.is_empty() { "legal".to_string() } else { format!("{problems:?}") }
            );
        }
        return if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }

    let engine = Engine::new(Arc::new(db));
    let setup = GameSetup { mode, ..GameSetup::standard(decks) };
    let (mut state, events) = match engine.new_game(&setup, seed) {
        Ok(started) => started,
        Err(e) => return fail(e),
    };
    let name = |i: usize| engine.db().superstar(state.player(PlayerId(i as u8)).superstar).name.clone();
    println!("OpenWWX, seed {seed}. You are P1 ({}) against P2 ({}).", name(0), name(1));

    let human = if watch { None } else { Some(PlayerId::ONE) };
    print_events(&engine, &state, &events, human.unwrap_or(PlayerId::ONE));

    let mut bots = [RandomBot::new(seed ^ 1), RandomBot::new(seed ^ 2)];
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while state.outcome.is_none() {
        let player = state.decision.as_ref().expect("live game").player;
        let legal = engine.legal_actions(&state, player);
        let action = if Some(player) == human {
            print_view(&engine, &engine.view(&state, player));
            for (i, a) in legal.iter().enumerate() {
                println!("  {i:>2}) {}", describe(&engine, &state, a));
            }
            loop {
                print!("> ");
                io::stdout().flush().ok();
                let Some(Ok(line)) = lines.next() else { return ExitCode::SUCCESS };
                match line.trim().parse::<usize>().ok().and_then(|i| legal.get(i)) {
                    Some(a) => break a.clone(),
                    None => println!("Pick a number from 0 to {}.", legal.len() - 1),
                }
            }
        } else {
            bots[player.index()].choose(&engine, &engine.view(&state, player), &legal)
        };
        let events = engine.apply(&mut state, player, action).expect("only legal actions are offered");
        print_events(&engine, &state, &events, human.unwrap_or(PlayerId::ONE));
    }
    ExitCode::SUCCESS
}

fn name(engine: &Engine, state: &GameState, card: CardId) -> String {
    let def = engine.db().card(state.card(card).def);
    format!("{} [F{} D{} SV{}]", def.title, def.fortitude, def.damage, def.stun_value)
}

fn names(engine: &Engine, state: &GameState, cards: &[CardId]) -> String {
    if cards.is_empty() {
        return "(none)".into();
    }
    cards.iter().map(|&c| name(engine, state, c)).collect::<Vec<_>>().join(", ")
}

fn print_view(engine: &Engine, view: &PlayerView) {
    let db = engine.db();
    let title = |c: &CardId| view.def_of(*c).map(|d| db.card(d).title.clone()).unwrap_or("?".into());
    println!();
    for (label, p) in [("Opponent", view.opponent()), ("You", view.me_public())] {
        let ring: Vec<String> = p.ring.iter().map(title).collect();
        println!(
            "{label}: {} | Fortitude {} | Arsenal {} | Hand {} | Ringside {} | Ring: {}",
            db.superstar(p.superstar).name,
            p.fortitude,
            p.arsenal_count,
            p.hand_count,
            p.ringside.len(),
            if ring.is_empty() { "(empty)".into() } else { ring.join(", ") }
        );
    }
}

fn describe(engine: &Engine, state: &GameState, action: &Action) -> String {
    match action {
        Action::PlayCard { card, kind } => format!("Play {} as {kind:?}", name(engine, state, *card)),
        Action::EndTurn => "End your turn".into(),
        Action::UseAbility => {
            let superstar = engine.db().superstar(state.player(state.active).superstar);
            format!("Use {}'s ability: {}", superstar.name, superstar.text)
        }
        Action::UseRingCard { card } => format!("Use {} in play", name(engine, state, *card)),
        Action::UseBackstageCard { card } => format!("Use {} in your Backstage Area", engine.db().card(*card).title),
        Action::Reverse { card } => format!("Reverse with {}", name(engine, state, *card)),
        Action::DeclineReversal => "Don't reverse".into(),
        Action::ReverseByVolley => "Reverse the Volley (remove random Ringside cards)".into(),
        Action::ChooseCards(cards) => format!("Choose {}", names(engine, state, cards)),
        Action::Yes => "Yes".into(),
        Action::No => "No".into(),
        Action::DrawCards(n) => format!("Draw {n}"),
        Action::Pass => "Pass".into(),
        Action::ChooseOption(i) => match &state.decision {
            Some(Decision { kind: DecisionKind::ChooseOption { options, .. }, .. }) => {
                let parts: Vec<String> = options[*i as usize].iter().map(|e| format!("{e:?}")).collect();
                format!("Choose: {}", parts.join(", "))
            }
            _ => format!("Option {i}"),
        },
    }
}

fn print_events(engine: &Engine, state: &GameState, events: &[Event], viewer: PlayerId) {
    for event in events {
        let line = match event.redacted_for(viewer) {
            Event::PreMatchStarted { first } => format!("Pre-match phase. {first} plays first."),
            Event::PreMatchEnded => "Pre-match phase over.".into(),
            Event::GameStarted { first } => format!("{first} goes first."),
            Event::PartialReversal { player, reversal, card } => format!(
                "{player} hits {} with {}, but it is Multi: it needs another Reversal!",
                name(engine, state, card),
                name(engine, state, reversal)
            ),
            Event::Revealed { player, card } => {
                format!("{player} reveals {} and puts it in hand.", name(engine, state, card))
            }
            Event::TurnStarted { player, turn } => format!("\n=== Turn {turn}: {player} ==="),
            Event::CardDrawn { player, card: Some(c) } => format!("{player} draws {}.", name(engine, state, c)),
            Event::CardDrawn { player, card: None } => format!("{player} draws a card."),
            Event::CardPlayed { player, card, .. } => format!("{player} plays {}.", name(engine, state, card)),
            Event::Reversed { player, reversal, reversed, from_hand } => format!(
                "{player} REVERSES {} with {}{}!",
                name(engine, state, reversed),
                name(engine, state, reversal),
                if from_hand { "" } else { " (overturned while taking damage)" }
            ),
            Event::Overturned { player, card } => format!("  {player} takes 1 damage: {}.", name(engine, state, card)),
            Event::CountOutRefused { player } => format!("{player} refuses the count out and goes for the pin."),
            Event::TurnEnded { player } => format!("{player} ends the turn."),
            Event::TurnContinues { player } => format!("{player} continues the turn."),
            Event::Aggression { player } => {
                format!("{player} tried no maneuver and overturns 6 cards (Aggression rule).")
            }
            Event::TurnSkipped { player } => format!("{player} skips the turn."),
            Event::HandShown { player, viewer: v, cards } if v == viewer => {
                format!("{player}'s hand: {}", names(engine, state, &cards))
            }
            Event::CardMoved { player, card: Some(c), to: Zone::Ringside, from: Zone::Arsenal } => {
                format!("{player} puts {} from the Arsenal into Ringside.", name(engine, state, c))
            }
            Event::GameOver(Outcome::Win { winner, method }) => format!("\n*** {winner} wins by {method:?}! ***"),
            Event::GameOver(Outcome::Draw) => "\n*** Draw! ***".into(),
            Event::CardMoved { player, card: Some(c), to: Zone::Ringside, from: Zone::Hand } => {
                format!("{player} discards {}.", name(engine, state, c))
            }
            _ => continue,
        };
        println!("{line}");
    }
}
