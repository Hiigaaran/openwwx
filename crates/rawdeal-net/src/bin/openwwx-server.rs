//! The OpenWWX common server: lists open matches and passes messages between
//! each match's host and its players and spectators. It needs no card data.
//!
//! Usage: `openwwx-server [PORT]` (7778 by default).

use std::net::TcpListener;
use std::process::ExitCode;

use rawdeal_net::{DEFAULT_SERVER_PORT, relay};

fn main() -> ExitCode {
    let port = match std::env::args().nth(1) {
        None => DEFAULT_SERVER_PORT,
        Some(p) => match p.parse() {
            Ok(p) => p,
            Err(_) => {
                eprintln!("usage: openwwx-server [PORT]");
                return ExitCode::FAILURE;
            }
        },
    };
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on port {port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("OpenWWX server {} listening on port {port}", rawdeal_net::APP_VERSION);
    relay::serve(listener);
    ExitCode::SUCCESS
}
