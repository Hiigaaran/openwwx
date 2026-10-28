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
}

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
        let user = User { name: name.into(), hash, sessions: vec![token.clone()], created: now() };
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
