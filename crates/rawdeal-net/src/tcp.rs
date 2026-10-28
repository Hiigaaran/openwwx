//! The direct host (a TCP listener in front of a [`HostCore`]) and the
//! connection a guest, or the hosting player's own app, uses.

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::host::{ConnId, HostCore};
use crate::wire::{read_msg, write_msg};
use crate::{ClientMsg, ServerMsg};

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
}

impl Peer {
    fn send(&mut self, msg: &ServerMsg) -> bool {
        match self {
            Peer::Tcp(stream) => write_msg(stream, msg).is_ok(),
            Peer::Local(tx) => tx.send(msg.clone()).is_ok(),
        }
    }

    fn close(&self) {
        if let Peer::Tcp(stream) = self {
            stream.shutdown(Shutdown::Both).ok();
        }
    }
}

/// A match being hosted. Dropping it stops the match.
pub struct HostHandle {
    tx: Sender<Input>,
    port: u16,
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
        thread::spawn(move || run(core, rx));
        Ok(HostHandle { tx, port, next_local: 1 << 62 })
    }

    pub fn port(&self) -> u16 {
        self.port
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
        // Wake the accept loop so it notices.
        TcpStream::connect(("127.0.0.1", self.port)).ok();
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

fn run(mut core: HostCore, rx: Receiver<Input>) {
    let mut peers: HashMap<ConnId, Peer> = HashMap::new();
    while let Ok(input) = rx.recv() {
        let out = match input {
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
    }
    for peer in peers.values() {
        peer.close();
    }
}

enum Out {
    Tcp(TcpStream),
    Local { id: ConnId, tx: Sender<Input> },
}

/// A guest's link to a hosted match.
pub struct Connection {
    out: Out,
    rx: Receiver<ServerMsg>,
}

impl Connection {
    /// Connects to a host at `addr` ("192.168.0.10:7777").
    pub fn connect(addr: impl ToSocketAddrs) -> io::Result<Connection> {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("no address"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
        stream.set_nodelay(true).ok();
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
            Out::Local { id, tx } => {
                tx.send(Input::Gone(*id)).ok();
            }
        }
    }
}

/// The host closed the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected;
