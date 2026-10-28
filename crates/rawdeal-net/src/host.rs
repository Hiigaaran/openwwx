//! The hosted match, without any networking: connections come in as ids and
//! messages, and [`HostCore`] answers with the messages to send to each.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rawdeal_core::{
    CardDb, DeckList, DeckProblem, Engine, Event, GameSetup, GameState, Outcome, PlayerId, WinMethod,
    validate_deck_with,
};

use crate::{APP_VERSION, ClientMsg, Lobby, MatchInfo, PlayerSlot, Rejection, Role, ServerMsg, Update, compare_sets};

/// A connection to the host, numbered by the transport.
pub type ConnId = u64;

/// How long a player who lost the connection during a match has to come
/// back before losing by forfeit.
pub const ABANDON_AFTER: Duration = Duration::from_secs(120);

/// Longest chat line passed on, in characters.
const MAX_CHAT: usize = 300;

struct Member {
    name: String,
    seat: Option<PlayerId>,
    deck: Option<DeckList>,
    /// Gets a player back into their seat.
    token: u64,
    /// When they lost the connection, if they did.
    left: Option<Instant>,
}

pub struct HostCore {
    engine: Engine,
    info: MatchInfo,
    sets: Vec<(String, u64)>,
    seed: u64,
    members: BTreeMap<ConnId, Member>,
    state: Option<GameState>,
    abandon_after: Duration,
    tokens: RandomState,
}

/// Messages to send, each to one connection.
pub type Outbox = Vec<(ConnId, ServerMsg)>;

impl HostCore {
    pub fn new(db: Arc<CardDb>, info: MatchInfo, seed: u64) -> Self {
        let sets = db.fingerprints();
        HostCore {
            engine: Engine::new(db),
            info,
            sets,
            seed,
            members: BTreeMap::new(),
            state: None,
            abandon_after: ABANDON_AFTER,
            tokens: RandomState::new(),
        }
    }

    /// Changes how long a player may be away before losing by forfeit.
    pub fn with_abandon_after(mut self, after: Duration) -> Self {
        self.abandon_after = after;
        self
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
                Some(p) => {
                    players[p.index()] =
                        Some(PlayerSlot { name: m.name.clone(), ready: m.deck.is_some(), away: m.left.is_some() })
                }
                None => spectators.push(m.name.clone()),
            }
        }
        Lobby { players, spectators, started: self.state.is_some() }
    }

    fn running(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.outcome.is_none())
    }

    /// Handles a message from `conn`. A [`ServerMsg::Rejected`] in the outbox
    /// means the transport should close that connection afterwards.
    pub fn handle(&mut self, conn: ConnId, msg: ClientMsg) -> Outbox {
        match (self.members.contains_key(&conn), msg) {
            (false, ClientMsg::Hello { app, name, role, sets, rejoin }) => {
                self.join(conn, app, name, role, &sets, rejoin)
            }
            (false, _) => vec![(conn, ServerMsg::Rejected(Rejection::NoHello))],
            (true, ClientMsg::Hello { .. }) => vec![],
            (true, ClientMsg::Deck(deck)) => self.take_deck(conn, deck),
            (true, ClientMsg::Act(action)) => self.act(conn, action),
            (true, ClientMsg::Concede) => match self.members[&conn].seat {
                Some(p) if self.running() => self.forfeit(p),
                _ => vec![],
            },
            (true, ClientMsg::Chat(text)) => {
                let text: String = text.trim().chars().take(MAX_CHAT).collect();
                if text.is_empty() {
                    return vec![];
                }
                let from = self.members[&conn].name.clone();
                self.members.keys().map(|&c| (c, ServerMsg::Chat { from: from.clone(), text: text.clone() })).collect()
            }
        }
    }

    /// `conn` went away. During a match its player seat stays taken: they
    /// can come back with their token until [`HostCore::tick`] finds them
    /// gone too long.
    pub fn disconnected(&mut self, conn: ConnId) -> Outbox {
        let running = self.running();
        let Some(member) = self.members.get_mut(&conn) else { return vec![] };
        if member.seat.is_some() && running {
            member.left = Some(Instant::now());
        } else if member.seat.is_some() && self.state.is_some() {
            // The match is over: keep the seat for the lobby.
            return vec![];
        } else {
            self.members.remove(&conn);
        }
        self.broadcast_lobby()
    }

    /// Checks for players away too long; call it every second or so.
    pub fn tick(&mut self) -> Outbox {
        if !self.running() {
            return vec![];
        }
        let gone = self.members.values().find(|m| m.left.is_some_and(|t| t.elapsed() >= self.abandon_after));
        match gone.and_then(|m| m.seat) {
            Some(p) => self.forfeit(p),
            None => vec![],
        }
    }

    /// `loser` gives up the match.
    fn forfeit(&mut self, loser: PlayerId) -> Outbox {
        let Some(state) = self.state.as_mut() else { return vec![] };
        let outcome = Outcome::Win { winner: loser.opponent(), method: WinMethod::Forfeit };
        state.outcome = Some(outcome);
        state.decision = None;
        self.broadcast_update(&[Event::GameOver(outcome)])
    }

    fn join(
        &mut self,
        conn: ConnId,
        app: String,
        name: String,
        role: Role,
        sets: &[(String, u64)],
        rejoin: Option<u64>,
    ) -> Outbox {
        if app != APP_VERSION {
            return vec![(conn, ServerMsg::Rejected(Rejection::WrongVersion { host: APP_VERSION.into() }))];
        }
        if let Some(r) = compare_sets(&self.sets, sets) {
            return vec![(conn, ServerMsg::Rejected(r))];
        }
        let back = rejoin.and_then(|t| self.members.iter().find(|(_, m)| m.seat.is_some() && m.token == t));
        let seat = match (back.map(|(&old, _)| old), role) {
            (Some(old), _) => {
                let mut member = self.members.remove(&old).expect("found");
                member.left = None;
                let seat = member.seat;
                self.members.insert(conn, member);
                seat
            }
            (None, Role::Spectator) => {
                self.add(conn, name, None);
                None
            }
            (None, Role::Player) => {
                let taken: Vec<PlayerId> = self.members.values().filter_map(|m| m.seat).collect();
                match [PlayerId::ONE, PlayerId::TWO].into_iter().find(|p| !taken.contains(p)) {
                    Some(p) if self.state.is_none() => {
                        self.add(conn, name, Some(p));
                        Some(p)
                    }
                    _ => return vec![(conn, ServerMsg::Rejected(Rejection::Full))],
                }
            }
        };
        let token = seat.map(|_| self.members[&conn].token);
        let mut out = vec![(conn, ServerMsg::Welcome { seat, info: self.info.clone(), token })];
        out.extend(self.broadcast_lobby());
        if self.state.is_some() {
            out.push((conn, self.update_for(conn, &[])));
        }
        out
    }

    fn add(&mut self, conn: ConnId, name: String, seat: Option<PlayerId>) {
        let token = self.tokens.hash_one((conn, self.seed, Instant::now()));
        self.members.insert(conn, Member { name, seat, deck: None, token, left: None });
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

    /// Connections that get messages: everyone but players who are away.
    fn present(&self) -> impl Iterator<Item = ConnId> + '_ {
        self.members.iter().filter(|(_, m)| m.left.is_none()).map(|(&c, _)| c)
    }

    fn broadcast_lobby(&self) -> Outbox {
        let lobby = self.lobby();
        self.present().map(|c| (c, ServerMsg::Lobby(lobby.clone()))).collect()
    }

    fn broadcast_update(&self, events: &[Event]) -> Outbox {
        self.present().map(|c| (c, self.update_for(c, events))).collect()
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
