//! Where a match on screen comes from: played here against the computer, or
//! hosted somewhere and reached over the network. The match screen only uses
//! what its seat may see.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use rawdeal_ai::Bot;
use rawdeal_core::{Action, CardDb, CardId, DeckList, DefIdx, Engine, Event, GameState, PlayerId, PlayerView, Seat};
use rawdeal_net::host::ABANDON_AFTER;
use rawdeal_net::{APP_VERSION, ClientMsg, Connection, Dial, HostHandle, Lobby, MatchInfo, Rejection, Role, ServerMsg};

/// Wait between attempts to get back into a match.
const RETRY: Duration = Duration::from_secs(3);

pub struct Table {
    pub db: Arc<CardDb>,
    source: Source,
}

enum Source {
    Local { engine: Engine, state: Box<GameState>, bot: Box<dyn Bot + Send> },
    Online(Box<Online>),
}

/// A match hosted here or elsewhere, as one seat sees it.
pub struct Online {
    conn: Connection,
    /// Keeps a match this app hosts running.
    host: Option<HostHandle>,
    deck: Option<DeckList>,
    name: String,
    role: Role,
    /// How to reach the match again if the connection drops.
    dial: Option<Dial>,
    /// Gets you back into your seat.
    token: Option<u64>,
    reconnecting: Option<Reconnecting>,
    pub seat: Option<PlayerId>,
    pub info: Option<MatchInfo>,
    pub lobby: Option<Lobby>,
    view: Option<PlayerView>,
    legal: Vec<Action>,
    known: HashMap<CardId, DefIdx>,
    /// Why the match cannot go on, for display.
    pub problem: Option<String>,
    /// Lines for the match log, such as a rejected action.
    notes: Vec<String>,
}

impl Table {
    pub fn local(db: Arc<CardDb>, state: GameState, bot: Box<dyn Bot + Send>) -> Self {
        Table { source: Source::Local { engine: Engine::new(db.clone()), state: Box::new(state), bot }, db }
    }

    /// Joins a match through `conn`: as a player with `deck`, or as a
    /// spectator without one. `host` is the match this app hosts, if any;
    /// `dial` reaches the match again if the connection drops.
    pub fn online(
        db: Arc<CardDb>,
        conn: Connection,
        name: &str,
        deck: Option<DeckList>,
        host: Option<HostHandle>,
        dial: Option<Dial>,
    ) -> Self {
        let role = if deck.is_some() { Role::Player } else { Role::Spectator };
        let mut online = Online {
            conn,
            host,
            deck,
            name: name.into(),
            role,
            dial,
            token: None,
            reconnecting: None,
            seat: None,
            info: None,
            lobby: None,
            view: None,
            legal: vec![],
            known: HashMap::new(),
            problem: None,
            notes: vec![],
        };
        online.hello(&db);
        Table { db, source: Source::Online(Box::new(online)) }
    }

    pub fn online_state(&self) -> Option<&Online> {
        match &self.source {
            Source::Online(o) => Some(o),
            Source::Local { .. } => None,
        }
    }

    /// Whose side of the table this is.
    pub fn me(&self) -> PlayerId {
        match &self.source {
            Source::Local { .. } => PlayerId::ONE,
            Source::Online(o) => o.seat.unwrap_or(PlayerId::ONE),
        }
    }

    /// The table as this seat sees it; `None` while an online match has not
    /// started.
    pub fn view(&self) -> Option<PlayerView> {
        match &self.source {
            Source::Local { engine, state, .. } => Some(engine.view(state, PlayerId::ONE)),
            Source::Online(o) => o.view.clone(),
        }
    }

    pub fn legal(&self) -> Vec<Action> {
        match &self.source {
            Source::Local { engine, state, .. } => engine.legal_actions(state, PlayerId::ONE),
            Source::Online(o) => o.legal.clone(),
        }
    }

    /// The card's definition, if this seat has seen it.
    pub fn def(&self, card: CardId) -> Option<DefIdx> {
        match &self.source {
            Source::Local { state, .. } => Some(state.card(card).def),
            Source::Online(o) => o.known.get(&card).copied().or_else(|| o.view.as_ref().and_then(|v| v.def_of(card))),
        }
    }

    /// Plays `action` for this seat. Events come back from [`Table::act`]
    /// here, or later from [`Table::poll`] online.
    pub fn act(&mut self, action: Action) -> Result<Vec<Event>, String> {
        match &mut self.source {
            Source::Local { engine, state, .. } => engine
                .apply(state, PlayerId::ONE, action)
                .map(|events| events.iter().map(|e| e.redacted_for(PlayerId::ONE)).collect())
                .map_err(|e| e.to_string()),
            Source::Online(o) => {
                o.legal.clear();
                o.conn.send(ClientMsg::Act(action)).map(|_| vec![]).map_err(|e| e.to_string())
            }
        }
    }

    /// Does the computer have a move to make here?
    pub fn computer_to_move(&self) -> bool {
        match &self.source {
            Source::Local { state, .. } => {
                state.outcome.is_none() && state.decision.as_ref().is_some_and(|d| d.player != PlayerId::ONE)
            }
            Source::Online(_) => false,
        }
    }

    /// The computer's move, and what happened.
    pub fn computer_move(&mut self) -> Vec<Event> {
        let Source::Local { engine, state, bot } = &mut self.source else { return vec![] };
        let Some(player) = state.decision.as_ref().map(|d| d.player) else { return vec![] };
        let action = bot.choose(&Seat::new(engine, state, player));
        match engine.apply(state, player, action) {
            Ok(events) => events.iter().map(|e| e.redacted_for(PlayerId::ONE)).collect(),
            Err(_) => vec![],
        }
    }

    /// Takes in what the host sent: new events to replay, if any.
    pub fn poll(&mut self) -> Vec<Event> {
        let Source::Online(o) = &mut self.source else { return vec![] };
        o.reconnect(&self.db);
        let mut events = Vec::new();
        while o.reconnecting.is_none() {
            let msg = match o.conn.try_recv() {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(_) => {
                    o.lost();
                    break;
                }
            };
            match msg {
                ServerMsg::Welcome { seat, info, token } => {
                    o.seat = seat;
                    o.info = Some(info);
                    o.token = token;
                    o.problem = None;
                    if let Some(deck) = o.deck.clone() {
                        o.conn.send(ClientMsg::Deck(deck)).ok();
                    }
                }
                ServerMsg::Chat { from, text } => o.notes.push(format!("» {from}: {text}")),
                ServerMsg::Rejected(r) => o.problem = Some(rejection(&r)),
                ServerMsg::Lobby(lobby) => o.lobby = Some(lobby),
                ServerMsg::DeckRejected(problems) => {
                    o.problem = Some(format!("Tu mazo no es legal para esta partida: {problems:?}"))
                }
                ServerMsg::ActionRejected(e) => o.notes.push(format!("(jugada rechazada: {e})")),
                ServerMsg::Update(u) => {
                    o.known.extend(u.cards.iter().copied());
                    events.extend(u.events);
                    o.legal = u.legal;
                    o.view = Some(u.view);
                }
            }
        }
        events
    }

    /// Says `text` in the table chat.
    pub fn chat(&mut self, text: &str) {
        if let Source::Online(o) = &mut self.source {
            o.conn.send(ClientMsg::Chat(text.into())).ok();
        }
    }

    /// Leaving an online match you play: you concede it.
    pub fn leave(&mut self) {
        if let Source::Online(o) = &mut self.source
            && o.seat.is_some()
            && o.view.as_ref().is_some_and(|v| v.outcome.is_none())
        {
            o.conn.send(ClientMsg::Concede).ok();
        }
    }

    /// Log lines from the host since the last call.
    pub fn notes(&mut self) -> Vec<String> {
        match &mut self.source {
            Source::Online(o) => std::mem::take(&mut o.notes),
            Source::Local { .. } => vec![],
        }
    }

    /// Port of the match this app hosts.
    pub fn hosting_port(&self) -> Option<u16> {
        self.online_state().and_then(|o| o.host.as_ref()).and_then(HostHandle::port)
    }

    /// Does this app host the match?
    pub fn hosting(&self) -> bool {
        self.online_state().is_some_and(|o| o.host.is_some())
    }
}

/// Trying to get back into a match after losing the connection.
struct Reconnecting {
    since: Instant,
    /// The attempt under way.
    attempt: Option<Receiver<io::Result<Connection>>>,
    /// When to try again.
    next: Instant,
}

impl Online {
    fn hello(&mut self, db: &CardDb) {
        let hello = ClientMsg::Hello {
            app: APP_VERSION.into(),
            name: self.name.clone(),
            role: self.role,
            sets: db.fingerprints(),
            rejoin: self.token,
        };
        if let Err(e) = self.conn.send(hello) {
            self.problem = Some(format!("No se pudo conectar: {e}"));
        }
    }

    /// The connection dropped: try to get back in, while the match goes on.
    fn lost(&mut self) {
        let over = self.view.as_ref().is_some_and(|v| v.outcome.is_some());
        if over || self.problem.is_some() {
            return;
        }
        if self.dial.is_some() {
            self.problem = Some("Se cortó la conexión. Intentando volver a la partida…".into());
            let now = Instant::now();
            self.reconnecting = Some(Reconnecting { since: now, attempt: None, next: now });
        } else {
            self.problem = Some("Se cortó la conexión con la partida.".into());
        }
    }

    /// One step of getting back in: start an attempt, or take its result.
    fn reconnect(&mut self, db: &CardDb) {
        let Some(Reconnecting { since, attempt, next }) = &mut self.reconnecting else { return };
        let Some(dial) = &self.dial else { return };
        match attempt.as_ref().map(|rx| rx.try_recv()) {
            None if Instant::now() >= *next => {
                let (tx, rx) = mpsc::channel();
                let dial = dial.clone();
                std::thread::spawn(move || tx.send(dial.connect()).ok());
                *attempt = Some(rx);
            }
            Some(Ok(Ok(conn))) => {
                self.conn = conn;
                self.reconnecting = None;
                self.hello(db);
            }
            Some(Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected)) => {
                if since.elapsed() >= ABANDON_AFTER {
                    self.reconnecting = None;
                    self.problem = Some("No se pudo volver a la partida.".into());
                } else {
                    *attempt = None;
                    *next = Instant::now() + RETRY;
                }
            }
            _ => {}
        }
    }
}

fn rejection(r: &Rejection) -> String {
    match r {
        Rejection::WrongVersion { host } => {
            format!("La partida usa OpenWWX {host} y tú tienes {APP_VERSION}. Tienen que usar la misma versión.")
        }
        Rejection::SetsDiffer { missing, different, extra } => {
            let mut parts = Vec::new();
            if !missing.is_empty() {
                parts.push(format!("te faltan {}", missing.join(", ")));
            }
            if !different.is_empty() {
                parts.push(format!("son distintos {}", different.join(", ")));
            }
            if !extra.is_empty() {
                parts.push(format!("la partida no tiene {}", extra.join(", ")));
            }
            if parts.is_empty() {
                parts.push("los sets están en otro orden".into());
            }
            format!("Los sets de cartas no coinciden: {}.", parts.join("; "))
        }
        Rejection::Full => "Los dos lugares de jugador ya están ocupados. Puedes entrar a mirar.".into(),
        Rejection::NoHello => "La partida no entendió la conexión.".into(),
        Rejection::NoMatch => "Esa partida ya no está en el servidor.".into(),
    }
}
