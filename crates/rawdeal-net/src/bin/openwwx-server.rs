//! The OpenWWX common server: player accounts, the list of open matches,
//! and a relay between each match's host and its players and spectators.
//! It needs no card data.
//!
//! Usage:
//! - `openwwx-server [PORT] [--data DIR]`: runs the server (port 7778 by
//!   default). DIR (`openwwx-server-data` by default) keeps the accounts,
//!   the match history and the server's certificate; keep it to keep them.
//! - `openwwx-server [--data DIR] --reset-password NAME PASSWORD`: sets a
//!   player's password, for when they forget it.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use rawdeal_net::{Accounts, DEFAULT_SERVER_PORT, Stats, relay, secure};

const USAGE: &str =
    "usage: openwwx-server [PORT] [--data DIR]\n       openwwx-server [--data DIR] --reset-password NAME PASSWORD";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut port = DEFAULT_SERVER_PORT;
    let mut data = PathBuf::from("openwwx-server-data");
    let mut reset = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => match args.next() {
                Some(dir) => data = dir.into(),
                None => return usage(),
            },
            "--reset-password" => match (args.next(), args.next()) {
                (Some(name), Some(password)) => reset = Some((name, password)),
                _ => return usage(),
            },
            p => match p.parse() {
                Ok(p) => port = p,
                Err(_) => return usage(),
            },
        }
    }
    if let Err(e) = std::fs::create_dir_all(&data) {
        eprintln!("cannot make {}: {e}", data.display());
        return ExitCode::FAILURE;
    }
    let accounts = match Accounts::open(data.join("accounts.json")) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("cannot read the accounts in {}: {e}", data.display());
            return ExitCode::FAILURE;
        }
    };
    if let Some((name, password)) = reset {
        return match accounts.reset_password(&name, &password) {
            Ok(()) => {
                println!("New password set for {name}; their sessions ended.");
                ExitCode::SUCCESS
            }
            Err(why) => {
                eprintln!("cannot set the password: {why:?}");
                ExitCode::FAILURE
            }
        };
    }
    let stats = match Stats::open(data.join("results.json")) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("cannot read the match history in {}: {e}", data.display());
            return ExitCode::FAILURE;
        }
    };
    let (tls, fingerprint) = match secure::server_config(&data) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot set up the certificate in {}: {e}", data.display());
            return ExitCode::FAILURE;
        }
    };
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on port {port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("OpenWWX server {} listening on port {port}", rawdeal_net::APP_VERSION);
    println!("Data in {} ({} accounts)", data.display(), accounts.len());
    println!("Certificate fingerprint: {fingerprint}");
    relay::serve(listener, tls, accounts, stats);
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::FAILURE
}
