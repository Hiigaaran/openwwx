//! One JSON message per line.

use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Longest line accepted, so a broken peer cannot fill memory.
const MAX_LINE: usize = 16 << 20;

pub fn write_msg(out: &mut impl Write, msg: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg).map_err(io::Error::other)?;
    line.push(b'\n');
    out.write_all(&line)?;
    out.flush()
}

/// The next message, or `None` at the end of the stream.
pub fn read_msg<T: DeserializeOwned>(input: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = Vec::new();
    let n = BufRead::read_until(&mut io::Read::take(input, MAX_LINE as u64 + 1), b'\n', &mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too long"));
    }
    serde_json::from_slice(&line).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
