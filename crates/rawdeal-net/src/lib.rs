//! Online play. The player who hosts a match runs it with the rules engine
//! ([`HostCore`]); everyone else connects, sends their deck and their actions,
//! and gets back only what their seat may see. Spectators see public cards.
//!
//! Messages are JSON, one per line, over TCP ([`wire`]). [`tcp`] has the
//! host and the client connection, direct or through the common server
//! ([`relay`]), which keeps player [`accounts`], lists open matches and
//! passes messages along, encrypted ([`secure`]).

pub mod accounts;
pub mod host;
pub mod pump;
pub mod relay;
pub mod secure;
pub mod stats;
pub mod tcp;
pub mod wire;

use rawdeal_core::{Action, CardId, DeckList, DeckProblem, DefIdx, Event, Mode, PlayerId, PlayerView, RevolutionRules};
use serde::{Deserialize, Serialize};

pub use accounts::{Accounts, Session};
pub use host::{ConnId, HostCore};
pub use relay::{DEFAULT_SERVER_PORT, Friend, FromPlayer, Listing, MatchId, Notice, PlayerHistory, Presence, Refusal};
pub use secure::Pin;
pub use stats::{Record, Standing, Stats};
pub use tcp::{Connection, Dial, HostHandle};

/// Port a host listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7777;

/// The app version; host and guests must match.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Player,
    Spectator,
}

/// What a guest sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    /// First message: who you are and which card sets you have
    /// ([`rawdeal_core::CardDb::fingerprints`]).
    Hello {
        app: String,
        name: String,
        role: Role,
        sets: Vec<(String, u64)>,
        /// Back to your seat after losing the connection: the token from
        /// your [`ServerMsg::Welcome`].
        #[serde(default)]
        rejoin: Option<u64>,
    },
    /// Your deck, as a player. Sending another one before the match starts
    /// replaces it.
    Deck(DeckList),
    /// An action for your pending decision.
    Act(Action),
    /// Give up the match: the other player wins by forfeit.
    Concede,
    /// A line for the table chat.
    Chat(String),
}

/// The match settings the host chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchInfo {
    pub name: String,
    pub mode: Mode,
    pub revolution: RevolutionRules,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSlot {
    pub name: String,
    /// Their deck is in and legal.
    pub ready: bool,
    /// They lost the connection during the match; they lose by forfeit if
    /// they do not come back in time ([`host::ABANDON_AFTER`]).
    #[serde(default)]
    pub away: bool,
}

/// Who is at the table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lobby {
    pub players: [Option<PlayerSlot>; 2],
    pub spectators: Vec<String>,
    pub started: bool,
}

/// Why the host turned a guest away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Rejection {
    /// Different app versions.
    WrongVersion { host: String },
    /// Different card sets: sets only the host has, sets that differ, sets
    /// only the guest has, by id prefix.
    SetsDiffer { missing: Vec<String>, different: Vec<String>, extra: Vec<String> },
    /// Both player seats are taken.
    Full,
    /// The first message must be a Hello.
    NoHello,
    /// The common server has no such match (it ended).
    NoMatch,
    /// The common server needs you logged in.
    NotLoggedIn,
}

/// One step of the match as a seat sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    /// What happened, with hidden cards left out.
    pub events: Vec<Event>,
    /// The table after those events.
    pub view: PlayerView,
    /// Your legal actions; empty when the match is not waiting on you.
    pub legal: Vec<Action>,
    /// Definitions of the cards the events name.
    pub cards: Vec<(CardId, DefIdx)>,
}

/// What the host sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    /// You are in: your player seat, or `None` as a spectator. `token` gets
    /// you back into your seat if the connection drops.
    Welcome {
        seat: Option<PlayerId>,
        info: MatchInfo,
        #[serde(default)]
        token: Option<u64>,
    },
    /// You were turned away; the connection closes.
    Rejected(Rejection),
    /// Who is at the table now.
    Lobby(Lobby),
    /// Your deck is not legal for this match.
    DeckRejected(Vec<DeckProblem>),
    Update(Box<Update>),
    /// Your action was not legal; the match still waits on you.
    ActionRejected(String),
    /// A line of the table chat.
    Chat {
        from: String,
        text: String,
    },
}

/// Compares the host's card sets with a guest's.
pub fn compare_sets(host: &[(String, u64)], guest: &[(String, u64)]) -> Option<Rejection> {
    if host == guest {
        return None;
    }
    let find = |list: &[(String, u64)], name: &str| list.iter().find(|(n, _)| n == name).map(|(_, h)| *h);
    let missing = host.iter().filter(|(n, _)| find(guest, n).is_none()).map(|(n, _)| n.clone()).collect();
    let different =
        host.iter().filter(|(n, h)| find(guest, n).is_some_and(|g| g != *h)).map(|(n, _)| n.clone()).collect();
    let extra = guest.iter().filter(|(n, _)| find(host, n).is_none()).map(|(n, _)| n.clone()).collect();
    Some(Rejection::SetsDiffer { missing, different, extra })
}
