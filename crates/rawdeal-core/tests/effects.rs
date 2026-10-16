//! Scenario tests for the card effects added for the Premiere set: boosts,
//! follow-ups, searches, choices and other text the engine interprets.

use std::sync::Arc;

use rawdeal_core::*;

const TECH: &str = "test/technician"; // Hand Size 5, Superstar Value 4: goes first.
const BRAWLER: &str = "test/brawler"; // Hand Size 6, Superstar Value 3.
const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

fn engine() -> Engine {
    Engine::new(Arc::new(rawdeal_cards::test_db()))
}

/// `cards` top first, padded with Lock Ups to 20.
fn stack(superstar: &str, cards: &[&str]) -> DeckList {
    let mut arsenal: Vec<String> = cards.iter().map(|c| format!("test/{c}")).collect();
    arsenal.resize(20.max(cards.len()), "test/lock-up".to_string());
    DeckList { superstar: superstar.to_string(), arsenal, backlash: vec![], backstage: vec![] }
}

fn start(e: &Engine, p1: &[&str], p2: &[&str]) -> GameState {
    let setup = GameSetup {
        decks: [stack(TECH, p1), stack(BRAWLER, p2)],
        enforce_deck_rules: false,
        shuffle_arsenals: false,
        mode: Mode::QuickStart,
    };
    e.new_game(&setup, 1).expect("valid setup").0
}

fn find(e: &Engine, s: &GameState, cards: &[CardId], id: &str) -> Option<CardId> {
    let id = format!("test/{id}");
    cards.iter().copied().find(|&c| e.db().card(s.card(c).def).id == id)
}

fn in_hand(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> CardId {
    find(e, s, &s.player(p).hand, id).unwrap_or_else(|| panic!("{id} not in {p}'s hand"))
}

fn play_as(e: &Engine, s: &mut GameState, p: PlayerId, id: &str, kind: CardKind) -> Vec<Event> {
    let card = in_hand(e, s, p, id);
    let events = e.apply(s, p, Action::PlayCard { card, kind }).expect("legal play");
    s.check_invariants().unwrap();
    events
}

fn play(e: &Engine, s: &mut GameState, p: PlayerId, id: &str) -> Vec<Event> {
    let kind = e.db().card(s.card(in_hand(e, s, p, id)).def).kinds[0];
    play_as(e, s, p, id, kind)
}

fn playable(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> bool {
    let card = in_hand(e, s, p, id);
    e.legal_actions(s, p).iter().any(|a| matches!(a, Action::PlayCard { card: c, .. } if *c == card))
}

fn waiting(s: &GameState) -> (PlayerId, &DecisionKind) {
    let d = s.decision.as_ref().expect("a pending decision");
    (d.player, &d.kind)
}

fn apply(e: &Engine, s: &mut GameState, p: PlayerId, a: Action) -> Vec<Event> {
    let events = e.apply(s, p, a).expect("legal action");
    s.check_invariants().unwrap();
    events
}

const STRIKE: CardKind = CardKind::Maneuver(ManeuverType::Strike);

#[test]
fn whip_boosts_the_next_strike_and_enables_follow_ups() {
    let e = engine();
    let mut s = start(&e, &["whip", "rebound"], &[]);
    assert!(!playable(&e, &s, P1, "rebound"), "Rebound needs Whip first");
    play_as(&e, &mut s, P1, "whip", CardKind::Action);
    assert!(playable(&e, &s, P1, "rebound"));
    play(&e, &mut s, P1, "rebound");
    assert_eq!(s.player(P2).ringside.len(), 2 + 3);
}

#[test]
fn next_card_boosts_are_used_up_by_any_card() {
    let e = engine();
    let mut s = start(&e, &["whip", "catch-breath", "jab"], &[]);
    play_as(&e, &mut s, P1, "whip", CardKind::Action);
    play(&e, &mut s, P1, "catch-breath");
    play(&e, &mut s, P1, "jab");
    assert_eq!(s.player(P2).ringside.len(), 2, "the boost went to Catch Your Breath");
}

#[test]
fn next_matching_boosts_wait_for_a_maneuver() {
    let e = engine();
    let mut s = start(&e, &["hype", "catch-breath", "jab"], &[]);
    play(&e, &mut s, P1, "hype");
    play(&e, &mut s, P1, "catch-breath");
    play(&e, &mut s, P1, "jab");
    assert_eq!(s.player(P2).ringside.len(), 2 + 3);
}

#[test]
fn reversing_whip_carries_over_to_the_next_turn() {
    let e = engine();
    let mut s = start(&e, &["whip"], &["whip", "rebound"]);
    play_as(&e, &mut s, P1, "whip", CardKind::Action);
    let whip = in_hand(&e, &s, P2, "whip");
    apply(&e, &mut s, P2, Action::Reverse { card: whip });
    assert_eq!((s.active, s.turn), (P2, 2));
    assert!(playable(&e, &s, P2, "rebound"), "the first card counts as played after Whip");
    let before = s.player(P1).ringside.len();
    play(&e, &mut s, P2, "rebound");
    assert_eq!(s.player(P1).ringside.len(), before + 2 + 3);
}

#[test]
fn bonuses_lower_fortitude_after_the_right_card() {
    let e = engine();
    let mut s = start(&e, &["stunner", "jab"], &[]);
    assert!(!playable(&e, &s, P1, "stunner"));
    play(&e, &mut s, P1, "jab");
    assert!(playable(&e, &s, P1, "stunner"), "-10F right after a Jab");
}

#[test]
fn hybrid_played_as_action_is_discarded_and_deals_no_damage() {
    let e = engine();
    let mut s = start(&e, &["rope-break"], &[]);
    let rope = in_hand(&e, &s, P1, "rope-break");
    let hand = s.player(P1).hand.len();
    play_as(&e, &mut s, P1, "rope-break", CardKind::Action);
    assert_eq!(s.player(P1).ringside, vec![rope]);
    assert_eq!(s.player(P1).hand.len(), hand - 1 + 1);
    assert!(s.player(P2).ringside.is_empty());
}

#[test]
fn irreversible_boost_stops_reversals() {
    let e = engine();
    let mut s = start(&e, &["sneak", "jab"], &["block"]);
    play(&e, &mut s, P1, "sneak");
    play_as(&e, &mut s, P1, "jab", STRIKE);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main), "no chance to reverse");
    assert_eq!(s.player(P2).ringside.len(), 2);
}

#[test]
fn controller_picks_from_the_opponents_hand_and_forced_discards_trigger() {
    let e = engine();
    let mut s = start(&e, &["scout"], &["taunt"]);
    let events = play(&e, &mut s, P1, "scout");
    assert!(events.iter().any(|ev| matches!(ev, Event::HandShown { player: P2, viewer: P1, .. })));
    let taunt = in_hand(&e, &s, P2, "taunt");
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseCards { options, .. }) if options.contains(&taunt)));
    assert!(e.view(&s, P1).def_of(taunt).is_some(), "P1 sees the cards to choose from");
    apply(&e, &mut s, P1, Action::ChooseCards(vec![taunt]));
    assert_eq!(s.player(P2).hand.len(), 6 - 1 + 2, "Taunt draws 2 when the opponent discards it");
}

#[test]
fn arrange_puts_the_top_cards_back_in_the_chosen_order() {
    let e = engine();
    let mut s =
        start(&e, &["plan", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "jab", "suplex", "haymaker"], &[]);
    play(&e, &mut s, P1, "plan");
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseOption { options, .. }) if options.len() == 2));
    apply(&e, &mut s, P1, Action::ChooseOption(0));
    let arsenal = s.player(P1).arsenal.clone();
    let top: Vec<CardId> = arsenal.iter().rev().take(3).copied().collect();
    let [jab, suplex, haymaker] = [top[0], top[1], top[2]];
    apply(&e, &mut s, P1, Action::ChooseCards(vec![haymaker]));
    apply(&e, &mut s, P1, Action::ChooseCards(vec![jab]));
    let new_top: Vec<CardId> = s.player(P1).arsenal.iter().rev().take(3).copied().collect();
    assert_eq!(new_top, vec![haymaker, jab, suplex]);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
}

#[test]
fn skipped_turns_go_back_to_the_same_player() {
    let e = engine();
    let mut s = start(&e, &["stall"], &[]);
    play(&e, &mut s, P1, "stall");
    let events = apply(&e, &mut s, P1, Action::EndTurn);
    assert!(events.iter().any(|ev| matches!(ev, Event::TurnSkipped { player: P2 })));
    assert_eq!((s.active, s.turn), (P1, 2));
}

#[test]
fn end_of_turn_effects() {
    let e = engine();
    let mut s = start(&e, &["rest"], &[]);
    play(&e, &mut s, P1, "rest");
    assert_eq!(s.player(P1).hand.len(), 6 - 1 + 3);
    apply(&e, &mut s, P1, Action::EndTurn);
    assert!(s.player(P1).hand.is_empty());
    assert_eq!(s.active, P2);
}

#[test]
fn overturn_until_a_reversal_shows_up() {
    let e = engine();
    let p2 = ["lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "escape"];
    let mut s = start(&e, &["chin-lock", "hold-on"], &p2);
    assert!(!playable(&e, &s, P1, "hold-on"));
    play(&e, &mut s, P1, "chin-lock");
    play(&e, &mut s, P1, "hold-on");
    assert_eq!(s.player(P2).ringside.len(), 1 + 2, "Chin Lock's damage, then up to Escape");
    assert_eq!(s.active, P2, "Hold On ends the turn");
}

#[test]
fn disqualification_wins_even_when_overturned() {
    let e = engine();
    let p2 = ["lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "dq"];
    let mut s = start(&e, &["cheap-shot"], &p2);
    play(&e, &mut s, P1, "cheap-shot");
    assert_eq!(s.outcome, Some(Outcome::Win { winner: P2, method: WinMethod::Disqualification }));
}

#[test]
fn level_fortitude() {
    let e = engine();
    let mut s = start(&e, &["jab", "level"], &[]);
    let jab = in_hand(&e, &s, P1, "jab");
    play(&e, &mut s, P1, "jab");
    play(&e, &mut s, P1, "level");
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseCards { options, .. }) if options == &vec![jab]));
    apply(&e, &mut s, P1, Action::ChooseCards(vec![jab]));
    assert_eq!(e.fortitude(&s, P1), 0);
    assert_eq!(waiting(&s), (P1, &DecisionKind::Main));
}

#[test]
fn search_reveals_and_shuffles() {
    let e = engine();
    let mut s = start(&e, &["search", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "lock-up", "suplex"], &[]);
    play(&e, &mut s, P1, "search");
    let suplex = find(&e, &s, &s.player(P1).arsenal, "suplex").unwrap();
    assert!(matches!(waiting(&s), (P1, DecisionKind::ChooseCards { options, .. }) if options == &vec![suplex]));
    let events = apply(&e, &mut s, P1, Action::ChooseCards(vec![suplex]));
    assert!(events.iter().any(|ev| matches!(ev, Event::Revealed { card, .. } if *card == suplex)));
    assert!(events.iter().any(|ev| matches!(ev, Event::ArsenalShuffled { player: P1 })));
    assert!(s.player(P1).hand.contains(&suplex));
}
