//! Friends and invitations on the common server, while logged in: a live
//! link kept in the background, and back again when it drops.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use rawdeal_net::{Friend, FromPlayer, MatchId, MatchInfo, Notice, Pin, Presence, Refusal};

/// How long to wait before connecting again after the link drops.
const RETRY: Duration = Duration::from_secs(10);

/// A friend's invitation to their match.
#[derive(Clone)]
pub struct Invite {
    pub from: String,
    pub id: MatchId,
    pub info: MatchInfo,
}

#[derive(Default)]
struct State {
    link: Option<Presence>,
    connecting: bool,
    friends: Vec<Friend>,
    requests: Vec<String>,
    invites: Vec<Invite>,
    /// What went wrong last, for display.
    problem: Option<String>,
    /// The session ended on the server.
    expired: bool,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Social {
    pub addr: String,
    pub token: String,
    pin: Pin,
    state: Arc<Mutex<State>>,
    retry_at: Option<Instant>,
    ctx: egui::Context,
}

impl Social {
    pub fn new(addr: String, token: String, pin: Pin, ctx: egui::Context) -> Social {
        let mut social = Social { addr, token, pin, state: Arc::default(), retry_at: None, ctx };
        social.connect();
        social
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn connect(&mut self) {
        self.state().connecting = true;
        let (addr, token, pin, state, ctx) =
            (self.addr.clone(), self.token.clone(), self.pin.clone(), self.state.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let notices = match Presence::connect(addr.as_str(), &pin, &token) {
                Ok((link, notices)) => {
                    let mut s = lock(&state);
                    s.link = Some(link);
                    s.problem = None;
                    notices
                }
                Err(e) => {
                    let mut s = lock(&state);
                    s.connecting = false;
                    s.problem = Some(format!("No se pudo conectar para ver a tus amigos: {e}"));
                    ctx.request_repaint();
                    return;
                }
            };
            lock(&state).connecting = false;
            ctx.request_repaint();
            while let Ok(notice) = notices.recv() {
                let mut s = lock(&state);
                match notice {
                    Notice::Friends { friends, requests } => {
                        s.friends = friends;
                        s.requests = requests;
                    }
                    Notice::Invited { from, id, info } => {
                        s.invites.retain(|i| i.from != from);
                        s.invites.push(Invite { from, id, info });
                    }
                    Notice::Refused(Refusal::NotLoggedIn) => s.expired = true,
                    Notice::Refused(why) => s.problem = Some(crate::text::refusal(why)),
                }
                drop(s);
                ctx.request_repaint();
            }
            let mut s = lock(&state);
            s.link = None;
            for f in &mut s.friends {
                f.online = false;
            }
            ctx.request_repaint();
        });
    }

    /// Connects again if the link dropped. Returns `false` once the session
    /// ended on the server.
    pub fn poll(&mut self) -> bool {
        let (down, expired) = {
            let s = self.state();
            (s.link.is_none() && !s.connecting, s.expired)
        };
        if expired {
            return false;
        }
        if down {
            match self.retry_at {
                None => self.retry_at = Some(Instant::now() + RETRY),
                Some(t) if Instant::now() >= t => {
                    self.retry_at = None;
                    self.connect();
                }
                Some(t) => self.ctx.request_repaint_after(t - Instant::now()),
            }
        }
        true
    }

    pub fn connected(&self) -> bool {
        self.state().link.is_some()
    }

    pub fn send(&self, msg: FromPlayer) {
        let mut s = self.state();
        s.problem = None;
        if let Some(link) = &s.link {
            link.send(&msg);
        }
    }

    pub fn friends(&self) -> Vec<Friend> {
        self.state().friends.clone()
    }

    pub fn requests(&self) -> Vec<String> {
        self.state().requests.clone()
    }

    pub fn invites(&self) -> Vec<Invite> {
        self.state().invites.clone()
    }

    pub fn dismiss(&self, from: &str) {
        self.state().invites.retain(|i| i.from != from);
    }

    pub fn problem(&self) -> Option<String> {
        self.state().problem.clone()
    }
}
