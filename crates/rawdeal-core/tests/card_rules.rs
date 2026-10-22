//! Scenario tests for card texts that change how a card is reversed or where
//! it fits: conditional Multi, reversal counts and sources, counted bonuses
//! Pre-match slots and cards played after a successful maneuver, face-down cards and cards under the Superstar. The cards are invented for the tests.

use std::sync::Arc;

use rawdeal_core::*;

const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

const SET: &str = r#"(
    name: "Card rule tests",
    superstars: [
        (id: "m/first", name: "First", hand_size: 5, superstar_value: 5),
        (id: "m/second", name: "Second", hand_size: 6, superstar_value: 1),
        (id: "m/third", name: "Third", hand_size: 5, superstar_value: 5, abilities: [PlayFromUnder(Maneuver)]),
    ],
    cards: [
        (id: "m/filler", title: "Filler", kinds: [Action], fortitude: 0, damage: 0),
        (id: "m/hit", title: "Hit", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3),
        (id: "m/opener", title: "Opener", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3,
            script: (multi_when: Some(FirstCardThisTurn))),
        (id: "m/big", title: "Big", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3,
            script: (reversals_needed: 3)),
        (id: "m/arsenal-only", title: "Arsenal Only", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3,
            script: (reversal_limits: [(from: [Hand, Backlash, Ringside, Ring, Under])])),
        (id: "m/scaled", title: "Scaled", kinds: [Maneuver(Strike)], fortitude: 0, damage: 1,
            script: (bonuses: [(damage: 1, count: Some(OpponentHandSize))])),
        (id: "m/block", title: "Block", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Maneuver)))),
        (id: "m/event", title: "Event", kinds: [PreMatch(Event)], fortitude: 0, damage: 0),
        (id: "m/free", title: "Free", kinds: [PreMatch(Event)], fortitude: 0, damage: 0, script: (no_slot: true)),
        (id: "m/host", title: "Host", kinds: [PreMatch(Event)], fortitude: 0, damage: 0,
            script: (frees_slots: Some(Title("Event")))),
        (id: "m/riser", title: "Riser", kinds: [Maneuver(Strike)], fortitude: 0, damage: 3,
            script: (on_flip_up: [Draw(who: You, n: 1)])),
        (id: "m/flip", title: "Flip", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [FlipDown(who: You, n: 1, filter: Some(Maneuver))])),
        (id: "m/unflip", title: "Unflip", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [FlipUp(who: You, n: 1)])),
        (id: "m/stash", title: "Stash", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [PutUnder(from: Hand, n: 1, filter: Some(Maneuver), superstar: true)])),
        (id: "m/stop", title: "Stop", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (after_success: Some(()))),
        (id: "m/cancel", title: "Cancel", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (after_success: Some((at: BeforeEffects, cancel: true)))),
    ],
)"#;

fn engine() -> Engine {
    let set = rawdeal_cards::parse_set(SET).expect("test set parses");
    Engine::new(Arc::new(rawdeal_cards::build_db([set]).expect("consistent")))
}

/// `cards` top first, padded with Fillers to 30.
fn deck(superstar: &str, cards: &[&str], backlash: &[&str]) -> DeckList {
    let ids = |l: &[&str]| l.iter().map(|c| format!("m/{c}")).collect::<Vec<_>>();
    let mut arsenal = ids(cards);
    arsenal.resize(30.max(cards.len()), "m/filler".to_string());
    DeckList { superstar: format!("m/{superstar}"), arsenal, backlash: ids(backlash), backstage: vec![] }
}

fn start(e: &Engine, mode: Mode, p1: DeckList, p2: DeckList) -> GameState {
    let setup = GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false, mode };
    e.new_game(&setup, 1).expect("valid setup").0
}

fn in_hand(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> CardId {
    let id = format!("m/{id}");
    *s.player(p).hand.iter().find(|&&c| e.db().card(s.card(c).def).id == id).expect("card in hand")
}

fn apply(e: &Engine, s: &mut GameState, p: PlayerId, a: Action) {
    e.apply(s, p, a).expect("legal action");
    s.check_invariants().unwrap();
}

fn play(e: &Engine, s: &mut GameState, p: PlayerId, id: &str) {
    let card = in_hand(e, s, p, id);
    let kind = e.db().card(s.card(card).def).kinds[0];
    apply(e, s, p, Action::PlayCard { card, kind });
}

fn reversal_options(s: &GameState) -> usize {
    match &s.decision {
        Some(Decision { kind: DecisionKind::Reversal { options, .. }, .. }) => options.len(),
        _ => 0,
    }
}

fn reverse(e: &Engine, s: &mut GameState) {
    let card = in_hand(e, s, P2, "block");
    apply(e, s, P2, Action::Reverse { card });
}

#[test]
fn a_card_can_be_multi_when_a_condition_holds() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["opener"], &[]), deck("second", &["block", "block"], &[]));
    play(&e, &mut s, P1, "opener");
    reverse(&e, &mut s);
    assert_eq!(reversal_options(&s), 1, "played first, it is Multi: another Reversal is needed");
    reverse(&e, &mut s);
    assert_eq!(s.active, P2);
}

#[test]
fn some_cards_need_three_reversals() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["big"], &[]), deck("second", &["block"; 3], &[]));
    play(&e, &mut s, P1, "big");
    reverse(&e, &mut s);
    reverse(&e, &mut s);
    assert_eq!(s.active, P1, "two are not enough");
    reverse(&e, &mut s);
    assert_eq!(s.active, P2);
}

#[test]
fn a_card_reversed_only_from_the_arsenal() {
    let e = engine();
    // P2: six cards drawn (one Block), then a Filler and a Block on top of the Arsenal.
    let p2 = ["block", "filler", "filler", "filler", "filler", "filler", "filler", "block"];
    let mut s = start(&e, Mode::QuickStart, deck("first", &["arsenal-only"], &[]), deck("second", &p2, &[]));
    play(&e, &mut s, P1, "arsenal-only");
    assert_eq!(reversal_options(&s), 0, "not from hand");
    assert_eq!(s.active, P2, "the overturned Block reverses it");
    assert_eq!(s.player(P2).ringside.len(), 2);
}

#[test]
fn bonuses_can_be_multiplied_by_a_count() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["scaled"], &[]), deck("second", &[], &[]));
    play(&e, &mut s, P1, "scaled");
    assert_eq!(s.player(P2).ringside.len(), 1 + 6, "+1D for each of the 6 cards in his hand");
}

fn play_pre_match(e: &Engine, s: &mut GameState) {
    while s.turn == 0 {
        let p = s.decision.as_ref().unwrap().player;
        let a = e.legal_actions(s, p).into_iter().find(|a| matches!(a, Action::PlayCard { .. }));
        apply(e, s, p, a.unwrap_or(Action::Pass));
    }
}

#[test]
fn cards_that_do_not_take_up_a_pre_match_slot() {
    let e = engine();
    let events = ["event"; 6];
    let mut s = start(&e, Mode::Advanced, deck("first", &[], &events), deck("second", &[], &[]));
    play_pre_match(&e, &mut s);
    let limit = s.player(P1).ring.len();
    for extra in ["free", "host"] {
        // The host must be in the Ring before the slots fill up.
        let mut backlash = vec![extra];
        backlash.extend(events);
        let mut s = start(&e, Mode::Advanced, deck("first", &[], &backlash), deck("second", &[], &[]));
        play_pre_match(&e, &mut s);
        assert!(s.player(P1).ring.len() > limit, "{extra}: more than the {limit} slots");
    }
}

fn play_after_success(e: &Engine, s: &mut GameState, id: &str) {
    let card = in_hand(e, s, P2, id);
    let Some(Decision {
        kind: DecisionKind::ChooseCards { options, purpose: ChoicePurpose::AfterSuccess(_), .. }, ..
    }) = &s.decision
    else {
        panic!("no after-success window: {:?}", s.decision);
    };
    assert_eq!(options, &[card]);
    apply(e, s, P2, Action::ChooseCards(vec![card]));
}

#[test]
fn a_card_played_after_a_successful_maneuver_ends_the_turn() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["hit"], &[]), deck("second", &["stop"], &[]));
    play(&e, &mut s, P1, "hit");
    assert_eq!(s.player(P2).ringside.len(), 3, "the maneuver deals its damage first");
    play_after_success(&e, &mut s, "stop");
    assert_eq!(s.active, P2);
    assert_eq!(s.player(P2).ring.len(), 1);
    assert_eq!(s.player(P1).ring.len(), 1, "the maneuver was still successful");
}

#[test]
fn a_card_played_after_success_can_cancel_the_damage() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["hit"], &[]), deck("second", &["cancel"], &[]));
    play(&e, &mut s, P1, "hit");
    play_after_success(&e, &mut s, "cancel");
    assert_eq!(s.active, P2);
    assert!(s.player(P2).ringside.is_empty(), "no damage");
}

fn choose_first(e: &Engine, s: &mut GameState) {
    let Some(Decision { player, kind: DecisionKind::ChooseCards { options, .. } }) = s.decision.clone() else {
        panic!("no choice: {:?}", s.decision);
    };
    apply(e, s, player, Action::ChooseCards(vec![options[0]]));
}

#[test]
fn face_down_cards_are_blank_until_flipped_face_up() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("first", &["riser", "flip", "unflip"], &[]), deck("second", &[], &[]));
    play(&e, &mut s, P1, "riser");
    assert_eq!(e.fortitude(&s, P1), 3);
    play(&e, &mut s, P1, "flip");
    choose_first(&e, &mut s);
    assert_eq!(e.fortitude(&s, P1), 0, "a face-down card is D: 0");
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "unflip");
    choose_first(&e, &mut s);
    assert_eq!(e.fortitude(&s, P1), 3);
    assert_eq!(s.player(P1).hand.len(), hand, "Unflip left the hand and Riser drew 1");
}

#[test]
fn cards_under_the_superstar_can_be_played() {
    let e = engine();
    let mut s = start(&e, Mode::QuickStart, deck("third", &["stash", "hit"], &[]), deck("second", &[], &[]));
    play(&e, &mut s, P1, "stash");
    choose_first(&e, &mut s);
    let hit = s.player(P1).under[0];
    assert_eq!(s.card(hit).host, None);
    let legal = e.legal_actions(&s, P1);
    assert!(legal.iter().any(|a| matches!(a, Action::PlayCard { card, .. } if *card == hit)), "{legal:?}");
    apply(&e, &mut s, P1, Action::PlayCard { card: hit, kind: CardKind::Maneuver(ManeuverType::Strike) });
    assert_eq!(s.player(P2).ringside.len(), 3);
}
