//! One thread that reads and writes a connection, for streams that cannot
//! be split into a reading and a writing half, such as an encrypted one.

use std::io::{self, ErrorKind};
use std::net::Shutdown;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::secure::Socket;

/// Longest line accepted, as in [`crate::wire`].
const MAX_LINE: usize = 16 << 20;

/// How long the thread waits for something to read before looking for
/// lines to write.
const POLL: Duration = Duration::from_millis(20);

/// Sends lines on a pumped connection. Clones share the connection.
#[derive(Clone)]
pub struct Writer(Sender<Vec<u8>>);

impl Writer {
    /// Queues `msg`; `false` once the connection is gone.
    pub fn send(&self, msg: &impl Serialize) -> bool {
        let Ok(mut line) = serde_json::to_vec(msg) else { return false };
        line.push(b'\n');
        self.0.send(line).is_ok()
    }

    /// Closes the connection once what was queued before is written.
    pub fn close(&self) {
        self.0.send(Vec::new()).ok();
    }
}

/// A connection run by its own thread.
pub struct Pump {
    pub writer: Writer,
    /// Lines read, without the newline. Disconnected once the connection
    /// ends.
    pub lines: Receiver<Vec<u8>>,
}

impl Pump {
    /// The next message within `timeout`.
    pub fn recv<T: DeserializeOwned>(&self, timeout: Duration) -> io::Result<T> {
        let line = self.lines.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => io::Error::from(ErrorKind::TimedOut),
            RecvTimeoutError::Disconnected => io::Error::from(ErrorKind::UnexpectedEof),
        })?;
        parse(&line)
    }
}

pub fn parse<T: DeserializeOwned>(line: &[u8]) -> io::Result<T> {
    serde_json::from_slice(line).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
}

/// Starts the thread for `stream`. It ends when the other side closes,
/// on [`Writer::close`], or once every [`Writer`] is dropped.
pub fn pump<S: Socket>(mut stream: S) -> io::Result<Pump> {
    stream.socket().set_read_timeout(Some(POLL))?;
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    let (in_tx, in_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16 << 10];
        'run: loop {
            loop {
                match out_rx.try_recv() {
                    Ok(line) if line.is_empty() => break 'run,
                    Ok(line) => {
                        if stream.write_all(&line).and_then(|_| stream.flush()).is_err() {
                            break 'run;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'run,
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(end) = buf.iter().position(|&b| b == b'\n') {
                        let rest = buf.split_off(end + 1);
                        buf.pop();
                        if in_tx.send(std::mem::replace(&mut buf, rest)).is_err() {
                            break 'run;
                        }
                    }
                    if buf.len() > MAX_LINE {
                        break;
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
                Err(_) => break,
            }
        }
        stream.socket().shutdown(Shutdown::Both).ok();
    });
    Ok(Pump { writer: Writer(out_tx), lines: in_rx })
}
