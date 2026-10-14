//! Text-mode client: play the test set against the random bot.
//!
//! Usage: `rawdeal-cli [--seed N] [--watch] [--advanced]`. With `--advanced`
//! the match has a Pre-match phase and Backlash decks. With `--watch` two bots play
//! each other and the match log is printed.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    let watch = args.iter().any(|a| a == "--watch");

    let engine = Engine::new(Arc::new(rawdeal_cards::test_db()));
    let decks = [rawdeal_cards::test_deck("test/technician"), rawdeal_cards::test_deck("test/brawler")];
    let setup =
        if args.iter().any(|a| a == "--advanced") { GameSetup::advanced(decks) } else { GameSetup::standard(decks) };
    let (mut state, events) = engine.new_game(&setup, seed).expect("test decks are legal");
    println!("OpenWWX, seed {seed}. You are P1 (The Technician) against P2 (The Brawler).");
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
                let Some(Ok(line)) = lines.next() else { return };
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
        Action::Reverse { card } => format!("Reverse with {}", name(engine, state, *card)),
        Action::DeclineReversal => "Don't reverse".into(),
        Action::ChooseCards(cards) => format!("Choose {}", names(engine, state, cards)),
        Action::Yes => "Yes".into(),
        Action::No => "No".into(),
        Action::DrawCards(n) => format!("Draw {n}"),
        Action::Pass => "Pass".into(),
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
