//! A hosted match over real TCP connections, direct or through the common
//! server: two bot players and a spectator.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rawdeal_ai::RandomBot;
use rawdeal_core::{Mode, PlayerId, RevolutionRules};
use rawdeal_net::{
    APP_VERSION, ClientMsg, Connection, HostCore, HostHandle, MatchInfo, Rejection, Role, ServerMsg, Update, relay,
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
    ClientMsg::Hello { app: APP_VERSION.into(), name: name.into(), role, sets }
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
fn play(mut me: Connection, mut rival: Connection, watcher: Connection) {
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
}

#[test]
fn a_match_through_the_common_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    std::thread::spawn(move || relay::serve(listener));
    assert!(relay::list(server).unwrap().is_empty());

    let mut h = HostHandle::start_on_server(core(), server, "Ana").unwrap();
    let id = h.match_id().unwrap();
    assert_eq!(h.port(), None);
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Ana")).unwrap();
    assert!(matches!(next(&me), ServerMsg::Welcome { seat: Some(PlayerId::ONE), .. }));

    // The list shows the match and who is in it.
    let listed = |pred: &dyn Fn(&[relay::Listing]) -> bool| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if pred(&relay::list(server).unwrap()) {
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

    let (rival, seat) = welcome(Connection::join_on_server(server, id).unwrap(), Role::Player, "Beto");
    assert_eq!(seat, Some(PlayerId::TWO));
    let (watcher, seat) = welcome(Connection::join_on_server(server, id).unwrap(), Role::Spectator, "Fan");
    assert_eq!(seat, None);
    let lost = Connection::join_on_server(server, id + 100).unwrap();
    assert_eq!(next(&lost), ServerMsg::Rejected(Rejection::NoMatch));
    assert!(listed(&|l| l[0].lobby.as_ref().is_some_and(|lobby| lobby.spectators == ["Fan"])));

    play(me, rival, watcher);

    // When the host leaves, the match leaves the list.
    drop(h);
    assert!(listed(&|l| l.is_empty()));
}

#[test]
fn guests_with_other_versions_or_sets_are_turned_away() {
    let h = host();
    let port = h.port().unwrap();
    let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
    c.send(ClientMsg::Hello { app: "0.0.1".into(), name: "Old".into(), role: Role::Player, sets: vec![] }).unwrap();
    assert!(matches!(next(&c), ServerMsg::Rejected(Rejection::WrongVersion { .. })));

    let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
    let mut sets = rawdeal_cards::test_db().fingerprints();
    sets[0].1 ^= 1;
    sets.push(("homebrew".into(), 1));
    c.send(ClientMsg::Hello { app: APP_VERSION.into(), name: "Other".into(), role: Role::Player, sets }).unwrap();
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
