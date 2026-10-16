//! The match screen: you against the computer.

use std::collections::VecDeque;
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Pos2, Rect, RichText, ScrollArea, Stroke,
    StrokeKind, Ui, vec2,
};
use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::{
    Action, CardDb, CardDef, CardId, Decision, DecisionKind, Engine, Event, GameState, PlayerId, PlayerView, Zone,
};

use crate::anim::{Banner, Display, Float, Fly, Layout, Step, ease};
use crate::cards::{self, Look};
use crate::text;

/// Pause after the animations before the computer moves, in seconds.
const BOT_DELAY: f64 = 0.3;

pub enum Leave {
    Stay,
    /// Back to the start screen.
    Menu,
    /// Same Superstars and decks, new shuffle.
    Rematch,
}

/// How fast the board replays what happened.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Speed {
    Slow,
    #[default]
    Normal,
    Fast,
    Off,
}

impl Speed {
    fn factor(self) -> f64 {
        match self {
            Speed::Slow => 1.7,
            Speed::Normal => 1.0,
            Speed::Fast => 0.45,
            Speed::Off => 0.0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Speed::Slow => "Lentas",
            Speed::Normal => "Normales",
            Speed::Fast => "Rápidas",
            Speed::Off => "Sin animaciones",
        }
    }
}

pub struct Match {
    engine: Engine,
    state: GameState,
    bot: RandomBot,
    me: PlayerId,
    log: Vec<String>,
    /// Cards picked so far in a "choose cards" decision.
    selected: Vec<CardId>,
    /// Hybrid card waiting for you to pick how to play it.
    kind_menu: Option<CardId>,
    /// Whose Ringside pile is open in a window.
    ringside: Option<PlayerId>,
    pub speed: Speed,
    /// What the board shows, replayed event by event.
    display: Display,
    layout: Layout,
    /// Events not replayed yet.
    queue: VecDeque<Event>,
    step: Option<Step>,
    banners: Vec<Banner>,
    floats: Vec<Float>,
    /// Red flashes over a player's area when they take damage.
    flashes: Vec<(PlayerId, f64)>,
    /// Replay everything left at once.
    skip: bool,
    idle_since: f64,
}

impl Match {
    pub fn new(db: Arc<CardDb>, state: GameState, events: Vec<Event>, seed: u64, speed: Speed) -> Self {
        let engine = Engine::new(db);
        let me = PlayerId::ONE;
        let display = Display::from_view(&engine.view(&state, me));
        let mut m = Match {
            engine,
            state,
            bot: RandomBot::new(seed ^ 0x5eed),
            me,
            log: vec![],
            selected: vec![],
            kind_menu: None,
            ringside: None,
            speed,
            display,
            layout: Layout::default(),
            queue: VecDeque::new(),
            step: None,
            banners: vec![],
            floats: vec![],
            flashes: vec![],
            skip: false,
            idle_since: 0.0,
        };
        // The deal is not animated: the board starts with the hands drawn.
        for ev in &events {
            m.log_event(&ev.redacted_for(me));
        }
        m
    }

    fn db(&self) -> &CardDb {
        self.engine.db()
    }

    /// Definition of a card the event stream or your view exposed.
    fn def_of(&self, card: CardId) -> &CardDef {
        self.db().card(self.state.card(card).def)
    }

    fn view(&self) -> PlayerView {
        self.engine.view(&self.state, self.me)
    }

    fn name(&self, c: CardId) -> String {
        let d = self.def_of(c);
        format!("{} [F{} D{}]", d.title, d.fortitude, d.damage)
    }

    fn log_event(&mut self, ev: &Event) {
        let name = |c: CardId| self.name(c);
        if let Some(line) = text::event(ev, self.me, &name) {
            self.log.push(line);
        }
    }

    fn act(&mut self, player: PlayerId, action: Action) {
        match self.engine.apply(&mut self.state, player, action) {
            Ok(events) => self.queue.extend(events.iter().map(|ev| ev.redacted_for(self.me))),
            Err(e) => self.log.push(format!("(acción rechazada: {e})")),
        }
        self.selected.clear();
        self.kind_menu = None;
    }

    fn idle(&self) -> bool {
        self.step.is_none() && self.queue.is_empty()
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Leave {
        let now = ui.input(|i| i.time);
        self.advance(now);
        let idle = self.idle();
        if idle {
            let truth = Display::from_view(&self.view());
            if self.display != truth {
                self.display = truth;
            }
            self.computer_moves(ui, now);
        } else {
            self.idle_since = now;
        }
        let view = self.view();
        let legal = if self.idle() { self.engine.legal_actions(&self.state, self.me) } else { vec![] };
        let mut leave = Leave::Stay;

        egui::Panel::right("log").default_size(270.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Animaciones:");
                egui::ComboBox::from_id_salt("speed").selected_text(self.speed.label()).show_ui(ui, |ui| {
                    for s in [Speed::Slow, Speed::Normal, Speed::Fast, Speed::Off] {
                        ui.selectable_value(&mut self.speed, s, s.label());
                    }
                });
                if !self.idle() && ui.button("Saltar").clicked() {
                    self.skip = true;
                }
            });
            ui.separator();
            ui.heading("Relato");
            ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                for line in &self.log {
                    ui.label(line);
                }
            });
        });
        egui::Panel::right("preview")
            .exact_size(cards::PREVIEW.x + 16.0)
            .resizable(false)
            .show(ui, cards::preview_panel);
        egui::Panel::top("opponent").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                self.piles(ui, self.me.opponent());
                self.superstar_card(ui, &view, self.me.opponent());
                ui.vertical(|ui| {
                    self.player_bar(ui, &view, self.me.opponent());
                    let start = ui.cursor().min;
                    ui.horizontal(|ui| {
                        let hand = self.display.sides[self.me.opponent().index()].hand.clone();
                        for c in hand.into_iter().take(14) {
                            let def = c.map(|c| self.def_of(c).clone());
                            cards::card(ui, def.as_ref(), cards::SMALL * 0.6, Look::Normal);
                        }
                    });
                    self.layout.hand[self.me.opponent().index()] =
                        Rect::from_min_size(start, vec2(ui.available_width().min(500.0), cards::SMALL.y * 0.6));
                });
            });
            ui.add_space(4.0);
        });
        egui::Panel::bottom("me").show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(l) = self.prompt(ui, &view, &legal) {
                leave = l;
            }
            ui.add_space(4.0);
            self.hand(ui, &legal);
            ui.horizontal(|ui| {
                self.piles(ui, self.me);
                self.superstar_card(ui, &view, self.me);
                ui.vertical(|ui| self.player_bar(ui, &view, self.me));
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.layout.board = ui.max_rect();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(cards::SIZE.x + 8.0);
                    ui.label(RichText::new("En juego").strong());
                    let (rect, _) = ui.allocate_exact_size(cards::SIZE, egui::Sense::hover());
                    self.layout.in_play = rect;
                    match self.display.in_play {
                        Some((c, p)) => {
                            let def = self.def_of(c);
                            cards::paint(&ui.painter_at(rect.expand(4.0)), rect, Some(def), Look::Normal, 1.0);
                            let who = if p == self.me { "La juegas tú" } else { "La juega el rival" };
                            let how = match view.in_play {
                                Some(pl) if pl.card == c && self.idle() => format!(" como {}", text::kind(pl.kind)),
                                _ => String::new(),
                            };
                            ui.label(format!("{who}{how}"));
                        }
                        None => {
                            ui.painter().rect_stroke(
                                rect,
                                CornerRadius::same(6),
                                Stroke::new(1.0, Color32::from_gray(60)),
                                StrokeKind::Inside,
                            );
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    self.ring(ui, self.me.opponent());
                    ui.separator();
                    self.ring(ui, self.me);
                });
            });
        });
        if self.idle() {
            self.choose_cards_window(ui, &view);
        }
        self.ringside_window(ui);
        self.paint_effects(ui, now);
        leave
    }

    /// Replays queued events until one needs time on screen.
    fn advance(&mut self, now: f64) {
        loop {
            if let Some(step) = &self.step {
                if !self.skip && now < step.start + step.duration {
                    return;
                }
                let step = self.step.take().expect("checked");
                if let Some((p, zone, card)) = step.land {
                    self.display.put(p, zone, card);
                }
            }
            let Some(ev) = self.queue.pop_front() else {
                self.skip = false;
                return;
            };
            self.begin(ev, now);
        }
    }

    /// Starts showing one event.
    fn begin(&mut self, ev: Event, now: f64) {
        self.log_event(&ev);
        let k = if self.skip { 0.0 } else { self.speed.factor() };
        let me = self.me;
        let mut step = Step { start: now, duration: 0.0, fly: None, show: vec![], land: None };
        let banner = |m: &mut Match, text: String, color: Color32, secs: f64| {
            if k > 0.0 {
                m.banners.push(Banner { text, color, start: now, duration: secs * k });
            }
        };
        let gold = Color32::from_rgb(255, 210, 90);
        let red = Color32::from_rgb(240, 80, 70);
        match &ev {
            Event::CardDrawn { player, card } => {
                let from = self.layout.arsenal[player.index()];
                self.display.take(*player, Zone::Arsenal, None);
                let size = if *player == me { cards::SIZE } else { cards::SMALL * 0.6 };
                let to = self.layout.target(*player, Zone::Hand, &self.display, size);
                step.fly = Some(Fly { card: *card, from, to, hurt: false });
                step.duration = 0.22;
                step.land = Some((*player, Zone::Hand, *card));
            }
            Event::CardPlayed { player, card, .. } => {
                if self.display.sides[player.index()].ring.contains(card) {
                    // Pre-match cards go straight to the Ring.
                    let title = self.def_of(*card).title.clone();
                    banner(self, title, gold, 1.0);
                    step.duration = 0.6;
                } else {
                    let size = if *player == me { cards::SIZE } else { cards::SMALL * 0.6 };
                    let from = self.layout.source(*player, Zone::Hand, Some(*card), size);
                    self.display.take_played(*player, *card);
                    step.fly = Some(Fly { card: Some(*card), from, to: self.layout.in_play, hurt: false });
                    step.duration = 0.55;
                    step.land = Some((*player, Zone::InPlay, Some(*card)));
                    if *player != me {
                        let title = self.def_of(*card).title.clone();
                        banner(self, format!("El rival juega {title}"), gold, 1.2);
                    }
                }
            }
            Event::CardMoved { player, card, from, to } => {
                let size = if *to == Zone::Ring { cards::SMALL } else { cards::PILE };
                let src = self.layout.source(*player, *from, *card, size);
                self.display.take(*player, *from, *card);
                let dst = self.layout.target(*player, *to, &self.display, size);
                let hurt = *from == Zone::InPlay && *to == Zone::Ringside;
                step.fly = Some(Fly { card: *card, from: src, to: dst, hurt });
                step.duration = if *from == Zone::InPlay { 0.4 } else { 0.32 };
                step.land = Some((*player, *to, *card));
            }
            Event::Overturned { player, card } => {
                let first = !matches!(self.display_last_overturn(), Some(p) if p == *player);
                if first {
                    let n = 1 + self
                        .queue
                        .iter()
                        .take_while(|e| matches!(e, Event::Overturned { player: q, .. } if q == player))
                        .count();
                    let text = if *player == me {
                        format!("¡Recibes {n} de daño!")
                    } else {
                        format!("¡El rival recibe {n} de daño!")
                    };
                    banner(self, text, red, 1.1);
                }
                let from = self.layout.arsenal[player.index()];
                self.display.take(*player, Zone::Arsenal, None);
                step.fly = Some(Fly { card: Some(*card), from, to: self.layout.ringside[player.index()], hurt: false });
                step.duration = 0.24;
                step.land = Some((*player, Zone::Ringside, Some(*card)));
                if k > 0.0 {
                    self.flashes.push((*player, now));
                    let bar = self.layout.bar[player.index()];
                    self.floats.push(Float {
                        text: "-1".into(),
                        at: Pos2::new(bar.left() + 220.0, bar.center().y),
                        start: now,
                    });
                }
            }
            Event::Reversed { player, .. } => {
                let text =
                    if *player == me { "¡REVERSAL!".to_string() } else { "¡El rival te revierte!".to_string() };
                banner(self, text, red, 1.3);
                step.duration = 0.7;
            }
            Event::PartialReversal { .. } => {
                banner(self, "¡Es Multi! Hace falta otro Reversal".into(), gold, 1.2);
                step.duration = 0.6;
            }
            Event::TurnStarted { player, turn } => {
                let text =
                    if *player == me { format!("Tu turno ({turn})") } else { format!("Turno del rival ({turn})") };
                banner(self, text, Color32::WHITE, 1.0);
                step.duration = 0.6;
            }
            Event::TurnSkipped { player } => {
                let text = if *player == me { "Te saltas el turno" } else { "El rival se salta el turno" };
                banner(self, text.into(), gold, 1.1);
                step.duration = 0.6;
            }
            Event::HandShown { viewer, cards, .. } if *viewer == me => {
                step.show = cards.clone();
                step.duration = 1.0 + 0.25 * cards.len() as f64;
            }
            Event::HandShown { .. } => {
                banner(self, "El rival mira tu mano".into(), gold, 1.1);
                step.duration = 0.5;
            }
            Event::Revealed { card, .. } => {
                step.show = vec![*card];
                step.duration = 1.0;
            }
            Event::PreMatchStarted { .. } | Event::PreMatchEnded | Event::GameStarted { .. } => {
                let name = |c| self.name(c);
                if let Some(text) = text::event(&ev, me, &name) {
                    banner(self, text, Color32::WHITE, 1.0);
                }
                step.duration = 0.5;
            }
            Event::CountOutRefused { .. } => {
                banner(self, "¡Rechaza el Count Out!".into(), gold, 1.0);
                step.duration = 0.5;
            }
            Event::GameOver(o) => {
                self.banners.push(Banner { text: text::outcome(o, me), color: gold, start: now, duration: 4.0 });
            }
            _ => {}
        }
        step.duration *= k;
        self.step = Some(step);
    }

    /// The player whose card the previous replay step overturned, if it was an overturn.
    fn display_last_overturn(&self) -> Option<PlayerId> {
        // `self.step` was just finished; the log line tells whether it was an overturn.
        let last = self.log.iter().rev().nth(1)?;
        if last.starts_with("  Volteas") {
            Some(self.me)
        } else if last.starts_with("El rival voltea") {
            Some(self.me.opponent())
        } else {
            None
        }
    }

    fn computer_moves(&mut self, ui: &Ui, now: f64) {
        let Some(Decision { player, .. }) = self.state.decision.clone() else { return };
        if player == self.me || self.state.is_over() {
            return;
        }
        let wait = self.idle_since + BOT_DELAY * self.speed.factor().max(0.3) - now;
        if wait > 0.0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        let legal = self.engine.legal_actions(&self.state, player);
        let action = self.bot.choose(&self.engine, &self.engine.view(&self.state, player), &legal);
        self.act(player, action);
        ui.ctx().request_repaint();
    }

    fn piles(&mut self, ui: &mut Ui, p: PlayerId) {
        let side = &self.display.sides[p.index()];
        let top = side.ringside.last().map(|&c| self.def_of(c).clone());
        let (arsenal, ringside) = (side.arsenal, side.ringside.len());
        let a = cards::pile(ui, "Arsenal", None, arsenal, false);
        self.layout.arsenal[p.index()] = Rect::from_min_size(a.rect.min, cards::PILE);
        let r = cards::pile(ui, "Ringside", top.as_ref(), ringside, true).on_hover_text("Clic para ver el Ringside");
        self.layout.ringside[p.index()] = Rect::from_min_size(r.rect.min, cards::PILE);
        if r.clicked() {
            self.ringside = if self.ringside == Some(p) { None } else { Some(p) };
        }
        let removed = self.state.player(p).removed.len();
        if removed > 0 {
            ui.small(format!("Fuera del juego: {removed}"));
        }
        let under = self.state.player(p).under.len();
        if under > 0 {
            ui.small(format!("Debajo de cartas: {under}"));
        }
    }

    fn superstar_card(&self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
        let star = self.db().superstar(view.players[p.index()].superstar);
        cards::superstar(ui, star);
    }

    fn player_bar(&mut self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
        let info = &view.players[p.index()];
        let star = self.db().superstar(info.superstar).clone();
        let side = &self.display.sides[p.index()];
        let fortitude: u32 = side.ring.iter().map(|&c| self.def_of(c).damage as u32).sum();
        let hand = side.hand.len();
        let backlash = side.backlash.len();
        let r = ui.horizontal(|ui| {
            let label = if p == self.me { "Tú" } else { "Rival" };
            ui.label(RichText::new(format!("{label}: {}", star.name)).strong().size(16.0));
            ui.separator();
            ui.label(RichText::new(format!("Fortitude {fortitude}")).color(Color32::from_rgb(255, 210, 90)));
            ui.separator();
            ui.label(format!("Mano {hand}"));
            if backlash > 0 {
                ui.separator();
                ui.label(format!("Backlash {backlash}"));
            }
            if view.active == p && !self.state.is_over() {
                ui.separator();
                let text = if p == self.me { "tu turno" } else { "su turno" };
                ui.label(RichText::new(text).color(Color32::from_rgb(90, 220, 120)));
            }
        });
        self.layout.bar[p.index()] = r.response.rect;
    }

    fn ring(&mut self, ui: &mut Ui, p: PlayerId) {
        ui.label(format!("Ring de {}", if p == self.me { "tu Superstar" } else { "tu rival" }));
        let ring = self.display.sides[p.index()].ring.clone();
        let start = ui.cursor().min;
        let r = ScrollArea::horizontal().id_salt(("ring", p.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ring.is_empty() {
                    ui.add_sized(cards::SMALL, egui::Label::new("(vacío)"));
                }
                for c in ring {
                    let def = self.def_of(c);
                    let rect = cards::card(ui, Some(def), cards::SMALL, Look::Normal).rect;
                    self.layout.cards.insert(c, rect);
                }
            });
        });
        let width = r.inner_rect.width().max(cards::SMALL.x);
        self.layout.ring[p.index()] = Rect::from_min_size(start, vec2(width, cards::SMALL.y));
    }

    /// Your hand (and Backlash deck), with the cards you can use highlighted.
    fn hand(&mut self, ui: &mut Ui, legal: &[Action]) {
        let mut clicked = None;
        let side = self.display.sides[self.me.index()].clone();
        let start = ui.cursor().min;
        ScrollArea::horizontal().id_salt("hand").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(cards::SIZE.y + 6.0);
                for (i, c) in side.hand.iter().chain(&side.backlash).enumerate() {
                    let Some(c) = *c else { continue };
                    if i == side.hand.len() {
                        ui.separator();
                        ui.label("Backlash:");
                    }
                    let usable = legal.iter().any(|a| card_of(a) == Some(c));
                    let look = if self.kind_menu == Some(c) {
                        Look::Selected
                    } else if usable {
                        Look::Playable
                    } else {
                        Look::Normal
                    };
                    let def = self.def_of(c);
                    let r = cards::card(ui, Some(def), cards::SIZE, look);
                    self.layout.cards.insert(c, r.rect);
                    if r.clicked() && usable {
                        clicked = Some(c);
                    }
                }
            });
        });
        self.layout.hand[self.me.index()] = Rect::from_min_size(start, cards::SIZE);
        if let Some(c) = clicked {
            let actions: Vec<&Action> = legal.iter().filter(|a| card_of(a) == Some(c)).collect();
            if actions.len() == 1 {
                self.act(self.me, actions[0].clone());
            } else {
                self.kind_menu = Some(c);
            }
        }
    }

    /// What the game is waiting for, with buttons to answer.
    fn prompt(&mut self, ui: &mut Ui, view: &PlayerView, legal: &[Action]) -> Option<Leave> {
        if let (Some(outcome), true) = (&self.state.outcome, self.idle()) {
            let mut leave = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new(text::outcome(outcome, self.me)).size(20.0).strong());
                if ui.button("Revancha").clicked() {
                    leave = Some(Leave::Rematch);
                }
                if ui.button("Volver al menú").clicked() {
                    leave = Some(Leave::Menu);
                }
            });
            return leave;
        }
        let decision = view.decision.clone().filter(|_| self.idle());
        let Some(decision) = decision else {
            let text = if self.idle() { "Esperando al rival…" } else { "…" };
            ui.label(RichText::new(text).italics());
            return None;
        };
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| match &decision.kind {
            DecisionKind::Main => {
                if let Some(card) = self.kind_menu {
                    let title = self.def_of(card).title.clone();
                    ui.label(format!("¿Cómo juegas {title}?"));
                    for a in legal.iter().filter(|a| card_of(a) == Some(card)) {
                        if let Action::PlayCard { kind, .. } = a
                            && ui.button(format!("Como {}", text::kind(*kind))).clicked()
                        {
                            chosen = Some(a.clone());
                        }
                    }
                    if ui.button("Cancelar").clicked() {
                        self.kind_menu = None;
                    }
                } else {
                    ui.label(
                        RichText::new("Tu turno: juega una carta (las marcadas en verde) o termina el turno.").strong(),
                    );
                }
                if legal.contains(&Action::UseAbility) {
                    let star = self.db().superstar(view.me_public().superstar);
                    if ui.button("Usar habilidad").on_hover_text(&star.text).clicked() {
                        chosen = Some(Action::UseAbility);
                    }
                }
                for a in legal {
                    if let Action::PlayCard { card, kind } = a
                        && self.state.card(*card).zone == Zone::Ringside
                    {
                        let def = self.def_of(*card);
                        let label = format!("Jugar {} desde el Ringside ({})", def.title, text::kind(*kind));
                        if ui.button(label).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::PlayCard { card, kind } = a
                        && self.state.card(*card).zone == Zone::Under
                    {
                        let def = self.def_of(*card);
                        let host =
                            self.state.card(*card).host.map(|h| self.def_of(h).title.clone()).unwrap_or_default();
                        let label = format!("Jugar {} de debajo de {} ({})", def.title, host, text::kind(*kind));
                        if ui.button(label).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::UseRingCard { card } = a {
                        let def = self.def_of(*card);
                        if ui.button(format!("Usar {}", def.title)).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                }
                if ui.button("Terminar turno").clicked() {
                    chosen = Some(Action::EndTurn);
                }
            }
            DecisionKind::Reversal { card, .. } => {
                let title = self.def_of(*card).title.clone();
                ui.label(
                    RichText::new(format!("El rival juega {title}. Puedes revertirla con una carta marcada.")).strong(),
                );
                // Cards in your Ring that can be sent to the Ringside pile to reverse.
                for a in legal {
                    if let Action::Reverse { card } = a
                        && self.state.card(*card).zone == Zone::Under
                    {
                        let def = self.def_of(*card);
                        if ui
                            .button(format!("Revertir con {} (de debajo de una carta)", def.title))
                            .on_hover_text(&def.text)
                            .clicked()
                        {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::Reverse { card } = a
                        && view.me_public().ring.contains(card)
                    {
                        let def = self.def_of(*card);
                        if ui
                            .button(format!("Revertir con {} (del Ring)", def.title))
                            .on_hover_text(&def.text)
                            .clicked()
                        {
                            chosen = Some(a.clone());
                        }
                    }
                }
                if ui.button("No revertir").clicked() {
                    chosen = Some(Action::DeclineReversal);
                }
            }
            DecisionKind::ChooseCards { .. } => {
                ui.label(RichText::new("Elige cartas en la ventana.").strong());
            }
            DecisionKind::YesNo { purpose } => {
                ui.label(RichText::new(text::yes_no(purpose)).strong());
                if ui.button("Sí").clicked() {
                    chosen = Some(Action::Yes);
                }
                if ui.button("No").clicked() {
                    chosen = Some(Action::No);
                }
            }
            DecisionKind::StunDraw { max } => {
                ui.label(RichText::new(format!("Puedes robar hasta {max} cartas.")).strong());
                for n in 0..=*max {
                    if ui.button(format!("Robar {n}")).clicked() {
                        chosen = Some(Action::DrawCards(n));
                    }
                }
            }
            DecisionKind::ChooseOption { options, .. } => {
                ui.label(RichText::new("Elige una opción:").strong());
                for (i, o) in options.iter().enumerate() {
                    if ui.button(text::effects(o)).clicked() {
                        chosen = Some(Action::ChooseOption(i as u8));
                    }
                }
            }
            DecisionKind::PreMatch { .. } => {
                ui.label(RichText::new("Pre-match: juega una carta marcada de tu Backlash o pasa.").strong());
                if ui.button("Pasar").clicked() {
                    chosen = Some(Action::Pass);
                }
            }
        });
        if let Some(a) = chosen {
            self.act(self.me, a);
        }
        None
    }

    fn choose_cards_window(&mut self, ui: &Ui, view: &PlayerView) {
        let Some(Decision { kind: DecisionKind::ChooseCards { options, min, max, purpose }, .. }) =
            view.decision.clone()
        else {
            return;
        };
        let what = if min == max { format!("{min}") } else { format!("de {min} a {max}") };
        let mut confirm = false;
        egui::Window::new(format!("Elige {what} {}", text::purpose(purpose)))
            .collapsible(false)
            .resizable(true)
            .default_width(720.0)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ScrollArea::both().max_height(420.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for &c in &options {
                            let picked = self.selected.contains(&c);
                            let look = if picked { Look::Selected } else { Look::Playable };
                            let def = view.def_of(c).map(|d| self.db().card(d));
                            if cards::card(ui, def, cards::SIZE, look).clicked() {
                                if picked {
                                    self.selected.retain(|&s| s != c);
                                } else if (self.selected.len() as u8) < max {
                                    self.selected.push(c);
                                } else if max == 1 {
                                    self.selected = vec![c];
                                }
                            }
                        }
                    });
                });
                ui.separator();
                let n = self.selected.len() as u8;
                ui.horizontal(|ui| {
                    ui.label(format!("Elegidas: {n}"));
                    if ui.add_enabled((min..=max).contains(&n), egui::Button::new("Confirmar")).clicked() {
                        confirm = true;
                    }
                });
            });
        if confirm {
            let chosen = std::mem::take(&mut self.selected);
            self.act(self.me, Action::ChooseCards(chosen));
        }
    }

    fn ringside_window(&mut self, ui: &Ui) {
        let Some(p) = self.ringside else { return };
        let mut open = true;
        let whose = if p == self.me { "Tu Ringside" } else { "Ringside del rival" };
        let cards: Vec<CardId> = self.display.sides[p.index()].ringside.iter().rev().copied().collect();
        egui::Window::new(whose).open(&mut open).default_width(640.0).show(ui.ctx(), |ui| {
            ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &c in &cards {
                        cards::card(ui, Some(self.def_of(c)), cards::SMALL, Look::Normal);
                    }
                });
            });
        });
        if !open {
            self.ringside = None;
        }
    }

    /// Flying cards, banners, damage numbers and flashes, over everything else.
    fn paint_effects(&mut self, ui: &Ui, now: f64) {
        let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::new("effects")));
        let mut busy = !self.idle();

        self.flashes.retain(|&(_, t)| now - t < 0.35);
        for &(p, t) in &self.flashes {
            let a = (1.0 - (now - t) / 0.35) as f32;
            let area = self.layout.ring[p.index()].union(self.layout.bar[p.index()]).expand(6.0);
            painter.rect_filled(
                area,
                CornerRadius::same(8),
                Color32::from_rgba_unmultiplied(220, 40, 40, (70.0 * a) as u8),
            );
            busy = true;
        }

        if let Some(step) = &self.step {
            let t = if step.duration > 0.0 { (now - step.start) / step.duration } else { 1.0 };
            if let Some(fly) = &step.fly {
                let e = ease(t);
                let center = fly.from.center().lerp(fly.to.center(), e);
                let size = fly.from.size() + (fly.to.size() - fly.from.size()) * e;
                // A little hop in the middle of the flight.
                let hop = (std::f32::consts::PI * e).sin() * 24.0;
                let rect = Rect::from_center_size(center - vec2(0.0, hop), size);
                let def = fly.card.map(|c| self.def_of(c));
                painter.rect_filled(
                    rect.translate(vec2(4.0, 6.0)),
                    CornerRadius::same(6),
                    Color32::from_black_alpha(90),
                );
                cards::paint(&painter, rect, def, Look::Normal, 1.0);
                if fly.hurt {
                    painter.rect_filled(rect, CornerRadius::same(6), Color32::from_rgba_unmultiplied(200, 30, 30, 70));
                }
            }
            if !step.show.is_empty() {
                let n = step.show.len() as f32;
                let gap = 10.0;
                let width = n * cards::SIZE.x + (n - 1.0) * gap;
                let board = self.layout.board;
                let back = Rect::from_center_size(board.center(), vec2(width + 40.0, cards::SIZE.y + 40.0));
                painter.rect_filled(back, CornerRadius::same(10), Color32::from_black_alpha(200));
                let mut x = board.center().x - width / 2.0;
                for &c in &step.show {
                    let r = Rect::from_min_size(Pos2::new(x, board.center().y - cards::SIZE.y / 2.0), cards::SIZE);
                    cards::paint(&painter, r, Some(self.def_of(c)), Look::Normal, ease(t * 4.0));
                    x += cards::SIZE.x + gap;
                }
            }
        }

        self.banners.retain(|b| now - b.start < b.duration);
        let board = self.layout.board;
        for (i, b) in self.banners.iter().enumerate() {
            let t = (now - b.start) / b.duration;
            let alpha = (t * 6.0).min((1.0 - t) * 4.0).clamp(0.0, 1.0) as f32;
            let y = board.top() + board.height() * 0.38 + i as f32 * 52.0;
            let galley =
                painter.layout_no_wrap(b.text.clone(), FontId::proportional(30.0), b.color.gamma_multiply(alpha));
            let rect = Rect::from_center_size(Pos2::new(board.center().x, y), galley.size() + vec2(36.0, 16.0));
            painter.rect_filled(rect, CornerRadius::same(10), Color32::from_black_alpha((190.0 * alpha) as u8));
            painter.galley(rect.min + vec2(18.0, 8.0), galley, Color32::WHITE);
            busy = true;
        }

        self.floats.retain(|f| now - f.start < 0.9);
        for f in &self.floats {
            let t = ((now - f.start) / 0.9) as f32;
            let pos = f.at - vec2(0.0, 36.0 * t);
            let color = Color32::from_rgb(255, 90, 80).gamma_multiply(1.0 - t);
            painter.text(pos, Align2::CENTER_CENTER, &f.text, FontId::proportional(24.0), color);
            busy = true;
        }

        if busy {
            ui.ctx().request_repaint();
        }
        self.layout.cards.clear();
    }
}

fn card_of(action: &Action) -> Option<CardId> {
    match action {
        Action::PlayCard { card, .. } | Action::Reverse { card } => Some(*card),
        _ => None,
    }
}
