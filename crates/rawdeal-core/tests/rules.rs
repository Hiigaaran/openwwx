//! Scenario tests: a stacked deck, a few actions, and the expected outcome.

use std::sync::Arc;

use rawdeal_core::*;

const TECH: &str = "test/technician"; // Hand Size 5, Superstar Value 4: goes first.
const BRAWLER: &str = "test/brawler"; // Hand Size 6, Superstar Value 3.
const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

fn engine() -> Engine {
    Engine::new(Arc::new(rawdeal_cards::test_db()))
}

/// `cards` top first, padded with Lock Ups to `len`.
fn stack(superstar: &str, cards: &[&str], len: usize) -> DeckList {
    let mut arsenal: Vec<String> = cards.iter().map(|c| format!("test/{c}")).collect();
    arsenal.resize(len.max(cards.len()), "test/lock-up".to_string());
    DeckList { superstar: superstar.to_string(), arsenal }
}

fn start(e: &Engine, p1: DeckList, p2: DeckList) -> GameState {
    let setup = GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false };
    e.new_game(&setup, 1).expect("valid setup").0
}

fn in_hand(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> CardId {
    let id = format!("test/{id}");
    *s.player(p)
        .hand
        .iter()
        .find(|&&c| e.db().card(s.card(c).def).id == id)
        .unwrap_or_else(|| panic!("{id} not in {p}'s hand"))
}

fn play(e: &Engine, s: &mut GameState, p: PlayerId, id: &str) -> Vec<Event> {
    let card = in_hand(e, s, p, id);
    let kind = e.db().card(s.card(card).def).kinds[0];
    let events = e.apply(s, p, Action::PlayCard { card, kind }).expect("legal play");
    s.check_invariants().unwrap();
    events
}

fn waiting(s: &GameState) -> (PlayerId, &DecisionKind) {
    let d = s.decision.as_ref().expect("a pending decision");
    (d.player, &d.kind)
}

#[test]
fn setup_draws_hand_sizes_and_highest_value_goes_first() {
    let e = engine();
    let s = start(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 20));
    assert_eq!(s.active, P1);
    assert_eq!(s.turn, 1);
    assert_eq!(s.player(P1).hand.len(), 5 + 1, "hand size plus the turn's draw");
    assert_eq!(s.player(P2).hand.len(), 6);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
    s.check_invariants().unwrap();
}

#[test]
fn superstar_value_decides_first_player_regardless_of_seat() {
    let e = engine();
    let s = start(&e, stack(BRAWLER, &[], 20), stack(TECH, &[], 20));
    assert_eq!(s.active, P2);
}

#[test]
fn cards_need_enough_fortitude() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["haymaker", "jab"], 20), stack(BRAWLER, &[], 20));
    let haymaker = in_hand(&e, &s, P1, "haymaker");
    let kind = CardKind::Maneuver(ManeuverType::Strike);
    assert!(!e.legal_actions(&s, P1).contains(&Action::PlayCard { card: haymaker, kind }));
    assert!(e.apply(&mut s, P1, Action::PlayCard { card: haymaker, kind }).is_err());
    s.check_invariants().unwrap();
}

#[test]
fn maneuver_damage_overturns_cards_and_builds_fortitude() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab"], 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "jab");
    assert_eq!(s.player(P2).ringside.len(), 2);
    assert_eq!(s.player(P2).arsenal.len(), 20 - 6 - 2);
    assert_eq!(s.player(P1).ring.len(), 1);
    assert_eq!(e.fortitude(&s, P1), 2);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "the turn goes on");
}

#[test]
fn reversal_from_hand_ends_the_turn() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab"], 20), stack(BRAWLER, &["block"], 20));
    play(&e, &mut s, P1, "jab");
    let block = in_hand(&e, &s, P2, "block");
    assert!(matches!(waiting(&s), (P2, DecisionKind::Reversal { options, .. }) if options == &vec![block]));

    let events = e.apply(&mut s, P2, Action::Reverse { card: block }).unwrap();
    s.check_invariants().unwrap();
    assert!(events.iter().any(|ev| matches!(ev, Event::Reversed { from_hand: true, .. })));
    assert_eq!(s.player(P1).ringside.len(), 1, "the reversed Jab goes to Ringside");
    assert_eq!(s.player(P2).ring, vec![block]);
    assert_eq!(s.player(P2).ringside.len(), 0, "no damage was taken");
    assert_eq!((s.active, s.turn), (P2, 2));
    assert_eq!(waiting(&s), (P2, &DecisionKind::Main));
}

#[test]
fn reversals_need_enough_fortitude() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["body-slam"], 20), stack(BRAWLER, &["counter-hold"], 20));
    play(&e, &mut s, P1, "body-slam");
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "Counter Hold needs 2 Fortitude");
    assert_eq!(s.player(P2).ringside.len(), 3);
}

#[test]
fn hybrid_can_be_played_as_maneuver() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab", "shove"], 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "jab");
    let shove = in_hand(&e, &s, P1, "shove");
    let legal = e.legal_actions(&s, P1);
    assert!(legal.contains(&Action::PlayCard { card: shove, kind: CardKind::Maneuver(ManeuverType::Strike) }));
    assert!(!legal.contains(&Action::PlayCard { card: shove, kind: CardKind::Reversal }));
}

#[test]
fn overturned_reversal_stops_damage_and_grants_stun_draw() {
    let e = engine();
    // P2: six cards in hand, then four Lock Ups for the two Jabs, then Lock Up, Block.
    let p2 = ["lock-up"; 11].iter().copied().chain(["block"]).collect::<Vec<_>>();
    let mut s = start(&e, stack(TECH, &["jab", "jab", "haymaker"], 30), stack(BRAWLER, &p2, 30));
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "jab");
    let before = s.player(P2).arsenal.len();
    let events = play(&e, &mut s, P1, "haymaker");

    assert!(events.iter().any(|ev| matches!(ev, Event::Reversed { from_hand: false, .. })));
    assert_eq!(s.player(P2).arsenal.len(), before - 2, "4 damage stopped after 2 cards");
    assert_eq!(s.player(P1).ring.len(), 3, "the Haymaker stays in the Ring");
    assert_eq!(waiting(&s), (P1, &DecisionKind::StunDraw { max: 2 }));

    let hand = s.player(P1).hand.len();
    e.apply(&mut s, P1, Action::DrawCards(2)).unwrap();
    assert_eq!(s.player(P1).hand.len(), hand + 2);
    assert_eq!((s.active, s.turn), (P2, 2), "the reversal ended P1's turn");
    s.check_invariants().unwrap();
}

#[test]
fn pinfall_when_damage_finds_an_empty_arsenal() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab"], 20), stack(BRAWLER, &[], 7));
    let events = play(&e, &mut s, P1, "jab");
    assert_eq!(s.outcome, Some(Outcome::Win { winner: P1, method: WinMethod::Pinfall }));
    assert!(events.contains(&Event::GameOver(s.outcome.unwrap())));
    assert!(e.legal_actions(&s, P1).is_empty());
}

#[test]
fn count_out_can_be_claimed_or_refused() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 6));
    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    assert!(matches!(waiting(&s), (P1, DecisionKind::YesNo { purpose: YesNoPurpose::ClaimCountOut })));

    let mut claimed = s.clone();
    e.apply(&mut claimed, P1, Action::Yes).unwrap();
    assert_eq!(claimed.outcome, Some(Outcome::Win { winner: P1, method: WinMethod::CountOut }));

    e.apply(&mut s, P1, Action::No).unwrap();
    assert_eq!(s.outcome, None);
    assert_eq!(waiting(&s), (P2, &DecisionKind::Main), "P2 plays on with no draw");
    e.apply(&mut s, P2, Action::EndTurn).unwrap();
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "a refused count out is not offered again");
}

#[test]
fn chosen_discard_and_ringside_recovery() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab", "second-wind", "suplex"], 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "second-wind");
    let suplex = in_hand(&e, &s, P1, "suplex");
    assert!(matches!(
        waiting(&s),
        (P1, DecisionKind::ChooseCards { purpose: ChoicePurpose::Discard, min: 1, max: 1, .. })
    ));
    assert!(e.apply(&mut s, P1, Action::ChooseCards(vec![])).is_err(), "must discard exactly one");

    e.apply(&mut s, P1, Action::ChooseCards(vec![suplex])).unwrap();
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseCards { purpose: ChoicePurpose::RingsideToHand, .. })));
    e.apply(&mut s, P1, Action::ChooseCards(vec![suplex])).unwrap();
    assert!(s.player(P1).hand.contains(&suplex));
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
    s.check_invariants().unwrap();
}

#[test]
fn optional_effects_ask_first() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["regroup"], 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "regroup");
    assert!(matches!(waiting(&s), (P1, DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { .. } })));
    e.apply(&mut s, P1, Action::Yes).unwrap();
    // Ringside is empty, so the effect does as much as it can: nothing.
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
}

#[test]
fn actions_can_be_reversed() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["catch-breath"], 20), stack(BRAWLER, &["no-chance", "jab"], 20));
    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    play(&e, &mut s, P2, "jab"); // Not A Chance needs 1 Fortitude.
    e.apply(&mut s, P2, Action::EndTurn).unwrap();

    play(&e, &mut s, P1, "catch-breath");
    let no_chance = in_hand(&e, &s, P2, "no-chance");
    assert!(matches!(waiting(&s), (P2, DecisionKind::Reversal { options, .. }) if options == &vec![no_chance]));
    e.apply(&mut s, P2, Action::Reverse { card: no_chance }).unwrap();
    assert_eq!((s.active, s.turn), (P2, 4));
}

#[test]
fn only_the_deciding_player_may_act() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 20));
    assert_eq!(e.apply(&mut s, P2, Action::EndTurn), Err(RuleError::NotYourDecision));
    assert!(e.legal_actions(&s, P2).is_empty());
}

#[test]
fn views_hide_the_opponents_hand() {
    let e = engine();
    let s = start(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 20));
    let v = e.view(&s, P2);
    assert_eq!(v.hand, s.player(P2).hand);
    assert_eq!(v.opponent().hand_count, 6);
    assert!(s.player(P1).hand.iter().all(|&c| v.def_of(c).is_none()));
    assert!(v.decision.is_none());
    assert_eq!(v.waiting_on, Some(P1));
}

#[test]
fn deck_rules() {
    let db = rawdeal_cards::test_db();
    let mut deck = rawdeal_cards::test_deck(TECH);
    assert!(validate_deck(&db, &deck).is_empty());
    deck.arsenal[59] = "test/jab".into();
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::TooManyCopies {
        card: "test/jab".into(),
        copies: 4,
        max: 3
    }));
    deck.arsenal.pop();
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::WrongSize(59)));
}
