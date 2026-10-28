//! Player accounts on the common server: a name only its owner can use,
//! with a password kept as an Argon2 hash. Logging in gives a session
//! token the app keeps, so the password is typed once.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};

use crate::relay::Refusal;

/// Sessions kept per account, one per device or so; older ones end.
const MAX_SESSIONS: usize = 10;

/// A logged-in player: their name as registered, and the token that proves
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub token: String,
}

#[derive(Serialize, Deserialize)]
struct User {
    name: String,
    hash: String,
    #[serde(default)]
    sessions: Vec<String>,
    /// Seconds since 1970.
    created: u64,
    /// Friends, by lowercase name.
    #[serde(default)]
    friends: Vec<String>,
    /// Who asked to be friends, by lowercase name.
    #[serde(default)]
    requests: Vec<String>,
    /// Saved decks, and deleted ones as [`SavedDeck::deleted`] marks.
    #[serde(default)]
    decks: Vec<SavedDeck>,
}

/// A deck kept in an account, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDeck {
    pub name: String,
    /// The deck's share code (`OWX1-…`); empty once deleted, so other
    /// computers delete their copy too.
    pub code: String,
    /// When it was saved or deleted, in seconds since 1970, by the clock of
    /// the computer that did it.
    pub saved: u64,
}

impl SavedDeck {
    pub fn deleted(&self) -> bool {
        self.code.is_empty()
    }
}

/// Decks an account can keep.
pub const MAX_DECKS: usize = 100;
/// Longest deck name and code the server keeps.
pub const MAX_DECK_NAME: usize = 60;
pub const MAX_DECK_CODE: usize = 8192;

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// By lowercase name.
    users: BTreeMap<String, User>,
}

struct Inner {
    store: Store,
    /// Session token to lowercase name.
    sessions: HashMap<String, String>,
}

pub struct Accounts {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

/// Whether `name` may be registered: 3 to 20 letters, digits, spaces, `_`,
/// `-` or `.`, not starting or ending with a space.
pub fn valid_name(name: &str) -> bool {
    let len = name.chars().count();
    (3..=20).contains(&len)
        && name.trim() == name
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
}

/// Shortest password accepted.
pub const MIN_PASSWORD: usize = 6;

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn new_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system random numbers");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(password: &str) -> String {
    Argon2::default().hash_password(password.as_bytes()).expect("argon2 hash").to_string()
}

fn verify(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

impl Accounts {
    /// The accounts kept in the file at `path` (made when the first one
    /// registers).
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Accounts> {
        let path = path.into();
        let store = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e),
        };
        Ok(Accounts::with(Some(path), store))
    }

    /// Accounts that are not saved anywhere, for tests.
    pub fn in_memory() -> Accounts {
        Accounts::with(None, Store::default())
    }

    fn with(path: Option<PathBuf>, store: Store) -> Accounts {
        let sessions =
            store.users.iter().flat_map(|(k, u)| u.sessions.iter().map(move |t| (t.clone(), k.clone()))).collect();
        Accounts { path, inner: Mutex::new(Inner { store, sessions }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, store: &Store) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("tmp");
        let saved = serde_json::to_vec_pretty(store)
            .map_err(io::Error::other)
            .and_then(|bytes| fs::write(&tmp, bytes))
            .and_then(|_| fs::rename(&tmp, path));
        if let Err(e) = saved {
            eprintln!("cannot save the accounts to {}: {e}", path.display());
        }
    }

    pub fn len(&self) -> usize {
        self.lock().store.users.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn register(&self, name: &str, password: &str) -> Result<Session, Refusal> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(Refusal::BadName);
        }
        if password.chars().count() < MIN_PASSWORD {
            return Err(Refusal::BadPassword);
        }
        if self.lock().store.users.contains_key(&key(name)) {
            return Err(Refusal::NameTaken);
        }
        // Hashing takes a while; do it without holding the lock.
        let hash = hash(password);
        let mut inner = self.lock();
        if inner.store.users.contains_key(&key(name)) {
            return Err(Refusal::NameTaken);
        }
        let token = new_token();
        let user = User {
            name: name.into(),
            hash,
            sessions: vec![token.clone()],
            created: now(),
            friends: vec![],
            requests: vec![],
            decks: vec![],
        };
        inner.store.users.insert(key(name), user);
        inner.sessions.insert(token.clone(), key(name));
        self.save(&inner.store);
        Ok(Session { name: name.into(), token })
    }

    pub fn login(&self, name: &str, password: &str) -> Result<Session, Refusal> {
        let k = key(name);
        let Some(hash) = self.lock().store.users.get(&k).map(|u| u.hash.clone()) else {
            return Err(Refusal::WrongPassword);
        };
        if !verify(password, &hash) {
            return Err(Refusal::WrongPassword);
        }
        let mut inner = self.lock();
        let token = new_token();
        let Some(user) = inner.store.users.get_mut(&k) else { return Err(Refusal::WrongPassword) };
        user.sessions.push(token.clone());
        let ended: Vec<String> = user.sessions.drain(..user.sessions.len().saturating_sub(MAX_SESSIONS)).collect();
        let name = user.name.clone();
        for t in ended {
            inner.sessions.remove(&t);
        }
        inner.sessions.insert(token.clone(), k);
        self.save(&inner.store);
        Ok(Session { name, token })
    }

    /// The account name a session token belongs to.
    pub fn session(&self, token: &str) -> Option<String> {
        let inner = self.lock();
        inner.sessions.get(token).and_then(|k| inner.store.users.get(k)).map(|u| u.name.clone())
    }

    pub fn logout(&self, token: &str) {
        let mut inner = self.lock();
        let Some(k) = inner.sessions.remove(token) else { return };
        if let Some(user) = inner.store.users.get_mut(&k) {
            user.sessions.retain(|t| t != token);
        }
        self.save(&inner.store);
    }

    /// `name` as registered.
    pub fn name(&self, name: &str) -> Option<String> {
        self.lock().store.users.get(&key(name)).map(|u| u.name.clone())
    }

    /// `me`'s friends and who asked to be, by name as registered.
    pub fn friends(&self, me: &str) -> (Vec<String>, Vec<String>) {
        let inner = self.lock();
        let users = &inner.store.users;
        let names = |keys: &[String]| -> Vec<String> {
            let mut names: Vec<String> = keys.iter().filter_map(|k| users.get(k)).map(|u| u.name.clone()).collect();
            names.sort_by_key(|n| n.to_lowercase());
            names
        };
        match users.get(&key(me)) {
            Some(u) => (names(&u.friends), names(&u.requests)),
            None => (vec![], vec![]),
        }
    }

    /// Whether `a` and `b` are friends.
    pub fn are_friends(&self, a: &str, b: &str) -> bool {
        self.lock().store.users.get(&key(a)).is_some_and(|u| u.friends.contains(&key(b)))
    }

    /// `me` asks `other` to be friends, or accepts their request. Returns
    /// `other`'s name as registered.
    pub fn befriend(&self, me: &str, other: &str) -> Result<String, Refusal> {
        let (me, other) = (key(me), key(other));
        let mut inner = self.lock();
        let users = &mut inner.store.users;
        let Some(name) = users.get(&other).map(|u| u.name.clone()) else { return Err(Refusal::NoAccount) };
        if me == other || !users.contains_key(&me) {
            return Err(Refusal::NoAccount);
        }
        let asked = users[&me].requests.contains(&other);
        if asked {
            for (a, b) in [(&me, &other), (&other, &me)] {
                let u = users.get_mut(a).expect("checked");
                u.requests.retain(|k| k != b);
                if !u.friends.contains(b) {
                    u.friends.push(b.clone());
                }
            }
        } else if !users[&me].friends.contains(&other) {
            let u = users.get_mut(&other).expect("checked");
            if !u.requests.contains(&me) {
                u.requests.push(me.clone());
            }
        }
        self.save(&inner.store);
        Ok(name)
    }

    /// Ends the friendship between `me` and `other`, or turns down their
    /// request, or takes back `me`'s own.
    pub fn unfriend(&self, me: &str, other: &str) {
        let (me, other) = (key(me), key(other));
        let mut inner = self.lock();
        for (a, b) in [(&me, &other), (&other, &me)] {
            if let Some(u) = inner.store.users.get_mut(a) {
                u.friends.retain(|k| k != b);
                u.requests.retain(|k| k != b);
            }
        }
        self.save(&inner.store);
    }

    /// `name`'s decks, deleted ones included.
    pub fn decks(&self, name: &str) -> Vec<SavedDeck> {
        self.lock().store.users.get(&key(name)).map(|u| u.decks.clone()).unwrap_or_default()
    }

    /// Keeps `deck` in `name`'s account in place of the one with its name, if
    /// it is newer. A deck with no code marks the deck deleted.
    pub fn put_deck(&self, name: &str, deck: SavedDeck) -> Result<(), Refusal> {
        let title = deck.name.trim();
        let code_ok = deck.code.is_empty() || (deck.code.starts_with("OWX") && deck.code.len() <= MAX_DECK_CODE);
        if title.is_empty() || title.chars().count() > MAX_DECK_NAME || !code_ok {
            return Err(Refusal::BadDeck);
        }
        let deck = SavedDeck { name: title.to_string(), ..deck };
        let mut inner = self.lock();
        let Some(user) = inner.store.users.get_mut(&key(name)) else { return Err(Refusal::NotLoggedIn) };
        match user.decks.iter_mut().find(|d| d.name == deck.name) {
            Some(old) if old.saved > deck.saved => return Ok(()),
            Some(old) => *old = deck,
            None => {
                if !deck.deleted() && user.decks.iter().filter(|d| !d.deleted()).count() >= MAX_DECKS {
                    return Err(Refusal::TooManyDecks);
                }
                user.decks.push(deck);
            }
        }
        // Keeps the newest deletion marks only.
        let mut marks: Vec<u64> = user.decks.iter().filter(|d| d.deleted()).map(|d| d.saved).collect();
        if marks.len() > MAX_DECKS {
            marks.sort_unstable();
            let oldest_kept = marks[marks.len() - MAX_DECKS];
            user.decks.retain(|d| !d.deleted() || d.saved >= oldest_kept);
        }
        self.save(&inner.store);
        Ok(())
    }

    /// Sets a new password for `name` and ends its sessions. For the
    /// server's administrator, as there is no email to recover it with.
    pub fn reset_password(&self, name: &str, password: &str) -> Result<(), Refusal> {
        if password.chars().count() < MIN_PASSWORD {
            return Err(Refusal::BadPassword);
        }
        let hash = hash(password);
        let mut inner = self.lock();
        let Some(user) = inner.store.users.get_mut(&key(name)) else { return Err(Refusal::NoAccount) };
        user.hash = hash;
        let ended = std::mem::take(&mut user.sessions);
        for t in ended {
            inner.sessions.remove(&t);
        }
        self.save(&inner.store);
        Ok(())
    }
}
