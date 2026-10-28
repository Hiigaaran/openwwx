//! Text-mode client: play against the random bot.
//!
//! Usage: `rawdeal-cli [--seed N] [--watch] [--level easy|normal|hard] [--advanced | --revolution | --virtual] [--cards DIR [--deck1 FILE | --superstar1 ID] [--deck2 FILE | --superstar2 ID]] [--check]`.
//!
//! - `--advanced`: Pre-match phase and Backlash decks.
//! - `--revolution`: Revolution format (Revolution cards only, Advanced rules).
//!   Its rules can be turned off one by one with `--no-revolution-only`,
//!   `--no-small-backlash`, `--no-ring-limit`, `--no-early-hands` and
//!   `--no-aggression`.
//! - `--virtual`: Advanced rules with the Virtual Backlash and Ring limits.
//! - `--watch`: two bots play each other and the match log is printed.
//! - `--level easy|normal|hard`: how well the computer plays (normal by default).
//! - `--host [PORT]`: host a match online (port 7777 by default) and play it
//!   with your deck (`--deck1`/`--superstar1`); `--join ADDR` plays in someone
//!   else's match and `--spectate ADDR` watches one. `--name NAME` is how the
//!   others see you; with `--watch` a random bot plays your seat. Everyone
//!   needs the same card sets.
//! - `--server ADDR`: go through the common server at ADDR (`openwwx-server`,
//!   port 7778 by default) instead: `--host` lists your match there, and
//!   `--join ID` / `--spectate ID` take the match number from `--list ADDR`,
//!   which prints the open matches.
//! - `--cards DIR`: load card sets from your own folder of `.ron` files instead
//!   of the bundled test set; `--deck1`/`--deck2` are deck list files, or
//!   `--superstar1`/`--superstar2` build a quick deck for that Superstar id.
//! - `--check`: load the cards and decks, report problems, and exit.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rawdeal_ai::{Difficulty, RandomBot};
use rawdeal_core::*;
use rawdeal_net::{
    APP_VERSION, ClientMsg, Connection, DEFAULT_PORT, DEFAULT_SERVER_PORT, HostCore, HostHandle, MatchInfo, Role,
    ServerMsg, relay,
};

/// `addr`, with `port` when it names none.
fn with_port(addr: &str, port: u16) -> String {
    if addr.contains(':') { addr.to_string() } else { format!("{addr}:{port}") }
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let Some(addr) = arg(&args, "--list") {
        let addr = with_port(addr, DEFAULT_SERVER_PORT);
        return match relay::list(addr.as_str()) {
            Ok(list) if list.is_empty() => {
                println!("No open matches on {addr}.");
                ExitCode::SUCCESS
            }
            Ok(list) => {
                for m in list {
                    let lobby = m.lobby.as_ref();
                    let players: Vec<&str> = lobby
                        .map(|l| l.players.iter().flatten().map(|p| p.name.as_str()).collect())
                        .unwrap_or_default();
                    let state = if lobby.is_some_and(|l| l.started) { "playing" } else { "waiting" };
                    println!(
                        "{:>4}  {} ({:?}, OpenWWX {}) {state}; players: {}; spectators: {}",
                        m.id,
                        m.info.name,
                        m.info.mode,
                        m.app,
                        players.join(", "),
                        lobby.map_or(0, |l| l.spectators.len())
                    );
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(format!("cannot reach the server at {addr}: {e}")),
        };
    }
    let server = arg(&args, "--server").map(|a| with_port(a, DEFAULT_SERVER_PORT));
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

    let flag = |name: &str| args.iter().any(|a| a == name);
    let mut rules = mode.revolution_rules();
    rules.revolution_cards_only &= !flag("--no-revolution-only");
    rules.small_backlash &= !flag("--no-small-backlash");
    rules.ring_limit &= !flag("--no-ring-limit");
    rules.hands_before_pre_match &= !flag("--no-early-hands");
    rules.aggression &= !flag("--no-aggression");

    let online = ["--host", "--join", "--spectate"].iter().any(|f| flag(f));
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
                    match rawdeal_cards::auto_deck_with(&db, id, mode, rules) {
                        Some(d) => decks.push(d),
                        None => return fail(format!("cannot build a deck for {id}")),
                    }
                } else if args.iter().any(|a| a == "--check") {
                    for s in db.superstars() {
                        let ok = rawdeal_cards::auto_deck_with(&db, &s.id, mode, rules).is_some();
                        println!("  {} ({}): {}", s.name, s.id, if ok { "auto deck OK" } else { "no legal auto deck" });
                    }
                    return ExitCode::SUCCESS;
                } else if online && !decks.is_empty() {
                    // Online you only bring your own deck.
                    decks.push(decks[0].clone());
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
            let mut problems = validate_deck_with(&db, deck, mode, rules);
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

    let player = arg(&args, "--name").unwrap_or("Player").to_string();
    if flag("--host") {
        let port = arg(&args, "--host").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
        let info = MatchInfo { name: format!("{player}'s match"), mode, revolution: rules };
        let db = Arc::new(db);
        let core = HostCore::new(db.clone(), info, seed);
        let mut host = match &server {
            Some(server) => match HostHandle::start_on_server(core, server.as_str(), &player) {
                Ok(h) => h,
                Err(e) => return fail(format!("cannot host on the server at {server}: {e}")),
            },
            None => match HostHandle::start(core, port) {
                Ok(h) => h,
                Err(e) => return fail(format!("cannot host on port {port}: {e}")),
            },
        };
        match (host.port(), host.match_id()) {
            (Some(port), _) => println!("Hosting on port {port}. Waiting for a rival..."),
            (_, Some(id)) => println!("Listed as match {id} on the server. Waiting for a rival..."),
            _ => {}
        }
        let conn = host.connect_local();
        let [deck, _] = decks;
        return play_online(conn, &db, &player, Some(deck), watch, seed);
    }
    for (option, role) in [("--join", Role::Player), ("--spectate", Role::Spectator)] {
        if let Some(addr) = arg(&args, option) {
            let conn = match &server {
                Some(server) => {
                    let Ok(id) = addr.parse() else { return fail(format!("{addr} is not a match number")) };
                    Connection::join_on_server(server.as_str(), id)
                        .map_err(|e| format!("cannot reach the server at {server}: {e}"))
                }
                None => {
                    let addr = with_port(addr, DEFAULT_PORT);
                    Connection::connect(addr.as_str()).map_err(|e| format!("cannot connect to {addr}: {e}"))
                }
            };
            let conn = match conn {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            let [deck, _] = decks;
            let deck = (role == Role::Player).then_some(deck);
            return play_online(conn, &db, &player, deck, watch, seed);
        }
    }

    let engine = Engine::new(Arc::new(db));
    let setup = GameSetup { mode, revolution: Some(rules), ..GameSetup::standard(decks) };
    let (mut state, events) = match engine.new_game(&setup, seed) {
        Ok(started) => started,
        Err(e) => return fail(e),
    };
    let name = |i: usize| engine.db().superstar(state.player(PlayerId(i as u8)).superstar).name.clone();
    println!("OpenWWX, seed {seed}. You are P1 ({}) against P2 ({}).", name(0), name(1));

    let human = if watch { None } else { Some(PlayerId::ONE) };
    print_events(&Cards { db: engine.db(), def: &local(&state) }, &events, human.unwrap_or(PlayerId::ONE));

    let level = match arg(&args, "--level") {
        None | Some("normal") => Difficulty::Normal,
        Some("easy") => Difficulty::Easy,
        Some("hard") => Difficulty::Hard,
        Some(other) => return fail(format!("unknown level {other}: use easy, normal or hard")),
    };
    let mut bots = [level.bot(seed ^ 1), level.bot(seed ^ 2)];
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while state.outcome.is_none() {
        let player = state.decision.as_ref().expect("live game").player;
        let legal = engine.legal_actions(&state, player);
        let action = if Some(player) == human {
            let view = engine.view(&state, player);
            print_view(engine.db(), &view);
            let c = Cards { db: engine.db(), def: &local(&state) };
            for (i, a) in legal.iter().enumerate() {
                println!("  {i:>2}) {}", describe(&c, &view, a));
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
            bots[player.index()].choose(&Seat::new(&engine, &state, player))
        };
        let events = engine.apply(&mut state, player, action).expect("only legal actions are offered");
        print_events(&Cards { db: engine.db(), def: &local(&state) }, &events, human.unwrap_or(PlayerId::ONE));
    }
    ExitCode::SUCCESS
}

/// Plays (or watches, without a deck) a match hosted elsewhere; `auto` lets
/// a random bot play your seat.
fn play_online(
    mut conn: Connection,
    db: &CardDb,
    name: &str,
    mut deck: Option<DeckList>,
    auto: bool,
    seed: u64,
) -> ExitCode {
    let role = if deck.is_some() { Role::Player } else { Role::Spectator };
    let hello = ClientMsg::Hello { app: APP_VERSION.into(), name: name.into(), role, sets: db.fingerprints() };
    if let Err(e) = conn.send(hello) {
        return fail(e);
    }
    let mut known: HashMap<CardId, DefIdx> = HashMap::new();
    let mut bot = RandomBot::new(seed);
    let mut last_legal = Vec::new();
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        let msg = match conn.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(msg)) => msg,
            Ok(None) => continue,
            Err(_) => {
                println!("The host closed the match.");
                return ExitCode::SUCCESS;
            }
        };
        let mut legal = Vec::new();
        match msg {
            ServerMsg::Welcome { seat, info } => {
                let seat = seat.map_or("a spectator".to_string(), |p| p.to_string());
                println!("Joined \"{}\" ({:?}) as {seat}.", info.name, info.mode);
                if let Some(deck) = deck.take() {
                    conn.send(ClientMsg::Deck(deck)).ok();
                }
            }
            ServerMsg::Rejected(r) => return fail(format!("the host turned you away: {r:?}")),
            ServerMsg::Lobby(lobby) => {
                let slot = |i: usize| {
                    lobby.players[i].as_ref().map_or("(free)".to_string(), |p| {
                        format!("{}{}", p.name, if p.ready { " (ready)" } else { "" })
                    })
                };
                if !lobby.started {
                    println!("P1: {} | P2: {} | spectators: {}", slot(0), slot(1), lobby.spectators.len());
                }
            }
            ServerMsg::DeckRejected(problems) => return fail(format!("your deck is not legal here: {problems:?}")),
            ServerMsg::ActionRejected(e) => {
                println!("Not legal: {e}");
                legal = std::mem::take(&mut last_legal);
            }
            ServerMsg::Update(u) => {
                known.extend(u.cards.iter().copied());
                let view = &u.view;
                let seen = view.players.iter().flat_map(|p| p.ring.iter().chain(&p.ringside)).chain(&view.hand);
                for &c in seen {
                    if let Some(d) = view.def_of(c) {
                        known.insert(c, d);
                    }
                }
                let def = |c: CardId| known.get(&c).copied();
                print_events(&Cards { db, def: &def }, &u.events, view.me);
                if view.outcome.is_some() {
                    return ExitCode::SUCCESS;
                }
                legal = u.legal.clone();
                if !legal.is_empty() && !auto {
                    print_view(db, view);
                    for (i, a) in legal.iter().enumerate() {
                        println!("  {i:>2}) {}", describe(&Cards { db, def: &def }, view, a));
                    }
                }
            }
        }
        if legal.is_empty() {
            continue;
        }
        let action = if auto {
            bot.pick(&legal)
        } else {
            loop {
                print!("> ");
                io::stdout().flush().ok();
                let Some(Ok(line)) = lines.next() else { return ExitCode::SUCCESS };
                match line.trim().parse::<usize>().ok().and_then(|i| legal.get(i)) {
                    Some(a) => break a.clone(),
                    None => println!("Pick a number from 0 to {}.", legal.len() - 1),
                }
            }
        };
        last_legal = legal;
        conn.send(ClientMsg::Act(action)).ok();
    }
}

/// Every card's definition, for a match played here.
fn local(state: &GameState) -> impl Fn(CardId) -> Option<DefIdx> + '_ {
    |card| Some(state.card(card).def)
}

/// Card names as one player knows them.
struct Cards<'a> {
    db: &'a CardDb,
    def: &'a dyn Fn(CardId) -> Option<DefIdx>,
}

fn name(c: &Cards, card: CardId) -> String {
    match (c.def)(card) {
        Some(d) => {
            let def = c.db.card(d);
            format!("{} [F{} D{} SV{}]", def.title, def.fortitude, def.damage, def.stun_value)
        }
        None => "a hidden card".into(),
    }
}

fn names(c: &Cards, cards: &[CardId]) -> String {
    if cards.is_empty() {
        return "(none)".into();
    }
    cards.iter().map(|&card| name(c, card)).collect::<Vec<_>>().join(", ")
}

fn print_view(db: &CardDb, view: &PlayerView) {
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

fn describe(c: &Cards, view: &PlayerView, action: &Action) -> String {
    match action {
        Action::PlayCard { card, kind } => format!("Play {} as {kind:?}", name(c, *card)),
        Action::EndTurn => "End your turn".into(),
        Action::UseAbility => {
            let superstar = c.db.superstar(view.players[view.active.index()].superstar);
            format!("Use {}'s ability: {}", superstar.name, superstar.text)
        }
        Action::UseRingCard { card } => format!("Use {} in play", name(c, *card)),
        Action::UseBackstageCard { card } => format!("Use {} in your Backstage Area", c.db.card(*card).title),
        Action::Reverse { card } => format!("Reverse with {}", name(c, *card)),
        Action::DeclineReversal => "Don't reverse".into(),
        Action::ReverseByVolley => "Reverse the Volley (remove random Ringside cards)".into(),
        Action::ChooseCards(cards) => format!("Choose {}", names(c, cards)),
        Action::Yes => "Yes".into(),
        Action::No => "No".into(),
        Action::DrawCards(n) => format!("Draw {n}"),
        Action::Pass => "Pass".into(),
        Action::ChooseOption(i) => match &view.decision {
            Some(Decision { kind: DecisionKind::ChooseOption { options, .. }, .. }) => {
                let parts: Vec<String> = options[*i as usize].iter().map(|e| format!("{e:?}")).collect();
                format!("Choose: {}", parts.join(", "))
            }
            _ => format!("Option {i}"),
        },
    }
}

fn print_events(c: &Cards, events: &[Event], viewer: PlayerId) {
    for event in events {
        let line = match event.redacted_for(viewer) {
            Event::PreMatchStarted { first } => format!("Pre-match phase. {first} plays first."),
            Event::PreMatchEnded => "Pre-match phase over.".into(),
            Event::GameStarted { first } => format!("{first} goes first."),
            Event::PartialReversal { player, reversal, card } => format!(
                "{player} hits {} with {}, but it is Multi: it needs another Reversal!",
                name(c, card),
                name(c, reversal)
            ),
            Event::Revealed { player, card } => {
                format!("{player} reveals {} and puts it in hand.", name(c, card))
            }
            Event::TurnStarted { player, turn } => format!("\n=== Turn {turn}: {player} ==="),
            Event::CardDrawn { player, card: Some(x) } => format!("{player} draws {}.", name(c, x)),
            Event::CardDrawn { player, card: None } => format!("{player} draws a card."),
            Event::CardPlayed { player, card, .. } => format!("{player} plays {}.", name(c, card)),
            Event::Reversed { player, reversal, reversed, from_hand } => format!(
                "{player} REVERSES {} with {}{}!",
                name(c, reversed),
                name(c, reversal),
                if from_hand { "" } else { " (overturned while taking damage)" }
            ),
            Event::Overturned { player, card } => format!("  {player} takes 1 damage: {}.", name(c, card)),
            Event::CountOutRefused { player } => format!("{player} refuses the count out and goes for the pin."),
            Event::TurnEnded { player } => format!("{player} ends the turn."),
            Event::TurnContinues { player } => format!("{player} continues the turn."),
            Event::Aggression { player } => {
                format!("{player} tried no maneuver and overturns 6 cards (Aggression rule).")
            }
            Event::TurnSkipped { player } => format!("{player} skips the turn."),
            Event::HandShown { player, viewer: v, cards } if v == viewer => {
                format!("{player}'s hand: {}", names(c, &cards))
            }
            Event::CardMoved { player, card: Some(x), to: Zone::Ringside, from: Zone::Arsenal } => {
                format!("{player} puts {} from the Arsenal into Ringside.", name(c, x))
            }
            Event::GameOver(Outcome::Win { winner, method }) => format!("\n*** {winner} wins by {method:?}! ***"),
            Event::GameOver(Outcome::Draw) => "\n*** Draw! ***".into(),
            Event::CardMoved { player, card: Some(x), to: Zone::Ringside, from: Zone::Hand } => {
                format!("{player} discards {}.", name(c, x))
            }
            _ => continue,
        };
        println!("{line}");
    }
}
