//! The host (a [`HostCore`] behind a TCP listener, or listed on the common
//! server) and the connection a guest, or the hosting player's own app, uses.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::host::{ConnId, HostCore};
use crate::pump::{Writer, parse, pump};
use crate::relay::{self, HostIn, HostOut, Hosted, MatchId, Refusal, ToServer};
use crate::secure::{self, Pin};
use crate::wire::{read_msg, write_msg};
use crate::{APP_VERSION, ClientMsg, ServerMsg};

/// The host's link to the common server.
type Link = Writer;

fn send_link(link: &Link, msg: &HostOut) -> bool {
    link.send(msg)
}

/// The error for a [`Refusal`] from the server.
pub fn refused(why: Refusal) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, RefusedBy(why))
}

/// The [`Refusal`] behind an error from [`refused`].
pub fn refusal(e: &io::Error) -> Option<Refusal> {
    e.get_ref().and_then(|e| e.downcast_ref::<RefusedBy>()).map(|r| r.0)
}

#[derive(Debug)]
struct RefusedBy(Refusal);

impl std::fmt::Display for RefusedBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the server said no: {:?}", self.0)
    }
}

impl std::error::Error for RefusedBy {}

/// An encrypted, pumped connection to the common server at `addr`.
fn to_server(addr: impl ToSocketAddrs, pin: &Pin) -> io::Result<crate::pump::Pump> {
    pump(secure::client(relay::connect(addr)?, pin)?)
}

enum Input {
    Joined(ConnId, Peer),
    Msg(ConnId, ClientMsg),
    Gone(ConnId),
    Stop,
}

/// Where the host's messages for one connection go.
enum Peer {
    Tcp(TcpStream),
    Local(Sender<ServerMsg>),
    /// A guest on the common server.
    Relay(ConnId, Link),
}

impl Peer {
    fn send(&mut self, msg: &ServerMsg) -> bool {
        match self {
            Peer::Tcp(stream) => write_msg(stream, msg).is_ok(),
            Peer::Local(tx) => tx.send(msg.clone()).is_ok(),
            Peer::Relay(conn, link) => send_link(link, &HostOut::Send { conn: *conn, msg: msg.clone() }),
        }
    }

    fn close(&self) {
        match self {
            Peer::Tcp(stream) => {
                stream.shutdown(Shutdown::Both).ok();
            }
            Peer::Local(_) => {}
            Peer::Relay(conn, link) => {
                send_link(link, &HostOut::Close { conn: *conn });
            }
        }
    }
}

/// A match being hosted. Dropping it stops the match.
pub struct HostHandle {
    tx: Sender<Input>,
    /// Where it listens, hosted directly.
    port: Option<u16>,
    /// Its link and number on the common server.
    server: Option<(Link, MatchId)>,
    next_local: ConnId,
}

impl HostHandle {
    /// Hosts `core` on `port` (0 picks a free one) on every network interface.
    pub fn start(core: HostCore, port: u16) -> io::Result<HostHandle> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        let accept_tx = tx.clone();
        thread::spawn(move || accept(listener, accept_tx));
        thread::spawn(move || run(core, rx, None));
        Ok(HostHandle { tx, port: Some(port), server: None, next_local: 1 << 62 })
    }

    /// Lists `core` on the common server at `addr`; guests reach it through
    /// the server. `session` is the hosting player's, whose account name the
    /// list shows.
    pub fn start_on_server(
        core: HostCore,
        addr: impl ToSocketAddrs,
        session: &str,
        pin: &Pin,
    ) -> io::Result<HostHandle> {
        let server = to_server(addr, pin)?;
        let hello = ToServer::Host { app: APP_VERSION.into(), session: session.into(), info: core.info().clone() };
        server.writer.send(&hello);
        let hosted = server.recv::<Result<Hosted, Refusal>>(Duration::from_secs(15))?.map_err(refused)?;
        let link: Link = server.writer.clone();
        let lines = server.lines;
        let (tx, rx) = mpsc::channel();
        let relay_tx = tx.clone();
        let relay_link = link.clone();
        thread::spawn(move || {
            let mut guests = HashSet::new();
            while let Some(msg) = lines.recv().ok().and_then(|l| parse::<HostIn>(&l).ok()) {
                let input = match msg {
                    HostIn::Joined { conn } => {
                        guests.insert(conn);
                        Input::Joined(conn, Peer::Relay(conn, relay_link.clone()))
                    }
                    HostIn::Msg { conn, msg } => Input::Msg(conn, msg),
                    HostIn::Gone { conn } => {
                        guests.remove(&conn);
                        Input::Gone(conn)
                    }
                };
                if relay_tx.send(input).is_err() {
                    return;
                }
            }
            // The server is gone, and with it every guest.
            for conn in guests {
                relay_tx.send(Input::Gone(conn)).ok();
            }
        });
        let run_link = link.clone();
        thread::spawn(move || run(core, rx, Some(run_link)));
        Ok(HostHandle { tx, port: None, server: Some((link, hosted.id)), next_local: 1 << 62 })
    }

    /// The port it listens on, hosted directly.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Its number on the common server, listed there.
    pub fn match_id(&self) -> Option<MatchId> {
        self.server.as_ref().map(|(_, id)| *id)
    }

    /// A connection from this app to its own match, for the hosting player.
    pub fn connect_local(&mut self) -> Connection {
        let id = self.next_local;
        self.next_local += 1;
        let (tx, rx) = mpsc::channel();
        self.tx.send(Input::Joined(id, Peer::Local(tx))).ok();
        Connection { out: Out::Local { id, tx: self.tx.clone() }, rx }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        self.tx.send(Input::Stop).ok();
        if let Some(port) = self.port {
            // Wake the accept loop so it notices.
            TcpStream::connect(("127.0.0.1", port)).ok();
        }
        if let Some((link, _)) = &self.server {
            link.close();
        }
    }
}

fn accept(listener: TcpListener, tx: Sender<Input>) {
    for (id, stream) in listener.incoming().enumerate() {
        let Ok(stream) = stream else { continue };
        let (Ok(reader), Ok(writer)) = (stream.try_clone(), stream.try_clone()) else { continue };
        let id = id as ConnId;
        if tx.send(Input::Joined(id, Peer::Tcp(writer))).is_err() {
            return;
        }
        let tx = tx.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(msg)) = read_msg::<ClientMsg>(&mut reader) {
                if tx.send(Input::Msg(id, msg)).is_err() {
                    return;
                }
            }
            tx.send(Input::Gone(id)).ok();
        });
    }
}

fn run(mut core: HostCore, rx: Receiver<Input>, link: Option<Link>) {
    let mut peers: HashMap<ConnId, Peer> = HashMap::new();
    let mut listed = None;
    loop {
        let out = match rx.recv_timeout(Duration::from_secs(1)) {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => core.tick(),
            Ok(input) => match input {
                Input::Stop => break,
                Input::Joined(id, peer) => {
                    peers.insert(id, peer);
                    vec![]
                }
                Input::Msg(id, msg) => core.handle(id, msg),
                Input::Gone(id) => {
                    peers.remove(&id);
                    core.disconnected(id)
                }
            },
        };
        let mut gone = Vec::new();
        for (id, msg) in out {
            let Some(peer) = peers.get_mut(&id) else { continue };
            if !peer.send(&msg) || matches!(msg, ServerMsg::Rejected(_)) {
                peer.close();
                gone.push(id);
            }
        }
        for id in gone {
            peers.remove(&id);
            core.disconnected(id);
        }
        if let Some(link) = &link {
            let lobby = core.lobby();
            if listed.as_ref() != Some(&lobby) {
                send_link(link, &HostOut::Lobby(lobby.clone()));
                listed = Some(lobby);
            }
        }
    }
    for peer in peers.values() {
        peer.close();
    }
}

enum Out {
    Tcp(TcpStream),
    Pump(Writer),
    Local { id: ConnId, tx: Sender<Input> },
}

/// How to reach a match again after losing the connection.
#[derive(Debug, Clone)]
pub enum Dial {
    /// A host's address, "192.168.0.10:7777".
    Direct(String),
    /// A match on the common server at an address, with the player's
    /// session there.
    Server { addr: String, id: MatchId, session: String, pin: Pin },
}

impl Dial {
    pub fn connect(&self) -> io::Result<Connection> {
        match self {
            Dial::Direct(addr) => Connection::connect(addr.as_str()),
            Dial::Server { addr, id, session, pin } => Connection::join_on_server(addr.as_str(), *id, session, pin),
        }
    }
}

/// A guest's link to a hosted match.
pub struct Connection {
    out: Out,
    rx: Receiver<ServerMsg>,
}

impl Connection {
    /// Connects to a host at `addr` ("192.168.0.10:7777").
    pub fn connect(addr: impl ToSocketAddrs) -> io::Result<Connection> {
        Connection::over(relay::connect(addr)?)
    }

    /// Connects to match `id` on the common server at `addr`, as the
    /// account `session` belongs to.
    pub fn join_on_server(addr: impl ToSocketAddrs, id: MatchId, session: &str, pin: &Pin) -> io::Result<Connection> {
        let server = to_server(addr, pin)?;
        server.writer.send(&ToServer::Join { id, session: session.into() });
        let (tx, rx) = mpsc::channel();
        let lines = server.lines;
        thread::spawn(move || {
            while let Some(msg) = lines.recv().ok().and_then(|l| parse::<ServerMsg>(&l).ok()) {
                if tx.send(msg).is_err() {
                    return;
                }
            }
        });
        Ok(Connection { out: Out::Pump(server.writer), rx })
    }

    fn over(stream: TcpStream) -> io::Result<Connection> {
        let reader = stream.try_clone()?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(msg)) = read_msg::<ServerMsg>(&mut reader) {
                if tx.send(msg).is_err() {
                    return;
                }
            }
        });
        Ok(Connection { out: Out::Tcp(stream), rx })
    }

    pub fn send(&mut self, msg: ClientMsg) -> io::Result<()> {
        match &mut self.out {
            Out::Tcp(stream) => write_msg(stream, &msg),
            Out::Pump(w) => {
                if w.send(&msg) {
                    Ok(())
                } else {
                    Err(io::Error::from(io::ErrorKind::BrokenPipe))
                }
            }
            Out::Local { id, tx } => {
                tx.send(Input::Msg(*id, msg)).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
            }
        }
    }

    /// The next message if one has arrived; `Err` once the host is gone.
    pub fn try_recv(&self) -> Result<Option<ServerMsg>, Disconnected> {
        match self.rx.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(Disconnected),
        }
    }

    /// Waits up to `timeout` for the next message.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<ServerMsg>, Disconnected> {
        match self.rx.recv_timeout(timeout) {
            Ok(msg) => Ok(Some(msg)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Disconnected),
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        match &self.out {
            Out::Tcp(stream) => {
                stream.shutdown(Shutdown::Both).ok();
            }
            Out::Pump(w) => w.close(),
            Out::Local { id, tx } => {
                tx.send(Input::Gone(*id)).ok();
            }
        }
    }
}

/// The host closed the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected;
