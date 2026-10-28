//! The deck builder: every card with filters, the deck being built with its
//! problems under the rules of the chosen format, and decks saved in the
//! app's `mazos` folder.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, Ui};
use rawdeal_core::{
    CardDb, CardDef, CardKind, DeckList, DeckProblem, DefIdx, ManeuverType, Mode, RevolutionRules, Trait,
    is_revolution, validate_deck_with,
};

use crate::analysis::{self, AXES, Analysis};
use crate::cards::{self, Look, Preview};
use crate::text;

/// Where saved decks live, one `.ron` file each.
pub fn decks_dir() -> Option<PathBuf> {
    eframe::storage_dir("OpenWWX").map(|d| d.join("mazos"))
}

/// The names of the saved decks, sorted.
pub fn saved_decks() -> Vec<String> {
    let Some(dir) = decks_dir() else { return vec![] };
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// The file of the saved deck `name`.
pub fn deck_path(name: &str) -> Option<PathBuf> {
    let clean: String =
        name.trim().chars().map(|c| if c.is_alphanumeric() || " -_.()'".contains(c) { c } else { '_' }).collect();
    let clean = clean.trim_matches('.').trim();
    if clean.is_empty() {
        return None;
    }
    decks_dir().map(|d| d.join(format!("{clean}.ron")))
}

/// Which cards the catalog shows, by kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KindFilter {
    All,
    Maneuver(Option<ManeuverType>),
    Action,
    Reversal,
    PreMatch,
    MidMatch,
    Antic,
    Backstage,
}

impl KindFilter {
    const MENU: [KindFilter; 19] = [
        KindFilter::All,
        KindFilter::Maneuver(None),
        KindFilter::Maneuver(Some(ManeuverType::Strike)),
        KindFilter::Maneuver(Some(ManeuverType::Grapple)),
        KindFilter::Maneuver(Some(ManeuverType::Submission)),
        KindFilter::Maneuver(Some(ManeuverType::HighRisk)),
        KindFilter::Maneuver(Some(ManeuverType::TrademarkFinisher)),
        KindFilter::Maneuver(Some(ManeuverType::Throw)),
        KindFilter::Maneuver(Some(ManeuverType::Hold)),
        KindFilter::Maneuver(Some(ManeuverType::Assault)),
        KindFilter::Maneuver(Some(ManeuverType::Trademark)),
        KindFilter::Maneuver(Some(ManeuverType::Extreme)),
        KindFilter::Action,
        KindFilter::Reversal,
        KindFilter::PreMatch,
        KindFilter::MidMatch,
        KindFilter::Antic,
        KindFilter::Backstage,
        KindFilter::All,
    ];

    fn label(self) -> String {
        match self {
            KindFilter::All => "Todos los tipos".into(),
            KindFilter::Maneuver(None) => "Maniobras".into(),
            KindFilter::Maneuver(Some(t)) => format!("Maniobra {}", text::maneuver(t)),
            KindFilter::Action => "Acciones".into(),
            KindFilter::Reversal => "Reversals".into(),
            KindFilter::PreMatch => "Pre-match".into(),
            KindFilter::MidMatch => "Mid-match".into(),
            KindFilter::Antic => "Antics".into(),
            KindFilter::Backstage => "Backstage y Enforcers".into(),
        }
    }

    fn matches(self, def: &CardDef) -> bool {
        match self {
            KindFilter::All => true,
            KindFilter::Maneuver(t) => {
                def.kinds.iter().any(|k| matches!(k, CardKind::Maneuver(m) if t.is_none_or(|t| *m == t)))
            }
            KindFilter::Action => def.kinds.contains(&CardKind::Action),
            KindFilter::Reversal => def.kinds.contains(&CardKind::Reversal),
            KindFilter::PreMatch => def.is_pre_match(),
            KindFilter::MidMatch => def.has_trait(Trait::MidMatch),
            KindFilter::Antic => def.kinds.contains(&CardKind::Antic),
            KindFilter::Backstage => def.is_backstage(),
        }
    }
}

/// Where a card goes in a deck.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Arsenal,
    Backlash,
    Backstage,
}

fn section(def: &CardDef) -> Section {
    if def.is_backstage() {
        Section::Backstage
    } else if def.is_backlash() {
        Section::Backlash
    } else {
        Section::Arsenal
    }
}

/// The formats a deck can be built for.
const FORMATS: [(Mode, &str); 4] = [
    (Mode::QuickStart, "Inicio rápido"),
    (Mode::Advanced, "Avanzadas"),
    (Mode::Virtual, "Virtual"),
    (Mode::Revolution, "Revolution"),
];

/// What the builder asks of the app after a frame.
pub enum Exit {
    Stay,
    /// Back to the menu; `Some` names the deck saved last, to play with.
    Menu(Option<String>),
}

pub struct Builder {
    db: Arc<CardDb>,
    /// Card sets by id prefix, with their names, in load order.
    sets: Vec<(String, String)>,
    name: String,
    deck: DeckList,
    mode: Mode,
    problems: Vec<DeckProblem>,
    saved: Vec<String>,
    /// The deck saved last, if the deck has not changed since.
    saved_as: Option<String>,
    // Catalog filters.
    search: String,
    set: Option<String>,
    kind: KindFilter,
    only_packable: bool,
    fortitude: (u8, u8),
    damage: (u8, u8),
    /// The highest Fortitude and Damage in the collection.
    top: (u8, u8),
    /// The catalog as filtered, and the filters it was filtered with.
    shown: Vec<DefIdx>,
    shown_for: Option<String>,
    message: Option<(String, bool)>,
    /// Asked to delete this saved deck; a second click does it.
    deleting: Option<String>,
    tab: Tab,
    analysis: Analysis,
    /// The shuffled Arsenal of the test hand, and how many cards are drawn.
    test_order: Vec<String>,
    drawn: usize,
}

/// What the deck panel shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cards,
    Analysis,
    Hands,
}

impl Builder {
    pub fn new(db: Arc<CardDb>, sets: Vec<(String, String)>, mode: Mode, superstar: Option<String>) -> Builder {
        let superstar = superstar
            .filter(|s| db.superstar_by_id(s).is_some())
            .or_else(|| db.superstars().first().map(|s| s.id.clone()))
            .unwrap_or_default();
        let deck = DeckList { superstar, arsenal: vec![], backlash: vec![], backstage: vec![] };
        let top = db.cards().iter().fold((0, 0), |(f, d), c| (f.max(c.fortitude), d.max(c.damage)));
        let mut b = Builder {
            db,
            sets,
            name: String::new(),
            deck,
            mode,
            problems: vec![],
            saved: saved_decks(),
            saved_as: None,
            search: String::new(),
            set: None,
            kind: KindFilter::All,
            only_packable: true,
            fortitude: (0, top.0),
            damage: (0, top.1),
            top,
            shown: vec![],
            shown_for: None,
            message: None,
            tab: Tab::Cards,
            analysis: Analysis::default(),
            test_order: vec![],
            drawn: 0,
            deleting: None,
        };
        b.check();
        b
    }

    /// Opens the saved deck `name`.
    pub fn open(&mut self, name: &str) {
        let loaded = deck_path(name)
            .ok_or("nombre inválido".to_string())
            .and_then(|p| rawdeal_cards::load_deck(&p).map_err(|e| e.to_string()));
        match loaded {
            Ok(deck) => {
                self.deck = deck;
                self.name = name.to_string();
                self.saved_as = Some(name.to_string());
                self.message = None;
                self.check();
            }
            Err(e) => self.message = Some((format!("No se pudo abrir {name}: {e}"), true)),
        }
    }

    fn rules(&self) -> RevolutionRules {
        self.mode.revolution_rules()
    }

    fn changed(&mut self) {
        self.saved_as = None;
        self.check();
    }

    /// Checks the deck under the format's rules.
    fn check(&mut self) {
        let mut problems = validate_deck_with(&self.db, &self.deck, self.mode, self.rules());
        if !self.mode.is_advanced() {
            problems.retain(|p| !matches!(p, DeckProblem::NotABacklashCard { .. }));
        }
        self.problems = problems;
        self.analysis = analysis::analyze(&self.db, &self.deck);
        self.test_order.clear();
    }

    fn hand_size(&self) -> usize {
        self.db.superstar_by_id(&self.deck.superstar).map_or(0, |s| usize::from(self.db.superstar(s).hand_size))
    }

    /// Shuffles the Arsenal and draws the Superstar's starting hand.
    fn deal(&mut self) {
        use rand::SeedableRng;
        use rand::seq::SliceRandom;
        let seed =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        self.test_order = self.deck.arsenal.clone();
        self.test_order.shuffle(&mut rand_chacha::ChaCha8Rng::seed_from_u64(seed));
        self.drawn = self.hand_size().min(self.test_order.len());
    }

    fn title(&self, id: &str) -> String {
        text::card_title(&self.db, id)
    }

    fn superstar_name(&self) -> String {
        self.db
            .superstar_by_id(&self.deck.superstar)
            .map(|i| self.db.superstar(i).name.clone())
            .unwrap_or_else(|| self.deck.superstar.clone())
    }

    /// Could the current Superstar put this card in a deck of this format?
    fn packable(&self, def: &CardDef) -> bool {
        let mut one = DeckList { superstar: self.deck.superstar.clone(), ..Default::default() };
        match section(def) {
            Section::Arsenal => one.arsenal.push(def.id.clone()),
            Section::Backlash if !self.mode.is_advanced() => return false,
            Section::Backlash => one.backlash.push(def.id.clone()),
            Section::Backstage => one.backstage.push(def.id.clone()),
        }
        validate_deck_with(&self.db, &one, self.mode, self.rules()).iter().all(|p| match p {
            DeckProblem::WrongSize(_) => true,
            // About the Superstar, not the card.
            DeckProblem::NotRevolution { card } => *card == one.superstar,
            _ => false,
        })
    }

    fn list(&mut self, s: Section) -> &mut Vec<String> {
        match s {
            Section::Arsenal => &mut self.deck.arsenal,
            Section::Backlash => &mut self.deck.backlash,
            Section::Backstage => &mut self.deck.backstage,
        }
    }

    fn copies(&self, id: &str) -> usize {
        let d = &self.deck;
        d.arsenal.iter().chain(&d.backlash).chain(&d.backstage).filter(|c| *c == id).count()
    }

    fn add(&mut self, id: &str) {
        let Some(i) = self.db.card_by_id(id) else { return };
        let s = section(self.db.card(i));
        self.list(s).push(id.to_string());
        self.changed();
    }

    fn remove(&mut self, id: &str) {
        for s in [Section::Arsenal, Section::Backlash, Section::Backstage] {
            let list = self.list(s);
            if let Some(pos) = list.iter().rposition(|c| c == id) {
                list.remove(pos);
                break;
            }
        }
        self.changed();
    }

    /// Fills the Arsenal up to 60 with the automatic deck's cards, keeping
    /// what is there and adding only cards that keep the deck legal.
    fn complete(&mut self) {
        let Some(auto) = rawdeal_cards::auto_deck_with(&self.db, &self.deck.superstar, self.mode, self.rules()) else {
            self.message = Some(("No se pudo armar un mazo automático para esta Superstar.".into(), true));
            return;
        };
        let before = self.problems.len();
        for id in auto.arsenal {
            if self.deck.arsenal.len() >= rawdeal_core::ARSENAL_SIZE {
                break;
            }
            self.deck.arsenal.push(id);
            let mut problems = validate_deck_with(&self.db, &self.deck, self.mode, self.rules());
            problems.retain(|p| !matches!(p, DeckProblem::WrongSize(_) | DeckProblem::NotABacklashCard { .. }));
            if problems.len() > before {
                self.deck.arsenal.pop();
            }
        }
        self.changed();
    }

    fn save(&mut self) {
        let name = self.name.trim().to_string();
        let Some(path) = deck_path(&name) else {
            self.message = Some(("Ponle un nombre al mazo para guardarlo.".into(), true));
            return;
        };
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|e| e.to_string())
            .and_then(|_| rawdeal_cards::save_deck(&path, &self.deck).map_err(|e| e.to_string()));
        match saved {
            Ok(()) => {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
                self.name = stem.clone();
                self.saved_as = Some(stem.clone());
                self.saved = saved_decks();
                let legal = if self.problems.is_empty() { "" } else { " Todavía no es legal para jugar." };
                self.message = Some((format!("Mazo «{stem}» guardado.{legal}"), false));
            }
            Err(e) => self.message = Some((format!("No se pudo guardar: {e}"), true)),
        }
    }

    fn delete(&mut self, name: &str) {
        if let Some(path) = deck_path(name) {
            match std::fs::remove_file(&path) {
                Ok(()) => self.message = Some((format!("Mazo «{name}» borrado."), false)),
                Err(e) => self.message = Some((format!("No se pudo borrar {name}: {e}"), true)),
            }
        }
        if self.saved_as.as_deref() == Some(name) {
            self.saved_as = None;
        }
        self.saved = saved_decks();
    }

    /// Filters the catalog again if a filter changed.
    fn refilter(&mut self) {
        let key = format!(
            "{}|{:?}|{:?}|{}|{:?}|{:?}|{}|{:?}",
            self.search,
            self.set,
            self.kind,
            self.only_packable,
            self.fortitude,
            self.damage,
            self.deck.superstar,
            self.mode
        );
        if self.shown_for.as_ref() == Some(&key) {
            return;
        }
        let words: Vec<String> = self.search.to_lowercase().split_whitespace().map(String::from).collect();
        let shown = self
            .db
            .cards()
            .iter()
            .enumerate()
            .filter(|(_, def)| {
                self.set.as_ref().is_none_or(|s| def.id.split('/').next() == Some(s.as_str()))
                    && self.kind.matches(def)
                    && (self.fortitude.0..=self.fortitude.1).contains(&def.fortitude)
                    && (self.damage.0..=self.damage.1).contains(&def.damage)
                    && words.iter().all(|w| {
                        def.title.to_lowercase().contains(w.as_str()) || def.text.to_lowercase().contains(w.as_str())
                    })
                    && (!self.only_packable || self.packable(def))
            })
            .map(|(i, _)| DefIdx(i as u16))
            .collect();
        self.shown = shown;
        self.shown_for = Some(key);
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Exit {
        let mut exit = Exit::Stay;
        egui::Panel::top("builder-top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Volver al menú").clicked() {
                    exit = Exit::Menu(self.saved_as.clone());
                }
                ui.separator();
                ui.label("Mazo:");
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("nombre").desired_width(180.0));
                if ui.button("Guardar").clicked() {
                    self.save();
                }
                let mut open = None;
                egui::ComboBox::from_id_salt("builder-open").selected_text("Abrir…").width(160.0).show_ui(ui, |ui| {
                    if self.saved.is_empty() {
                        ui.label(RichText::new("No hay mazos guardados.").weak());
                    }
                    for name in &self.saved {
                        if ui.selectable_label(false, name).clicked() {
                            open = Some(name.clone());
                        }
                    }
                });
                if let Some(name) = open {
                    self.open(&name);
                }
                if ui.button("Nuevo").clicked() {
                    self.deck = DeckList { superstar: self.deck.superstar.clone(), ..Default::default() };
                    self.name.clear();
                    self.message = None;
                    self.changed();
                }
                if let Some(name) = self.saved_as.clone() {
                    let asking = self.deleting.as_deref() == Some(name.as_str());
                    let label = if asking { "¿Borrar de verdad?" } else { "Borrar" };
                    if ui.button(label).clicked() {
                        if asking {
                            self.deleting = None;
                            self.delete(&name);
                        } else {
                            self.deleting = Some(name);
                        }
                    }
                }
                ui.separator();
                ui.label("Superstar:");
                let mut star = None;
                egui::ComboBox::from_id_salt("builder-star").width(200.0).selected_text(self.superstar_name()).show_ui(
                    ui,
                    |ui| {
                        for s in self.db.superstars() {
                            if self.rules().revolution_cards_only && !is_revolution(&s.name) {
                                continue;
                            }
                            let picked = s.id == self.deck.superstar;
                            if ui.selectable_label(picked, &s.name).on_hover_text(&s.text).clicked() {
                                star = Some(s.id.clone());
                            }
                        }
                    },
                );
                if let Some(star) = star {
                    self.deck.superstar = star;
                    self.changed();
                }
                ui.separator();
                ui.label("Formato:");
                let current = FORMATS.iter().find(|(m, _)| *m == self.mode).map_or("", |(_, n)| n);
                let mut picked = None;
                egui::ComboBox::from_id_salt("builder-format").width(130.0).selected_text(current).show_ui(ui, |ui| {
                    for (mode, name) in FORMATS {
                        if ui.selectable_label(self.mode == mode, name).clicked() {
                            picked = Some(mode);
                        }
                    }
                });
                if let Some(mode) = picked.filter(|m| *m != self.mode) {
                    self.mode = mode;
                    self.check();
                }
            });
            if let Some((msg, bad)) = &self.message {
                let color = if *bad { Color32::from_rgb(240, 110, 110) } else { Color32::from_rgb(120, 200, 120) };
                ui.colored_label(color, msg);
            }
            ui.add_space(4.0);
        });
        egui::Panel::right("builder-preview").resizable(false).exact_size(260.0).show(ui, cards::preview_panel);
        egui::Panel::right("builder-deck").resizable(true).default_size(380.0).show(ui, |ui| self.deck_ui(ui));
        egui::CentralPanel::default().show(ui, |ui| self.catalog_ui(ui));
        exit
    }

    fn catalog_ui(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.heading("Cartas");
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search).hint_text("buscar en título o texto").desired_width(200.0),
            );
            let set_name = match &self.set {
                None => "Todos los sets".to_string(),
                Some(p) => self.sets.iter().find(|(id, _)| id == p).map_or(p.clone(), |(_, n)| n.clone()),
            };
            egui::ComboBox::from_id_salt("builder-set").width(200.0).selected_text(set_name).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.set, None, "Todos los sets");
                for (id, name) in &self.sets {
                    ui.selectable_value(&mut self.set, Some(id.clone()), name);
                }
            });
            egui::ComboBox::from_id_salt("builder-kind").width(170.0).selected_text(self.kind.label()).show_ui(
                ui,
                |ui| {
                    for k in &KindFilter::MENU[..KindFilter::MENU.len() - 1] {
                        ui.selectable_value(&mut self.kind, *k, k.label());
                    }
                },
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Fortitude");
            ui.add(egui::DragValue::new(&mut self.fortitude.0).range(0..=self.fortitude.1));
            ui.label("a");
            ui.add(egui::DragValue::new(&mut self.fortitude.1).range(self.fortitude.0..=self.top.0));
            ui.separator();
            ui.label("Daño");
            ui.add(egui::DragValue::new(&mut self.damage.0).range(0..=self.damage.1));
            ui.label("a");
            ui.add(egui::DragValue::new(&mut self.damage.1).range(self.damage.0..=self.top.1));
            ui.separator();
            ui.checkbox(&mut self.only_packable, "Solo las que puede llevar esta Superstar").on_hover_text(
                "Oculta las cartas de otras Superstars, las prohibidas y las que el formato no permite.",
            );
            if ui.button("Quitar filtros").clicked() {
                self.search.clear();
                self.set = None;
                self.kind = KindFilter::All;
                self.fortitude = (0, self.top.0);
                self.damage = (0, self.top.1);
            }
        });
        self.refilter();
        ui.label(
            RichText::new(format!("{} cartas. Clic para agregar al mazo; clic derecho para quitar.", self.shown.len()))
                .weak(),
        );
        ui.separator();
        let row = 22.0;
        let shown = self.shown.clone();
        let mut add = None;
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("catalog").auto_shrink([false; 2]).show_rows(
            ui,
            row,
            shown.len(),
            |ui, range| {
                for &idx in &shown[range] {
                    let def = self.db.card(idx);
                    let copies = self.copies(&def.id);
                    let row_rect = ui
                        .horizontal(|ui| {
                            ui.set_min_height(row);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 16.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, kind_color(def));
                            let title = if copies > 0 {
                                RichText::new(format!("{}  ×{copies}", def.title)).strong()
                            } else {
                                RichText::new(&def.title)
                            };
                            cell(ui, 230.0, title);
                            cell(ui, 160.0, RichText::new(kinds(def)).weak());
                            cell(ui, 60.0, RichText::new(format!("F{} D{}", def.fortitude, def.damage)));
                            ui.add(egui::Label::new(RichText::new(self.set_name(&def.id)).weak()).truncate());
                        })
                        .response
                        .rect;
                    let response = ui.interact(row_rect, ui.id().with(("card", idx.0)), egui::Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(
                            row_rect,
                            2.0,
                            ui.visuals().widgets.hovered.weak_bg_fill.gamma_multiply(0.4),
                        );
                    }
                    if response.hovered() {
                        cards::set_preview(ui.ctx(), Preview::Card(Box::new(def.clone())));
                    }
                    if response.clicked() {
                        add = Some(def.id.clone());
                    }
                    if response.secondary_clicked() && copies > 0 {
                        remove = Some(def.id.clone());
                    }
                }
            },
        );
        if let Some(id) = add {
            self.add(&id);
        }
        if let Some(id) = remove {
            self.remove(&id);
        }
    }

    fn set_name(&self, id: &str) -> String {
        let prefix = id.split('/').next().unwrap_or(id);
        self.sets.iter().find(|(p, _)| p == prefix).map_or(prefix.to_string(), |(_, n)| n.clone())
    }

    fn deck_ui(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        let size = self.deck.arsenal.len();
        ui.horizontal(|ui| {
            ui.heading(format!("Arsenal {size}/{}", rawdeal_core::ARSENAL_SIZE));
            if size < rawdeal_core::ARSENAL_SIZE
                && ui.button("Completar").on_hover_text("Llena lo que falta con cartas del mazo automático.").clicked()
            {
                self.complete();
            }
            if size > 0 && ui.button("Vaciar").clicked() {
                self.deck.arsenal.clear();
                self.deck.backlash.clear();
                self.deck.backstage.clear();
                self.changed();
            }
        });
        if self.problems.is_empty() {
            ui.colored_label(Color32::from_rgb(120, 200, 120), "Mazo legal para este formato.");
        } else {
            egui::ScrollArea::vertical().id_salt("problems").max_height(110.0).show(ui, |ui| {
                for p in &self.problems {
                    ui.colored_label(Color32::from_rgb(240, 110, 110), text::deck_problem(p, |id| self.title(id)));
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Cards, "Cartas");
            ui.selectable_value(&mut self.tab, Tab::Analysis, "Análisis");
            ui.selectable_value(&mut self.tab, Tab::Hands, "Manos de prueba");
        });
        ui.separator();
        match self.tab {
            Tab::Cards => self.cards_ui(ui),
            Tab::Analysis => {
                egui::ScrollArea::vertical()
                    .id_salt("analysis")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| self.analysis_ui(ui));
            }
            Tab::Hands => {
                egui::ScrollArea::vertical().id_salt("hands").auto_shrink([false; 2]).show(ui, |ui| self.hands_ui(ui));
            }
        }
    }

    /// The deck's cards by group, with − and + buttons.
    fn cards_ui(&mut self, ui: &mut Ui) {
        let mut add = None;
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("deck").auto_shrink([false; 2]).show(ui, |ui| {
            let mut groups: Vec<(&str, Vec<(String, usize)>)> = vec![];
            let mut push = |group: &'static str, id: &String| match groups.iter_mut().find(|(g, _)| *g == group) {
                Some((_, cards)) => match cards.iter_mut().find(|(c, _)| c == id) {
                    Some((_, n)) => *n += 1,
                    None => cards.push((id.clone(), 1)),
                },
                None => groups.push((group, vec![(id.clone(), 1)])),
            };
            for id in &self.deck.arsenal {
                let group = self.db.card_by_id(id).map_or("Otras", |i| group(self.db.card(i)));
                push(group, id);
            }
            for id in &self.deck.backlash {
                push("Backlash", id);
            }
            for id in &self.deck.backstage {
                push("Backstage", id);
            }
            groups.sort_by_key(|(g, _)| GROUPS.iter().position(|o| o == g).unwrap_or(GROUPS.len()));
            if groups.is_empty() {
                ui.label(RichText::new("El mazo está vacío. Agrega cartas desde la lista de la izquierda.").weak());
            }
            for (group, mut list) in groups {
                let total: usize = list.iter().map(|(_, n)| n).sum();
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{group} ({total})")).strong());
                list.sort_by_key(|(id, _)| self.title(id).to_lowercase());
                for (id, n) in list {
                    let Some(def) = self.db.card_by_id(&id).map(|i| self.db.card(i)) else { continue };
                    ui.horizontal(|ui| {
                        if ui.small_button("−").clicked() {
                            remove = Some(id.clone());
                        }
                        if ui.small_button("+").clicked() {
                            add = Some(id.clone());
                        }
                        let r = ui.add(
                            egui::Label::new(format!("{n}× {}  F{} D{}", def.title, def.fortitude, def.damage))
                                .truncate()
                                .sense(egui::Sense::hover()),
                        );
                        if r.hovered() {
                            cards::set_preview(ui.ctx(), Preview::Card(Box::new(def.clone())));
                        }
                    });
                }
            }
        });
        if let Some(id) = add {
            self.add(&id);
        }
        if let Some(id) = remove {
            self.remove(&id);
        }
    }
}

impl Builder {
    fn analysis_ui(&mut self, ui: &mut Ui) {
        if self.deck.arsenal.is_empty() {
            ui.label(RichText::new("Agrega cartas al Arsenal para ver su análisis.").weak());
            return;
        }
        let a = &self.analysis;
        ui.label(RichText::new("Foco del mazo").strong());
        hexagon(ui, &a.focus);
        for ((name, value), why) in AXES.iter().zip(a.focus).zip(&a.why) {
            let mut job = egui::text::LayoutJob::default();
            let style = ui.style().clone();
            RichText::new(format!("{name} {value:.1}  ")).strong().append_to(
                &mut job,
                &style,
                egui::FontSelection::Default,
                egui::Align::Center,
            );
            RichText::new(why).weak().append_to(&mut job, &style, egui::FontSelection::Default, egui::Align::Center);
            ui.label(job);
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Curva de Fortitude").strong());
        curve(ui, &a.curve);
        ui.add_space(10.0);
        let st = &a.stats;
        ui.label(
            RichText::new(format!("En {} manos iniciales de {} cartas", thousands(analysis::HANDS), st.hand_size))
                .strong(),
        );
        ui.label(format!("Cartas jugables con 0 de Fortitude: {:.1} en promedio.", st.playable));
        ui.label(format!(
            "Con al menos una maniobra jugable en el primer turno: {:.0}% (daño de la mejor: {:.1}).",
            st.with_opener * 100.0,
            st.opener_damage
        ));
        ui.label(format!("Con al menos un Reversal: {:.0}%.", st.with_reversal * 100.0));
    }

    fn hands_ui(&mut self, ui: &mut Ui) {
        if self.deck.arsenal.is_empty() {
            ui.label(RichText::new("Agrega cartas al Arsenal para repartir manos de prueba.").weak());
            return;
        }
        if self.test_order.is_empty() {
            self.deal();
        }
        ui.horizontal(|ui| {
            if ui.button("Otra mano").clicked() {
                self.deal();
            }
            if ui.add_enabled(self.drawn < self.test_order.len(), egui::Button::new("Robar")).clicked() {
                self.drawn += 1;
            }
            ui.label(
                RichText::new(format!(
                    "Mano inicial de {}, {} en el Arsenal",
                    self.hand_size(),
                    self.test_order.len() - self.drawn
                ))
                .weak(),
            );
        });
        let hand: Vec<&CardDef> = self.test_order[..self.drawn]
            .iter()
            .filter_map(|id| self.db.card_by_id(id))
            .map(|i| self.db.card(i))
            .collect();
        let playable = hand.iter().filter(|c| analysis::playable_first(c)).count();
        let reversals = hand.iter().filter(|c| c.is_reversal()).count();
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        ui.label(format!(
            "{} con 0 de Fortitude, {}.",
            plural(playable, "jugable", "jugables"),
            plural(reversals, "Reversal", "Reversals")
        ));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for def in hand {
                let look = if analysis::playable_first(def) { Look::Playable } else { Look::Normal };
                cards::card(ui, Some(def), egui::vec2(100.0, 140.0), look);
            }
        });
    }
}

/// `n` with a dot every three digits, as written in Spanish.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// The six axes of `focus` (0 to 10) as a hexagon.
fn hexagon(ui: &mut Ui, focus: &[f32; 6]) {
    let size = ui.available_width().min(300.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size * 0.8), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = rect.height() * 0.36;
    let point = |i: usize, v: f32| {
        let angle = std::f32::consts::TAU * i as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
        center + egui::vec2(angle.cos(), angle.sin()) * radius * v / 10.0
    };
    let grid = ui.visuals().widgets.noninteractive.bg_stroke;
    for ring in [2.5, 5.0, 7.5, 10.0] {
        let ring: Vec<egui::Pos2> = (0..6).map(|i| point(i, ring)).collect();
        painter.add(egui::Shape::closed_line(ring, grid));
    }
    for i in 0..6 {
        painter.line_segment([center, point(i, 10.0)], grid);
    }
    let fill = Color32::from_rgba_unmultiplied(230, 170, 60, 90);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, fill);
    for (i, &v) in focus.iter().enumerate() {
        mesh.colored_vertex(point(i, v), fill);
    }
    for i in 0..6u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 6);
    }
    painter.add(mesh);
    let outline: Vec<egui::Pos2> = (0..6).map(|i| point(i, focus[i])).collect();
    painter.add(egui::Shape::closed_line(outline, egui::Stroke::new(2.0, Color32::from_rgb(240, 180, 70))));
    let text = ui.visuals().text_color();
    for (i, name) in AXES.iter().enumerate() {
        let at = point(i, 12.6);
        painter.text(
            at,
            egui::Align2::CENTER_CENTER,
            format!("{name} {:.1}", focus[i]),
            egui::FontId::proportional(13.0),
            text,
        );
    }
}

/// Arsenal cards by Fortitude, as bars.
fn curve(ui: &mut Ui, counts: &[usize; 14]) {
    let width = ui.available_width().min(340.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 110.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let max = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    let slot = rect.width() / counts.len() as f32;
    let text = ui.visuals().text_color();
    let font = egui::FontId::proportional(11.0);
    for (i, &n) in counts.iter().enumerate() {
        let x = rect.left() + slot * i as f32;
        let bottom = rect.bottom() - 16.0;
        let height = (rect.height() - 34.0) * n as f32 / max;
        let bar = egui::Rect::from_min_max(egui::pos2(x + 3.0, bottom - height), egui::pos2(x + slot - 3.0, bottom));
        painter.rect_filled(bar, 2.0, Color32::from_rgb(196, 152, 52));
        let label = if i == counts.len() - 1 { format!("{i}+") } else { i.to_string() };
        painter.text(
            egui::pos2(x + slot / 2.0, rect.bottom() - 7.0),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            text,
        );
        if n > 0 {
            painter.text(
                egui::pos2(x + slot / 2.0, bar.top() - 7.0),
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                font.clone(),
                text,
            );
        }
    }
}

/// Deck groups, in the order shown.
const GROUPS: [&str; 7] = ["Maniobras", "Acciones", "Reversals", "Antics", "Otras", "Backlash", "Backstage"];

fn group(def: &CardDef) -> &'static str {
    match def.kinds.first() {
        Some(CardKind::Maneuver(_)) => "Maniobras",
        Some(CardKind::Action) => "Acciones",
        Some(CardKind::Reversal) => "Reversals",
        Some(CardKind::Antic) => "Antics",
        _ => "Otras",
    }
}

fn kinds(def: &CardDef) -> String {
    def.kinds.iter().map(|&k| text::kind(k)).collect::<Vec<_>>().join(" / ")
}

fn kind_color(def: &CardDef) -> Color32 {
    match def.kinds.first() {
        Some(CardKind::Maneuver(_)) => Color32::from_rgb(196, 152, 52),
        Some(CardKind::Action) => Color32::from_rgb(62, 104, 172),
        Some(CardKind::Reversal) => Color32::from_rgb(166, 56, 56),
        Some(CardKind::PreMatch(_)) => Color32::from_rgb(120, 72, 150),
        Some(CardKind::Antic) => Color32::from_rgb(70, 150, 70),
        _ => Color32::from_rgb(60, 140, 110),
    }
}

/// The set prefix of each set loaded, with its name.
pub fn set_names(sets: &[rawdeal_cards::CardSet]) -> Vec<(String, String)> {
    sets.iter()
        .filter_map(|s| {
            let id = s.cards.first().map(|c| &c.id).or_else(|| s.superstars.first().map(|x| &x.id))?;
            Some((id.split('/').next()?.to_string(), s.name.clone()))
        })
        .collect()
}

/// A left-aligned catalog column `width` wide.
fn cell(ui: &mut Ui, width: f32, text: RichText) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(text).truncate());
    });
}
