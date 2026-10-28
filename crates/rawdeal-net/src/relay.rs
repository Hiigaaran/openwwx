//! The common server: a list of open matches, and a relay between each
//! match's host and its guests. It runs no match and needs no card data:
//! hosts still run their matches ([`crate::HostCore`]), and the server only
//! passes messages along, so nobody has to open a port on their router.
//!
//! A connection's first line says what it is for ([`ToServer`]):
//! - `Host`: the server answers [`Hosted`], then sends [`HostIn`] lines and
//!   takes [`HostOut`] lines.
//! - `Join`: from then on the connection is a guest's link to that match,
//!   with the same messages as a direct connection to the host.
//! - `List`: the server answers with the open matches and closes.

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::host::ConnId;
use crate::wire::{read_msg, write_msg};
use crate::{ClientMsg, Lobby, MatchInfo, Rejection, ServerMsg};

/// Port the common server listens on unless told otherwise.
pub const DEFAULT_SERVER_PORT: u16 = 7778;

/// Most matches listed at once, so a broken client cannot fill memory.
const MAX_MATCHES: usize = 500;

/// Longest name kept for a match or a host.
const MAX_NAME: usize = 40;

/// Identifies a match on the server.
pub type MatchId = u64;

/// A connection's first line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    /// List this match. `host` is the hosting player's name, `app` their
    /// OpenWWX version.
    Host { app: String, host: String, info: MatchInfo },
    /// Connect to match `id` as a guest.
    Join { id: MatchId },
    /// Ask for the open matches.
    List,
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
}

/// The open matches on the server at `addr`.
pub fn list(addr: impl ToSocketAddrs) -> io::Result<Vec<Listing>> {
    let mut stream = connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write_msg(&mut stream, &ToServer::List)?;
    read_msg(&mut BufReader::new(stream))?.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
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
    host: Shared<TcpStream>,
    guests: HashMap<ConnId, Shared<TcpStream>>,
}

#[derive(Default)]
struct Server {
    matches: HashMap<MatchId, Listed>,
    next_id: MatchId,
    next_conn: ConnId,
}

/// Runs the common server on `listener` until the process ends.
pub fn serve(listener: TcpListener) {
    let server: Shared<Server> = Arc::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        stream.set_nodelay(true).ok();
        let server = server.clone();
        thread::spawn(move || {
            let Ok(reader) = stream.try_clone() else { return };
            let mut reader = BufReader::new(reader);
            match read_msg::<ToServer>(&mut reader) {
                Ok(Some(ToServer::Host { app, host, info })) => serve_host(&server, stream, reader, app, host, info),
                Ok(Some(ToServer::Join { id })) => serve_guest(&server, stream, reader, id),
                Ok(Some(ToServer::List)) => {
                    let list: Vec<Listing> = {
                        let s = server.lock().expect("server lock");
                        let mut list: Vec<Listing> = s.matches.values().map(|m| m.listing.clone()).collect();
                        list.sort_by_key(|l| l.id);
                        list
                    };
                    let mut stream = stream;
                    write_msg(&mut stream, &list).ok();
                }
                _ => {}
            }
        });
    }
}

fn short(name: String) -> String {
    let name = name.trim();
    match name.char_indices().nth(MAX_NAME) {
        Some((i, _)) => name[..i].to_string(),
        None => name.to_string(),
    }
}

fn send(stream: &Shared<TcpStream>, msg: &impl Serialize) -> bool {
    stream.lock().map(|mut s| write_msg(&mut *s, msg).is_ok()).unwrap_or(false)
}

fn serve_host(
    server: &Shared<Server>,
    stream: TcpStream,
    mut reader: BufReader<TcpStream>,
    app: String,
    host: String,
    mut info: MatchInfo,
) {
    let stream = Arc::new(Mutex::new(stream));
    let id = {
        let mut s = server.lock().expect("server lock");
        if s.matches.len() >= MAX_MATCHES {
            return;
        }
        s.next_id += 1;
        let id = s.next_id;
        info.name = short(info.name);
        let listing = Listing { id, app: short(app), host: short(host), info, lobby: None };
        s.matches.insert(id, Listed { listing, host: stream.clone(), guests: HashMap::new() });
        id
    };
    if send(&stream, &Hosted { id }) {
        while let Ok(Some(out)) = read_msg::<HostOut>(&mut reader) {
            // Writes to a guest happen outside the lock, so a slow guest
            // holds up only its own match.
            let (guest, msg) = {
                let mut s = server.lock().expect("server lock");
                let Some(m) = s.matches.get_mut(&id) else { break };
                match out {
                    HostOut::Send { conn, msg } => (m.guests.get(&conn).cloned(), Some(msg)),
                    HostOut::Close { conn } => (m.guests.remove(&conn), None),
                    HostOut::Lobby(lobby) => {
                        m.listing.lobby = Some(lobby);
                        (None, None)
                    }
                }
            };
            match (guest, msg) {
                (Some(guest), Some(msg)) => {
                    send(&guest, &msg);
                }
                (Some(guest), None) => {
                    guest.lock().map(|g| g.shutdown(Shutdown::Both)).ok();
                }
                _ => {}
            }
        }
    }
    let gone = server.lock().expect("server lock").matches.remove(&id);
    for guest in gone.iter().flat_map(|m| m.guests.values()) {
        guest.lock().map(|g| g.shutdown(Shutdown::Both)).ok();
    }
    stream.lock().map(|s| s.shutdown(Shutdown::Both)).ok();
}

fn serve_guest(server: &Shared<Server>, stream: TcpStream, mut reader: BufReader<TcpStream>, id: MatchId) {
    let stream = Arc::new(Mutex::new(stream));
    let joined = {
        let mut s = server.lock().expect("server lock");
        s.next_conn += 1;
        let conn = s.next_conn;
        s.matches.get_mut(&id).map(|m| {
            m.guests.insert(conn, stream.clone());
            (conn, m.host.clone())
        })
    };
    let Some((conn, host)) = joined else {
        send(&stream, &ServerMsg::Rejected(Rejection::NoMatch));
        return;
    };
    if send(&host, &HostIn::Joined { conn }) {
        while let Ok(Some(msg)) = read_msg::<ClientMsg>(&mut reader) {
            if !send(&host, &HostIn::Msg { conn, msg }) {
                break;
            }
        }
    }
    if let Some(m) = server.lock().expect("server lock").matches.get_mut(&id) {
        m.guests.remove(&conn);
    }
    send(&host, &HostIn::Gone { conn });
}
