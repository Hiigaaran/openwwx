//! Scenario tests for the rules added for the Fully Loaded and Backlash sets:
//! Ring abilities, lasting restrictions, new Superstar abilities, reversal
//! limits and timing. The cards are invented for the tests.

use std::sync::Arc;

use rawdeal_core::*;

const P1: PlayerId = PlayerId::ONE;
const P2: PlayerId = PlayerId::TWO;

const SET: &str = r#"(
    name: "Mechanics",
    superstars: [
        (id: "m/first", name: "First", hand_size: 6, superstar_value: 5),
        (id: "m/second", name: "Second", hand_size: 6, superstar_value: 1),
        (id: "m/blocker", name: "Blocker", hand_size: 6, superstar_value: 1, abilities: [ReverseByDiscardingSameType]),
        (id: "m/escalator", name: "Escalator", hand_size: 6, superstar_value: 5, abilities: [EscalatingManeuverDamage]),
        (id: "m/flyer", name: "Flyer", hand_size: 6, superstar_value: 5,
            abilities: [FirstOfTurnIrreversible(ManeuverType(HighRisk))]),
        (id: "m/twice", name: "Twice", hand_size: 0, superstar_value: 1,
            abilities: [HandSizeFromOpponentValue(times: 2, min: 6)]),
        (id: "m/censor", name: "Censor", hand_size: 6, superstar_value: 1, abilities: [ReverseByDiscardingAny(Action)]),
        (id: "m/team", name: "Team", hand_size: 6, superstar_value: 5, abilities: [CountsAs(["First"])]),
        (id: "m/tough", name: "Tough", hand_size: 6, superstar_value: 1, abilities: [ReduceDamage(2)]),
        (id: "m/giant", name: "Giant", hand_size: 6, superstar_value: 3, abilities: [
            PlayCost(filter: All([Maneuver, EarlierThisTurn(filter: Maneuver, n: 1)]), discard: 2),
            OpponentDamageCap(filter: ManeuverType(Grapple), max: 1)]),
        (id: "m/sore", name: "Sore", hand_size: 6, superstar_value: 5,
            abilities: [OnOpponentReverses([If(condition: OpponentReversedFrom([Hand]), effects: [Draw(who: You, n: 2)])])]),
        (id: "m/stubborn", name: "Stubborn", hand_size: 6, superstar_value: 5,
            abilities: [OnOpponentReverses([ContinueTurn])]),
        (id: "m/smug", name: "Smug", hand_size: 6, superstar_value: 1,
            abilities: [OnYouReverse([If(condition: YouReversedFrom([Hand]), effects: [Draw(who: You, n: 1)])])]),
        (id: "m/watcher", name: "Watcher", hand_size: 6, superstar_value: 1,
            abilities: [OnOpponentPlayMatching(filter: Maneuver, effects: [Draw(who: You, n: 1)])]),
    ],
    cards: [
        (id: "m/filler", title: "Filler", kinds: [Action], fortitude: 0, damage: 0),
        (id: "m/hit", title: "Hit", kinds: [Maneuver(Strike)], fortitude: 0, damage: 2),
        (id: "m/grab", title: "Grab", kinds: [Maneuver(Grapple)], fortitude: 0, damage: 3),
        (id: "m/heavy", title: "Heavy", kinds: [Maneuver(Strike)], fortitude: 2, damage: 1),
        (id: "m/leap", title: "Leap", kinds: [Maneuver(HighRisk)], fortitude: 0, damage: 4),
        (id: "m/counter", title: "Counter", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Maneuver)))),
        (id: "m/named-counter", title: "Named Counter", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Title("Big Move"))))),
        (id: "m/big-move", title: "Big Move", kinds: [Maneuver(Grapple)], fortitude: 0, damage: 5,
            script: (only_reversed_by_title: true)),
        (id: "m/bell", title: "Bell", kinds: [Action], fortitude: 0, damage: 0,
            script: (ring_ability: Some((requires: Some(After(Maneuver)), sacrifice: true,
                effects: [Mill(who: Opponent, n: 4)])))),
        (id: "m/cover", title: "Cover", kinds: [Action], fortitude: 0, damage: 0,
            script: (while_in_ring_opponent: [(applies_to: Maneuver, damage: -2)], lapses_after_own_maneuver: true)),
        (id: "m/stop", title: "Stop", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [CannotPlay(who: Opponent, filter: Maneuver, turns: 1)])),
        (id: "m/doubler", title: "Doubler", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [BoostOpponent((applies_to: Any, duration: Turns(2), double_fortitude: true))])),
        (id: "m/dig", title: "Dig", kinds: [Action], fortitude: 0, damage: 0, script: (on_play: [Dig(n: 3, take: 1)])),
        (id: "m/mirror", title: "Mirror", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [DiscardMirror(max: 3)])),
        (id: "m/study", title: "Study", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [Search(filter: Title("Hit"), n: 3, up_to: true, random_back: 1)])),
        (id: "m/spy", title: "Spy", kinds: [Action], fortitude: 0, damage: 0, script: (reveals_opponent_hand: true)),
        (id: "m/lid", title: "Lid", kinds: [Action], fortitude: 0, damage: 0,
            script: (reverses_from_ring: Some((targets: Maneuver, damage: Fixed(3))))),
        (id: "m/follow", title: "Follow", kinds: [Maneuver(Strike)], fortitude: 0, damage: 1,
            script: (after_damage: [Draw(who: You, n: 1)])),
        (id: "m/picky", title: "Picky", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [NextCardMustBe(Title("Hit"))])),
        (id: "m/twin", title: "Twin", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [Boost((applies_to: Maneuver, duration: NextCards(2), damage: 1))])),
        (id: "m/ego", title: "Ego", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [If(condition: Superstar("First"), effects: [Draw(who: You, n: 2)])])),
        (id: "m/injury", title: "Injury", kinds: [Action], fortitude: 0, damage: 0,
            script: (blanks_opponent_ability: true)),
        (id: "m/arm", title: "Arm", kinds: [Action], fortitude: 0, damage: 0,
            script: (blanks_opponent_maneuvers: true)),
        (id: "m/backed", title: "Backed", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [OpponentChooses(n: 2, options: [[Draw(who: You, n: 1)], [Mill(who: Opponent, n: 1)],
                [Draw(who: You, n: 3)]])])),
        (id: "m/count", title: "Count", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [Counted(count: HandSize, effect: Mill(who: Opponent, n: 0))])),
        (id: "m/exile", title: "Exile", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [RemoveFromGame(who: Opponent, from: Arsenal, n: 2), RemoveThis])),
        (id: "m/proud", title: "Proud", kinds: [Action], fortitude: 0, damage: 0, script: (value_bonus: 5)),
        (id: "m/check", title: "Check", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [If(condition: SuperstarValueAtLeast(9), effects: [Draw(who: You, n: 2)])])),
        (id: "m/phoenix", title: "Phoenix", kinds: [Maneuver(Strike)], fortitude: 0, damage: 1,
            script: (from_ringside: Some(Always))),
        (id: "m/dump", title: "Dump", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [DiscardAll(who: You, filter: Title("Phoenix"))])),
        (id: "m/comeback", title: "Comeback", kinds: [Action], fortitude: 0, damage: 0,
            script: (ringside_ability: Some((cost: [Discard(who: You, n: 1, pick: Chosen)],
                effects: [ReturnThis(to: Hand)])))),
        (id: "m/grudge", title: "Grudge", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_removed_by_opponent: [Draw(who: You, n: 2)])),
        (id: "m/sweep", title: "Sweep", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [RemoveFromRing(who: Opponent, n: 1, filter: Any)])),
        (id: "m/toss", title: "Toss", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [DiscardAll(who: You, filter: Title("Comeback"))])),
        (id: "m/precise", title: "Precise", kinds: [Maneuver(Strike)], fortitude: 4, damage: 3,
            script: (fortitude_discount: Some((per: 2, max: 2)))),
        (id: "m/revival", title: "Revival", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [If(condition: CountAtLeast(count: YourRemoved, n: 1),
                effects: [MoveCards(who: You, from: Removed, to: Hand, n: 1)])])),
        (id: "m/blind-counter", title: "Blind Counter", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Maneuver)), from_ringside: Some(Always))),
        (id: "m/sink", title: "Sink", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [ThisToArsenal(bottom: true)])),
        (id: "m/float", title: "Float", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [ThisToArsenal(bottom: false)])),
        (id: "m/rebound", title: "Rebound", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_opponent_reverses: [ReversedCardTo(to: Hand)])),
        (id: "m/margin", title: "Margin", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [If(condition: CountAtLeast(count: Minus(OpponentHandSize, HandSize), n: 1),
                effects: [Draw(who: You, n: 1)])])),
        (id: "m/bury", title: "Bury", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [MoveCards(who: You, from: Hand, to: Arsenal, n: 1, filter: Some(Title("Hit")), bottom: true)])),
        (id: "m/behind", title: "Behind", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [If(condition: CountBelow(a: OpponentHandSize, b: HandPlus(1)), effects: [Draw(who: You, n: 2)])])),
        (id: "m/ambush", title: "Ambush", kinds: [Reversal], fortitude: 0, damage: 0,
            script: (reverses: Some((targets: Maneuver, ringside_only: true, reversed_removed: true)),
                from_ringside: Some(Always))),
        (id: "m/ditch", title: "Ditch", kinds: [Action], fortitude: 0, damage: 0,
            script: (on_play: [DiscardAll(who: You, filter: Title("Ambush"))])),
        (id: "m/talk", title: "Talk", kinds: [Action], fortitude: 0, damage: 0,
            script: (restricts_self: Some((filter: Action)))),
        (id: "m/duo", title: "Duo", kinds: [Action], fortitude: 0, damage: 0, superstar: Some("m/first"),
            also_superstars: ["m/second"]),
    ],
)"#;

fn engine() -> Engine {
    let set = rawdeal_cards::parse_set(SET).expect("test set parses");
    Engine::new(Arc::new(rawdeal_cards::build_db([set]).expect("consistent")))
}

/// `cards` top first, padded with Fillers to 30.
fn stack(superstar: &str, cards: &[&str]) -> DeckList {
    let mut arsenal: Vec<String> = cards.iter().map(|c| format!("m/{c}")).collect();
    arsenal.resize(30.max(cards.len()), "m/filler".to_string());
    DeckList { superstar: format!("m/{superstar}"), arsenal, backlash: vec![] }
}

/// P1 (`first`, going first) and P2 (`second`) with stacked Arsenals.
fn start(e: &Engine, first: &str, p1: &[&str], second: &str, p2: &[&str]) -> GameState {
    let setup = GameSetup {
        decks: [stack(first, p1), stack(second, p2)],
        enforce_deck_rules: false,
        shuffle_arsenals: false,
        mode: Mode::QuickStart,
    };
    let s = e.new_game(&setup, 1).expect("valid setup").0;
    assert_eq!(s.active, P1);
    s
}

fn find(e: &Engine, s: &GameState, cards: &[CardId], id: &str) -> Option<CardId> {
    let id = format!("m/{id}");
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

fn playable(e: &Engine, s: &GameState, p: PlayerId, id: &str) -> bool {
    let card = in_hand(e, s, p, id);
    e.legal_actions(s, p).iter().any(|a| matches!(a, Action::PlayCard { card: c, .. } if *c == card))
}

fn end_turn(e: &Engine, s: &mut GameState) {
    let p = s.active;
    apply(e, s, p, Action::EndTurn);
}

/// Declines a pending reversal, if any.
fn no_reversal(e: &Engine, s: &mut GameState) {
    if let Some(Decision { player, kind: DecisionKind::Reversal { .. } }) = s.decision.clone() {
        apply(e, s, player, Action::DeclineReversal);
    }
}

#[test]
fn ring_ability_after_a_maneuver_sacrifices_the_card() {
    let e = engine();
    let mut s = start(&e, "first", &["bell", "hit"], "second", &[]);
    play(&e, &mut s, P1, "bell");
    let bell = find(&e, &s, &s.player(P1).ring, "bell").unwrap();
    assert!(!e.can_use_ring_card(&s, P1, bell), "needs a maneuver first");
    play(&e, &mut s, P1, "hit");
    let before = s.player(P2).ringside.len();
    apply(&e, &mut s, P1, Action::UseRingCard { card: bell });
    assert_eq!(s.player(P2).ringside.len(), before + 4);
    assert_eq!(s.card(bell).zone, Zone::Ringside);
}

#[test]
fn cover_softens_opponent_maneuvers_until_you_land_one() {
    let e = engine();
    let mut s = start(&e, "first", &["cover", "filler", "hit"], "second", &["hit", "hit"]);
    play(&e, &mut s, P1, "cover");
    end_turn(&e, &mut s);
    play(&e, &mut s, P2, "hit");
    assert_eq!(s.player(P1).ringside.len(), 0, "2D - 2");
    end_turn(&e, &mut s);
    play(&e, &mut s, P1, "hit");
    end_turn(&e, &mut s);
    let before = s.player(P1).ringside.len();
    play(&e, &mut s, P2, "hit");
    assert_eq!(s.player(P1).ringside.len(), before + 2, "Cover lapsed");
}

#[test]
fn cannot_play_lasts_for_the_opponents_next_turn() {
    let e = engine();
    let mut s = start(&e, "first", &["stop"], "second", &["hit"]);
    play(&e, &mut s, P1, "stop");
    end_turn(&e, &mut s);
    assert!(!playable(&e, &s, P2, "hit"));
    end_turn(&e, &mut s);
    end_turn(&e, &mut s);
    assert!(playable(&e, &s, P2, "hit"));
}

#[test]
fn discarding_the_same_type_reverses_without_damage() {
    let e = engine();
    let mut s = start(&e, "first", &["hit"], "blocker", &["hit"]);
    play(&e, &mut s, P1, "hit");
    let block = in_hand(&e, &s, P2, "hit");
    match &s.decision {
        Some(Decision { player: P2, kind: DecisionKind::Reversal { options, .. } }) => {
            assert!(options.contains(&block))
        }
        other => panic!("expected a reversal window, got {other:?}"),
    }
    apply(&e, &mut s, P2, Action::Reverse { card: block });
    assert_eq!(s.active, P2);
    assert_eq!(s.player(P1).ringside.len(), 1, "only the reversed Hit");
    assert_eq!(s.card(block).zone, Zone::Ringside);
}

#[test]
fn escalating_damage_grows_with_each_maneuver() {
    let e = engine();
    let mut s = start(&e, "escalator", &["hit", "hit"], "second", &[]);
    play(&e, &mut s, P1, "hit");
    assert_eq!(s.player(P2).ringside.len(), 3);
    play(&e, &mut s, P1, "hit");
    assert_eq!(s.player(P2).ringside.len(), 3 + 4);
}

#[test]
fn first_high_risk_of_the_turn_cannot_be_reversed() {
    let e = engine();
    let mut s = start(&e, "flyer", &["leap", "leap"], "second", &["counter"]);
    play(&e, &mut s, P1, "leap");
    assert!(matches!(s.decision, Some(Decision { player: P1, kind: DecisionKind::Main })), "no reversal window");
    play(&e, &mut s, P1, "leap");
    assert!(matches!(s.decision, Some(Decision { player: P2, kind: DecisionKind::Reversal { .. } })));
}

#[test]
fn doubled_fortitude_blocks_cards_the_opponent_could_afford() {
    let e = engine();
    let mut s = start(&e, "first", &["filler", "doubler"], "second", &["grab", "heavy"]);
    end_turn(&e, &mut s);
    play(&e, &mut s, P2, "grab");
    no_reversal(&e, &mut s);
    assert!(playable(&e, &s, P2, "heavy"));
    end_turn(&e, &mut s);
    play(&e, &mut s, P1, "doubler");
    end_turn(&e, &mut s);
    assert!(!playable(&e, &s, P2, "heavy"), "F 2 x 2 is more than 3");
}

#[test]
fn some_cards_only_fall_to_reversals_that_name_them() {
    let e = engine();
    let mut s = start(&e, "first", &["big-move"], "second", &["counter", "named-counter"]);
    play(&e, &mut s, P1, "big-move");
    let named = in_hand(&e, &s, P2, "named-counter");
    match &s.decision {
        Some(Decision { kind: DecisionKind::Reversal { options, .. }, .. }) => assert_eq!(options, &vec![named]),
        other => panic!("expected a reversal window, got {other:?}"),
    }
}

#[test]
fn dig_keeps_one_and_sends_the_rest_to_the_bottom() {
    let e = engine();
    let mut s = start(&e, "first", &["dig"], "second", &[]);
    play(&e, &mut s, P1, "dig");
    let options = match &s.decision {
        Some(Decision {
            kind: DecisionKind::ChooseCards { options, purpose: ChoicePurpose::DigToHand, .. }, ..
        }) => options.clone(),
        other => panic!("expected a dig choice, got {other:?}"),
    };
    assert_eq!(options.len(), 3);
    apply(&e, &mut s, P1, Action::ChooseCards(vec![options[0]]));
    assert_eq!(s.card(options[0]).zone, Zone::Hand);
    assert!(options[1..].iter().all(|c| s.player(P1).arsenal[..2].contains(c)), "the others are at the bottom");
}

#[test]
fn mirror_discards_make_the_opponent_match() {
    let e = engine();
    let mut s = start(&e, "first", &["mirror"], "second", &[]);
    play(&e, &mut s, P1, "mirror");
    let two: Vec<CardId> = s.player(P1).hand.iter().take(2).copied().collect();
    apply(&e, &mut s, P1, Action::ChooseCards(two));
    match &s.decision {
        Some(Decision { player: P2, kind: DecisionKind::ChooseCards { min: 2, max: 2, .. } }) => {}
        other => panic!("expected P2 to discard 2, got {other:?}"),
    }
}

#[test]
fn study_keeps_all_but_a_random_card() {
    let e = engine();
    let mut s = start(
        &e,
        "first",
        &["study", "filler", "filler", "filler", "filler", "filler", "filler", "hit", "hit", "hit"],
        "second",
        &[],
    );
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "study");
    // Asked one card at a time or all at once, depending on the options.
    while let Some(Decision { player: P1, kind: DecisionKind::ChooseCards { options, max, .. } }) = s.decision.clone() {
        apply(&e, &mut s, P1, Action::ChooseCards(options[..max as usize].to_vec()));
    }
    assert_eq!(s.player(P1).hand.len(), hand - 1 + 2);
}

#[test]
fn spy_reveals_the_opponents_hand() {
    let e = engine();
    let mut s = start(&e, "first", &["spy"], "second", &[]);
    assert!(e.view(&s, P1).opponent_hand.is_none());
    play(&e, &mut s, P1, "spy");
    let view = e.view(&s, P1);
    assert_eq!(view.opponent_hand.as_ref().map(Vec::len), Some(s.player(P2).hand.len()));
    assert!(view.def_of(s.player(P2).hand[0]).is_some());
}

#[test]
fn reprints_share_copy_limits_and_logos_cross_sets() {
    assert!(same_superstar("set-b/the-rock", "set-a/the-rock"));
    assert!(!same_superstar("set-b/kane", "set-a/the-rock"));
}

#[test]
fn hand_size_can_follow_the_opponents_superstar_value() {
    let e = engine();
    let s = start(&e, "first", &[], "twice", &[]);
    assert_eq!(e.hand_size(&s, P2), 10, "twice First's Superstar Value of 5");
    assert_eq!(s.player(P2).hand.len(), 10);
}

#[test]
fn a_ring_card_can_be_sacrificed_to_reverse() {
    let e = engine();
    let mut s = start(
        &e,
        "first",
        &["filler", "filler", "filler", "filler", "filler", "filler", "filler", "hit"],
        "second",
        &["lid"],
    );
    end_turn(&e, &mut s);
    play(&e, &mut s, P2, "lid");
    end_turn(&e, &mut s);
    play(&e, &mut s, P1, "hit");
    let lid = find(&e, &s, &s.player(P2).ring, "lid").unwrap();
    match &s.decision {
        Some(Decision { player: P2, kind: DecisionKind::Reversal { options, .. } }) => assert!(options.contains(&lid)),
        other => panic!("expected a reversal window, got {other:?}"),
    }
    apply(&e, &mut s, P2, Action::Reverse { card: lid });
    assert_eq!(s.card(lid).zone, Zone::Ringside);
    assert_eq!(s.active, P2);
    assert_eq!(s.player(P1).ringside.len(), 1 + 3, "the reversed Hit plus 3 damage");
}

#[test]
fn after_damage_effects_need_the_card_to_stand() {
    let e = engine();
    let mut s = start(&e, "first", &["follow", "follow"], "second", &["counter"]);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "follow");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).hand.len(), hand, "played 1, drew 1");
    play(&e, &mut s, P1, "follow");
    let counter = in_hand(&e, &s, P2, "counter");
    apply(&e, &mut s, P2, Action::Reverse { card: counter });
    assert_eq!(s.player(P1).hand.len(), hand - 1, "reversed: no draw");
}

#[test]
fn next_card_must_match() {
    let e = engine();
    let mut s = start(&e, "first", &["picky", "hit", "grab"], "second", &[]);
    play(&e, &mut s, P1, "picky");
    assert!(!playable(&e, &s, P1, "grab"));
    assert!(playable(&e, &s, P1, "hit"));
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    assert!(playable(&e, &s, P1, "grab"));
}

#[test]
fn a_boost_can_cover_the_next_two_cards() {
    let e = engine();
    let mut s = start(&e, "first", &["twin", "hit", "hit", "hit"], "second", &[]);
    play(&e, &mut s, P1, "twin");
    let mut taken = Vec::new();
    for _ in 0..3 {
        let before = s.player(P2).ringside.len();
        play(&e, &mut s, P1, "hit");
        taken.push(s.player(P2).ringside.len() - before);
    }
    assert_eq!(taken, vec![3, 3, 2]);
}

#[test]
fn any_card_can_be_discarded_to_reverse_matching_cards() {
    let e = engine();
    let mut s = start(&e, "first", &["stop"], "censor", &[]);
    play(&e, &mut s, P1, "stop");
    let filler = in_hand(&e, &s, P2, "filler");
    match &s.decision {
        Some(Decision { player: P2, kind: DecisionKind::Reversal { options, .. } }) => {
            assert!(options.contains(&filler))
        }
        other => panic!("expected a reversal window, got {other:?}"),
    }
    apply(&e, &mut s, P2, Action::Reverse { card: filler });
    assert_eq!(s.active, P2);
    assert_eq!(s.card(filler).zone, Zone::Ringside);
}

#[test]
fn cards_with_several_logos_go_in_either_deck() {
    let e = engine();
    for star in ["m/first", "m/second"] {
        let deck = DeckList { superstar: star.into(), arsenal: vec!["m/duo".into()], backlash: vec![] };
        let problems = validate_deck(e.db(), &deck);
        assert!(!problems.iter().any(|p| matches!(p, DeckProblem::WrongSuperstar { .. })), "{star}: {problems:?}");
    }
    let deck = DeckList { superstar: "m/flyer".into(), arsenal: vec!["m/duo".into()], backlash: vec![] };
    assert!(validate_deck(e.db(), &deck).iter().any(|p| matches!(p, DeckProblem::WrongSuperstar { .. })));
}

#[test]
fn shortened_logos_still_match() {
    assert!(same_superstar("set-b/billy-gunn", "set-a/b-a-billy-gunn"));
    assert!(!same_superstar("set-b/gunn", "set-a/b-a-billy-gunnx"));
}

#[test]
fn a_team_counts_as_its_members() {
    let e = engine();
    let deck = DeckList { superstar: "m/team".into(), arsenal: vec!["m/duo".into()], backlash: vec![] };
    let problems = validate_deck(e.db(), &deck);
    assert!(!problems.iter().any(|p| matches!(p, DeckProblem::WrongSuperstar { .. })), "{problems:?}");
    let mut s = start(&e, "team", &["ego"], "second", &[]);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "ego");
    assert_eq!(s.player(P1).hand.len(), hand - 1 + 2);
}

#[test]
fn a_ring_card_can_blank_the_opponents_ability() {
    let e = engine();
    let mut s = start(&e, "first", &["hit", "injury", "hit"], "tough", &[]);
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).ringside.len(), 0, "Tough reduces the damage to 0");
    play(&e, &mut s, P1, "injury");
    no_reversal(&e, &mut s);
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).ringside.len(), 2);
}

#[test]
fn a_ring_card_can_blank_the_opponents_maneuvers() {
    let e = engine();
    let mut s = start(&e, "first", &["hit", "arm"], "second", &["follow"]);
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    play(&e, &mut s, P1, "arm");
    no_reversal(&e, &mut s);
    end_turn(&e, &mut s);
    let hand = s.player(P2).hand.len();
    play(&e, &mut s, P2, "follow");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).hand.len(), hand - 1, "Follow draws nothing");
}

#[test]
fn the_opponent_picks_two_different_options() {
    let e = engine();
    let mut s = start(&e, "first", &["backed"], "second", &[]);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "backed");
    no_reversal(&e, &mut s);
    let Some(Decision { player: P2, kind: DecisionKind::ChooseOption { options, .. } }) = s.decision.clone() else {
        panic!("P2 chooses, got {:?}", s.decision)
    };
    assert_eq!(options.len(), 3);
    apply(&e, &mut s, P2, Action::ChooseOption(0));
    let Some(Decision { player: P2, kind: DecisionKind::ChooseOption { options, .. } }) = s.decision.clone() else {
        panic!("P2 chooses again, got {:?}", s.decision)
    };
    assert_eq!(options.len(), 2);
    apply(&e, &mut s, P2, Action::ChooseOption(0));
    assert_eq!(s.player(P1).hand.len(), hand - 1 + 1, "P1 drew 1");
    assert_eq!(s.player(P2).ringside.len(), 1, "P2 overturned 1");
}

#[test]
fn later_maneuvers_can_cost_discards() {
    let e = engine();
    let mut s = start(&e, "giant", &["hit", "hit"], "second", &[]);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).hand.len(), hand - 1, "the first maneuver is free");
    play(&e, &mut s, P1, "hit");
    let Some(Decision { player: P1, kind: DecisionKind::ChooseCards { min: 2, .. } }) = s.decision.clone() else {
        panic!("P1 discards 2, got {:?}", s.decision)
    };
}

#[test]
fn opponent_grapples_can_be_capped() {
    let e = engine();
    let mut s = start(&e, "first", &["grab"], "giant", &[]);
    play(&e, &mut s, P1, "grab");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).ringside.len(), 1, "Grab's 3D is capped at 1");
}

#[test]
fn counted_effects_use_the_count() {
    let e = engine();
    let mut s = start(&e, "first", &["count"], "second", &[]);
    play(&e, &mut s, P1, "count");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).ringside.len(), s.player(P1).hand.len());
}

#[test]
fn abilities_can_trigger_on_opponent_maneuvers() {
    let e = engine();
    let mut s = start(&e, "first", &["hit"], "watcher", &[]);
    let hand = s.player(P2).hand.len();
    play(&e, &mut s, P1, "hit");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).hand.len(), hand + 1);
}

#[test]
fn a_ring_card_can_restrict_its_owner() {
    let e = engine();
    let mut s = start(&e, "first", &["talk", "filler"], "second", &[]);
    assert!(playable(&e, &s, P1, "filler"));
    play(&e, &mut s, P1, "talk");
    no_reversal(&e, &mut s);
    assert!(!playable(&e, &s, P1, "filler"));
}

#[test]
fn cards_can_be_removed_from_the_game() {
    let e = engine();
    let mut s = start(&e, "first", &["exile"], "second", &[]);
    let arsenal = s.player(P2).arsenal.len();
    play(&e, &mut s, P1, "exile");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P2).removed.len(), 2);
    assert_eq!(s.player(P2).arsenal.len(), arsenal - 2);
    assert!(s.player(P2).ringside.is_empty(), "removed cards are not damage");
    assert_eq!(s.player(P1).removed.len(), 1, "Exile removes itself");
    assert!(s.player(P1).ring.is_empty());
}

#[test]
fn ring_cards_can_change_superstar_value() {
    let e = engine();
    let mut s = start(&e, "first", &["proud", "check"], "second", &[]);
    play(&e, &mut s, P1, "proud");
    no_reversal(&e, &mut s);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P1, "check");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).hand.len(), hand + 1, "Check left the hand and drew 2");
}

#[test]
fn diva_and_female_groups_restrict_packing() {
    let star = SuperstarDef {
        id: "m/diva".into(),
        name: "Diva".into(),
        hand_size: 6,
        superstar_value: 2,
        female: true,
        traits: vec!["Diva".into()],
        text: String::new(),
        abilities: vec![],
        source_id: None,
    };
    assert!(star.is("Female") && star.is("diva") && !star.is("Male") && !star.is("Legend"));
}

#[test]
fn some_cards_are_played_from_the_ringside_pile() {
    let e = engine();
    let mut s = start(&e, "first", &["phoenix", "dump"], "second", &[]);
    play(&e, &mut s, P1, "dump");
    no_reversal(&e, &mut s);
    let phoenix = *s.player(P1).ringside.last().expect("Phoenix was discarded");
    let action = Action::PlayCard { card: phoenix, kind: CardKind::Maneuver(ManeuverType::Strike) };
    assert!(e.legal_actions(&s, P1).contains(&action));
    apply(&e, &mut s, P1, action);
    no_reversal(&e, &mut s);
    assert!(s.player(P1).ring.contains(&phoenix));
}

#[test]
fn a_ringside_card_can_return_to_hand() {
    let e = engine();
    let mut s = start(&e, "first", &["comeback", "toss", "filler"], "second", &[]);
    play(&e, &mut s, P1, "toss");
    no_reversal(&e, &mut s);
    let comeback = *s.player(P1).ringside.last().expect("discarded");
    let action = Action::UseRingCard { card: comeback };
    assert!(e.legal_actions(&s, P1).contains(&action));
    apply(&e, &mut s, P1, action);
    let filler = in_hand(&e, &s, P1, "filler");
    apply(&e, &mut s, P1, Action::ChooseCards(vec![filler]));
    assert!(s.player(P1).hand.contains(&comeback));
}

#[test]
fn ring_cards_react_when_the_opponent_removes_them() {
    let e = engine();
    let mut s = start(&e, "first", &["grudge"], "second", &["sweep"]);
    play(&e, &mut s, P1, "grudge");
    no_reversal(&e, &mut s);
    end_turn(&e, &mut s);
    let hand = s.player(P1).hand.len();
    play(&e, &mut s, P2, "sweep");
    no_reversal(&e, &mut s);
    if let Some(Decision { kind: DecisionKind::ChooseCards { options, .. }, player }) = s.decision.clone() {
        apply(&e, &mut s, player, Action::ChooseCards(vec![options[0]]));
    }
    assert_eq!(s.player(P1).hand.len(), hand + 2);
}

#[test]
fn discards_can_pay_for_fortitude() {
    let e = engine();
    let mut s = start(&e, "first", &["precise", "filler", "filler"], "second", &[]);
    assert!(playable(&e, &s, P1, "precise"), "4F at 0F: discard 2 for -4F");
    play(&e, &mut s, P1, "precise");
    if let Some(Decision { kind: DecisionKind::ChooseCards { options, min, .. }, player }) = s.decision.clone() {
        assert_eq!(min, 2);
        apply(&e, &mut s, player, Action::ChooseCards(options[..2].to_vec()));
    }
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).ringside.len(), 2);
}

#[test]
fn removed_cards_can_come_back() {
    let e = engine();
    let mut s = start(&e, "first", &["exile", "revival"], "second", &[]);
    play(&e, &mut s, P1, "exile");
    no_reversal(&e, &mut s);
    let exile = s.player(P1).removed[0];
    play(&e, &mut s, P1, "revival");
    if let Some(Decision { kind: DecisionKind::ChooseCards { options, .. }, player }) = s.decision.clone() {
        apply(&e, &mut s, player, Action::ChooseCards(vec![options[0]]));
    }
    no_reversal(&e, &mut s);
    assert!(s.player(P1).hand.contains(&exile));
}

#[test]
fn a_played_card_can_go_to_the_bottom_of_the_arsenal() {
    let e = engine();
    let mut s = start(&e, "first", &["sink"], "second", &[]);
    let sink = in_hand(&e, &s, P1, "sink");
    play(&e, &mut s, P1, "sink");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).arsenal[0], sink);
}

#[test]
fn chosen_cards_can_go_to_the_bottom_of_the_arsenal() {
    let e = engine();
    let mut s = start(&e, "first", &["bury", "hit"], "second", &[]);
    let hit = in_hand(&e, &s, P1, "hit");
    play(&e, &mut s, P1, "bury");
    if let Some(Decision { kind: DecisionKind::ChooseCards { .. }, player }) = s.decision.clone() {
        apply(&e, &mut s, player, Action::ChooseCards(vec![hit]));
    }
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).arsenal[0], hit);
}

#[test]
fn two_counts_can_be_compared() {
    let e = engine();
    let mut s = start(&e, "first", &["behind"], "second", &[]);
    let before = s.player(P1).hand.len();
    play(&e, &mut s, P1, "behind");
    no_reversal(&e, &mut s);
    // The opponent's 6 cards are fewer than 6 + 1, so it draws 2.
    assert_eq!(s.player(P1).hand.len(), before + 1);
}

#[test]
fn some_reversals_only_work_from_the_ringside_pile() {
    let e = engine();
    let mut s = start(&e, "first", &["hit", "hit"], "second", &["ambush", "ditch"]);
    play(&e, &mut s, P1, "hit");
    assert!(!matches!(s.decision, Some(Decision { kind: DecisionKind::Reversal { .. }, .. })));
    end_turn(&e, &mut s);
    play(&e, &mut s, P2, "ditch");
    no_reversal(&e, &mut s);
    let ambush = *s.player(P2).ringside.last().expect("Ambush was discarded");
    end_turn(&e, &mut s);
    let hit = in_hand(&e, &s, P1, "hit");
    play(&e, &mut s, P1, "hit");
    match s.decision.clone() {
        Some(Decision { kind: DecisionKind::Reversal { options, .. }, .. }) => assert!(options.contains(&ambush)),
        other => panic!("expected a reversal window, got {other:?}"),
    }
    apply(&e, &mut s, P2, Action::Reverse { card: ambush });
    assert!(s.player(P1).removed.contains(&hit));
}

#[test]
fn a_played_card_can_go_on_top_of_the_arsenal() {
    let e = engine();
    let mut s = start(&e, "first", &["float"], "second", &[]);
    let float = in_hand(&e, &s, P1, "float");
    play(&e, &mut s, P1, "float");
    no_reversal(&e, &mut s);
    assert_eq!(s.player(P1).arsenal.last(), Some(&float));
}

#[test]
fn ring_cards_can_react_to_a_reversal() {
    let e = engine();
    let mut s = start(&e, "first", &["rebound", "hit"], "second", &["counter"]);
    play(&e, &mut s, P1, "rebound");
    no_reversal(&e, &mut s);
    let hit = in_hand(&e, &s, P1, "hit");
    play(&e, &mut s, P1, "hit");
    let counter = in_hand(&e, &s, P2, "counter");
    apply(&e, &mut s, P2, Action::Reverse { card: counter });
    assert!(s.player(P1).hand.contains(&hit));
}

#[test]
fn counts_can_be_subtracted() {
    let e = engine();
    let mut s = start(&e, "first", &["margin"], "second", &[]);
    let before = s.player(P1).hand.len();
    play(&e, &mut s, P1, "margin");
    no_reversal(&e, &mut s);
    // 6 cards were left against the opponent's 6: no difference, no draw.
    assert_eq!(s.player(P1).hand.len(), before - 1);
}

#[test]
fn a_superstar_can_react_to_a_reversal_from_hand() {
    let e = engine();
    let mut s = start(&e, "sore", &["hit"], "second", &["counter"]);
    let before = s.player(P1).hand.len();
    play(&e, &mut s, P1, "hit");
    let counter = in_hand(&e, &s, P2, "counter");
    apply(&e, &mut s, P2, Action::Reverse { card: counter });
    assert_eq!(s.player(P1).hand.len(), before + 1);
}

#[test]
fn a_turn_can_go_on_after_a_reversal() {
    let e = engine();
    let mut s = start(&e, "stubborn", &["hit", "filler"], "second", &["counter"]);
    play(&e, &mut s, P1, "hit");
    let counter = in_hand(&e, &s, P2, "counter");
    apply(&e, &mut s, P2, Action::Reverse { card: counter });
    assert_eq!(s.active, P1);
    assert!(playable(&e, &s, P1, "filler"));
}

#[test]
fn reversing_can_trigger_your_own_cards() {
    let e = engine();
    let mut s = start(&e, "first", &["hit"], "smug", &["counter"]);
    let before = s.player(P2).hand.len();
    play(&e, &mut s, P1, "hit");
    let counter = in_hand(&e, &s, P2, "counter");
    apply(&e, &mut s, P2, Action::Reverse { card: counter });
    // The Counter left the hand, the ability drew 1 back, and P2's own turn
    // drew 1 more.
    assert_eq!(s.player(P2).hand.len(), before + 1);
}
