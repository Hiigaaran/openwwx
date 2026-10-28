//! Saved decks kept in the player's account on the common server, so they
//! follow them to any computer. Decks are matched by name, and the one saved
//! last wins; deleting a deck leaves a mark on the server so the other
//! computers delete their copy too.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rawdeal_cards::share::{deck_code, read_deck_code};
use rawdeal_core::DeckList;
use rawdeal_net::{Pin, Refusal, SavedDeck, relay};

use crate::builder::{deck_path, saved_decks};

/// Where and as whom decks are kept.
#[derive(Clone)]
pub struct Account {
    pub addr: String,
    pub pin: Pin,
    pub token: String,
    pub name: String,
}

/// What a background sync or upload ended with, for the builder to show.
pub type Outcome = Arc<Mutex<Option<Result<String, String>>>>;

/// A deck saved on this computer.
pub struct LocalDeck {
    pub name: String,
    pub code: String,
    /// Seconds since 1970.
    pub modified: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Save the account's deck on this computer.
    Download {
        name: String,
        code: String,
    },
    /// Delete this computer's copy: it was deleted elsewhere.
    DeleteLocal(String),
    Upload(SavedDeck),
}

/// What to do to make both sides the same.
pub fn plan(local: &[LocalDeck], remote: &[SavedDeck]) -> Vec<Step> {
    let mut steps = vec![];
    for r in remote {
        match local.iter().find(|l| l.name == r.name) {
            None if !r.deleted() => steps.push(Step::Download { name: r.name.clone(), code: r.code.clone() }),
            None => {}
            Some(l) if l.code == r.code => {}
            Some(l) if r.saved >= l.modified => steps.push(if r.deleted() {
                Step::DeleteLocal(r.name.clone())
            } else {
                Step::Download { name: r.name.clone(), code: r.code.clone() }
            }),
            Some(l) => {
                steps.push(Step::Upload(SavedDeck { name: l.name.clone(), code: l.code.clone(), saved: l.modified }))
            }
        }
    }
    for l in local {
        if !remote.iter().any(|r| r.name == l.name) {
            steps.push(Step::Upload(SavedDeck { name: l.name.clone(), code: l.code.clone(), saved: l.modified }));
        }
    }
    steps
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn local_decks() -> Vec<LocalDeck> {
    saved_decks()
        .into_iter()
        .filter_map(|name| {
            let path = deck_path(&name)?;
            let deck = rawdeal_cards::load_deck(&path).ok()?;
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            Some(LocalDeck { name, code: deck_code(&deck), modified })
        })
        .collect()
}

fn refused(why: Refusal) -> String {
    crate::text::refusal(why)
}

fn io_error(account: &Account, e: std::io::Error) -> String {
    format!("No se pudo conectar al servidor {}: {e}", account.addr)
}

fn write_local(name: &str, deck: &DeckList) -> Result<(), String> {
    let path = deck_path(name).ok_or("nombre de mazo vacío")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    rawdeal_cards::save_deck(&path, deck).map_err(|e| e.to_string())
}

/// Brings this computer's saved decks and the account's in line, and says
/// what changed.
pub fn sync(account: &Account) -> Result<String, String> {
    let remote = relay::decks(account.addr.as_str(), &account.pin, &account.token)
        .map_err(|e| io_error(account, e))?
        .map_err(refused)?;
    let (mut down, mut up, mut gone) = (0, 0, 0);
    for step in plan(&local_decks(), &remote) {
        match step {
            Step::Download { name, code } => {
                if let Ok(deck) = read_deck_code(&code) {
                    write_local(&name, &deck)?;
                    down += 1;
                }
            }
            Step::DeleteLocal(name) => {
                if let Some(path) = deck_path(&name) {
                    std::fs::remove_file(path).map_err(|e| e.to_string())?;
                    gone += 1;
                }
            }
            Step::Upload(deck) => {
                relay::save_deck(account.addr.as_str(), &account.pin, &account.token, deck)
                    .map_err(|e| io_error(account, e))?
                    .map_err(refused)?;
                up += 1;
            }
        }
    }
    let mut changes = vec![];
    for (n, one, many) in
        [(down, "traído de", "traídos de"), (up, "subido a", "subidos a"), (gone, "borrado según", "borrados según")]
    {
        match n {
            0 => {}
            1 => changes.push(format!("1 mazo {one} tu cuenta")),
            n => changes.push(format!("{n} mazos {many} tu cuenta")),
        }
    }
    Ok(if changes.is_empty() {
        format!("Tus mazos están al día con la cuenta {}.", account.name)
    } else {
        format!("Mazos sincronizados: {}.", changes.join(", "))
    })
}

/// Keeps `deck` in the account under `name`, or with `None` deletes it there.
pub fn push(account: &Account, name: &str, deck: Option<&DeckList>) -> Result<String, String> {
    let saved = SavedDeck { name: name.into(), code: deck.map(deck_code).unwrap_or_default(), saved: now() };
    relay::save_deck(account.addr.as_str(), &account.pin, &account.token, saved)
        .map_err(|e| io_error(account, e))?
        .map_err(refused)?;
    Ok(match deck {
        Some(_) => format!("Mazo «{name}» guardado también en tu cuenta."),
        None => format!("Mazo «{name}» borrado también de tu cuenta."),
    })
}

/// Runs `work` in the background and leaves its result in the returned slot.
pub fn spawn(ctx: &eframe::egui::Context, work: impl FnOnce() -> Result<String, String> + Send + 'static) -> Outcome {
    let slot: Outcome = Arc::new(Mutex::new(None));
    let (out, ctx) = (slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let result = work();
        if let Ok(mut s) = out.lock() {
            *s = Some(result);
        }
        ctx.request_repaint();
    });
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(name: &str, code: &str, modified: u64) -> LocalDeck {
        LocalDeck { name: name.into(), code: code.into(), modified }
    }

    fn remote(name: &str, code: &str, saved: u64) -> SavedDeck {
        SavedDeck { name: name.into(), code: code.into(), saved }
    }

    #[test]
    fn the_last_save_wins() {
        let steps = plan(
            &[local("Igual", "A", 5), local("Viejo", "B", 5), local("Nuevo", "C", 50), local("Solo aquí", "D", 5)],
            &[
                remote("Igual", "A", 1),
                remote("Viejo", "B2", 10),
                remote("Nuevo", "C1", 10),
                remote("Solo allá", "E", 10),
                remote("Ya borrado", "", 10),
            ],
        );
        assert_eq!(
            steps,
            vec![
                Step::Download { name: "Viejo".into(), code: "B2".into() },
                Step::Upload(remote("Nuevo", "C", 50)),
                Step::Download { name: "Solo allá".into(), code: "E".into() },
                Step::Upload(remote("Solo aquí", "D", 5)),
            ]
        );
    }

    #[test]
    fn deletions_travel_unless_saved_again() {
        let steps = plan(
            &[local("Borrado", "A", 5), local("Revivido", "B", 50)],
            &[remote("Borrado", "", 10), remote("Revivido", "", 10)],
        );
        assert_eq!(steps, vec![Step::DeleteLocal("Borrado".into()), Step::Upload(remote("Revivido", "B", 50))]);
    }
}
