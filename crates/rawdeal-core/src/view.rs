//! What one player is allowed to know. AIs and network clients only ever get
//! this, never the full [`GameState`].

use serde::{Deserialize, Serialize};

use crate::db::{DefIdx, SuperstarIdx};
use crate::decision::{Decision, DecisionKind};
use crate::engine::Engine;
use crate::state::{CardId, GameState, Outcome, Played, PlayerId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicPlayer {
    pub superstar: SuperstarIdx,
    pub arsenal_count: usize,
    pub hand_count: usize,
    pub backlash_count: usize,
    pub ring: Vec<CardId>,
    pub ringside: Vec<CardId>,
    pub fortitude: u16,
    /// The chosen Enforcer (public): Enforcer card and index on it.
    #[serde(default)]
    pub enforcer: Option<(DefIdx, u8)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerView {
    pub me: PlayerId,
    pub active: PlayerId,
    pub turn: u32,
    pub outcome: Option<Outcome>,
    pub players: [PublicPlayer; 2],
    pub hand: Vec<CardId>,
    pub backlash: Vec<CardId>,
    /// The opponent's hand, when a card makes them play with it revealed.
    #[serde(default)]
    pub opponent_hand: Option<Vec<CardId>>,
    pub in_play: Option<Played>,
    /// The pending decision, if it is mine.
    pub decision: Option<Decision>,
    /// Who the game is waiting on.
    pub waiting_on: Option<PlayerId>,
    /// Card definitions this player can see, indexed by `CardId`.
    known: Vec<Option<DefIdx>>,
}

impl PlayerView {
    pub(crate) fn new(engine: &Engine, state: &GameState, me: PlayerId) -> Self {
        let players = [PlayerId::ONE, PlayerId::TWO].map(|p| {
            let ps = state.player(p);
            PublicPlayer {
                superstar: ps.superstar,
                arsenal_count: ps.arsenal.len(),
                hand_count: ps.hand.len(),
                backlash_count: ps.backlash.len(),
                ring: ps.ring.clone(),
                ringside: ps.ringside.clone(),
                fortitude: engine.fortitude(state, p),
                enforcer: ps.enforcer,
            }
        });
        let hand = state.player(me).hand.clone();
        let backlash = state.player(me).backlash.clone();
        let opponent_hand = engine.hand_revealed_to(state, me).then(|| state.player(me.opponent()).hand.clone());

        let mut known = vec![None; state.cards.len()];
        let visible = players
            .iter()
            .flat_map(|p| p.ring.iter().chain(&p.ringside))
            .chain(&hand)
            .chain(&backlash)
            .chain(&state.player(me).under)
            .chain(opponent_hand.iter().flatten())
            .chain(state.in_play.as_ref().map(|p| &p.card));
        // Cards you are asked to choose from (an opponent's hand, the top of an
        // Arsenal) are shown to you.
        let choosing = match &state.decision {
            Some(Decision { player, kind: DecisionKind::ChooseCards { options, .. } }) if *player == me => {
                options.as_slice()
            }
            _ => &[],
        };
        for &c in visible.chain(choosing) {
            known[c.0 as usize] = Some(state.card(c).def);
        }

        PlayerView {
            me,
            active: state.active,
            turn: state.turn,
            outcome: state.outcome,
            players,
            hand,
            backlash,
            opponent_hand,
            in_play: state.in_play,
            decision: state.decision.clone().filter(|d| d.player == me),
            waiting_on: state.decision.as_ref().map(|d| d.player),
            known,
        }
    }

    /// The definition of `card`, if this player can see it.
    pub fn def_of(&self, card: CardId) -> Option<DefIdx> {
        self.known.get(card.0 as usize).copied().flatten()
    }

    pub fn me_public(&self) -> &PublicPlayer {
        &self.players[self.me.index()]
    }

    pub fn opponent(&self) -> &PublicPlayer {
        &self.players[self.me.opponent().index()]
    }
}
