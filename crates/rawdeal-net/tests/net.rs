//! A hosted match over real TCP connections: two bot players and a spectator.

use std::sync::Arc;
use std::time::Duration;

use rawdeal_ai::RandomBot;
use rawdeal_core::{Mode, PlayerId, RevolutionRules};
use rawdeal_net::{
    APP_VERSION, ClientMsg, Connection, HostCore, HostHandle, MatchInfo, Rejection, Role, ServerMsg, Update,
};

fn host() -> HostHandle {
    let db = Arc::new(rawdeal_cards::test_db());
    let info = MatchInfo { name: "Test".into(), mode: Mode::QuickStart, revolution: RevolutionRules::NONE };
    HostHandle::start(HostCore::new(db, info, 5), 0).expect("host starts")
}

fn hello(role: Role, name: &str) -> ClientMsg {
    let sets = rawdeal_cards::test_db().fingerprints();
    ClientMsg::Hello { app: APP_VERSION.into(), name: name.into(), role, sets }
}

fn next(c: &Connection) -> ServerMsg {
    c.recv_timeout(Duration::from_secs(10)).expect("connected").expect("a message in time")
}

fn join(port: u16, role: Role, name: &str) -> (Connection, Option<PlayerId>) {
    let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
    c.send(hello(role, name)).unwrap();
    match next(&c) {
        ServerMsg::Welcome { seat, .. } => (c, seat),
        other => panic!("not welcomed: {other:?}"),
    }
}

#[test]
fn two_players_and_a_spectator_play_a_match() {
    let mut h = host();
    let port = h.port();
    // The host plays from its own app; the rival and a spectator connect.
    let mut me = h.connect_local();
    me.send(hello(Role::Player, "Host")).unwrap();
    assert!(matches!(next(&me), ServerMsg::Welcome { seat: Some(PlayerId::ONE), .. }));
    let (mut rival, seat) = join(port, Role::Player, "Rival");
    assert_eq!(seat, Some(PlayerId::TWO));
    let (watcher, seat) = join(port, Role::Spectator, "Fan");
    assert_eq!(seat, None);
    let (third, _) = {
        let mut c = Connection::connect(("127.0.0.1", port)).unwrap();
        c.send(hello(Role::Player, "Late")).unwrap();
        (c, ())
    };
    assert_eq!(next(&third), ServerMsg::Rejected(Rejection::Full));

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
fn guests_with_other_versions_or_sets_are_turned_away() {
    let h = host();
    let mut c = Connection::connect(("127.0.0.1", h.port())).unwrap();
    c.send(ClientMsg::Hello { app: "0.0.1".into(), name: "Old".into(), role: Role::Player, sets: vec![] }).unwrap();
    assert!(matches!(next(&c), ServerMsg::Rejected(Rejection::WrongVersion { .. })));

    let mut c = Connection::connect(("127.0.0.1", h.port())).unwrap();
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
    let (mut c, _) = join(h.port(), Role::Player, "P");
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
