//! A hosted match over real TCP connections, direct or through the common
//! server: two bot players and a spectator.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rawdeal_ai::RandomBot;
use rawdeal_core::{Mode, Outcome, PlayerId, RevolutionRules, WinMethod};
use rawdeal_net::{
    APP_VERSION, Accounts, ClientMsg, Connection, HostCore, HostHandle, MatchInfo, Refusal, Rejection, Role, ServerMsg,
    Stats, Update, relay, secure,
};

fn core() -> HostCore {
    let db = Arc::new(rawdeal_cards::test_db());
    let info = MatchInfo { name: "Test".into(), mode: Mode::QuickStart, revolution: RevolutionRules::NONE };
    HostCore::new(db, info, 5)
}

fn host() -> HostHandle {
    HostHandle::start(core(), 0).expect("host starts")
}

fn hello(role: Role, name: &str) -> ClientMsg {
    let sets = rawdeal_cards::test_db().fingerprints();
    ClientMsg::Hello { app: APP_VERSION.into(), name: name.into(), role, sets, rejoin: None }
}

fn next(c: &Connection) -> ServerMsg {
    c.recv_timeout(Duration::from_secs(10)).expect("connected").expect("a message in time")
}

fn join(port: u16, role: Role, name: &str) -> (Connection, Option<PlayerId>) {
    welcome(Connection::connect(("127.0.0.1", port)).unwrap(), role, name)
}

fn welcome(mut c: Connection, role: Role, name: &str) -> (Connection, Option<PlayerId>) {
    c.send(hello(role, name)).unwrap();
    match next(&c) {
        ServerMsg::Welcome { seat, .. } => (c, seat),
        other => panic!("not welcomed: {other:?}"),
    }
}

#[test]
fn two_players_and_a_spectator_play_a_match() {
    let mut h = host();
    let port = h.port().unwrap();
    // The host plays from its own app; the rival and a spectator connect.
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Host")).unwrap();
    assert!(matches!(next(&me), ServerMsg::Welcome { seat: Some(PlayerId::ONE), .. }));
    let (rival, seat) = join(port, Role::Player, "Rival");
    assert_eq!(seat, Some(PlayerId::TWO));
    let (watcher, seat) = join(port, Role::Spectator, "Fan");
    assert_eq!(seat, None);
    let (third, _) = {
        let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
        c.send(hello(Role::Player, "Late")).unwrap();
        (c, ())
    };
    assert_eq!(next(&third), ServerMsg::Rejected(Rejection::Full));

    play(me, rival, watcher);
}

/// Sends both decks and lets random bots play the match out.
fn play(mut me: Connection, mut rival: Connection, watcher: Connection) -> Outcome {
    me.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/technician"))).unwrap();
    rival.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/brawler"))).unwrap();

    let mut players = [(me, RandomBot::new(1)), (rival, RandomBot::new(2))];
    let mut last: [Option<Update>; 3] = [None, None, None];
    for _ in 0..20_000 {
        for (i, (c, bot)) in players.iter_mut().enumerate() {
            while let Ok(Some(msg)) = c.try_recv() {
                if let ServerMsg::Update(u) = msg {
                    if !u.legal.is_empty() {
                        c.send(ClientMsg::Act(bot.pick(&u.legal))).unwrap();
                    }
                    last[i] = Some(*u);
                }
            }
        }
        while let Ok(Some(msg)) = watcher.try_recv() {
            if let ServerMsg::Update(u) = msg {
                assert!(u.view.spectator && u.view.hand.is_empty() && u.legal.is_empty());
                for e in &u.events {
                    assert_eq!(*e, e.redacted_for_spectator(), "spectators get no hidden card");
                }
                last[2] = Some(*u);
            }
        }
        if last.iter().all(|u| u.as_ref().is_some_and(|u| u.view.outcome.is_some())) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let outcomes: Vec<_> = last.iter().map(|u| u.as_ref().and_then(|u| u.view.outcome)).collect();
    assert!(outcomes[0].is_some(), "the match ended");
    assert!(outcomes.iter().all(|o| *o == outcomes[0]), "everyone saw the same result");
    // Each player only knows their own hand.
    let p1 = &last[0].as_ref().unwrap().view;
    if p1.opponent_hand.is_none() {
        for &c in &last[1].as_ref().unwrap().view.hand {
            assert!(p1.def_of(c).is_none(), "P1 cannot see P2's hand");
        }
    }
    outcomes[0].unwrap()
}

/// A common server on a free port, with its data in a new folder.
fn server() -> (std::net::SocketAddr, std::path::PathBuf) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("openwwx-test-{}-{n}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let (tls, _) = secure::server_config(&dir).unwrap();
    let accounts = Arc::new(Accounts::open(dir.join("accounts.json")).unwrap());
    let stats = Arc::new(Stats::open(dir.join("results.json")).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || relay::serve(listener, tls, accounts, stats));
    (addr, dir)
}

#[test]
fn accounts_reserve_names() {
    let (server, dir) = server();
    let pin = secure::pin(None);
    let ana = relay::register(server, &pin, "Ana", "secreto1").unwrap().unwrap();
    assert_eq!(ana.name, "Ana");
    assert!(pin.lock().unwrap().is_some(), "the first connection trusts the certificate");
    assert_eq!(relay::register(server, &pin, "ana", "otro1234").unwrap(), Err(Refusal::NameTaken));
    assert_eq!(relay::register(server, &pin, "x", "secreto1").unwrap(), Err(Refusal::BadName));
    assert_eq!(relay::register(server, &pin, "Beto", "123").unwrap(), Err(Refusal::BadPassword));
    assert_eq!(relay::login(server, &pin, "Ana", "mal").unwrap(), Err(Refusal::WrongPassword));
    let again = relay::login(server, &pin, "ANA", "secreto1").unwrap().unwrap();
    assert_eq!(again.name, "Ana");
    assert_ne!(again.token, ana.token);

    // Accounts are kept on disk; a logged-out session no longer works.
    relay::logout(server, &pin, &ana.token).unwrap();
    let saved = Accounts::open(dir.join("accounts.json")).unwrap();
    assert_eq!(saved.session(&again.token).as_deref(), Some("Ana"));
    assert_eq!(saved.session(&ana.token), None);
    saved.reset_password("ana", "nuevo123").unwrap();
    assert_eq!(saved.session(&again.token), None);
    assert!(saved.login("Ana", "nuevo123").is_ok());

    // Without a session, there is no hosting or joining.
    let refused = HostHandle::start_on_server(core(), server, "nope", &pin).err().unwrap();
    assert_eq!(rawdeal_net::tcp::refusal(&refused), Some(Refusal::NotLoggedIn));
    let c = Connection::join_on_server(server, 1, "nope", &pin).unwrap();
    assert_eq!(next(&c), ServerMsg::Rejected(Rejection::NotLoggedIn));
}

#[test]
fn a_server_with_another_certificate_is_refused() {
    let (first, _) = server();
    let pin = secure::pin(None);
    relay::list(first, &pin).unwrap();
    let (second, _) = server();
    let e = relay::list(second, &pin).unwrap_err();
    assert!(e.to_string().contains(secure::CHANGED), "{e}");
}

#[test]
fn a_match_through_the_common_server() {
    let (server, _) = server();
    let pin = secure::pin(None);
    assert!(relay::list(server, &pin).unwrap().is_empty());
    let account = |name: &str| relay::register(server, &pin, name, "secreto1").unwrap().unwrap().token;
    let (ana, beto, fan) = (account("Ana"), account("Beto"), account("Fan"));

    let mut h = HostHandle::start_on_server(core(), server, &ana, &pin).unwrap();
    let id = h.match_id().unwrap();
    assert_eq!(h.port(), None);
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Ana")).unwrap();
    assert!(matches!(next(&me), ServerMsg::Welcome { seat: Some(PlayerId::ONE), .. }));

    // The list shows the match and who is in it.
    let listed = |pred: &dyn Fn(&[relay::Listing]) -> bool| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if pred(&relay::list(server, &pin).unwrap()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    assert!(listed(&|l| l.len() == 1
        && l[0].id == id
        && l[0].host == "Ana"
        && l[0].info.name == "Test"
        && l[0].lobby.as_ref().is_some_and(|lobby| lobby.players[0].is_some())));

    // Guests show up by their account's name, whatever they say.
    let (rival, seat) = welcome(Connection::join_on_server(server, id, &beto, &pin).unwrap(), Role::Player, "Ana");
    assert_eq!(seat, Some(PlayerId::TWO));
    let (watcher, seat) = welcome(Connection::join_on_server(server, id, &fan, &pin).unwrap(), Role::Spectator, "X");
    assert_eq!(seat, None);
    let lost = Connection::join_on_server(server, id + 100, &fan, &pin).unwrap();
    assert_eq!(next(&lost), ServerMsg::Rejected(Rejection::NoMatch));
    assert!(listed(&|l| l[0].lobby.as_ref().is_some_and(
        |lobby| lobby.spectators == ["Fan"] && lobby.players[1].as_ref().is_some_and(|p| p.name == "Beto")
    )));

    let outcome = play(me, rival, watcher);

    // When the host leaves, the match leaves the list.
    drop(h);
    assert!(listed(&|l| l.is_empty()));

    // The result is in both players' history and in the ranking.
    let ana = relay::history(server, &pin, "ana").unwrap();
    assert_eq!(ana.records.len(), 1);
    let record = &ana.records[0];
    assert_eq!(record.players, ["Ana".to_string(), "Beto".to_string()]);
    let ranking = relay::ranking(server, &pin).unwrap();
    assert_eq!(ranking.len(), 2);
    match outcome {
        Outcome::Win { winner, method } => {
            assert_eq!(record.winner, Some(winner.index() as u8));
            assert_eq!(record.method, Some(method));
            assert_eq!(ranking[0].name, if winner == PlayerId::ONE { "Ana" } else { "Beto" });
            assert_eq!((ranking[0].rating, ranking[0].wins, ranking[1].rating, ranking[1].losses), (1016, 1, 984, 1));
        }
        Outcome::Draw => assert_eq!(record.winner, None),
    }
    assert_eq!(relay::history(server, &pin, "Fan").unwrap().standing, None);
}

#[test]
fn a_host_who_leaves_a_match_on_the_server_loses_it() {
    let (server, _) = server();
    let pin = secure::pin(None);
    let account = |name: &str| relay::register(server, &pin, name, "secreto1").unwrap().unwrap().token;
    let (ana, beto) = (account("Ana"), account("Beto"));
    let mut h = HostHandle::start_on_server(core(), server, &ana, &pin).unwrap();
    let id = h.match_id().unwrap();
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Ana")).unwrap();
    let mut rival = Connection::join_on_server(server, id, &beto, &pin).unwrap();
    rival.send(hello(Role::Player, "Beto")).unwrap();
    me.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/technician"))).unwrap();
    rival.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/brawler"))).unwrap();
    wait_for(&rival, |m| matches!(m, ServerMsg::Update(_)));
    // The server must have heard the match started before the host leaves.
    let start = Instant::now();
    while !relay::list(server, &pin).unwrap()[0].lobby.as_ref().is_some_and(|l| l.started) {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(me);
    drop(h);
    let start = Instant::now();
    let beto = loop {
        let h = relay::history(server, &pin, "Beto").unwrap();
        if !h.records.is_empty() {
            break h;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "the result was recorded");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(beto.records[0].winner, Some(1));
    assert_eq!(beto.records[0].method, Some(WinMethod::Forfeit));
    assert_eq!(beto.standing.unwrap().wins, 1);
}

#[test]
fn guests_with_other_versions_or_sets_are_turned_away() {
    let h = host();
    let port = h.port().unwrap();
    let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
    c.send(ClientMsg::Hello {
        app: "0.0.1".into(),
        name: "Old".into(),
        role: Role::Player,
        sets: vec![],
        rejoin: None,
    })
    .unwrap();
    assert!(matches!(next(&c), ServerMsg::Rejected(Rejection::WrongVersion { .. })));

    let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
    let mut sets = rawdeal_cards::test_db().fingerprints();
    sets[0].1 ^= 1;
    sets.push(("homebrew".into(), 1));
    c.send(ClientMsg::Hello { app: APP_VERSION.into(), name: "Other".into(), role: Role::Player, sets, rejoin: None })
        .unwrap();
    match next(&c) {
        ServerMsg::Rejected(Rejection::SetsDiffer { missing, different, extra }) => {
            assert!(missing.is_empty());
            assert_eq!(different.len(), 1);
            assert_eq!(extra, vec!["homebrew".to_string()]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_illegal_deck_is_rejected() {
    let h = host();
    let (mut c, _) = join(h.port().unwrap(), Role::Player, "P");
    let mut deck = rawdeal_cards::test_deck("test/technician");
    deck.arsenal.truncate(10);
    c.send(ClientMsg::Deck(deck)).unwrap();
    loop {
        match next(&c) {
            ServerMsg::Lobby(_) => continue,
            ServerMsg::DeckRejected(problems) => {
                assert!(!problems.is_empty());
                break;
            }
            other => panic!("{other:?}"),
        }
    }
}

/// A match with both decks in, the host's player local and the rival over
/// TCP: the rival's connection and their rejoin token.
fn started(h: &mut HostHandle) -> (Connection, Connection, u64) {
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Host")).unwrap();
    let mut rival = Connection::connect(("127.0.0.1", h.port().unwrap())).unwrap();
    rival.send(hello(Role::Player, "Rival")).unwrap();
    let token = loop {
        if let ServerMsg::Welcome { token, .. } = next(&rival) {
            break token.expect("players get a token");
        }
    };
    me.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/technician"))).unwrap();
    rival.send(ClientMsg::Deck(rawdeal_cards::test_deck("test/brawler"))).unwrap();
    wait_for(&rival, |m| matches!(m, ServerMsg::Update(_)));
    (me, rival, token)
}

/// Reads `c` until a message matches.
fn wait_for(c: &Connection, pred: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
    loop {
        let msg = next(c);
        if pred(&msg) {
            return msg;
        }
    }
}

fn forfeit_win(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::Update(u) if u.view.outcome.is_some_and(|o| matches!(o, Outcome::Win { winner: PlayerId::ONE, method: WinMethod::Forfeit })))
}

#[test]
fn a_player_who_loses_the_connection_gets_back_into_their_seat() {
    let mut h = host();
    let (me, rival, token) = started(&mut h);
    drop(rival);
    wait_for(&me, |m| matches!(m, ServerMsg::Lobby(l) if l.players[1].as_ref().is_some_and(|p| p.away)));

    let mut back = Connection::connect(("127.0.0.1", h.port().unwrap())).unwrap();
    let sets = rawdeal_cards::test_db().fingerprints();
    back.send(ClientMsg::Hello {
        app: APP_VERSION.into(),
        name: "Rival".into(),
        role: Role::Player,
        sets,
        rejoin: Some(token),
    })
    .unwrap();
    assert!(matches!(next(&back), ServerMsg::Welcome { seat: Some(PlayerId::TWO), token: Some(t), .. } if t == token));
    match wait_for(&back, |m| matches!(m, ServerMsg::Update(_))) {
        ServerMsg::Update(u) => assert!(u.view.outcome.is_none() && !u.view.spectator && !u.view.hand.is_empty()),
        _ => unreachable!(),
    }
    wait_for(&me, |m| matches!(m, ServerMsg::Lobby(l) if l.players[1].as_ref().is_some_and(|p| !p.away)));

    // A wrong token is not a way in.
    let (mut other, _) = (Connection::connect(("127.0.0.1", h.port().unwrap())).unwrap(), ());
    let sets = rawdeal_cards::test_db().fingerprints();
    other
        .send(ClientMsg::Hello {
            app: APP_VERSION.into(),
            name: "X".into(),
            role: Role::Player,
            sets,
            rejoin: Some(token ^ 1),
        })
        .unwrap();
    assert_eq!(next(&other), ServerMsg::Rejected(Rejection::Full));
}

#[test]
fn a_player_who_does_not_come_back_loses_by_forfeit() {
    let mut h = HostHandle::start(core().with_abandon_after(Duration::from_millis(300)), 0).unwrap();
    let (me, rival, _) = started(&mut h);
    drop(rival);
    wait_for(&me, forfeit_win);
}

#[test]
fn chat_and_conceding() {
    let mut h = host();
    let (mut me, mut rival, _) = started(&mut h);
    me.send(ClientMsg::Chat("  ¡suerte!  ".into())).unwrap();
    assert_eq!(
        wait_for(&rival, |m| matches!(m, ServerMsg::Chat { .. })),
        ServerMsg::Chat { from: "Host".into(), text: "¡suerte!".into() }
    );
    rival.send(ClientMsg::Concede).unwrap();
    wait_for(&me, forfeit_win);
}
