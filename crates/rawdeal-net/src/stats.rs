//! Match history and ranking on the common server. The server learns each
//! match's result from its host ([`crate::relay::HostOut::Over`]), or gives
//! the match to the other player when the host leaves it unfinished. Ratings
//! are Elo, starting at [`START`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rawdeal_core::{Mode, WinMethod};
use serde::{Deserialize, Serialize};

/// A new player's rating.
pub const START: i32 = 1000;

/// How far one match moves a rating at most.
const K: f64 = 32.0;

/// Matches listed in a player's history.
const HISTORY: usize = 50;

/// Players listed in the ranking.
const RANKING: usize = 100;

/// One finished match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Seconds since 1970.
    pub when: u64,
    /// Account names, by seat.
    pub players: [String; 2],
    /// The winner's seat (0 or 1); `None` for a draw.
    pub winner: Option<u8>,
    pub method: Option<WinMethod>,
    pub mode: Mode,
    /// Rating points `players[0]` gained (`players[1]` lost as many).
    pub change: i32,
}

/// A player's place in the ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Standing {
    pub name: String,
    pub rating: i32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    records: Vec<Record>,
    /// By lowercase name.
    players: BTreeMap<String, Standing>,
}

pub struct Stats {
    path: Option<PathBuf>,
    store: Mutex<Store>,
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Rating points the first player gains: `score` is 1 for a win, 0.5 for a
/// draw and 0 for a loss.
fn elo(a: i32, b: i32, score: f64) -> i32 {
    let expected = 1.0 / (1.0 + 10f64.powf(f64::from(b - a) / 400.0));
    (K * (score - expected)).round() as i32
}

impl Stats {
    /// The results kept in the file at `path`.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Stats> {
        let path = path.into();
        let store = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e),
        };
        Ok(Stats { path: Some(path), store: Mutex::new(store) })
    }

    /// Results that are not saved anywhere, for tests.
    pub fn in_memory() -> Stats {
        Stats { path: None, store: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a finished match between two accounts.
    pub fn record(&self, players: [String; 2], winner: Option<u8>, method: Option<WinMethod>, mode: Mode) -> Record {
        let mut store = self.lock();
        let standing = |s: &mut Store, name: &str| -> Standing {
            s.players
                .entry(key(name))
                .or_insert_with(|| Standing { name: name.into(), rating: START, wins: 0, losses: 0, draws: 0 })
                .clone()
        };
        let (a, b) = (standing(&mut store, &players[0]), standing(&mut store, &players[1]));
        let score = match winner {
            Some(0) => 1.0,
            Some(_) => 0.0,
            None => 0.5,
        };
        let change = elo(a.rating, b.rating, score);
        for (i, name) in players.iter().enumerate() {
            let s = store.players.get_mut(&key(name)).expect("added above");
            s.rating += if i == 0 { change } else { -change };
            match winner {
                None => s.draws += 1,
                Some(w) if usize::from(w) == i => s.wins += 1,
                Some(_) => s.losses += 1,
            }
        }
        let when = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let record = Record { when, players, winner, method, mode, change };
        store.records.push(record.clone());
        self.save(&store);
        record
    }

    fn save(&self, store: &Store) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("tmp");
        let saved = serde_json::to_vec(store)
            .map_err(io::Error::other)
            .and_then(|bytes| fs::write(&tmp, bytes))
            .and_then(|_| fs::rename(&tmp, path));
        if let Err(e) = saved {
            eprintln!("cannot save the results to {}: {e}", path.display());
        }
    }

    /// The best players, best first.
    pub fn ranking(&self) -> Vec<Standing> {
        let mut list: Vec<Standing> = self.lock().players.values().cloned().collect();
        list.sort_by(|a, b| {
            b.rating.cmp(&a.rating).then_with(|| b.wins.cmp(&a.wins)).then_with(|| a.name.cmp(&b.name))
        });
        list.truncate(RANKING);
        list
    }

    /// A player's latest matches, newest first, and their standing.
    pub fn history(&self, name: &str) -> (Option<Standing>, Vec<Record>) {
        let store = self.lock();
        let k = key(name);
        let records = store
            .records
            .iter()
            .rev()
            .filter(|r| r.players.iter().any(|p| key(p) == k))
            .take(HISTORY)
            .cloned()
            .collect();
        (store.players.get(&k).cloned(), records)
    }
}
