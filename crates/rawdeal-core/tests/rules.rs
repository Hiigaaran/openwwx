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
    DeckList { superstar: superstar.to_string(), arsenal, backlash: vec![], backstage: vec![] }
}

fn start(e: &Engine, p1: DeckList, p2: DeckList) -> GameState {
    let setup =
        GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false, mode: Mode::QuickStart };
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
    for i in 57..60 {
        deck.arsenal[i] = "test/jab".into();
    }
    let jabs = deck.arsenal.iter().filter(|c| *c == "test/jab").count();
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::TooManyCopies {
        card: "test/jab".into(),
        copies: jabs,
        max: 3
    }));
    deck.arsenal.pop();
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::WrongSize(59)));
}

const VETERAN: &str = "test/veteran";
const HARDCORE: &str = "test/hardcore";

fn playable(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> bool {
    let card = in_hand(e, s, p, id);
    e.db()
        .card(s.card(card).def)
        .kinds
        .iter()
        .any(|&kind| e.legal_actions(s, p).contains(&Action::PlayCard { card, kind }))
}

#[test]
fn activated_ability_is_once_per_turn() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 20));
    assert!(e.legal_actions(&s, P1).contains(&Action::UseAbility));
    e.apply(&mut s, P1, Action::UseAbility).unwrap();
    let discard = s.player(P1).hand[0];
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseCards { purpose: ChoicePurpose::Discard, .. })));
    e.apply(&mut s, P1, Action::ChooseCards(vec![discard])).unwrap();
    assert_eq!(s.player(P1).ringside, vec![discard]);
    assert_eq!(s.player(P2).hand.len(), 5, "the opponent discarded at random");
    assert_eq!(s.player(P2).ringside.len(), 1);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
    assert!(!e.legal_actions(&s, P1).contains(&Action::UseAbility));
    assert_eq!(
        e.apply(&mut s, P1, Action::UseAbility),
        Err(RuleError::Illegal("action does not answer the pending decision"))
    );

    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    assert!(!e.legal_actions(&s, P2).contains(&Action::UseAbility), "the Brawler has no ability");
    e.apply(&mut s, P2, Action::EndTurn).unwrap();
    assert!(e.legal_actions(&s, P1).contains(&Action::UseAbility), "usable again next turn");
}

#[test]
fn start_of_turn_ability_happens_before_the_draw() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab"], 20), stack(VETERAN, &[], 20));
    play(&e, &mut s, P1, "jab");
    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    let hand = s.player(P2).hand.len();
    assert!(matches!(waiting(&s), (P2, DecisionKind::YesNo { purpose: YesNoPurpose::OptionalEffect { .. } })));
    assert_eq!(s.player(P2).hand.len(), hand, "not drawn yet");

    e.apply(&mut s, P2, Action::Yes).unwrap();
    let back = s.player(P2).ringside[0];
    e.apply(&mut s, P2, Action::ChooseCards(vec![back])).unwrap();
    assert_eq!(s.player(P2).arsenal[0], back, "on the bottom of the Arsenal");
    assert_eq!(s.player(P2).hand.len(), hand + 1);
    assert_eq!(waiting(&s), (P2, &DecisionKind::Main));
    s.check_invariants().unwrap();
}

#[test]
fn draw_segment_and_damage_reduction_abilities() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab"], 20), stack(HARDCORE, &[], 20));
    play(&e, &mut s, P1, "jab");
    assert_eq!(s.player(P2).ringside.len(), 1, "2 damage reduced by 1");
    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    assert_eq!(s.player(P2).hand.len(), 4 + 2);
}

#[test]
fn follow_ups_need_the_right_previous_card() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["follow-through", "jab", "body-slam"], 20), stack(BRAWLER, &[], 30));
    assert!(!playable(&e, &s, P1, "follow-through"));
    play(&e, &mut s, P1, "jab");
    assert!(!playable(&e, &s, P1, "follow-through"), "a Strike is not a Grapple");
    play(&e, &mut s, P1, "body-slam");
    assert!(playable(&e, &s, P1, "follow-through"));

    // Using the Superstar ability does not break the chain.
    e.apply(&mut s, P1, Action::UseAbility).unwrap();
    let discard = *s.player(P1).hand.iter().find(|&&c| e.db().card(s.card(c).def).title == "Lock Up").unwrap();
    e.apply(&mut s, P1, Action::ChooseCards(vec![discard])).unwrap();
    assert!(playable(&e, &s, P1, "follow-through"));

    play(&e, &mut s, P1, "lock-up");
    assert!(!playable(&e, &s, P1, "follow-through"), "another card broke the chain");
}

#[test]
fn active_cards_cannot_stack_in_your_ring() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["crowd-support", "crowd-support"], 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "crowd-support");
    assert!(!playable(&e, &s, P1, "crowd-support"));
}

#[test]
fn universally_unique_cards() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab", "jab", "title-belt", "title-belt"], 20), stack(BRAWLER, &[], 30));
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "jab");
    assert!(playable(&e, &s, P1, "title-belt"));
    play(&e, &mut s, P1, "title-belt");
    assert!(!playable(&e, &s, P1, "title-belt"));
}

// --- Multi and Chain ---

#[test]
fn multi_needs_two_reversals_from_hand() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab", "double-team"], 20), stack(BRAWLER, &["sprawl", "sprawl"], 20));
    play(&e, &mut s, P1, "jab");
    let events = play(&e, &mut s, P1, "double-team");
    let first = in_hand(&e, &s, P2, "sprawl");
    assert!(events.iter().any(|ev| matches!(ev, Event::CardPlayed { .. })));
    e.apply(&mut s, P2, Action::Reverse { card: first }).unwrap();
    assert!(s.player(P2).ring.contains(&first), "the first Reversal stays in the Ring");
    let second = in_hand(&e, &s, P2, "sprawl");
    assert!(matches!(waiting(&s), (P2, DecisionKind::Reversal { options, .. }) if options == &vec![second]));
    e.apply(&mut s, P2, Action::Reverse { card: second }).unwrap();
    assert_eq!((s.active, s.turn), (P2, 2), "two Reversals stop a Multi card");
    s.check_invariants().unwrap();
}

#[test]
fn multi_with_one_reversal_still_hits() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["jab", "double-team"], 20), stack(BRAWLER, &["sprawl"], 20));
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "double-team");
    let sprawl = in_hand(&e, &s, P2, "sprawl");
    e.apply(&mut s, P2, Action::Reverse { card: sprawl }).unwrap();
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "one Reversal is not enough");
    assert_eq!(s.player(P2).ringside.len(), 2 + 4);
    assert_eq!(s.player(P1).ring.len(), 2);
}

#[test]
fn multi_second_reversal_can_be_overturned() {
    let e = engine();
    // P2: six cards in hand (one Sprawl), two Lock Ups for the Jab, then Lock Up, Sprawl.
    let p2 =
        ["sprawl", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "sprawl"];
    let mut s = start(&e, stack(TECH, &["jab", "double-team"], 20), stack(BRAWLER, &p2, 20));
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "double-team");
    let sprawl = in_hand(&e, &s, P2, "sprawl");
    e.apply(&mut s, P2, Action::Reverse { card: sprawl }).unwrap();
    assert_eq!(waiting(&s), (P1, &DecisionKind::StunDraw { max: 2 }), "the overturned Sprawl was the second");
    assert_eq!(s.player(P2).ringside.len(), 2 + 2);
}

#[test]
fn female_superstars_cannot_play_multi_maneuvers() {
    let e = engine();
    let mut s = start(&e, stack("test/high-flyer", &["jab", "double-team"], 20), stack(BRAWLER, &[], 20));
    if s.active == P2 {
        e.apply(&mut s, P2, Action::EndTurn).unwrap();
    }
    play(&e, &mut s, P1, "jab");
    assert!(!playable(&e, &s, P1, "double-team"));
}

#[test]
fn reversing_a_chain_costs_two_discards() {
    let e = engine();
    let mut s = start(&e, stack(TECH, &["chain-punch"], 20), stack(BRAWLER, &["block"], 20));
    play(&e, &mut s, P1, "chain-punch");
    let block = in_hand(&e, &s, P2, "block");
    e.apply(&mut s, P2, Action::Reverse { card: block }).unwrap();
    let (who, DecisionKind::ChooseCards { options, min: 2, max: 2, .. }) = waiting(&s) else { panic!("discard 2") };
    assert_eq!(who, P2);
    assert!(!options.contains(&block));
    let pay = options[..2].to_vec();
    e.apply(&mut s, P2, Action::ChooseCards(pay.clone())).unwrap();
    assert!(pay.iter().all(|c| s.player(P2).ringside.contains(c)));
    assert_eq!(s.player(P2).ring, vec![block]);
    assert_eq!((s.active, s.turn), (P2, 2));
    s.check_invariants().unwrap();
}

#[test]
fn successful_chain_searches_for_another() {
    let e = engine();
    let p1 = ["chain-punch", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "chain-kick"];
    let mut s = start(&e, stack(TECH, &p1, 20), stack(BRAWLER, &[], 20));
    play(&e, &mut s, P1, "chain-punch");
    assert_eq!(s.player(P2).ringside.len(), 2, "damage comes first");
    assert!(matches!(waiting(&s), (P1, DecisionKind::YesNo { purpose: YesNoPurpose::ChainSearch })));
    e.apply(&mut s, P1, Action::Yes).unwrap();
    let pay = s.player(P1).hand[..2].to_vec();
    e.apply(&mut s, P1, Action::ChooseCards(pay)).unwrap();
    let (_, DecisionKind::ChooseCards { options, .. }) = waiting(&s) else { panic!("search") };
    let kick = options[0];
    let events = e.apply(&mut s, P1, Action::ChooseCards(vec![kick])).unwrap();
    assert!(s.player(P1).hand.contains(&kick));
    assert!(events.contains(&Event::Revealed { player: P1, card: kick }));
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
    s.check_invariants().unwrap();
}

// --- Advanced matches ---

fn advanced(e: &Engine, p1: DeckList, p2: DeckList) -> (GameState, Vec<Event>) {
    let setup = GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false, mode: Mode::Advanced };
    e.new_game(&setup, 1).expect("valid setup")
}

fn with_backlash(mut deck: DeckList, cards: &[&str]) -> DeckList {
    deck.backlash = cards.iter().map(|c| format!("test/{c}")).collect();
    deck
}

fn pre_match(e: &Engine, s: &mut GameState, p: PlayerId, id: &str) {
    let id = format!("test/{id}");
    let (card, kind) = e
        .legal_actions(s, p)
        .iter()
        .find_map(|a| match a {
            Action::PlayCard { card, kind } if e.db().card(s.card(*card).def).id == id => Some((*card, *kind)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{p} cannot play {id}"));
    e.apply(s, p, Action::PlayCard { card, kind }).unwrap();
    s.check_invariants().unwrap();
}

#[test]
fn pre_match_phase_in_order() {
    let e = engine();
    let all = ["arena", "bad-blood", "no-dq", "manager", "ppv", "pyro", "chair"];
    let (mut s, events) = advanced(
        &e,
        with_backlash(stack(TECH, &[], 30), &all),
        with_backlash(stack(BRAWLER, &[], 30), &["arena", "bad-blood", "manager", "pyro"]),
    );
    assert!(events.contains(&Event::PreMatchStarted { first: P1 }));
    assert!(s.player(P1).hand.is_empty(), "hands are drawn after Pre-match");

    // Venue: P1 plays one, so P2 cannot (one Venue in all Rings) and is skipped.
    assert!(matches!(waiting(&s), (P1, DecisionKind::PreMatch { step: 1, .. })));
    pre_match(&e, &mut s, P1, "arena");
    // Feud: both play.
    assert!(matches!(waiting(&s), (P1, DecisionKind::PreMatch { step: 2, .. })));
    pre_match(&e, &mut s, P1, "bad-blood");
    pre_match(&e, &mut s, P2, "bad-blood");
    // Stipulation: P1 passes; P2 has none.
    assert!(matches!(waiting(&s), (P1, DecisionKind::PreMatch { step: 3, .. })));
    e.apply(&mut s, P1, Action::Pass).unwrap();
    // Manager: P1 passes, P2 plays one and P1 discards at random (empty hand: nothing).
    e.apply(&mut s, P1, Action::Pass).unwrap();
    pre_match(&e, &mut s, P2, "manager");
    // Open step: Stipulations and Managers are no longer playable.
    let (_, DecisionKind::PreMatch { step: 5, .. }) = waiting(&s) else { panic!("open step") };
    let legal = e.legal_actions(&s, P1);
    let titles: Vec<&str> = legal
        .iter()
        .filter_map(|a| {
            if let Action::PlayCard { card, .. } = a { Some(e.db().card(s.card(*card).def).id.as_str()) } else { None }
        })
        .collect();
    assert_eq!(titles, vec!["test/ppv", "test/pyro", "test/chair"]);
    pre_match(&e, &mut s, P1, "ppv");
    pre_match(&e, &mut s, P2, "pyro");
    pre_match(&e, &mut s, P1, "pyro");
    // P2 has nothing left, so the engine passes for them.
    assert!(matches!(waiting(&s), (P1, DecisionKind::PreMatch { step: 5, passes: 1, .. })));
    let events = e.apply(&mut s, P1, Action::Pass).unwrap();

    // Two passes in a row end the phase: hands, then end-of-Pre-match effects.
    assert!(events.contains(&Event::PreMatchEnded));
    assert!(events.contains(&Event::GameStarted { first: P1 }));
    assert_eq!(s.player(P1).ring.len(), 4);
    assert_eq!(s.player(P2).ring.len(), 3);
    assert_eq!(s.player(P1).hand.len(), 5 + 1 + 1 + 1, "Hand Size, Arena, Pyro, turn draw");
    assert_eq!(s.player(P2).hand.len(), 6 + 1, "Hand Size, Pyro");
    assert_eq!(s.player(P1).ringside.len(), 1, "P2's Bad Blood");
    assert_eq!(s.player(P2).ringside.len(), 1, "P1's Bad Blood");
    assert_eq!((s.active, s.turn), (P1, 1));
}

#[test]
fn pre_match_cards_respect_ring_limits() {
    let e = engine();
    let (mut s, _) = advanced(
        &e,
        with_backlash(stack(TECH, &[], 30), &["ppv", "ppv", "pyro", "pyro", "chair", "chair", "pyro"]),
        stack(BRAWLER, &[], 30),
    );
    pre_match(&e, &mut s, P1, "ppv");
    assert!(
        e.legal_actions(&s, P1).iter().all(|a| match a {
            Action::PlayCard { card, .. } => e.db().card(s.card(*card).def).id != "test/ppv",
            _ => true,
        }),
        "only one Pay-Per-View"
    );
    for id in ["pyro", "pyro", "chair", "chair"] {
        pre_match(&e, &mut s, P1, id);
    }
    assert_eq!(s.player(P1).ring.len(), 5);
    assert!(matches!(waiting(&s), (P1, DecisionKind::Main)), "the sixth Pre-match card cannot be played");
}

#[test]
fn mid_match_cards_play_from_backlash() {
    let e = engine();
    let (mut s, _) = advanced(
        &e,
        with_backlash(stack(TECH, &[], 30), &["ladder-spot"]),
        with_backlash(stack(BRAWLER, &[], 30), &["ref-distraction"]),
    );
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "no Pre-match cards: straight to the match");
    let ladder = s.player(P1).backlash[0];
    let legal = e.legal_actions(&s, P1);
    assert!(legal.contains(&Action::PlayCard { card: ladder, kind: CardKind::Maneuver(ManeuverType::HighRisk) }));
    e.apply(&mut s, P1, Action::PlayCard { card: ladder, kind: CardKind::Maneuver(ManeuverType::HighRisk) }).unwrap();
    let ref_d = s.player(P2).backlash[0];
    assert!(matches!(waiting(&s), (P2, DecisionKind::Reversal { options, .. }) if options == &vec![ref_d]));
    e.apply(&mut s, P2, Action::Reverse { card: ref_d }).unwrap();
    assert_eq!(s.player(P2).ring, vec![ref_d]);
    assert!(s.player(P1).ringside.contains(&ladder));
    s.check_invariants().unwrap();
}

#[test]
fn backlash_cards_never_return_to_hand() {
    let e = engine();
    let (mut s, _) = advanced(
        &e,
        with_backlash(stack(TECH, &["jab", "second-wind"], 30), &["ladder-spot"]),
        with_backlash(stack(BRAWLER, &[], 30), &["ref-distraction"]),
    );
    let ladder = s.player(P1).backlash[0];
    e.apply(&mut s, P1, Action::PlayCard { card: ladder, kind: CardKind::Maneuver(ManeuverType::HighRisk) }).unwrap();
    let ref_d = s.player(P2).backlash[0];
    e.apply(&mut s, P2, Action::Reverse { card: ref_d }).unwrap();
    e.apply(&mut s, P2, Action::EndTurn).unwrap();
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "second-wind");
    let discard = in_hand(&e, &s, P1, "lock-up");
    e.apply(&mut s, P1, Action::ChooseCards(vec![discard])).unwrap();
    let (_, DecisionKind::ChooseCards { options, .. }) = waiting(&s) else { panic!("recover") };
    assert_eq!(options, &vec![discard], "the Ladder Spot in Ringside is not an option");
}

#[test]
fn advanced_count_out_only_after_your_own_turn() {
    let e = engine();
    let (mut s, _) = advanced(&e, stack(TECH, &[], 20), stack(BRAWLER, &[], 6));
    e.apply(&mut s, P1, Action::EndTurn).unwrap();
    assert_eq!(waiting(&s), (P2, &DecisionKind::Main), "P2 has not ended a turn yet");
    e.apply(&mut s, P2, Action::EndTurn).unwrap();
    assert!(matches!(waiting(&s), (P1, DecisionKind::YesNo { purpose: YesNoPurpose::ClaimCountOut })));
}

#[test]
fn backlash_deck_rules() {
    let db = rawdeal_cards::test_db();
    let mut deck = rawdeal_cards::test_deck(TECH);
    assert!(validate_deck(&db, &deck).is_empty());
    deck.backlash.push("test/jab".into());
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::NotABacklashCard { card: "test/jab".into() }));
    deck.backlash.pop();
    deck.arsenal[59] = "test/pyro".into();
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::BacklashCardInArsenal { card: "test/pyro".into() }));
    deck.arsenal[59] = "test/lock-up".into();
    deck.backlash.extend(["test/pyro", "test/pyro", "test/chair", "test/chair"].map(String::from));
    assert!(validate_deck(&db, &deck).contains(&DeckProblem::TooManyPreMatch(11)));
}

#[test]
fn exclusive_cards_cannot_share_a_deck() {
    let mut set = rawdeal_cards::test_set();
    let jab = set.cards.iter_mut().find(|c| c.id == "test/jab").unwrap();
    jab.script.not_with = vec!["Lock Up".into()];
    let db = rawdeal_cards::build_db([set]).unwrap();
    let deck = rawdeal_cards::test_deck(TECH);
    assert!(validate_deck(&db, &deck).iter().any(|p| matches!(p, DeckProblem::Exclusive { .. })));
}
