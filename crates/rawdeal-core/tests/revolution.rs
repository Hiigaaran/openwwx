//! Scenario tests for the Revolution rules: Antics, Momentum, Storylines,
//! Follow-up copies and the Revolution format. The cards are invented for the
//! tests.

use std::sync::Arc;

use rawdeal_core::*;

const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

const SET: &str = r#"(
    name: "Revolution tests",
    superstars: [
        (id: "r/first", name: "Revolution First", hand_size: 6, superstar_value: 5),
        (id: "r/second", name: "Revolution Second", hand_size: 6, superstar_value: 1),
        (id: "r/old", name: "Old", hand_size: 6, superstar_value: 1),
    ],
    cards: [
        (id: "r/filler", title: "Revolution Filler", kinds: [Antic], fortitude: 0, damage: 0),
        (id: "r/slap", title: "Revolution Slap", kinds: [Maneuver(Assault)], fortitude: 0, damage: 2,
            traits: [Core]),
        (id: "r/rush", title: "Revolution Rush", kinds: [Maneuver(Throw)], fortitude: 0, damage: 3,
            traits: [Core, Momentum]),
        (id: "r/action-stop", title: "Revolution Action Stop", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Action)))),
        (id: "r/antic-stop", title: "Revolution Antic Stop", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Antic)))),
        (id: "r/counter", title: "Revolution Counter", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Maneuver)))),
        (id: "r/momentum-stop", title: "Revolution Momentum Stop", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: AnyOf([Trait(Momentum), Trait(Mean)]))))),
        (id: "r/push", title: "Revolution Push", kinds: [Antic], fortitude: 0, damage: 0,
            script: (on_play: [Boost((applies_to: Maneuver, duration: NextMatching, momentum: true))])),
        (id: "r/tough-story", title: "Revolution Tough Story", kinds: [PreMatch(Storyline)], fortitude: 0,
            damage: 0, script: (storyline: [ReduceDamage(2)])),
        (id: "r/chain", title: "Revolution Chain", kinds: [Maneuver(Hold)], fortitude: 0, damage: 1,
            traits: [Core, FollowUp], script: (play_after: Some(Maneuver))),
        (id: "r/eye-poke", title: "Revolution Eye Poke", kinds: [Maneuver(Assault), Antic], fortitude: 0, damage: 2,
            script: (on_play: [Mill(who: Opponent, n: 1)], as_action: Some((on_play: [Draw(who: You, n: 1)])))),
        (id: "r/old-card", title: "Old Card", kinds: [Action], fortitude: 0, damage: 0),
    ],
)"#;

fn engine() -> Engine {
    let set = rawdeal_cards::parse_set(SET).expect("test set parses");
    Engine::new(Arc::new(rawdeal_cards::build_db([set]).expect("consistent")))
}

/// `cards` top first, padded with Fillers to 30.
fn stack(superstar: &str, cards: &[&str], backlash: &[&str]) -> DeckList {
    let mut arsenal: Vec<String> = cards.iter().map(|c| format!("r/{c}")).collect();
    arsenal.resize(30.max(cards.len()), "r/filler".to_string());
    DeckList {
        superstar: format!("r/{superstar}"),
        arsenal,
        backlash: backlash.iter().map(|c| format!("r/{c}")).collect(),
        backstage: vec![],
    }
}

fn start_in(e: &Engine, mode: Mode, p1: DeckList, p2: DeckList) -> GameState {
    let setup = GameSetup { decks: [p1, p2], enforce_deck_rules: false, shuffle_arsenals: false, mode };
    e.new_game(&setup, 1).expect("valid setup").0
}

fn start(e: &Engine, p1: &[&str], p2: &[&str]) -> GameState {
    let s = start_in(e, Mode::QuickStart, stack("first", p1, &[]), stack("second", p2, &[]));
    assert_eq!(s.active, P1);
    s
}

fn find(e: &Engine, s: &GameState, cards: &[CardId], id: &str) -> Option<CardId> {
    let id = format!("r/{id}");
    cards.iter().copied().find(|&c| e.db().card(s.card(c).def).id == id)
}

fn in_hand(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> CardId {
    find(e, s, &s.player(p).hand, id).unwrap_or_else(|| panic!("{id} not in {p}'s hand"))
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

/// The Reversals the defender is offered against the card just played.
fn reversal_options(e: &Engine, s: &GameState) -> Vec<String> {
    match &s.decision {
        Some(Decision { kind: DecisionKind::Reversal { options, .. }, .. }) => {
            options.iter().map(|&c| e.db().card(s.card(c).def).id.clone()).collect()
        }
        _ => vec![],
    }
}

fn no_reversal(e: &Engine, s: &mut GameState) {
    if let Some(Decision { player, kind: DecisionKind::Reversal { .. } }) = s.decision.clone() {
        apply(e, s, player, Action::DeclineReversal);
    }
}

#[test]
fn action_reversals_cannot_stop_antics() {
    let e = engine();
    let mut s = start(&e, &["filler"], &["action-stop", "antic-stop"]);
    play(&e, &mut s, P1, "filler");
    assert_eq!(reversal_options(&e, &s), vec!["r/antic-stop"]);
}

#[test]
fn momentum_needs_a_reversal_that_names_it() {
    let e = engine();
    let mut s = start(&e, &["rush", "slap"], &["counter", "momentum-stop"]);
    play(&e, &mut s, P1, "rush");
    assert_eq!(reversal_options(&e, &s), vec!["r/momentum-stop"]);
    no_reversal(&e, &mut s);
    // A plain Core is reversed by the generic Reversal; Momentum Stop names Mean, not Core.
    play(&e, &mut s, P1, "slap");
    assert_eq!(reversal_options(&e, &s), vec!["r/counter"]);
}

#[test]
fn a_boost_can_give_momentum() {
    let e = engine();
    let mut s = start(&e, &["push", "slap"], &["counter", "momentum-stop"]);
    play(&e, &mut s, P1, "push");
    no_reversal(&e, &mut s);
    play(&e, &mut s, P1, "slap");
    assert_eq!(reversal_options(&e, &s), vec!["r/momentum-stop"]);
}

/// Plays every Pre-match card offered until the match starts.
fn play_pre_match(e: &Engine, s: &mut GameState) -> usize {
    let mut played = 0;
    while s.turn == 0 {
        let p = s.decision.as_ref().unwrap().player;
        match e.legal_actions(s, p).into_iter().find(|a| matches!(a, Action::PlayCard { .. })) {
            Some(a) => {
                played += 1;
                apply(e, s, p, a);
            }
            None => apply(e, s, p, Action::Pass),
        }
    }
    played
}

#[test]
fn a_storyline_replaces_the_superstar_ability() {
    let e = engine();
    let mut s = start_in(&e, Mode::Advanced, stack("first", &["slap"], &[]), stack("second", &[], &["tough-story"]));
    assert_eq!(play_pre_match(&e, &mut s), 1);
    assert!(find(&e, &s, &s.player(P2).ring, "tough-story").is_some());
    play(&e, &mut s, P1, "slap");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).ringside.len(), 0, "the Storyline's ability reduces the 2D Slap by 2");
}

#[test]
fn aggression_rule_overturns_six_without_a_maneuver() {
    let e = engine();
    let mut s = start_in(&e, Mode::Revolution, stack("first", &[], &[]), stack("second", &["slap"], &[]));
    assert_eq!(s.player(P1).hand.len(), 6 + 1, "Hand Size and the first draw");
    let events = e.apply(&mut s, P1, Action::EndTurn).unwrap();
    assert!(events.contains(&Event::Aggression { player: P1 }));
    assert_eq!(s.player(P1).ringside.len(), 6);
    // P2 tries a maneuver: no Aggression.
    play(&e, &mut s, P2, "slap");
    no_reversal(&e, &mut s);
    let events = e.apply(&mut s, P2, Action::EndTurn).unwrap();
    assert!(!events.contains(&Event::Aggression { player: P2 }));
    assert_eq!(s.player(P2).ringside.len(), 0);
}

#[test]
fn revolution_hands_are_drawn_before_the_pre_match_phase() {
    let e = engine();
    let s = start_in(&e, Mode::Revolution, stack("first", &[], &[]), stack("second", &[], &["tough-story"]));
    assert!(s.turn == 0, "still in the Pre-match phase");
    assert_eq!(s.player(P2).hand.len(), 6);
}

#[test]
fn follow_ups_have_no_copy_limit_and_revolution_allows_only_revolution_cards() {
    let e = engine();
    let db = e.db();
    let mut deck = stack("first", &[], &[]);
    deck.arsenal = vec!["r/chain".to_string(); 30];
    deck.arsenal.resize(60, "r/filler".into());
    assert!(
        !validate_deck(db, &deck)
            .iter()
            .any(|p| matches!(p, DeckProblem::TooManyCopies { card, .. } if card == "r/chain"))
    );
    assert!(
        !validate_deck_in(db, &deck, Mode::Revolution).iter().any(|p| matches!(p, DeckProblem::NotRevolution { .. }))
    );
    deck.arsenal[59] = "r/old-card".into();
    assert!(
        validate_deck_in(db, &deck, Mode::Revolution)
            .contains(&DeckProblem::NotRevolution { card: "r/old-card".into() })
    );
    deck.superstar = "r/old".into();
    assert!(
        validate_deck_in(db, &deck, Mode::Revolution).contains(&DeckProblem::NotRevolution { card: "r/old".into() })
    );
}

#[test]
fn revolution_ring_holds_three_pre_match_cards() {
    let e = engine();
    let many = ["tough-story"; 5];
    let mut s = start_in(&e, Mode::Revolution, stack("first", &[], &many), stack("second", &[], &[]));
    let played = play_pre_match(&e, &mut s);
    assert_eq!(played, 3);
    assert_eq!(s.player(P1).ring.len(), 3);
}

#[test]
fn a_maneuver_antic_hybrid_played_as_antic_uses_its_antic_text() {
    let e = engine();
    let mut s = start(&e, &["eye-poke"], &[]);
    let card = in_hand(&e, &s, P1, "eye-poke");
    let hand = s.player(P1).hand.len();
    apply(&e, &mut s, P1, Action::PlayCard { card, kind: CardKind::Antic });
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).hand.len(), hand, "played 1, drew 1");
    assert_eq!(s.player(P2).ringside.len(), 0, "no damage and no Assault text");
}
