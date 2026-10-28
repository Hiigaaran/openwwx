//! Scenario tests for the Virtual rules: Backstage cards, the Allegiance
//! step and the Virtual Backlash and Ring limits. The cards are invented for
//! the tests.

use std::sync::Arc;

use rawdeal_core::*;

const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

const SET: &str = r#"(
    name: "Virtual tests",
    superstars: [
        (id: "v/first", name: "First", hand_size: 5, superstar_value: 5),
        (id: "v/second", name: "Second", hand_size: 5, superstar_value: 1),
        (id: "v/tough", name: "Tough", hand_size: 5, superstar_value: 1, abilities: [ReduceDamage(2)]),
    ],
    cards: [
        (id: "v/filler", title: "Filler", kinds: [Action], fortitude: 0, damage: 0),
        (id: "v/hit", title: "Hit", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3),
        (id: "v/crowd", title: "Crowd", kinds: [Backstage], fortitude: 0, damage: 0,
            script: (backstage_abilities: [ReduceDamage(2)])),
        (id: "v/favor", title: "Favor", kinds: [Backstage], fortitude: 0, damage: 0,
            script: (backstage_ability: Some((sacrifice: true, effects: [Draw(who: You, n: 2)])))),
        (id: "v/secret", title: "Secret", kinds: [Backstage], fortitude: 0, damage: 0,
            script: (starts_hidden: true, backstage_abilities: [ReduceDamage(2)])),
        (id: "v/side", title: "Side", kinds: [PreMatch(Allegiance)], fortitude: 0, damage: 0,
            script: (on_play: [Draw(who: You, n: 1)])),
        (id: "v/event", title: "Event", kinds: [PreMatch(Event)], fortitude: 0, damage: 0),
        (id: "v/belt", title: "Belt", kinds: [PreMatch(Event)], fortitude: 0, damage: 0,
            script: (blanks_opponent_ability: true, blanks_when: Some(OpponentSuperstar("Tough")))),
        (id: "v/other-belt", title: "Other Belt", kinds: [PreMatch(Event)], fortitude: 0, damage: 0,
            script: (blanks_opponent_ability: true, blanks_when: Some(OpponentSuperstar("Nobody")))),
    ],
)"#;

fn engine() -> Engine {
    let set = rawdeal_cards::parse_set(SET).expect("test set parses");
    Engine::new(Arc::new(rawdeal_cards::build_db([set]).expect("consistent")))
}

fn deck(superstar: &str, cards: &[&str], backlash: &[&str], backstage: &[&str]) -> DeckList {
    let ids = |l: &[&str]| l.iter().map(|c| format!("v/{c}")).collect::<Vec<_>>();
    let mut arsenal = ids(cards);
    arsenal.resize(30.max(cards.len()), "v/filler".to_string());
    DeckList { superstar: format!("v/{superstar}"), arsenal, backlash: ids(backlash), backstage: ids(backstage) }
}

fn start(e: &Engine, mode: Mode, p1: DeckList, p2: DeckList) -> GameState {
    let setup =
        GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false, mode, revolution: None };
    e.new_game(&setup, 1).expect("valid setup").0
}

fn apply(e: &Engine, s: &mut GameState, p: PlayerId, a: Action) {
    e.apply(s, p, a).expect("legal action");
    s.check_invariants().unwrap();
}

fn def(e: &Engine, id: &str) -> DefIdx {
    e.db().card_by_id(&format!("v/{id}")).unwrap()
}

fn play_hit(e: &Engine, s: &mut GameState, p: PlayerId) {
    let id = def(e, "hit");
    let card = s.player(p).hand.iter().copied().find(|&c| s.card(c).def == id).expect("Hit in hand");
    apply(e, s, p, Action::PlayCard { card, kind: CardKind::Maneuver(ManeuverType::Strike) });
    if let Some(Decision { player, kind: DecisionKind::Reversal { .. } }) = s.decision.clone() {
        apply(e, s, player, Action::DeclineReversal);
    }
}

#[test]
fn revealed_backstage_cards_add_abilities() {
    let e = engine();
    let mut s =
        start(&e, Mode::QuickStart, deck("first", &["hit"], &[], &[]), deck("second", &[], &[], &["crowd", "secret"]));
    play_hit(&e, &mut s, P1);
    assert_eq!(s.player(P2).ringside.len(), 1, "3D - 2 from Crowd; the hidden Secret does nothing");
}

#[test]
fn a_backstage_card_can_be_hidden_to_use_it() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &[], &[], &["favor"]), deck("second", &[], &[], &[]));
    let favor = def(&e, "favor");
    let hand = s.player(P1).hand.len();
    assert!(e.legal_actions(&s, P1).contains(&Action::UseBackstageCard { card: favor }));
    let events = e.apply(&mut s, P1, Action::UseBackstageCard { card: favor }).unwrap();
    assert!(events.contains(&Event::BackstageHidden { player: P1, card: favor }));
    assert_eq!(s.player(P1).hand.len(), hand + 2);
    assert!(!e.legal_actions(&s, P1).contains(&Action::UseBackstageCard { card: favor }), "hidden now");
    let view = e.view(&s, P2);
    assert_eq!(view.players[0].backstage, vec![(favor, true)]);
}

#[test]
fn allegiances_are_played_before_venues() {
    let e = engine();
    let s = start(&e, Mode::Advanced, deck("first", &[], &["side", "event"], &[]), deck("second", &[], &[], &[]));
    let Some(Decision { player: P1, kind: DecisionKind::PreMatch { step: 0, .. } }) = s.decision.clone() else {
        panic!("Allegiance step first: {:?}", s.decision)
    };
    let side = def(&e, "side");
    let titles: Vec<DefIdx> = e
        .legal_actions(&s, P1)
        .iter()
        .filter_map(|a| if let Action::PlayCard { card, .. } = a { Some(s.card(*card).def) } else { None })
        .collect();
    assert_eq!(titles, vec![side], "only the Allegiance in its step");
}

#[test]
fn virtual_backlash_and_ring_limits() {
    let e = engine();
    let db = e.db();
    let mut d = deck("first", &[], &["event"; 12], &[]);
    d.arsenal.resize(60, "v/filler".into());
    d.arsenal.truncate(60);
    assert!(validate_deck(db, &d).contains(&DeckProblem::TooManyPreMatch(12)));
    assert!(!validate_deck_in(db, &d, Mode::Virtual).iter().any(|p| matches!(p, DeckProblem::TooManyPreMatch(_))));
    let mut s = start(&e, Mode::Virtual, deck("first", &[], &["event"; 8], &[]), deck("second", &[], &[], &[]));
    while s.turn == 0 {
        let p = s.decision.as_ref().unwrap().player;
        let a = e.legal_actions(&s, p).into_iter().find(|a| matches!(a, Action::PlayCard { .. }));
        apply(&e, &mut s, p, a.unwrap_or(Action::Pass));
    }
    assert_eq!(s.player(P1).ring.len(), 6);
}

#[test]
fn blanking_that_names_the_opponent_superstar() {
    let e = engine();
    for (belt, ringside) in [("belt", 3), ("other-belt", 1)] {
        let mut s = start(&e, Mode::Advanced, deck("first", &["hit"], &[belt], &[]), deck("tough", &[], &[], &[]));
        while s.turn == 0 {
            let p = s.decision.as_ref().unwrap().player;
            let a = e.legal_actions(&s, p).into_iter().find(|a| matches!(a, Action::PlayCard { .. }));
            apply(&e, &mut s, p, a.unwrap_or(Action::Pass));
        }
        play_hit(&e, &mut s, P1);
        assert_eq!(s.player(P2).ringside.len(), ringside, "{belt}: Tough's ability blanked only by the Belt naming it");
    }
}
