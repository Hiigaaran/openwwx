//! Start screen and app state: choose the card folder, Superstars and decks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText, Ui};
use rawdeal_ai::Difficulty;
use rawdeal_core::{
    CardDb, DeckList, DeckProblem, Engine, GameSetup, Mode, RevolutionRules, is_revolution, validate_deck_with,
};
use rawdeal_net::{Connection, DEFAULT_PORT, HostCore, HostHandle, MatchInfo};
use serde::{Deserialize, Serialize};

use crate::game::{Leave, Match, Speed};
use crate::table::Table;

const SETTINGS_KEY: &str = "openwwx-settings";

/// Where a background image import leaves its result: images copied, or an error.
type ImportResult = Arc<Mutex<Option<Result<usize, String>>>>;

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
}

pub struct App {
    settings: Settings,
    db: Arc<CardDb>,
    /// Where the cards came from, for display.
    source: String,
    error: Option<String>,
    game: Option<Match>,
    /// Folder typed by hand, for systems without a folder picker.
    typed_dir: String,
    /// An image pack import running in the background, and its result.
    importing: Option<ImportResult>,
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
            source: String::new(),
            error: None,
            game: None,
            typed_dir: String::new(),
            importing: None,
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
            Some(dir) => match rawdeal_cards::load_dir(&dir).and_then(rawdeal_cards::build_db) {
                Ok(db) if !db.superstars().is_empty() => {
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
                    Err(format!("El mazo {} no es legal: {problems:?}", path.display()))
                }
            }
            None => rawdeal_cards::auto_deck_with(&self.db, &star, self.mode(), self.revolution_rules())
                .ok_or(format!("No se pudo armar un mazo para {star}.")),
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

    /// Hosts a match on this computer with the chosen rules, and sits you in
    /// it with your Superstar and deck.
    fn host(&mut self) {
        self.error = None;
        let deck = match self.deck(0) {
            Ok(deck) => deck,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let name = self.online_name();
        let info =
            MatchInfo { name: format!("Partida de {name}"), mode: self.mode(), revolution: self.revolution_rules() };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let port = self.settings.online_port.unwrap_or(DEFAULT_PORT);
        match HostHandle::start(HostCore::new(self.db.clone(), info, seed), port) {
            Ok(mut host) => {
                let conn = host.connect_local();
                let table = Table::online(self.db.clone(), conn, &name, Some(deck), Some(host));
                self.game = Some(Match::new(table, vec![], self.settings.speed));
            }
            Err(e) => self.error = Some(format!("No se pudo hospedar en el puerto {port}: {e}")),
        }
    }

    /// Joins the match at the typed address: as a player with your Superstar
    /// and deck, or to watch.
    fn join(&mut self, watch: bool) {
        self.error = None;
        let deck = if watch {
            None
        } else {
            match self.deck(0) {
                Ok(deck) => Some(deck),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        };
        let addr = self.settings.online_addr.trim().to_string();
        let addr = if addr.contains(':') { addr } else { format!("{addr}:{DEFAULT_PORT}") };
        match Connection::connect(addr.as_str()) {
            Ok(conn) => {
                let table = Table::online(self.db.clone(), conn, &self.online_name(), deck, None);
                self.game = Some(Match::new(table, vec![], self.settings.speed));
            }
            Err(e) => self.error = Some(format!("No se pudo conectar a {addr}: {e}")),
        }
    }

    fn online_form(&mut self, ui: &mut Ui) {
        ui.heading("En línea");
        ui.label(
            "Juegas con tu Superstar y tu mazo de arriba. Los dos jugadores y los espectadores necesitan los mismos \
             sets de cartas y la misma versión de OpenWWX.",
        );
        ui.add_space(8.0);
        egui::Grid::new("online").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
            ui.label("Tu nombre:");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.online_name).hint_text("Jugador").desired_width(240.0),
            );
            ui.end_row();

            ui.label("Hospedar:");
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
                    self.host();
                }
            });
            ui.end_row();

            ui.label("Unirse:");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.online_addr)
                        .hint_text(format!("IP:puerto (por defecto {DEFAULT_PORT})"))
                        .desired_width(240.0),
                );
                let typed = !self.settings.online_addr.trim().is_empty();
                if ui.add_enabled(typed, egui::Button::new("Jugar")).clicked() {
                    self.join(false);
                }
                if ui.add_enabled(typed, egui::Button::new("Mirar")).clicked() {
                    self.join(true);
                }
            });
            ui.end_row();
        });
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
                        Some(p) => format!("Mazo: {}", file_name(p)),
                        None => "Mazo: automático".into(),
                    };
                    ui.label(deck);
                    if ui.button("Cargar mazo…").clicked()
                        && let Some(file) = rfd::FileDialog::new().add_filter("Mazo", &["ron"]).pick_file()
                    {
                        match rawdeal_cards::load_deck(&file) {
                            Ok(d) => {
                                self.settings.superstars[i] = Some(d.superstar);
                                self.settings.decks[i] = Some(file);
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                    if self.settings.decks[i].is_some() && ui.button("Automático").clicked() {
                        self.settings.decks[i] = None;
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
        if ui.button(RichText::new("Empezar lucha").size(20.0)).clicked() {
            self.start();
        }
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
