//! The hosted match, without any networking: connections come in as ids and
//! messages, and [`HostCore`] answers with the messages to send to each.

use std::collections::BTreeMap;
use std::sync::Arc;

use rawdeal_core::{CardDb, DeckList, DeckProblem, Engine, Event, GameSetup, GameState, PlayerId, validate_deck_with};

use crate::{APP_VERSION, ClientMsg, Lobby, MatchInfo, PlayerSlot, Rejection, Role, ServerMsg, Update, compare_sets};

/// A connection to the host, numbered by the transport.
pub type ConnId = u64;

struct Member {
    name: String,
    seat: Option<PlayerId>,
    deck: Option<DeckList>,
}

pub struct HostCore {
    engine: Engine,
    info: MatchInfo,
    sets: Vec<(String, u64)>,
    seed: u64,
    members: BTreeMap<ConnId, Member>,
    state: Option<GameState>,
}

/// Messages to send, each to one connection.
pub type Outbox = Vec<(ConnId, ServerMsg)>;

impl HostCore {
    pub fn new(db: Arc<CardDb>, info: MatchInfo, seed: u64) -> Self {
        let sets = db.fingerprints();
        HostCore { engine: Engine::new(db), info, sets, seed, members: BTreeMap::new(), state: None }
    }

    pub fn info(&self) -> &MatchInfo {
        &self.info
    }

    pub fn state(&self) -> Option<&GameState> {
        self.state.as_ref()
    }

    pub fn lobby(&self) -> Lobby {
        let mut players = [None, None];
        let mut spectators = Vec::new();
        for m in self.members.values() {
            match m.seat {
                Some(p) => players[p.index()] = Some(PlayerSlot { name: m.name.clone(), ready: m.deck.is_some() }),
                None => spectators.push(m.name.clone()),
            }
        }
        Lobby { players, spectators, started: self.state.is_some() }
    }

    /// Handles a message from `conn`. A [`ServerMsg::Rejected`] in the outbox
    /// means the transport should close that connection afterwards.
    pub fn handle(&mut self, conn: ConnId, msg: ClientMsg) -> Outbox {
        match (self.members.contains_key(&conn), msg) {
            (false, ClientMsg::Hello { app, name, role, sets }) => self.join(conn, app, name, role, &sets),
            (false, _) => vec![(conn, ServerMsg::Rejected(Rejection::NoHello))],
            (true, ClientMsg::Hello { .. }) => vec![],
            (true, ClientMsg::Deck(deck)) => self.take_deck(conn, deck),
            (true, ClientMsg::Act(action)) => self.act(conn, action),
        }
    }

    /// `conn` went away. Its player seat stays taken during a match.
    pub fn disconnected(&mut self, conn: ConnId) -> Outbox {
        let Some(member) = self.members.get(&conn) else { return vec![] };
        if member.seat.is_some() && self.state.is_some() {
            return vec![];
        }
        self.members.remove(&conn);
        self.broadcast_lobby()
    }

    fn join(&mut self, conn: ConnId, app: String, name: String, role: Role, sets: &[(String, u64)]) -> Outbox {
        if app != APP_VERSION {
            return vec![(conn, ServerMsg::Rejected(Rejection::WrongVersion { host: APP_VERSION.into() }))];
        }
        if let Some(r) = compare_sets(&self.sets, sets) {
            return vec![(conn, ServerMsg::Rejected(r))];
        }
        let seat = match role {
            Role::Spectator => None,
            Role::Player => {
                let taken: Vec<PlayerId> = self.members.values().filter_map(|m| m.seat).collect();
                match [PlayerId::ONE, PlayerId::TWO].into_iter().find(|p| !taken.contains(p)) {
                    Some(p) if self.state.is_none() => Some(p),
                    _ => return vec![(conn, ServerMsg::Rejected(Rejection::Full))],
                }
            }
        };
        self.members.insert(conn, Member { name, seat, deck: None });
        let mut out = vec![(conn, ServerMsg::Welcome { seat, info: self.info.clone() })];
        out.extend(self.broadcast_lobby());
        if self.state.is_some() {
            out.push((conn, self.update_for(conn, &[])));
        }
        out
    }

    fn take_deck(&mut self, conn: ConnId, deck: DeckList) -> Outbox {
        let Some(member) = self.members.get_mut(&conn) else { return vec![] };
        if member.seat.is_none() || self.state.is_some() {
            return vec![];
        }
        let mut problems = validate_deck_with(self.engine.db(), &deck, self.info.mode, self.info.revolution);
        if !self.info.mode.is_advanced() {
            // Quick Start ignores the Backlash deck.
            problems.retain(|p| !matches!(p, DeckProblem::NotABacklashCard { .. }));
        }
        if !problems.is_empty() {
            return vec![(conn, ServerMsg::DeckRejected(problems))];
        }
        member.deck = Some(deck);
        let mut out = self.broadcast_lobby();
        out.extend(self.start_if_ready());
        out
    }

    fn start_if_ready(&mut self) -> Outbox {
        let deck = |p: PlayerId| self.members.values().find(|m| m.seat == Some(p)).and_then(|m| m.deck.clone());
        let (Some(a), Some(b)) = (deck(PlayerId::ONE), deck(PlayerId::TWO)) else { return vec![] };
        let setup =
            GameSetup { mode: self.info.mode, revolution: Some(self.info.revolution), ..GameSetup::standard([a, b]) };
        match self.engine.new_game(&setup, self.seed) {
            Ok((state, events)) => {
                self.state = Some(state);
                let mut out = self.broadcast_lobby();
                out.extend(self.broadcast_update(&events));
                out
            }
            // Decks were checked one by one; this is a host bug.
            Err(e) => self.members.keys().map(|&c| (c, ServerMsg::ActionRejected(format!("{e:?}")))).collect(),
        }
    }

    fn act(&mut self, conn: ConnId, action: rawdeal_core::Action) -> Outbox {
        let Some(seat) = self.members.get(&conn).and_then(|m| m.seat) else {
            return vec![(conn, ServerMsg::ActionRejected("spectators cannot act".into()))];
        };
        let Some(state) = self.state.as_mut() else {
            return vec![(conn, ServerMsg::ActionRejected("the match has not started".into()))];
        };
        match self.engine.apply(state, seat, action) {
            Ok(events) => self.broadcast_update(&events),
            Err(e) => vec![(conn, ServerMsg::ActionRejected(format!("{e:?}")))],
        }
    }

    fn broadcast_lobby(&self) -> Outbox {
        let lobby = self.lobby();
        self.members.keys().map(|&c| (c, ServerMsg::Lobby(lobby.clone()))).collect()
    }

    fn broadcast_update(&self, events: &[Event]) -> Outbox {
        self.members.keys().map(|&c| (c, self.update_for(c, events))).collect()
    }

    /// `events` and the table as `conn`'s seat sees them.
    fn update_for(&self, conn: ConnId, events: &[Event]) -> ServerMsg {
        let state = self.state.as_ref().expect("match started");
        let seat = self.members[&conn].seat;
        let events: Vec<Event> = events
            .iter()
            .map(|e| match seat {
                Some(p) => e.redacted_for(p),
                None => e.redacted_for_spectator(),
            })
            .collect();
        let (view, legal) = match seat {
            Some(p) => {
                let waiting = state.decision.as_ref().is_some_and(|d| d.player == p);
                let legal = if waiting { self.engine.legal_actions(state, p) } else { vec![] };
                (self.engine.view(state, p), legal)
            }
            None => (self.engine.spectator_view(state, PlayerId::ONE), vec![]),
        };
        let mut cards: Vec<_> = events.iter().flat_map(Event::cards).map(|c| (c, state.card(c).def)).collect();
        cards.sort_unstable_by_key(|(c, _)| c.0);
        cards.dedup();
        ServerMsg::Update(Box::new(Update { events, view, legal, cards }))
    }
}
