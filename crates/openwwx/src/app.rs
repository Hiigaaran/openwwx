//! Start screen and app state: choose the card folder, Superstars and decks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, RichText, Ui};
use rawdeal_core::{CardDb, DeckList, DeckProblem, Engine, GameSetup, validate_deck};
use serde::{Deserialize, Serialize};

use crate::game::{Leave, Match, Speed};

const SETTINGS_KEY: &str = "openwwx-settings";

/// Remembered between runs.
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    cards_dir: Option<PathBuf>,
    superstars: [Option<String>; 2],
    decks: [Option<PathBuf>; 2],
    advanced: bool,
    #[serde(default)]
    speed: Speed,
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

    fn use_test_set(&mut self) {
        self.db = Arc::new(rawdeal_cards::test_db());
        self.source = "cartas de prueba (inventadas)".into();
        self.settings.cards_dir = None;
    }

    fn deck(&self, i: usize) -> Result<DeckList, String> {
        let star = self.settings.superstars[i].clone().ok_or("Elige una Superstar.")?;
        match &self.settings.decks[i] {
            Some(path) => {
                let deck = rawdeal_cards::load_deck(path).map_err(|e| e.to_string())?;
                let mut problems = validate_deck(&self.db, &deck);
                if !self.settings.advanced {
                    problems.retain(|p| !matches!(p, DeckProblem::NotABacklashCard { .. }));
                }
                if problems.is_empty() {
                    Ok(deck)
                } else {
                    Err(format!("El mazo {} no es legal: {problems:?}", path.display()))
                }
            }
            None => rawdeal_cards::auto_deck(&self.db, &star).ok_or(format!("No se pudo armar un mazo para {star}.")),
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
        let setup = if self.settings.advanced { GameSetup::advanced(decks) } else { GameSetup::standard(decks) };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        match Engine::new(self.db.clone()).new_game(&setup, seed) {
            Ok((state, events)) => {
                self.game = Some(Match::new(self.db.clone(), state, events, seed, self.settings.speed))
            }
            Err(e) => self.error = Some(e.to_string()),
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
            ui.label("Raw Deal contra la computadora");
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

            ui.label("Reglas:");
            ui.checkbox(&mut self.settings.advanced, "Avanzadas (Pre-match y Backlash)");
            ui.end_row();
        });
        ui.add_space(16.0);
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(240, 110, 110), e);
            ui.add_space(8.0);
        }
        if ui.button(RichText::new("Empezar lucha").size(20.0)).clicked() {
            self.start();
        }
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
        egui::CentralPanel::default().show(ui, |ui| self.setup_screen(ui));
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
