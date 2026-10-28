//! Start screen and app state: choose the card folder, Superstars and decks.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText, Ui};
use rawdeal_ai::Difficulty;
use rawdeal_core::{
    CardDb, DeckList, DeckProblem, Engine, GameSetup, Mode, RevolutionRules, is_revolution, validate_deck_with,
};
use rawdeal_net::{
    APP_VERSION, DEFAULT_PORT, DEFAULT_SERVER_PORT, Dial, FromPlayer, HostCore, HostHandle, Listing, MatchId,
    MatchInfo, Pin, PlayerHistory, Refusal, Session, Standing, relay, secure, tcp,
};
use serde::{Deserialize, Serialize};

use crate::builder::{Builder, Exit};
use crate::game::{Leave, Match, Speed};
use crate::social::Social;
use crate::table::Table;
use crate::text;

const SETTINGS_KEY: &str = "openwwx-settings";

/// Where a background image import leaves its result: images copied, or an error.
type ImportResult = Arc<Mutex<Option<Result<usize, String>>>>;

/// Where a background request for the common server's matches leaves them.
type ListResult = Arc<Mutex<Option<Result<Vec<Listing>, String>>>>;

/// Where a background login or registration leaves its answer.
type AuthResult = Arc<Mutex<Option<Result<Result<Session, Refusal>, std::io::Error>>>>;

/// The ranking or a player's history, from the common server.
enum Board {
    Ranking(Vec<Standing>),
    History(String, PlayerHistory),
}

/// Where a background request for a [`Board`] leaves it.
type BoardResult = Arc<Mutex<Option<Result<Board, String>>>>;

/// How often an open match list asks the server again.
const LIST_REFRESH: std::time::Duration = std::time::Duration::from_secs(5);

/// Remembered between runs.
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    cards_dir: Option<PathBuf>,
    superstars: [Option<String>; 2],
    decks: [Option<PathBuf>; 2],
    advanced: bool,
    /// Revolution format: only Revolution cards (Advanced rules).
    #[serde(default)]
    revolution: bool,
    /// Which Revolution rules the Revolution format uses; `None` is all of them.
    #[serde(default)]
    revolution_rules: Option<RevolutionRules>,
    /// Virtual limits (Advanced rules).
    #[serde(default)]
    virtual_rules: bool,
    #[serde(default)]
    speed: Speed,
    #[serde(default)]
    difficulty: Difficulty,
    /// Your name in online matches.
    #[serde(default)]
    online_name: String,
    /// Port to host on; `None` is the default port.
    #[serde(default)]
    online_port: Option<u16>,
    /// Last address joined, as `host:port`.
    #[serde(default)]
    online_addr: String,
    /// The common server, as `host:port`.
    #[serde(default)]
    server_addr: String,
    /// Logged in on a common server.
    #[serde(default)]
    server_session: Option<SavedSession>,
    /// The certificate fingerprint trusted for each common server.
    #[serde(default)]
    server_pins: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedSession {
    /// The server, as `host:port`.
    addr: String,
    name: String,
    token: String,
}

pub struct App {
    settings: Settings,
    db: Arc<CardDb>,
    /// Card sets by id prefix, with their names, for the deck builder.
    sets: Vec<(String, String)>,
    builder: Option<Builder>,
    /// Where the cards came from, for display.
    source: String,
    error: Option<String>,
    game: Option<Match>,
    /// Folder typed by hand, for systems without a folder picker.
    typed_dir: String,
    /// An image pack import running in the background, and its result.
    importing: Option<ImportResult>,
    /// The common server's matches while the list is open.
    matches: Option<Result<Vec<Listing>, String>>,
    fetching: Option<ListResult>,
    fetched_at: Option<std::time::Instant>,
    /// The certificate trusted for each common server, as connections
    /// learn it.
    pins: HashMap<String, Pin>,
    /// Account form on the common server.
    login_name: String,
    login_password: String,
    logging_in: Option<(String, AuthResult)>,
    /// The ranking or a history on show, and one on its way.
    board: Option<Result<Board, String>>,
    board_fetch: Option<BoardResult>,
    /// A server whose certificate is not the one trusted before.
    pin_changed: Option<String>,
    /// Friends and invitations, while logged in.
    social: Option<Social>,
    /// A name typed to add as a friend.
    friend_name: String,
    /// Friends invited to the match you host.
    invited: Vec<String>,
    /// The last import's result, for display.
    notice: Option<String>,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        let mut app = App {
            settings,
            db: Arc::new(rawdeal_cards::test_db()),
            sets: Vec::new(),
            builder: None,
            source: String::new(),
            error: None,
            game: None,
            typed_dir: String::new(),
            importing: None,
            matches: None,
            fetching: None,
            fetched_at: None,
            pins: HashMap::new(),
            login_name: String::new(),
            login_password: String::new(),
            logging_in: None,
            board: None,
            board_fetch: None,
            pin_changed: None,
            social: None,
            friend_name: String::new(),
            invited: Vec::new(),
            notice: None,
            ctx: cc.egui_ctx.clone(),
        };
        // Cards next to the program (a "cards" folder) are used when nothing was chosen yet.
        let dir = app.settings.cards_dir.clone().or_else(bundled_cards_dir);
        app.load_cards(dir);
        app
    }

    fn load_cards(&mut self, dir: Option<PathBuf>) {
        self.error = None;
        match dir {
            None => self.use_test_set(),
            Some(dir) => match rawdeal_cards::load_dir(&dir).and_then(|sets| {
                let names = crate::builder::set_names(&sets);
                rawdeal_cards::build_db(sets).map(|db| (db, names))
            }) {
                Ok((db, names)) if !db.superstars().is_empty() => {
                    self.sets = names;
                    self.source = format!(
                        "{} ({} cartas, {} Superstars)",
                        dir.display(),
                        db.cards().len(),
                        db.superstars().len()
                    );
                    self.db = Arc::new(db);
                    self.settings.cards_dir = Some(dir);
                }
                Ok(_) => {
                    self.use_test_set();
                    self.error = Some(format!("No hay Superstars en {}.", dir.display()));
                }
                Err(e) => {
                    self.use_test_set();
                    self.error = Some(format!("No se pudieron cargar las cartas de {}: {e}", dir.display()));
                }
            },
        }
        let pictures = crate::art::install(&self.ctx, self.settings.cards_dir.as_deref());
        if pictures > 0 {
            self.source.push_str(&if pictures == 1 {
                ", 1 imagen de arte".to_owned()
            } else {
                format!(", {pictures} imágenes de arte")
            });
        }
        for (i, s) in self.settings.superstars.iter_mut().enumerate() {
            if s.as_ref().is_none_or(|id| self.db.superstar_by_id(id).is_none()) {
                *s = self.db.superstars().get(i.min(self.db.superstars().len() - 1)).map(|s| s.id.clone());
            }
        }
    }

    /// Copies the images of OCTGN packs into the card folder's `art` subfolder,
    /// in the background.
    fn import_images(&mut self, packs: Vec<PathBuf>) {
        let Some(dir) = self.settings.cards_dir.clone() else { return };
        let result: ImportResult = Arc::new(Mutex::new(None));
        self.importing = Some(result.clone());
        self.notice = None;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = crate::o8c::extract(&packs, &dir.join("art"));
            if let Ok(mut slot) = result.lock() {
                *slot = Some(r);
            }
            ctx.request_repaint();
        });
    }

    /// Picks up a finished import: re-indexes the art and says how many cards got a picture.
    fn poll_import(&mut self) {
        let Some(result) = self.importing.as_ref().and_then(|r| r.lock().ok().and_then(|mut s| s.take())) else {
            return;
        };
        self.importing = None;
        match result {
            Ok(copied) => {
                self.load_cards(self.settings.cards_dir.clone());
                let with_art = self.db.cards().iter().filter(|c| crate::art::has_card_art(&self.ctx, c)).count();
                self.notice = Some(format!(
                    "Se copiaron {copied} imágenes. {with_art} de {} cartas tienen imagen.",
                    self.db.cards().len()
                ));
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn use_test_set(&mut self) {
        self.db = Arc::new(rawdeal_cards::test_db());
        self.sets = crate::builder::set_names(&[rawdeal_cards::test_set()]);
        self.source = "cartas de prueba (inventadas)".into();
        self.settings.cards_dir = None;
    }

    fn mode(&self) -> Mode {
        if self.settings.revolution {
            Mode::Revolution
        } else if self.settings.virtual_rules {
            Mode::Virtual
        } else if self.settings.advanced {
            Mode::Advanced
        } else {
            Mode::QuickStart
        }
    }

    /// The Revolution rules in use: the chosen ones in the Revolution format.
    fn revolution_rules(&self) -> RevolutionRules {
        match self.mode() {
            Mode::Revolution => self.settings.revolution_rules.unwrap_or(RevolutionRules::ALL),
            mode => mode.revolution_rules(),
        }
    }

    fn deck(&self, i: usize) -> Result<DeckList, String> {
        let star = self.settings.superstars[i].clone().ok_or("Elige una Superstar.")?;
        match &self.settings.decks[i] {
            Some(path) => {
                let deck = rawdeal_cards::load_deck(path).map_err(|e| e.to_string())?;
                let mut problems = validate_deck_with(&self.db, &deck, self.mode(), self.revolution_rules());
                if !self.mode().is_advanced() {
                    problems.retain(|p| !matches!(p, DeckProblem::NotABacklashCard { .. }));
                }
                if problems.is_empty() {
                    Ok(deck)
                } else {
                    let problems: Vec<String> =
                        problems.iter().map(|p| text::deck_problem(p, |id| text::card_title(&self.db, id))).collect();
                    Err(format!("El mazo {} no es legal: {}", file_name(path), problems.join(" ")))
                }
            }
            None => rawdeal_cards::auto_deck_with(&self.db, &star, self.mode(), self.revolution_rules())
                .ok_or(format!("No se pudo armar un mazo para {star}.")),
        }
    }

    /// Plays seat `i` with the deck in `file`, and its Superstar.
    fn use_deck(&mut self, i: usize, file: PathBuf) {
        match rawdeal_cards::load_deck(&file) {
            Ok(d) => {
                self.settings.superstars[i] = Some(d.superstar);
                self.settings.decks[i] = Some(file);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn start(&mut self) {
        self.error = None;
        let decks = match (self.deck(0), self.deck(1)) {
            (Ok(a), Ok(b)) => [a, b],
            (Err(e), _) | (_, Err(e)) => {
                self.error = Some(e);
                return;
            }
        };
        let setup =
            GameSetup { mode: self.mode(), revolution: Some(self.revolution_rules()), ..GameSetup::standard(decks) };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        match Engine::new(self.db.clone()).new_game(&setup, seed) {
            Ok((state, events)) => {
                let table = Table::local(self.db.clone(), state, self.settings.difficulty.bot(seed ^ 0x5eed));
                self.game = Some(Match::new(table, events, self.settings.speed))
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn online_name(&self) -> String {
        match self.settings.online_name.trim() {
            "" => "Jugador".into(),
            name => name.into(),
        }
    }

    /// The deck you play online: your Superstar and deck from the form.
    fn online_deck(&mut self) -> Option<DeckList> {
        self.error = None;
        self.deck(0).map_err(|e| self.error = Some(e)).ok()
    }

    fn server_addr(&self) -> String {
        let addr = self.settings.server_addr.trim();
        if addr.contains(':') { addr.into() } else { format!("{addr}:{DEFAULT_SERVER_PORT}") }
    }

    /// The certificate trusted for the server at `addr`.
    fn pin(&mut self, addr: &str) -> Pin {
        let known = self.settings.server_pins.get(addr).cloned();
        self.pins.entry(addr.to_string()).or_insert_with(|| secure::pin(known)).clone()
    }

    /// Keeps the certificates connections learned.
    fn remember_pins(&mut self) {
        for (addr, pin) in &self.pins {
            if let Some(print) = pin.lock().ok().and_then(|p| p.clone()) {
                self.settings.server_pins.insert(addr.clone(), print);
            }
        }
    }

    /// Your session on the server in the form, if you logged in there.
    fn session(&self) -> Option<&SavedSession> {
        let addr = self.server_addr();
        self.settings.server_session.as_ref().filter(|s| s.addr == addr)
    }

    /// Says why talking to the server at `addr` failed.
    fn server_error(&mut self, addr: &str, e: &std::io::Error) -> String {
        if e.to_string().contains(secure::CHANGED) {
            self.pin_changed = Some(addr.to_string());
            return format!(
                "El certificado del servidor {addr} no es el de la última vez. Puede que alguien esté \
                 interceptando la conexión. Si su administrador lo cambió, pulsa «Confiar en el certificado nuevo»."
            );
        }
        match tcp::refusal(e) {
            Some(Refusal::NotLoggedIn) => {
                self.settings.server_session = None;
                "Tu sesión en el servidor terminó. Vuelve a entrar con tu cuenta.".into()
            }
            Some(Refusal::Full) => format!("El servidor {addr} no tiene lugar para más partidas."),
            Some(why) => text::refusal(why),
            None => format!("No se pudo conectar al servidor {addr}: {e}"),
        }
    }

    /// Keeps the link for friends and invitations up while you are logged in.
    fn poll_social(&mut self) {
        let wanted = self.session().map(|s| (s.addr.clone(), s.token.clone()));
        if self.social.as_ref().map(|s| (&s.addr, &s.token)) != wanted.as_ref().map(|(a, t)| (a, t)) {
            self.social = None;
            if let Some((addr, token)) = wanted {
                let pin = self.pin(&addr);
                self.social = Some(Social::new(addr, token, pin, self.ctx.clone()));
            }
        }
        if self.social.as_mut().is_some_and(|s| !s.poll()) {
            self.social = None;
            self.settings.server_session = None;
            self.error = Some(text::refusal(Refusal::NotLoggedIn));
        }
    }

    /// Your friends, their requests and invitations, and adding a friend.
    fn friends_ui(&mut self, ui: &mut Ui) {
        let Some(social) = &self.social else { return };
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Amigos").strong());
            if !social.connected() {
                ui.spinner();
                ui.label(RichText::new("conectando…").weak());
            }
        });
        if let Some(problem) = social.problem() {
            ui.colored_label(egui::Color32::from_rgb(240, 110, 110), problem);
        }
        let mut join = None;
        for invite in social.invites() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} te invita a «{}» ({}).",
                        invite.from,
                        invite.info.name,
                        text::mode(invite.info.mode)
                    ))
                    .color(egui::Color32::from_rgb(120, 200, 120)),
                );
                if ui.button("Jugar").clicked() {
                    join = Some((invite.clone(), false));
                }
                if ui.button("Mirar").clicked() {
                    join = Some((invite.clone(), true));
                }
                if ui.button("Descartar").clicked() {
                    social.dismiss(&invite.from);
                }
            });
        }
        for name in social.requests() {
            ui.horizontal(|ui| {
                ui.label(format!("{name} quiere ser tu amigo."));
                if ui.button("Aceptar").clicked() {
                    social.send(FromPlayer::Befriend { name: name.clone() });
                }
                if ui.button("Rechazar").clicked() {
                    social.send(FromPlayer::Unfriend { name: name.clone() });
                }
            });
        }
        let friends = social.friends();
        if friends.is_empty() {
            ui.label(RichText::new("Todavía no tienes amigos en este servidor.").weak());
        }
        egui::Grid::new("friends").spacing([16.0, 4.0]).show(ui, |ui| {
            for f in &friends {
                let (dot, state) = if f.online {
                    (egui::Color32::from_rgb(120, 200, 120), "en línea")
                } else {
                    (egui::Color32::GRAY, "desconectado")
                };
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, dot);
                    ui.label(&f.name);
                });
                ui.label(RichText::new(state).weak());
                if ui.small_button("Quitar").on_hover_text("Deja de ser su amigo").clicked() {
                    social.send(FromPlayer::Unfriend { name: f.name.clone() });
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            let field =
                ui.add(egui::TextEdit::singleline(&mut self.friend_name).hint_text("nombre").desired_width(140.0));
            let typed = !self.friend_name.trim().is_empty();
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(typed, egui::Button::new("Agregar amigo"))
                .on_hover_text("Le llega tu solicitud; son amigos cuando la acepte.")
                .clicked()
                || (enter && typed)
            {
                social.send(FromPlayer::Befriend { name: self.friend_name.trim().to_string() });
                self.friend_name.clear();
            }
        });
        if let Some((invite, watch)) = join {
            social.dismiss(&invite.from);
            self.join(Some(invite.id), watch);
        }
    }

    /// While you host a match on the server: invite friends who are online.
    fn invite_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.game.as_ref().and_then(|g| g.table().open_on_server()) else {
            self.invited.clear();
            return;
        };
        let Some(social) = &self.social else { return };
        let online: Vec<String> = social.friends().into_iter().filter(|f| f.online).map(|f| f.name).collect();
        if online.is_empty() {
            return;
        }
        egui::Window::new("Invitar amigos")
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                for name in online {
                    ui.horizontal(|ui| {
                        ui.label(&name);
                        if self.invited.contains(&name) {
                            ui.label(RichText::new("invitado").weak());
                        } else if ui.button("Invitar").clicked() {
                            social.send(FromPlayer::Invite { name: name.clone(), id });
                            self.invited.push(name);
                        }
                    });
                }
            });
    }

    /// Logs in, or makes the account, in the background.
    fn log_in(&mut self, register: bool) {
        let addr = self.server_addr();
        let pin = self.pin(&addr);
        let (name, password) = (self.login_name.trim().to_string(), self.login_password.clone());
        let slot: AuthResult = Arc::new(Mutex::new(None));
        self.logging_in = Some((addr.clone(), slot.clone()));
        self.error = None;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let answer = if register {
                relay::register(addr.as_str(), &pin, &name, &password)
            } else {
                relay::login(addr.as_str(), &pin, &name, &password)
            };
            if let Ok(mut s) = slot.lock() {
                *s = Some(answer);
            }
            ctx.request_repaint();
        });
    }

    fn poll_login(&mut self) {
        let Some((addr, slot)) = &self.logging_in else { return };
        let Some(answer) = slot.lock().ok().and_then(|mut s| s.take()) else { return };
        let addr = addr.clone();
        self.logging_in = None;
        self.remember_pins();
        match answer {
            Ok(Ok(session)) => {
                self.login_password.clear();
                self.settings.server_session = Some(SavedSession { addr, name: session.name, token: session.token });
            }
            Ok(Err(why)) => self.error = Some(text::refusal(why)),
            Err(e) => self.error = Some(self.server_error(&addr, &e)),
        }
    }

    fn log_out(&mut self) {
        let Some(session) = self.settings.server_session.take() else { return };
        let pin = self.pin(&session.addr);
        std::thread::spawn(move || relay::logout(session.addr.as_str(), &pin, &session.token).ok());
    }

    /// Hosts a match with the chosen rules and sits you in it with your
    /// Superstar and deck: on this computer's port, or listed on the common
    /// server.
    fn host(&mut self, on_server: bool) {
        let Some(deck) = self.online_deck() else { return };
        let session = if on_server { self.session().cloned() } else { None };
        if on_server && session.is_none() {
            self.error = Some("Entra con tu cuenta del servidor para publicar una partida.".into());
            return;
        }
        let name = session.as_ref().map_or_else(|| self.online_name(), |s| s.name.clone());
        let info =
            MatchInfo { name: format!("Partida de {name}"), mode: self.mode(), revolution: self.revolution_rules() };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let core = HostCore::new(self.db.clone(), info, seed);
        let port = self.settings.online_port.unwrap_or(DEFAULT_PORT);
        let started = if let Some(session) = session {
            let addr = self.server_addr();
            let pin = self.pin(&addr);
            let started = HostHandle::start_on_server(core, addr.as_str(), &session.token, &pin);
            self.remember_pins();
            started.map_err(|e| self.server_error(&addr, &e))
        } else {
            HostHandle::start(core, port).map_err(|e| format!("No se pudo hospedar en el puerto {port}: {e}"))
        };
        match started {
            Ok(mut host) => {
                let conn = host.connect_local();
                let table = Table::online(self.db.clone(), conn, &name, Some(deck), Some(host), None);
                self.game = Some(Match::new(table, vec![], self.settings.speed));
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Joins a match, at the typed address or on the common server: as a
    /// player with your Superstar and deck, or to watch.
    fn join(&mut self, on_server: Option<MatchId>, watch: bool) {
        let deck = if watch {
            self.error = None;
            None
        } else {
            let Some(deck) = self.online_deck() else { return };
            Some(deck)
        };
        let (dial, name) = match on_server {
            Some(id) => {
                let Some(session) = self.session().cloned() else {
                    self.error = Some("Entra con tu cuenta del servidor para jugar o mirar.".into());
                    return;
                };
                let pin = self.pin(&session.addr);
                (Dial::Server { addr: session.addr, id, session: session.token, pin }, session.name)
            }
            None => {
                let addr = self.settings.online_addr.trim().to_string();
                (
                    Dial::Direct(if addr.contains(':') { addr } else { format!("{addr}:{DEFAULT_PORT}") }),
                    self.online_name(),
                )
            }
        };
        let conn = dial.connect();
        self.remember_pins();
        let conn = conn.map_err(|e| match &dial {
            Dial::Server { addr, .. } => self.server_error(addr, &e),
            Dial::Direct(addr) => format!("No se pudo conectar a {addr}: {e}"),
        });
        match conn {
            Ok(conn) => {
                let table = Table::online(self.db.clone(), conn, &name, deck, None, Some(dial));
                self.game = Some(Match::new(table, vec![], self.settings.speed));
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Asks the common server for its matches, in the background.
    fn fetch_matches(&mut self) {
        let slot: ListResult = Arc::new(Mutex::new(None));
        self.fetching = Some(slot.clone());
        self.fetched_at = Some(std::time::Instant::now());
        let addr = self.server_addr();
        let pin = self.pin(&addr);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = relay::list(addr.as_str(), &pin).map_err(|e| {
                if e.to_string().contains(secure::CHANGED) {
                    format!("El certificado del servidor {addr} no es el de la última vez; no se pidió la lista.")
                } else {
                    format!("No se pudo conectar al servidor {addr}: {e}")
                }
            });
            if let Ok(mut s) = slot.lock() {
                *s = Some(result);
            }
            ctx.request_repaint();
        });
    }

    /// Asks the common server for the ranking, or for `who`'s history, in
    /// the background.
    fn fetch_board(&mut self, who: Option<String>) {
        let slot: BoardResult = Arc::new(Mutex::new(None));
        self.board_fetch = Some(slot.clone());
        let addr = self.server_addr();
        let pin = self.pin(&addr);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = match who {
                None => relay::ranking(addr.as_str(), &pin).map(Board::Ranking),
                Some(name) => relay::history(addr.as_str(), &pin, &name).map(|h| Board::History(name, h)),
            };
            let result = result.map_err(|e| {
                if e.to_string().contains(secure::CHANGED) {
                    format!("El certificado del servidor {addr} no es el de la última vez.")
                } else {
                    format!("No se pudo conectar al servidor {addr}: {e}")
                }
            });
            if let Ok(mut s) = slot.lock() {
                *s = Some(result);
            }
            ctx.request_repaint();
        });
    }

    /// The ranking or a player's history, if one is open.
    fn board_ui(&mut self, ui: &mut Ui) {
        if let Some(result) = self.board_fetch.as_ref().and_then(|s| s.lock().ok().and_then(|mut s| s.take())) {
            self.board_fetch = None;
            self.remember_pins();
            self.board = Some(result);
        }
        if self.board.is_none() && self.board_fetch.is_none() {
            return;
        }
        ui.add_space(12.0);
        let mut close = false;
        let mut show = None;
        ui.horizontal(|ui| {
            let title = match &self.board {
                Some(Ok(Board::History(name, _))) => format!("Historial de {name}"),
                _ => format!("Ranking de {}", self.server_addr()),
            };
            ui.label(RichText::new(title).strong());
            if self.board_fetch.is_some() {
                ui.spinner();
            }
            if matches!(self.board, Some(Ok(Board::History(..)))) && ui.button("Ver ranking").clicked() {
                show = Some(None);
            }
            if ui.button("Cerrar").clicked() {
                close = true;
            }
        });
        match &self.board {
            None => {}
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::from_rgb(240, 110, 110), e);
            }
            Some(Ok(Board::Ranking(list))) if list.is_empty() => {
                ui.label("Todavía no hay partidas terminadas en este servidor.");
            }
            Some(Ok(Board::Ranking(list))) => {
                egui::Grid::new("ranking").striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                    for h in ["", "Jugador", "Puntos", "Ganadas", "Perdidas", "Empates"] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for (i, p) in list.iter().enumerate() {
                        ui.label(format!("{}.", i + 1));
                        if ui.link(&p.name).on_hover_text("Ver su historial").clicked() {
                            show = Some(Some(p.name.clone()));
                        }
                        ui.label(p.rating.to_string());
                        ui.label(p.wins.to_string());
                        ui.label(p.losses.to_string());
                        ui.label(p.draws.to_string());
                        ui.end_row();
                    }
                });
            }
            Some(Ok(Board::History(name, history))) => {
                match &history.standing {
                    Some(p) => ui.label(format!(
                        "{} puntos: {} ganadas, {} perdidas, {} empates.",
                        p.rating, p.wins, p.losses, p.draws
                    )),
                    None => ui.label(format!("{name} todavía no termina ninguna partida en este servidor.")),
                };
                let me = name.to_lowercase();
                egui::Grid::new("history").striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                    for r in &history.records {
                        let seat = usize::from(r.players[1].to_lowercase() == me);
                        let rival = &r.players[1 - seat];
                        let change = if seat == 0 { r.change } else { -r.change };
                        let how = r.method.map_or("", text::win_method);
                        let result = match r.winner {
                            None => "Empate".to_string(),
                            Some(w) if usize::from(w) == seat => format!("Ganó {how}"),
                            Some(_) => format!("Perdió {how}"),
                        };
                        ui.label(text::ago(r.when));
                        ui.horizontal(|ui| {
                            ui.label("contra");
                            if ui.link(rival).on_hover_text("Ver su historial").clicked() {
                                show = Some(Some(rival.clone()));
                            }
                        });
                        ui.label(result);
                        ui.label(text::mode(r.mode));
                        ui.label(format!("{change:+}"));
                        ui.end_row();
                    }
                });
            }
        }
        if close {
            self.board = None;
            self.board_fetch = None;
        }
        if let Some(who) = show {
            self.fetch_board(who);
        }
    }

    /// Picks up the server's answer, and asks again every few seconds while
    /// the list is open.
    fn poll_matches(&mut self) {
        if let Some(result) = self.fetching.as_ref().and_then(|s| s.lock().ok().and_then(|mut s| s.take())) {
            self.fetching = None;
            self.remember_pins();
            if result.as_ref().is_err_and(|e| e.contains("certificado")) {
                self.pin_changed = Some(self.server_addr());
            }
            self.matches = Some(result);
        }
        if self.matches.is_some() && self.fetching.is_none() {
            let due = self.fetched_at.is_none_or(|t| t.elapsed() >= LIST_REFRESH);
            if due {
                self.fetch_matches();
            } else {
                self.ctx.request_repaint_after(LIST_REFRESH);
            }
        }
    }

    fn online_form(&mut self, ui: &mut Ui) {
        ui.heading("En línea");
        ui.label(
            "Juegas con tu Superstar y tu mazo de arriba. Los dos jugadores y los espectadores necesitan los mismos \
             sets de cartas y la misma versión de OpenWWX.",
        );
        ui.add_space(8.0);
        self.poll_matches();
        self.poll_login();
        egui::Grid::new("online").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
            ui.label("Tu nombre:");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.online_name).hint_text("Jugador").desired_width(240.0),
            )
            .on_hover_text("Para partidas directas. En el servidor común te ven con el nombre de tu cuenta.");
            ui.end_row();

            ui.label("Servidor común:");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.server_addr)
                        .hint_text(format!("dirección:puerto (por defecto {DEFAULT_SERVER_PORT})"))
                        .desired_width(240.0),
                );
                let typed = !self.settings.server_addr.trim().is_empty();
                if ui.add_enabled(typed, egui::Button::new("Ver partidas")).clicked() {
                    self.matches = Some(Ok(vec![]));
                    self.fetch_matches();
                }
                if ui.add_enabled(typed, egui::Button::new("Ranking")).clicked() {
                    self.fetch_board(None);
                }
                if ui
                    .add_enabled(typed && self.session().is_some(), egui::Button::new("Publicar partida"))
                    .on_hover_text(
                        "Crea una partida con las reglas elegidas arriba y la pone en la lista del servidor.",
                    )
                    .on_disabled_hover_text("Entra con tu cuenta del servidor para publicar una partida.")
                    .clicked()
                {
                    self.host(true);
                }
                let addr = self.server_addr();
                if self.pin_changed.as_ref() == Some(&addr)
                    && ui
                        .button("Confiar en el certificado nuevo")
                        .on_hover_text("Hazlo solo si el administrador del servidor te dijo que lo cambió.")
                        .clicked()
                {
                    self.settings.server_pins.remove(&addr);
                    self.pins.remove(&addr);
                    self.pin_changed = None;
                    self.error = None;
                }
            });
            ui.end_row();

            ui.label("Tu cuenta:");
            self.account_row(ui);
            ui.end_row();

            ui.label("Hospedar aquí:");
            ui.horizontal(|ui| {
                let mut port = self.settings.online_port.unwrap_or(DEFAULT_PORT);
                ui.label("puerto");
                if ui.add(egui::DragValue::new(&mut port).range(1024..=65535)).changed() {
                    self.settings.online_port = Some(port);
                }
                if ui
                    .button("Hospedar partida")
                    .on_hover_text("Con las reglas elegidas arriba. Los demás se conectan a tu IP y este puerto.")
                    .clicked()
                {
                    self.host(false);
                }
            });
            ui.end_row();

            ui.label("Unirse directo:");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.online_addr)
                        .hint_text(format!("IP:puerto (por defecto {DEFAULT_PORT})"))
                        .desired_width(240.0),
                );
                let typed = !self.settings.online_addr.trim().is_empty();
                if ui.add_enabled(typed, egui::Button::new("Jugar")).clicked() {
                    self.join(None, false);
                }
                if ui.add_enabled(typed, egui::Button::new("Mirar")).clicked() {
                    self.join(None, true);
                }
            });
            ui.end_row();
        });
        self.friends_ui(ui);
        self.match_list(ui);
        self.board_ui(ui);
    }

    /// Who you are on the common server, or the form to log in.
    fn account_row(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(session) = self.session() {
                let name = session.name.clone();
                ui.label(format!("Conectado como {name}"));
                if ui.button("Mi historial").clicked() {
                    self.fetch_board(Some(name));
                }
                if ui.button("Cerrar sesión").clicked() {
                    self.log_out();
                }
                return;
            }
            let typed = !self.settings.server_addr.trim().is_empty();
            if self.login_name.is_empty() {
                self.login_name = self.settings.online_name.trim().to_string();
            }
            ui.add(egui::TextEdit::singleline(&mut self.login_name).hint_text("nombre").desired_width(140.0));
            let password = ui.add(
                egui::TextEdit::singleline(&mut self.login_password)
                    .password(true)
                    .hint_text("contraseña")
                    .desired_width(140.0),
            );
            let busy = self.logging_in.is_some();
            let ready = typed && !busy && !self.login_name.trim().is_empty() && !self.login_password.is_empty();
            let enter = password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.add_enabled(ready, egui::Button::new("Entrar")).clicked() || (enter && ready) {
                self.log_in(false);
            }
            if ui
                .add_enabled(ready, egui::Button::new("Crear cuenta"))
                .on_hover_text(
                    "Reserva tu nombre en este servidor. Nombre de 3 a 20 letras o números; contraseña de 6 o más \
                     caracteres. No hay correo para recuperarla: si la olvidas, pídele al administrador que la cambie.",
                )
                .clicked()
            {
                self.log_in(true);
            }
            if busy {
                ui.spinner();
            }
        });
    }

    /// The common server's matches, with buttons to play or watch.
    fn match_list(&mut self, ui: &mut Ui) {
        let logged_in = self.session().is_some();
        let Some(result) = &self.matches else { return };
        ui.add_space(12.0);
        let mut chosen = None;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Partidas en {}", self.server_addr())).strong());
            if self.fetching.is_some() {
                ui.spinner();
            }
            if ui.button("Cerrar lista").clicked() {
                close = true;
            }
        });
        match result {
            Err(e) => {
                ui.colored_label(egui::Color32::from_rgb(240, 110, 110), e);
            }
            Ok(list) if list.is_empty() => {
                ui.label(if self.fetching.is_some() { "Buscando…" } else { "No hay partidas abiertas." });
            }
            Ok(list) => {
                egui::Grid::new("matches").striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    for m in list {
                        let lobby = m.lobby.as_ref();
                        let players: Vec<&str> = lobby
                            .map(|l| l.players.iter().flatten().map(|p| p.name.as_str()).collect())
                            .unwrap_or_default();
                        let started = lobby.is_some_and(|l| l.started);
                        let watchers = lobby.map_or(0, |l| l.spectators.len());
                        ui.label(RichText::new(&m.info.name).strong());
                        ui.label(text::mode(m.info.mode));
                        ui.label(if players.is_empty() { "sin jugadores".into() } else { players.join(" contra ") });
                        ui.label(match (started, watchers) {
                            (true, 0) => "jugando".to_string(),
                            (true, n) => format!("jugando, {n} mirando"),
                            (false, 0) => "esperando rival".to_string(),
                            (false, n) => format!("esperando rival, {n} mirando"),
                        });
                        if m.app != APP_VERSION {
                            ui.colored_label(egui::Color32::from_rgb(240, 110, 110), format!("usa OpenWWX {}", m.app));
                        } else if !logged_in {
                            ui.label("entra con tu cuenta para jugar o mirar");
                        } else {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!started && players.len() < 2, egui::Button::new("Jugar")).clicked() {
                                    chosen = Some((m.id, false));
                                }
                                if ui.button("Mirar").clicked() {
                                    chosen = Some((m.id, true));
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
            }
        }
        if close {
            self.matches = None;
            self.fetching = None;
        }
        if let Some((id, watch)) = chosen {
            self.join(Some(id), watch);
        }
    }

    fn setup_screen(&mut self, ui: &mut Ui) {
        let margin = ((ui.available_width() - 900.0) / 2.0).max(0.0);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_max_width(900.0);
                self.setup_form(ui);
            });
        });
    }

    fn setup_form(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading(RichText::new("OpenWWX").size(36.0).strong());
            ui.label("Raw Deal contra la computadora o en línea");
            ui.add_space(24.0);
        });
        egui::Grid::new("setup").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
            ui.label("Cartas:");
            ui.horizontal(|ui| {
                ui.label(&self.source);
                if ui.button("Elegir carpeta…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_title("Carpeta con tus sets (.ron)").pick_folder()
                {
                    self.load_cards(Some(dir));
                }
                if self.settings.cards_dir.is_some() && ui.button("Usar cartas de prueba").clicked() {
                    self.load_cards(None);
                }
            });
            ui.end_row();

            ui.label("");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.typed_dir)
                        .hint_text("o escribe la ruta de la carpeta")
                        .desired_width(360.0),
                );
                if ui.button("Cargar").clicked() && !self.typed_dir.trim().is_empty() {
                    let dir = PathBuf::from(self.typed_dir.trim());
                    self.load_cards(Some(dir));
                }
            });
            ui.end_row();

            if self.settings.cards_dir.is_some() {
                ui.label("Imágenes:");
                ui.horizontal(|ui| {
                    if self.importing.is_some() {
                        ui.spinner();
                        ui.label("Importando imágenes…");
                    } else if ui
                        .button("Importar imágenes de OCTGN (.o8c)…")
                        .on_hover_text("Copia las imágenes de un paquete de OCTGN a la carpeta \"art\" de tus cartas")
                        .clicked()
                        && let Some(packs) = rfd::FileDialog::new()
                            .set_title("Paquetes de imágenes de OCTGN")
                            .add_filter("Paquete de imágenes de OCTGN", &["o8c", "zip"])
                            .pick_files()
                    {
                        self.import_images(packs);
                    }
                    if let Some(n) = &self.notice {
                        ui.label(n);
                    }
                });
                ui.end_row();
            }

            for (i, who) in ["Tu Superstar:", "Rival:"].into_iter().enumerate() {
                ui.label(who);
                ui.horizontal(|ui| {
                    let current = self.settings.superstars[i]
                        .as_deref()
                        .and_then(|id| self.db.superstar_by_id(id))
                        .map(|s| self.db.superstar(s).name.clone())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt(("star", i)).width(240.0).selected_text(current).show_ui(ui, |ui| {
                        for s in self.db.superstars() {
                            if self.revolution_rules().revolution_cards_only && !is_revolution(&s.name) {
                                continue;
                            }
                            let picked = self.settings.superstars[i].as_deref() == Some(s.id.as_str());
                            if ui.selectable_label(picked, &s.name).on_hover_text(&s.text).clicked() {
                                self.settings.superstars[i] = Some(s.id.clone());
                            }
                        }
                    });
                    let deck = match &self.settings.decks[i] {
                        Some(p) => p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        None => "Automático".into(),
                    };
                    let mut picked = None;
                    egui::ComboBox::from_id_salt(("deck", i))
                        .width(200.0)
                        .selected_text(format!("Mazo: {deck}"))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.settings.decks[i].is_none(), "Automático").clicked() {
                                self.settings.decks[i] = None;
                            }
                            for name in crate::builder::saved_decks() {
                                if ui.selectable_label(name == deck, &name).clicked() {
                                    picked = crate::builder::deck_path(&name);
                                }
                            }
                            if ui.selectable_label(false, "Otro archivo…").clicked() {
                                picked = rfd::FileDialog::new().add_filter("Mazo", &["ron"]).pick_file();
                            }
                        });
                    if let Some(file) = picked {
                        self.use_deck(i, file);
                    }
                });
                ui.end_row();
            }

            ui.label("Dificultad:");
            ui.horizontal(|ui| {
                for level in Difficulty::ALL {
                    let (name, hint) = match level {
                        Difficulty::Easy => ("Fácil", "Juega cartas al azar."),
                        Difficulty::Normal => {
                            ("Normal", "Revierte siempre que puede y ataca con sus maniobras más fuertes.")
                        }
                        Difficulty::Hard => (
                            "Difícil",
                            "Prueba cada jugada contra varias suposiciones de tu mano y elige la que mejor le resulta. \
                             No ve tus cartas ocultas.",
                        ),
                    };
                    ui.radio_value(&mut self.settings.difficulty, level, name).on_hover_text(hint);
                }
            });
            ui.end_row();

            ui.label("Reglas:");
            ui.vertical(|ui| {
                ui.add_enabled(
                    !self.settings.revolution && !self.settings.virtual_rules,
                    egui::Checkbox::new(&mut self.settings.advanced, "Avanzadas (Pre-match y Backlash)"),
                );
                if ui
                    .add_enabled(
                        !self.settings.revolution,
                        egui::Checkbox::new(&mut self.settings.virtual_rules, "Formato Virtual (reglas avanzadas)"),
                    )
                    .on_hover_text("Backlash de hasta 12 Pre-match y 12 Mid-match, máximo 6 de cada en el Ring.")
                    .changed()
                    && self.settings.virtual_rules
                {
                    self.settings.revolution = false;
                }
                ui.add_enabled(
                    !self.settings.virtual_rules,
                    egui::Checkbox::new(&mut self.settings.revolution, "Formato Revolution (reglas avanzadas)"),
                )
                .on_hover_text("Elige abajo qué reglas del formato Revolution se usan.");
                if self.settings.revolution {
                    let mut rules = self.settings.revolution_rules.unwrap_or(RevolutionRules::ALL);
                    ui.indent("revolution_rules", |ui| {
                        ui.checkbox(&mut rules.revolution_cards_only, "Solo cartas y Superstars Revolution");
                        ui.checkbox(&mut rules.small_backlash, "Backlash de hasta 6 Pre-match y 6 Mid-match");
                        ui.checkbox(&mut rules.ring_limit, "Máximo 3 Pre-match y 3 Mid-match en el Ring");
                        ui.checkbox(&mut rules.hands_before_pre_match, "Robar la mano antes del Pre-match");
                        ui.checkbox(&mut rules.aggression, "Regla de agresión")
                            .on_hover_text("Si terminas el turno sin intentar una maniobra, volteas 6 cartas.");
                    });
                    self.settings.revolution_rules = Some(rules);
                }
            });
            ui.end_row();
        });
        ui.add_space(16.0);
        self.poll_import();
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(240, 110, 110), e);
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Empezar lucha").size(20.0)).clicked() {
                self.start();
            }
            ui.add_space(16.0);
            if ui.button(RichText::new("Construir mazos").size(20.0)).clicked() {
                let mut builder =
                    Builder::new(self.db.clone(), self.sets.clone(), self.mode(), self.settings.superstars[0].clone());
                // Opens the deck "Tu Superstar" plays with, when it is a saved one.
                if let Some(name) = self.settings.decks[0].as_ref().and_then(|p| p.file_stem())
                    && crate::builder::saved_decks().iter().any(|n| n.as_str() == name)
                {
                    builder.open(&name.to_string_lossy());
                }
                self.builder = Some(builder);
            }
        });
        ui.add_space(24.0);
        ui.separator();
        self.online_form(ui);
        ui.add_space(24.0);
        ui.label(
            "El texto y el arte de Raw Deal pertenecen a sus dueños, así que OpenWWX no los incluye. \
             Elige la carpeta donde tienes tus sets en formato .ron (por ejemplo, importados desde OCTGN), \
             o ponlos en una carpeta \"cards\" junto al programa.",
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.poll_social();
        if self.game.is_some() {
            let ctx = ui.ctx().clone();
            self.invite_window(&ctx);
        }
        if let Some(builder) = &mut self.builder {
            let exit = egui::CentralPanel::default().show(ui, |ui| builder.ui(ui)).inner;
            if let Exit::Menu(saved) = exit {
                self.builder = None;
                for deck in &mut self.settings.decks {
                    if deck.as_ref().is_some_and(|p| !p.exists()) {
                        *deck = None;
                    }
                }
                if let Some(file) = saved.and_then(|name| crate::builder::deck_path(&name)) {
                    self.use_deck(0, file);
                }
            }
            return;
        }
        if let Some(game) = &mut self.game {
            let leave = game.ui(ui);
            self.settings.speed = game.speed;
            match leave {
                Leave::Stay => {}
                Leave::Menu => self.game = None,
                Leave::Rematch => self.start(),
            }
            return;
        }
        egui::CentralPanel::default()
            .show(ui, |ui| egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| self.setup_screen(ui)));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A `cards` folder next to the executable, if there is one.
fn bundled_cards_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.join("cards");
    dir.is_dir().then_some(dir)
}
