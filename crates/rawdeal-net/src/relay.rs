//! The common server: player accounts, a list of open matches, and a relay
//! between each match's host and its guests. It runs no match and needs no
//! card data: hosts still run their matches ([`crate::HostCore`]), and the
//! server only passes messages along, so nobody has to open a port on their
//! router.
//!
//! Connections are encrypted ([`crate::secure`]). A connection's first line
//! says what it is for ([`ToServer`]):
//! - `Register`, `Login`, `Logout`, `List`, `Ranking`, `History`: the
//!   server answers once and closes.
//! - `Host`: the server answers `Result<Hosted, Refusal>`, then sends
//!   [`HostIn`] lines and takes [`HostOut`] lines.
//! - `Join`: from then on the connection is a guest's link to that match,
//!   with the same messages as a direct connection to the host. The server
//!   puts the account's name in the guest's `Hello`.
//!
//! Hosting and joining need a session from `Register` or `Login`. The
//! server keeps each finished match between two accounts ([`crate::stats`]).

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustls::ServerConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use rawdeal_core::{Outcome, PlayerId, WinMethod};

use crate::accounts::{Accounts, Session};
use crate::host::ConnId;
use crate::pump::{Writer, parse, pump};
use crate::secure::{self, Pin};
use crate::stats::{Record, Standing, Stats};
use crate::wire::{read_msg, write_msg};
use crate::{ClientMsg, Lobby, MatchInfo, Rejection, ServerMsg};

/// Port the common server listens on unless told otherwise.
pub const DEFAULT_SERVER_PORT: u16 = 7778;

/// Most matches listed at once, so a broken client cannot fill memory.
const MAX_MATCHES: usize = 500;

/// Longest name kept for a match.
const MAX_NAME: usize = 40;

/// Identifies a match on the server.
pub type MatchId = u64;

/// A connection's first line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    /// Make an account; answered with `Result<Session, Refusal>`.
    Register { name: String, password: String },
    /// Answered with `Result<Session, Refusal>`.
    Login { name: String, password: String },
    /// End a session; answered with `()`.
    Logout { session: String },
    /// List this match. `app` is the host's OpenWWX version; the host's
    /// name is the session's account.
    Host { app: String, session: String, info: MatchInfo },
    /// Connect to match `id` as a guest.
    Join { id: MatchId, session: String },
    /// Ask for the open matches.
    List,
    /// Ask for the best players; answered with `Vec<Standing>`.
    Ranking,
    /// Ask for a player's matches; answered with [`PlayerHistory`].
    History { name: String },
}

/// A player's rating and latest matches, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerHistory {
    /// `None` until they finish a match.
    pub standing: Option<Standing>,
    pub records: Vec<Record>,
}

/// Why the server said no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// Someone has that name already.
    NameTaken,
    /// Names are 3 to 20 letters, digits, spaces, `_`, `-` or `.`.
    BadName,
    /// Passwords are at least [`crate::accounts::MIN_PASSWORD`] long.
    BadPassword,
    /// Wrong name or password.
    WrongPassword,
    /// The session ended, or never was: log in again.
    NotLoggedIn,
    /// No account by that name.
    NoAccount,
    /// The server lists as many matches as it can.
    Full,
}

/// The server's answer to [`ToServer::Host`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hosted {
    pub id: MatchId,
}

/// One match in the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub id: MatchId,
    pub app: String,
    pub host: String,
    pub info: MatchInfo,
    /// Who is at the table, as the host last said.
    pub lobby: Option<Lobby>,
}

/// What the server tells a host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostIn {
    Joined { conn: ConnId },
    Msg { conn: ConnId, msg: ClientMsg },
    Gone { conn: ConnId },
}

/// What a host tells the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HostOut {
    /// Pass `msg` to guest `conn`.
    Send { conn: ConnId, msg: ServerMsg },
    /// Disconnect guest `conn`.
    Close { conn: ConnId },
    /// Who is at the table now, for the list.
    Lobby(Lobby),
    /// The match ended so, for the history.
    Over(Outcome),
}

/// Sends `msg` to the server at `addr` and reads its one answer.
fn ask<T: DeserializeOwned>(addr: impl ToSocketAddrs, pin: &Pin, msg: &ToServer) -> io::Result<T> {
    let mut tls = secure::client(connect(addr)?, pin)?;
    tls.sock.set_read_timeout(Some(Duration::from_secs(15)))?;
    write_msg(&mut tls, msg)?;
    read_msg(&mut BufReader::new(tls))?.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
}

/// The open matches on the server at `addr`.
pub fn list(addr: impl ToSocketAddrs, pin: &Pin) -> io::Result<Vec<Listing>> {
    ask(addr, pin, &ToServer::List)
}

/// Makes an account on the server at `addr`, and logs in with it.
pub fn register(
    addr: impl ToSocketAddrs,
    pin: &Pin,
    name: &str,
    password: &str,
) -> io::Result<Result<Session, Refusal>> {
    ask(addr, pin, &ToServer::Register { name: name.into(), password: password.into() })
}

pub fn login(addr: impl ToSocketAddrs, pin: &Pin, name: &str, password: &str) -> io::Result<Result<Session, Refusal>> {
    ask(addr, pin, &ToServer::Login { name: name.into(), password: password.into() })
}

/// The best players on the server at `addr`, best first.
pub fn ranking(addr: impl ToSocketAddrs, pin: &Pin) -> io::Result<Vec<Standing>> {
    ask(addr, pin, &ToServer::Ranking)
}

/// `name`'s rating and latest matches on the server at `addr`.
pub fn history(addr: impl ToSocketAddrs, pin: &Pin, name: &str) -> io::Result<PlayerHistory> {
    ask(addr, pin, &ToServer::History { name: name.into() })
}

pub fn logout(addr: impl ToSocketAddrs, pin: &Pin, session: &str) -> io::Result<()> {
    ask(addr, pin, &ToServer::Logout { session: session.into() })
}

pub(crate) fn connect(addr: impl ToSocketAddrs) -> io::Result<TcpStream> {
    let addr = addr.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("no address"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

type Shared<T> = Arc<Mutex<T>>;

struct Listed {
    listing: Listing,
    host: Writer,
    /// Each guest and their account.
    guests: HashMap<ConnId, (Writer, String)>,
    /// The accounts the host seated as players.
    seats: [Option<String>; 2],
    /// The result is in the history.
    recorded: bool,
}

impl Listed {
    fn started(&self) -> bool {
        self.listing.lobby.as_ref().is_some_and(|l| l.started)
    }

    /// The two players' accounts: guests in the seats the host gave them,
    /// and the host in the other one.
    fn players(&self) -> Option<[String; 2]> {
        let host = || Some(self.listing.host.clone());
        let players = match &self.seats {
            [Some(a), Some(b)] => [a.clone(), b.clone()],
            [Some(a), None] => [a.clone(), host()?],
            [None, Some(b)] => [host()?, b.clone()],
            [None, None] => return None,
        };
        (players[0].to_lowercase() != players[1].to_lowercase()).then_some(players)
    }

    /// Puts the result in the history, once.
    fn record(&mut self, stats: &Stats, outcome: Outcome) {
        if self.recorded {
            return;
        }
        let Some(players) = self.players() else { return };
        self.recorded = true;
        let (winner, method) = match outcome {
            Outcome::Win { winner, method } => (Some(winner.index() as u8), Some(method)),
            Outcome::Draw => (None, None),
        };
        stats.record(players, winner, method, self.listing.info.mode);
    }
}

#[derive(Default)]
struct Matches {
    matches: HashMap<MatchId, Listed>,
    next_id: MatchId,
    next_conn: ConnId,
}

struct Server {
    matches: Shared<Matches>,
    accounts: Arc<Accounts>,
    stats: Arc<Stats>,
}

impl Server {
    fn matches(&self) -> std::sync::MutexGuard<'_, Matches> {
        self.matches.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Runs the common server on `listener` until the process ends.
pub fn serve(listener: TcpListener, tls: Arc<ServerConfig>, accounts: Arc<Accounts>, stats: Arc<Stats>) {
    let server = Arc::new(Server { matches: Arc::default(), accounts, stats });
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        stream.set_nodelay(true).ok();
        let (server, tls) = (server.clone(), tls.clone());
        thread::spawn(move || {
            if let Ok(stream) = secure::server(stream, &tls) {
                serve_one(&server, stream);
            }
        });
    }
}

fn serve_one(server: &Server, stream: secure::Tls<rustls::ServerConnection>) {
    let Ok(pump) = pump(stream) else { return };
    let Ok(first) = pump.recv::<ToServer>(Duration::from_secs(15)) else { return };
    let w = &pump.writer;
    let accounts = &server.accounts;
    match first {
        ToServer::Register { name, password } => {
            w.send(&accounts.register(&name, &password));
        }
        ToServer::Login { name, password } => {
            let answer = accounts.login(&name, &password);
            if answer.is_err() {
                // Slows down guessing.
                thread::sleep(Duration::from_secs(1));
            }
            w.send(&answer);
        }
        ToServer::Logout { session } => {
            accounts.logout(&session);
            w.send(&());
        }
        ToServer::List => {
            let mut list: Vec<Listing> = server.matches().matches.values().map(|m| m.listing.clone()).collect();
            list.sort_by_key(|l| l.id);
            w.send(&list);
        }
        ToServer::Ranking => {
            w.send(&server.stats.ranking());
        }
        ToServer::History { name } => {
            let (standing, records) = server.stats.history(&name);
            w.send(&PlayerHistory { standing, records });
        }
        ToServer::Host { app, session, info } => match accounts.session(&session) {
            Some(name) => return serve_host(server, pump, app, name, info),
            None => {
                w.send(&Err::<Hosted, _>(Refusal::NotLoggedIn));
            }
        },
        ToServer::Join { id, session } => match accounts.session(&session) {
            Some(name) => return serve_guest(server, pump, id, name),
            None => {
                w.send(&ServerMsg::Rejected(Rejection::NotLoggedIn));
            }
        },
    }
    w.close();
}

fn short(name: String) -> String {
    let name = name.trim();
    match name.char_indices().nth(MAX_NAME) {
        Some((i, _)) => name[..i].to_string(),
        None => name.to_string(),
    }
}

fn serve_host(server: &Server, pump: crate::pump::Pump, app: String, host: String, mut info: MatchInfo) {
    let id = {
        let mut s = server.matches();
        if s.matches.len() >= MAX_MATCHES {
            pump.writer.send(&Err::<Hosted, _>(Refusal::Full));
            pump.writer.close();
            return;
        }
        s.next_id += 1;
        let id = s.next_id;
        info.name = short(info.name);
        let listing = Listing { id, app: short(app), host, info, lobby: None };
        let listed =
            Listed { listing, host: pump.writer.clone(), guests: HashMap::new(), seats: [None, None], recorded: false };
        s.matches.insert(id, listed);
        id
    };
    if pump.writer.send(&Ok::<_, Refusal>(Hosted { id })) {
        while let Ok(line) = pump.lines.recv() {
            let Ok(out) = parse::<HostOut>(&line) else { break };
            let mut s = server.matches();
            let Some(m) = s.matches.get_mut(&id) else { break };
            match out {
                HostOut::Send { conn, msg } => {
                    if let Some((guest, name)) = m.guests.get(&conn) {
                        if let ServerMsg::Welcome { seat: Some(seat), .. } = &msg {
                            m.seats[seat.index()] = Some(name.clone());
                        }
                        guest.send(&msg);
                    }
                }
                HostOut::Close { conn } => {
                    if let Some((guest, _)) = m.guests.remove(&conn) {
                        guest.close();
                    }
                }
                HostOut::Lobby(lobby) => m.listing.lobby = Some(lobby),
                HostOut::Over(outcome) => m.record(&server.stats, outcome),
            }
        }
    }
    let gone = server.matches().matches.remove(&id);
    if let Some(mut m) = gone {
        // A host who leaves a match unfinished loses it.
        let host_seat = match m.seats {
            [None, Some(_)] => Some(PlayerId::ONE),
            [Some(_), None] => Some(PlayerId::TWO),
            _ => None,
        };
        if let (true, Some(seat)) = (m.started(), host_seat) {
            m.record(&server.stats, Outcome::Win { winner: seat.opponent(), method: WinMethod::Forfeit });
        }
        for (guest, _) in m.guests.values() {
            guest.close();
        }
    }
    pump.writer.close();
}

fn serve_guest(server: &Server, pump: crate::pump::Pump, id: MatchId, name: String) {
    let joined = {
        let mut s = server.matches();
        s.next_conn += 1;
        let conn = s.next_conn;
        s.matches.get_mut(&id).map(|m| {
            m.guests.insert(conn, (pump.writer.clone(), name.clone()));
            (conn, m.host.clone())
        })
    };
    let Some((conn, host)) = joined else {
        pump.writer.send(&ServerMsg::Rejected(Rejection::NoMatch));
        pump.writer.close();
        return;
    };
    if host.send(&HostIn::Joined { conn }) {
        while let Ok(line) = pump.lines.recv() {
            let Ok(mut msg) = parse::<ClientMsg>(&line) else { break };
            if let ClientMsg::Hello { name: said, .. } = &mut msg {
                // Guests are who their account says.
                said.clone_from(&name);
            }
            if !host.send(&HostIn::Msg { conn, msg }) {
                break;
            }
        }
    }
    if let Some(m) = server.matches().matches.get_mut(&id) {
        m.guests.remove(&conn);
    }
    host.send(&HostIn::Gone { conn });
    pump.writer.close();
}
