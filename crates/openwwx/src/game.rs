//! The match screen: you against the computer, against someone online, or
//! watching an online match.

use std::collections::VecDeque;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Pos2, Rect, RichText, ScrollArea, Stroke,
    StrokeKind, Ui, vec2,
};
use rawdeal_core::{
    Action, CardDb, CardDef, CardId, CardKind, Decision, DecisionKind, Event, PlayerId, PlayerView, Zone,
};

use rawdeal_net::host::ABANDON_AFTER;

use crate::anim::{Banner, Display, Float, Fly, Layout, Step, ease};
use crate::cards::{self, Look};
use crate::table::Table;
use crate::text::{self, Voice};
use crate::theme;

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
    table: Table,
    /// This computer's address on its network, to tell guests where to connect.
    host_ip: Option<String>,
    /// Your seat, or the side a spectator sees from below.
    me: PlayerId,
    /// The players' names, online.
    names: Option<[String; 2]>,
    spectator: bool,
    /// The board has the dealt hands (online, the match may not have started).
    started: bool,
    /// Who the last replayed event overturned a card of, if it was an overturn.
    last_overturn: Option<PlayerId>,
    log: Vec<String>,
    /// Cards picked so far in a "choose cards" decision.
    selected: Vec<CardId>,
    /// Hybrid card waiting for you to pick how to play it.
    kind_menu: Option<CardId>,
    /// The chat line being typed.
    chat: String,
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
    /// A match on `table`; `events` are the deal, if it already happened.
    pub fn new(table: Table, events: Vec<Event>, speed: Speed) -> Self {
        let mut m = Match {
            me: table.me(),
            host_ip: table.hosting_port().and_then(|_| local_ip()),
            table,
            names: None,
            spectator: false,
            started: false,
            last_overturn: None,
            log: vec![],
            selected: vec![],
            kind_menu: None,
            chat: String::new(),
            ringside: None,
            speed,
            display: Display::default(),
            layout: Layout::default(),
            queue: VecDeque::new(),
            step: None,
            banners: vec![],
            floats: vec![],
            flashes: vec![],
            skip: false,
            idle_since: 0.0,
        };
        m.start(events);
        m
    }

    /// Puts the dealt hands on the board once the match has started. The deal
    /// is not animated.
    fn start(&mut self, events: Vec<Event>) {
        let Some(view) = self.table.view() else { return };
        self.see(&view);
        self.display = Display::from_view(&view);
        self.started = true;
        if events.is_empty() && self.table.online_state().is_some() {
            self.log.push("Entraste a la partida en curso.".into());
        }
        for ev in &events {
            self.log_event(ev);
        }
    }

    /// Takes in which seat and names this view is for.
    fn see(&mut self, view: &PlayerView) {
        self.me = view.me;
        self.spectator = view.spectator;
        self.names = self.table.online_state().and_then(|o| o.lobby.as_ref()).map(|l| {
            let name = |i: usize| l.players[i].as_ref().map_or(format!("Jugador {}", i + 1), |s| s.name.clone());
            [name(0), name(1)]
        });
    }

    fn db(&self) -> &CardDb {
        &self.table.db
    }

    /// Definition of a card the event stream or your view exposed.
    fn def_of(&self, card: CardId) -> &CardDef {
        self.db().card(self.table.def(card).unwrap_or(self.db().face_down()))
    }

    fn voice(&self) -> Voice {
        match &self.names {
            Some(names) if self.spectator => Voice::Spectator(names.clone()),
            _ => Voice::Player(self.me),
        }
    }

    /// Is `p` you, the player at this screen?
    fn mine(&self, p: PlayerId) -> bool {
        !self.spectator && p == self.me
    }

    /// How a player other than you is called: their name online, else "el rival".
    fn rival(&self, p: PlayerId) -> String {
        match &self.names {
            Some(names) => names[p.index()].clone(),
            None => "el rival".into(),
        }
    }

    /// [`Match::rival`] at the start of a sentence.
    fn rival_cap(&self, p: PlayerId) -> String {
        let r = self.rival(p);
        let mut c = r.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(r)
    }

    fn name(&self, c: CardId) -> String {
        let d = self.def_of(c);
        format!("{} [F{} D{}]", d.title, d.fortitude, d.damage)
    }

    fn log_event(&mut self, ev: &Event) {
        let name = |c: CardId| self.name(c);
        if let Some(line) = text::event(ev, &self.voice(), &name) {
            self.log.push(line);
        }
    }

    fn act(&mut self, action: Action) {
        match self.table.act(action) {
            Ok(events) => self.queue.extend(events),
            Err(e) => self.log.push(format!("(acción rechazada: {e})")),
        }
        self.selected.clear();
        self.kind_menu = None;
    }

    fn idle(&self) -> bool {
        self.step.is_none() && self.queue.is_empty()
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Leave {
        let now = ui.input(|i| i.time);
        let events = self.table.poll();
        self.log.extend(self.table.notes());
        if self.started {
            self.queue.extend(events);
        } else {
            self.start(events);
        }
        if self.table.online_state().is_some() {
            // Online, updates arrive on their own.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let Some(view) = self.table.view().filter(|_| self.started) else { return self.lobby(ui) };
        self.see(&view);
        self.advance(now);
        if self.idle() {
            let truth = Display::from_view(&view);
            if self.display != truth {
                self.display = truth;
            }
            self.computer_moves(ui, now);
        } else {
            self.idle_since = now;
        }
        let view = self.table.view().unwrap_or(view);
        let legal = if self.idle() { self.table.legal() } else { vec![] };
        let mut leave = Leave::Stay;

        egui::Panel::right("side")
            .exact_size(300.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(14, 13, 16))
                    .stroke(Stroke::new(1.5, Color32::from_rgb(90, 90, 98)))
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
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
                    if self.table.online_state().is_some() && view.outcome.is_none() {
                        let hint = match (self.spectator, self.table.hosting()) {
                            (true, _) => "Dejas de mirar la partida.",
                            (false, true) => "Abandonas la partida (tu rival gana) y la cierras para todos.",
                            (false, false) => "Abandonas la partida: tu rival gana.",
                        };
                        if ui.button("Salir").on_hover_text(hint).clicked() {
                            self.table.leave();
                            leave = Leave::Menu;
                        }
                    }
                });
                ui.add_space(4.0);
                cards::preview_with(ui, Some(110.0));
                ui.add_space(4.0);
                ui.separator();
                theme::heading(ui, "RELATO", 20.0);
                if self.table.online_state().is_some() {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.chat)
                            .hint_text("Chat: escribe y pulsa Enter")
                            .desired_width(f32::INFINITY),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let text = std::mem::take(&mut self.chat);
                        if !text.trim().is_empty() {
                            self.table.chat(&text);
                        }
                        r.request_focus();
                    }
                }
                ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                    for line in &self.log {
                        ui.label(line);
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(8, 7, 10))).show(ui, |ui| {
            let full = ui.max_rect();
            let look = theme::get(ui.ctx());
            theme::cover(ui, &look.arena, full);
            ui.painter().rect_filled(full, 0.0, Color32::from_black_alpha(150));

            // Piles outside the ring: your rival's at the top, yours at the bottom.
            // In a short window, Arsenal and Ringside go side by side.
            let pile_h = 3.0 * (cards::PILE.y + 22.0) + 30.0;
            let compact = full.height() < 2.0 * pile_h + 20.0;
            let side_w = if compact { 2.0 * cards::PILE.x + 30.0 } else { cards::PILE.x + 20.0 };
            let pile_h = if compact { 2.0 * (cards::PILE.y + 22.0) + 30.0 } else { pile_h };
            let side = Rect::from_min_max(full.min, egui::pos2(full.left() + side_w, full.bottom()));
            let mut top = ui.new_child(egui::UiBuilder::new().max_rect(side.shrink(8.0)));
            top.spacing_mut().item_spacing.y = 6.0;
            if compact {
                top.horizontal_top(|ui| self.piles(ui, &view, self.me.opponent()));
            } else {
                self.piles(&mut top, &view, self.me.opponent());
            }
            self.superstar_card(&mut top, &view, self.me.opponent());
            let low = Rect::from_min_max(egui::pos2(side.left(), side.bottom() - pile_h), side.max);
            let mut bottom = ui.new_child(
                egui::UiBuilder::new().max_rect(low.shrink(8.0)).layout(egui::Layout::bottom_up(egui::Align::Min)),
            );
            bottom.spacing_mut().item_spacing.y = 6.0;
            self.superstar_card(&mut bottom, &view, self.me);
            if compact {
                bottom.horizontal_top(|ui| self.piles(ui, &view, self.me));
            } else {
                self.piles(&mut bottom, &view, self.me);
            }

            let ring = Rect::from_min_max(egui::pos2(side.right(), full.top()), full.max).shrink(6.0);
            let canvas = paint_ring(ui, ring);
            self.layout.board = canvas;
            let inner = canvas.shrink(12.0);

            // Both Rings in a column on the right, your rival's above.
            let ring_w = cards::SMALL.x + 24.0;
            let column = Rect::from_min_max(egui::pos2(inner.right() - ring_w, inner.top()), inner.max);
            let (theirs, mine) = column.split_top_bottom_at_fraction(0.5);
            self.ring(ui, self.me.opponent(), theirs.shrink2(vec2(0.0, 3.0)));
            self.ring(ui, self.me, mine.shrink2(vec2(0.0, 3.0)));

            let play = Rect::from_min_max(inner.min, egui::pos2(column.left() - 10.0, inner.bottom()));
            // Your rival's name plate and hand along the top rope.
            let rival_h = cards::SMALL.y * 0.6 + 40.0;
            let mut top = ui
                .new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(play.min, vec2(play.width(), rival_h))));
            self.plate(&mut top, &view, self.me.opponent());
            let start = top.cursor().min;
            top.horizontal(|ui| {
                let hand = self.display.sides[self.me.opponent().index()].hand.clone();
                let fits = ((play.width() + 8.0) / (cards::SMALL.x * 0.6 + 8.0)) as usize;
                for c in hand.into_iter().take(fits.min(14)) {
                    let def = c.map(|c| self.def_of(c).clone());
                    cards::card(ui, def.as_ref(), cards::SMALL * 0.6, Look::Normal);
                }
            });
            self.layout.hand[self.me.opponent().index()] =
                Rect::from_min_size(start, vec2(play.width().min(500.0), cards::SMALL.y * 0.6));

            // Your hand, plate and what the game asks along the bottom.
            let mine_h = cards::SIZE.y + 110.0;
            let low = Rect::from_min_max(egui::pos2(play.left(), play.bottom() - mine_h), play.max);
            let mut bottom = ui.new_child(egui::UiBuilder::new().max_rect(low));
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(12, 11, 14, 215))
                .stroke(Stroke::new(1.0, Color32::from_rgb(90, 90, 98)))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(&mut bottom, |ui| {
                    ui.set_width(ui.available_width());
                    if let Some(l) = self.prompt(ui, &view, &legal) {
                        leave = l;
                    }
                });
            bottom.add_space(4.0);
            self.hand(&mut bottom, &legal);
            self.plate(&mut bottom, &view, self.me);

            // The card in play, in the middle.
            let middle =
                Rect::from_min_max(egui::pos2(play.left(), play.top() + rival_h), egui::pos2(play.right(), low.top()));
            self.in_play(ui, &view, middle);
            if let (Some(outcome), true) = (&view.outcome, self.idle()) {
                let text = text::outcome(outcome, &self.voice());
                let size = (middle.width() / (text.chars().count() as f32 * 0.42)).clamp(28.0, 64.0);
                theme::shadow_text(
                    ui,
                    middle.center(),
                    Align2::CENTER_CENTER,
                    &text,
                    theme::display_font(size),
                    Color32::from_rgb(255, 214, 90),
                );
            }
        });
        if self.idle() {
            self.choose_cards_window(ui, &view);
        }
        self.ringside_window(ui);
        self.paint_effects(ui, now);
        leave
    }

    /// Who is connected, while an online match waits for its players.
    fn lobby(&mut self, ui: &mut Ui) -> Leave {
        let mut leave = Leave::Stay;
        let port = self.table.hosting_port();
        let hosting = self.table.hosting();
        let Some(online) = self.table.online_state() else { return leave };
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                match &online.info {
                    Some(info) => {
                        ui.heading(RichText::new(&info.name).size(28.0).strong());
                        ui.label(format!("Reglas: {}", text::mode(info.mode)));
                    }
                    None => {
                        ui.heading("Conectando…");
                    }
                }
                ui.add_space(12.0);
                if let Some(port) = port {
                    let ip = self.host_ip.as_deref().unwrap_or("tu IP");
                    ui.label(format!(
                        "Estás hospedando la partida. Tu rival y los espectadores se conectan a {ip}:{port}."
                    ));
                    ui.small("Fuera de tu red local, tienes que abrir ese puerto en tu router.");
                    ui.add_space(12.0);
                } else if hosting {
                    ui.label("Tu partida está en la lista del servidor común. Los demás entran desde ahí.");
                    ui.add_space(12.0);
                }
                if let Some(lobby) = &online.lobby {
                    for (i, slot) in lobby.players.iter().enumerate() {
                        let you = online.seat.is_some_and(|s| s.index() == i);
                        let line = match slot {
                            Some(s) => {
                                let deck = if s.ready { "mazo listo" } else { "eligiendo mazo" };
                                format!("Jugador {}: {}{} ({deck})", i + 1, s.name, if you { " (tú)" } else { "" })
                            }
                            None => format!("Jugador {}: esperando…", i + 1),
                        };
                        ui.label(RichText::new(line).size(16.0));
                    }
                    if !lobby.spectators.is_empty() {
                        ui.label(format!("Mirando: {}", lobby.spectators.join(", ")));
                    }
                }
                if online.info.is_some() && online.seat.is_none() && online.problem.is_none() {
                    ui.label("Entraste a mirar. La partida empieza cuando estén los dos jugadores.");
                }
                ui.add_space(12.0);
                match &online.problem {
                    Some(p) => {
                        ui.colored_label(Color32::from_rgb(240, 110, 110), p);
                    }
                    None => {
                        ui.spinner();
                    }
                }
                ui.add_space(12.0);
                if ui.button("Volver al menú").clicked() {
                    leave = Leave::Menu;
                }
            });
        });
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
        let last_overturn = self.last_overturn.take();
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
                let size = if self.mine(*player) { cards::SIZE } else { cards::SMALL * 0.6 };
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
                    let size = if self.mine(*player) { cards::SIZE } else { cards::SMALL * 0.6 };
                    let from = self.layout.source(*player, Zone::Hand, Some(*card), size);
                    self.display.take_played(*player, *card);
                    step.fly = Some(Fly { card: Some(*card), from, to: self.layout.in_play, hurt: false });
                    step.duration = 0.55;
                    step.land = Some((*player, Zone::InPlay, Some(*card)));
                    if !self.mine(*player) {
                        let title = self.def_of(*card).title.clone();
                        banner(self, format!("{} juega {title}", self.rival_cap(*player)), gold, 1.2);
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
                self.last_overturn = Some(*player);
                if last_overturn != Some(*player) {
                    let n = 1 + self
                        .queue
                        .iter()
                        .take_while(|e| matches!(e, Event::Overturned { player: q, .. } if q == player))
                        .count();
                    let text = if self.mine(*player) {
                        format!("¡Recibes {n} de daño!")
                    } else {
                        format!("¡{} recibe {n} de daño!", self.rival_cap(*player))
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
            Event::Reversed { player, .. } | Event::VolleyReversed { player, .. } => {
                let text = if self.mine(*player) {
                    "¡REVERSAL!".to_string()
                } else if self.spectator {
                    format!("¡{} revierte!", self.rival_cap(*player))
                } else {
                    format!("¡{} te revierte!", self.rival_cap(*player))
                };
                banner(self, text, red, 1.3);
                step.duration = 0.7;
            }
            Event::PartialReversal { .. } => {
                banner(self, "¡Es Multi! Hace falta otro Reversal".into(), gold, 1.2);
                step.duration = 0.6;
            }
            Event::TurnStarted { player, turn } => {
                let text = match &self.names {
                    _ if self.mine(*player) => format!("Tu turno ({turn})"),
                    Some(names) => format!("Turno de {} ({turn})", names[player.index()]),
                    None => format!("Turno del rival ({turn})"),
                };
                banner(self, text, Color32::WHITE, 1.0);
                step.duration = 0.6;
            }
            Event::TurnSkipped { player } => {
                let text = if self.mine(*player) {
                    "Te saltas el turno".into()
                } else {
                    format!("{} se salta el turno", self.rival_cap(*player))
                };
                banner(self, text, gold, 1.1);
                step.duration = 0.6;
            }
            Event::HandShown { viewer, cards, .. } if self.mine(*viewer) => {
                step.show = cards.clone();
                step.duration = 1.0 + 0.25 * cards.len() as f64;
            }
            Event::HandShown { player, viewer, .. } => {
                let text = if self.spectator {
                    format!("{} mira la mano de {}", self.rival_cap(*viewer), self.rival(*player))
                } else {
                    format!("{} mira tu mano", self.rival_cap(*viewer))
                };
                banner(self, text, gold, 1.1);
                step.duration = 0.5;
            }
            Event::Revealed { card, .. } => {
                step.show = vec![*card];
                step.duration = 1.0;
            }
            Event::PreMatchStarted { .. } | Event::PreMatchEnded | Event::GameStarted { .. } => {
                let name = |c| self.name(c);
                if let Some(text) = text::event(&ev, &self.voice(), &name) {
                    banner(self, text, Color32::WHITE, 1.0);
                }
                step.duration = 0.5;
            }
            Event::CountOutRefused { .. } => {
                banner(self, "¡Rechaza el Count Out!".into(), gold, 1.0);
                step.duration = 0.5;
            }
            Event::GameOver(o) => {
                self.banners.push(Banner {
                    text: text::outcome(o, &self.voice()),
                    color: gold,
                    start: now,
                    duration: 4.0,
                });
            }
            _ => {}
        }
        step.duration *= k;
        self.step = Some(step);
    }

    fn computer_moves(&mut self, ui: &Ui, now: f64) {
        if !self.table.computer_to_move() {
            return;
        }
        let wait = self.idle_since + BOT_DELAY * self.speed.factor().max(0.3) - now;
        if wait > 0.0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        let events = self.table.computer_move();
        self.queue.extend(events);
        ui.ctx().request_repaint();
    }

    fn piles(&mut self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
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
        let removed = view.players[p.index()].removed;
        if removed > 0 {
            ui.small(format!("Fuera del juego: {removed}"));
        }
        let under = view.players[p.index()].under;
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
        let r = ui.horizontal_wrapped(|ui| {
            let label = if self.mine(p) { "Tú".into() } else { self.rival_cap(p) };
            ui.label(RichText::new(format!("{label}: {}", star.name)).strong().size(16.0));
            if let Some((card, index)) = info.enforcer
                && let Some(e) = self.db().card(card).enforcers.get(index as usize)
            {
                ui.label(format!("Enforcer: {}", e.name)).on_hover_text(format!(
                    "Mano +{}, Superstar Value +{}. {}",
                    e.hand_size, e.superstar_value, e.text
                ));
            }
            let backstage: Vec<_> = info
                .backstage
                .iter()
                .filter(|(c, _)| self.db().card(*c).kinds.contains(&CardKind::Backstage))
                .collect();
            if !backstage.is_empty() {
                let lines: Vec<String> = backstage
                    .iter()
                    .map(|(c, hidden)| {
                        let def = self.db().card(*c);
                        let state = if *hidden { " (oculta)" } else { "" };
                        format!("{}{state}: {}", def.title, def.text)
                    })
                    .collect();
                ui.label(format!("Backstage: {}", backstage.len())).on_hover_text(lines.join("\n\n"));
            }
            ui.separator();
            ui.label(RichText::new(format!("Fortitude {fortitude}")).color(Color32::from_rgb(255, 210, 90)));
            ui.separator();
            ui.label(format!("Mano {hand}"));
            if backlash > 0 {
                ui.separator();
                ui.label(format!("Backlash {backlash}"));
            }
            if view.active == p && view.outcome.is_none() {
                ui.separator();
                let text = if self.mine(p) { "tu turno" } else { "su turno" };
                ui.label(RichText::new(text).color(Color32::from_rgb(90, 220, 120)));
            }
        });
        self.layout.bar[p.index()] = r.response.rect;
    }

    /// A player's name plate: gold edge for you, red for your rival.
    fn plate(&mut self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
        let edge = if self.mine(p) { theme::GOLD } else { theme::RED };
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(10, 10, 12, 225))
            .stroke(Stroke::new(1.5, edge))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| self.player_bar(ui, view, p));
    }

    /// A player's Ring in `area`: cards overlapping from the top down, like a
    /// pile you can read the titles of.
    fn ring(&mut self, ui: &mut Ui, p: PlayerId, area: Rect) {
        let mine = self.mine(p);
        let edge = if mine { theme::BLUE } else { theme::RED };
        let painter = ui.painter_at(area.expand(2.0));
        painter.rect_filled(area, CornerRadius::same(4), Color32::from_black_alpha(35));
        painter.rect_stroke(area, CornerRadius::same(4), Stroke::new(2.0, edge), StrokeKind::Inside);
        let label = if mine { "TU RING" } else { "RING RIVAL" };
        painter.text(
            area.center_top() + vec2(0.0, 3.0),
            Align2::CENTER_TOP,
            label,
            theme::display_font(14.0),
            Color32::WHITE,
        );
        let body = Rect::from_min_max(area.min + vec2(0.0, 22.0), area.max - vec2(0.0, 6.0));
        let ring = self.display.sides[p.index()].ring.clone();
        let n = ring.len();
        let step = if n > 1 { ((body.height() - cards::SMALL.y) / (n - 1) as f32).clamp(8.0, 34.0) } else { 34.0 };
        let left = body.center().x - cards::SMALL.x / 2.0;
        let clip = ui.painter_at(area);
        for (i, &c) in ring.iter().enumerate() {
            let r = Rect::from_min_size(egui::pos2(left, body.top() + i as f32 * step), cards::SMALL);
            let def = self.def_of(c).clone();
            cards::paint(&clip, r, Some(&def), Look::Normal, 1.0);
            if ui.interact(r.intersect(area), Id::new(("ring-card", c)), egui::Sense::hover()).hovered() {
                cards::set_preview(ui.ctx(), cards::Preview::Card(Box::new(def)));
            }
            self.layout.cards.insert(c, r);
        }
        self.layout.ring[p.index()] = Rect::from_min_size(egui::pos2(left, body.top()), cards::SMALL);
        self.layout.ring_step[p.index()] = step;
    }

    /// The card being played, in the middle of the ring.
    fn in_play(&mut self, ui: &mut Ui, view: &PlayerView, area: Rect) {
        let size = cards::SIZE * ((area.height() - 50.0) / cards::SIZE.y).clamp(0.5, 1.0);
        let rect = Rect::from_center_size(egui::pos2(area.left() + size.x / 2.0 + 16.0, area.center().y), size);
        self.layout.in_play = rect;
        let painter = ui.painter_at(area.expand(6.0));
        painter.text(
            rect.center_top() - vec2(0.0, 4.0),
            Align2::CENTER_BOTTOM,
            "EN JUEGO",
            theme::display_font(15.0),
            Color32::from_gray(60),
        );
        match self.display.in_play {
            Some((c, p)) => {
                let def = self.def_of(c).clone();
                painter.rect_filled(
                    rect.translate(vec2(4.0, 6.0)),
                    CornerRadius::same(6),
                    Color32::from_black_alpha(90),
                );
                cards::paint(&painter, rect, Some(&def), Look::Normal, 1.0);
                if ui.interact(rect, Id::new("in-play"), egui::Sense::hover()).hovered() {
                    cards::set_preview(ui.ctx(), cards::Preview::Card(Box::new(def)));
                }
                let who = if self.mine(p) { "La juegas tú".into() } else { format!("La juega {}", self.rival(p)) };
                let how = match view.in_play {
                    Some(pl) if pl.card == c && self.idle() => format!(" como {}", text::kind(pl.kind)),
                    _ => String::new(),
                };
                let caption = format!("{who}{how}");
                let galley = painter.layout_no_wrap(caption, FontId::proportional(13.0), Color32::WHITE);
                let back = Rect::from_min_size(rect.left_bottom() + vec2(-4.0, 6.0), galley.size() + vec2(8.0, 4.0));
                painter.rect_filled(back, CornerRadius::same(3), Color32::from_black_alpha(190));
                painter.galley(back.min + vec2(4.0, 2.0), galley, Color32::WHITE);
            }
            None => {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(6),
                    Stroke::new(1.5, Color32::from_black_alpha(60)),
                    StrokeKind::Inside,
                );
            }
        }
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
                    if i == side.hand.len() {
                        ui.separator();
                        ui.label("Backlash:");
                    }
                    let Some(c) = *c else {
                        // A spectator sees the backs of the cards.
                        cards::card(ui, None, cards::SMALL * 0.6, Look::Normal);
                        continue;
                    };
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
                self.act(actions[0].clone());
            } else {
                self.kind_menu = Some(c);
            }
        }
    }

    /// What the game is waiting for, with buttons to answer.
    fn prompt(&mut self, ui: &mut Ui, view: &PlayerView, legal: &[Action]) -> Option<Leave> {
        let problem = self.table.online_state().and_then(|o| o.problem.clone());
        if let Some(problem) = &problem {
            ui.colored_label(Color32::from_rgb(240, 110, 110), problem);
        }
        let away = self.table.online_state().and_then(|o| o.lobby.as_ref()).map(|l| l.players.clone());
        for slot in away.iter().flatten().flatten().filter(|s| s.away) {
            if view.outcome.is_none() {
                let minutes = ABANDON_AFTER.as_secs() / 60;
                ui.colored_label(
                    Color32::from_rgb(255, 210, 90),
                    format!(
                        "{} perdió la conexión. Si no vuelve en {minutes} minutos, pierde por abandono.",
                        slot.name
                    ),
                );
            }
        }
        if let (Some(outcome), true) = (&view.outcome, self.idle()) {
            let mut leave = None;
            let local = self.table.online_state().is_none();
            ui.horizontal(|ui| {
                ui.label(RichText::new(text::outcome(outcome, &self.voice())).size(20.0).strong());
                if local && ui.button("Revancha").clicked() {
                    leave = Some(Leave::Rematch);
                }
                if ui.button("Volver al menú").clicked() {
                    leave = Some(Leave::Menu);
                }
            });
            return leave;
        }
        let decision = view.decision.clone().filter(|_| self.idle() && !legal.is_empty());
        let Some(decision) = decision else {
            let mut leave = None;
            ui.horizontal(|ui| {
                let text = match view.waiting_on {
                    _ if !self.idle() => "…".into(),
                    Some(p) if self.mine(p) => "Esperando a la partida…".into(),
                    Some(p) => format!("Esperando a {}…", self.rival(p)),
                    None => "…".into(),
                };
                ui.label(RichText::new(text).italics());
                if (self.spectator || problem.is_some()) && ui.button("Salir").clicked() {
                    leave = Some(Leave::Menu);
                }
            });
            return leave;
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
                        && view.me_public().ringside.contains(card)
                    {
                        let def = self.def_of(*card);
                        let label = format!("Jugar {} desde el Ringside ({})", def.title, text::kind(*kind));
                        if ui.button(label).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::PlayCard { card, kind } = a
                        && let Some((_, host)) = view.under.iter().find(|(c, _)| c == card)
                    {
                        let def = self.def_of(*card);
                        let host = host.map_or("tu Superstar".into(), |h| self.def_of(h).title.clone());
                        let label = format!("Jugar {} de debajo de {} ({})", def.title, host, text::kind(*kind));
                        if ui.button(label).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::UseBackstageCard { card } = a {
                        let def = self.db().card(*card);
                        if ui.button(format!("Usar {} (Backstage)", def.title)).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                    if let Action::UseRingCard { card } = a {
                        let def = self.def_of(*card);
                        let whose = if view.me_public().ring.contains(card) { "" } else { " (del rival)" };
                        if ui.button(format!("Usar {}{whose}", def.title)).on_hover_text(&def.text).clicked() {
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
                        && view.under.iter().any(|(c, _)| c == card)
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
                if legal.contains(&Action::ReverseByVolley) {
                    let n = self.def_of(*card).volley().unwrap_or(0);
                    let label = format!("Revertir el Volley (retirar {n} cartas al azar de tu Ringside)");
                    if ui.button(label).clicked() {
                        chosen = Some(Action::ReverseByVolley);
                    }
                }
                if ui.button("No revertir").clicked() {
                    chosen = Some(Action::DeclineReversal);
                }
            }
            DecisionKind::ChooseEnforcer { who, options } => {
                let whose = if *who == self.me { "tu Superstar" } else { "el rival" };
                ui.label(RichText::new(format!("Elige un Enforcer para {whose}:")).strong());
                for (i, (card, index)) in options.iter().enumerate() {
                    if let Some(e) = self.db().card(*card).enforcers.get(*index as usize) {
                        let label = format!("{} (mano +{}, valor +{})", e.name, e.hand_size, e.superstar_value);
                        if ui.button(label).on_hover_text(&e.text).clicked() {
                            chosen = Some(Action::ChooseOption(i as u8));
                        }
                    }
                }
            }
            DecisionKind::CounterReversal { reversal, .. } => {
                let title = self.def_of(*reversal).title.clone();
                ui.label(RichText::new(format!("El rival te revierte con {title}. Puedes responder:")).strong());
                for a in legal {
                    if let Action::Reverse { card } = a {
                        let def = self.def_of(*card);
                        if ui.button(format!("Responder con {}", def.title)).on_hover_text(&def.text).clicked() {
                            chosen = Some(a.clone());
                        }
                    }
                }
                if ui.button("No responder").clicked() {
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
            self.act(a);
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
            self.act(Action::ChooseCards(chosen));
        }
    }

    fn ringside_window(&mut self, ui: &Ui) {
        let Some(p) = self.ringside else { return };
        let mut open = true;
        let whose = match &self.names {
            _ if self.mine(p) => "Tu Ringside".into(),
            Some(names) => format!("Ringside de {}", names[p.index()]),
            None => "Ringside del rival".into(),
        };
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

/// This computer's address on its local network. Nothing is sent: connecting
/// a UDP socket only picks the outgoing interface.
fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

fn card_of(action: &Action) -> Option<CardId> {
    match action {
        Action::PlayCard { card, .. } | Action::Reverse { card } => Some(*card),
        _ => None,
    }
}

/// Paints the ring from above in `rect` (apron, ropes, turnbuckles and canvas)
/// and returns the canvas.
fn paint_ring(ui: &Ui, rect: Rect) -> Rect {
    let look = theme::get(ui.ctx());
    let p = ui.painter_at(rect.expand(4.0));
    p.rect_filled(rect.translate(vec2(0.0, 6.0)), CornerRadius::same(8), Color32::from_black_alpha(160));
    p.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(18, 17, 20));
    p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, Color32::from_rgb(70, 70, 78)), StrokeKind::Inside);
    for (y, align) in [(rect.top() + 4.0, Align2::CENTER_TOP), (rect.bottom() - 4.0, Align2::CENTER_BOTTOM)] {
        p.text(
            egui::pos2(rect.center().x, y),
            align,
            "OPENWWX",
            theme::display_font(18.0),
            Color32::from_rgb(70, 22, 24),
        );
    }
    let canvas = rect.shrink(46.0);
    let uv = Rect::from_min_size(egui::pos2(0.0, 0.0), canvas.size() / 256.0);
    p.image(look.canvas.id(), canvas, uv, Color32::WHITE);
    // Shade near the ropes.
    for i in 0..8 {
        let a = 46 - i * 5;
        p.rect_stroke(
            canvas.shrink(i as f32 * 3.0),
            CornerRadius::ZERO,
            Stroke::new(3.0, Color32::from_black_alpha(a as u8)),
            StrokeKind::Inside,
        );
    }
    // Three ropes, outside in.
    let ropes = [Color32::from_rgb(200, 26, 30), Color32::from_rgb(235, 235, 235), Color32::from_rgb(30, 70, 190)];
    for (k, color) in ropes.into_iter().enumerate() {
        let r = canvas.expand(22.0 - k as f32 * 8.0);
        p.rect_stroke(
            r.translate(vec2(2.0, 4.0)),
            CornerRadius::ZERO,
            Stroke::new(5.0, Color32::from_black_alpha(110)),
            StrokeKind::Middle,
        );
        p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(5.0, color), StrokeKind::Middle);
        p.rect_stroke(
            r.shrink(1.2),
            CornerRadius::ZERO,
            Stroke::new(1.0, Color32::from_white_alpha(70)),
            StrokeKind::Middle,
        );
    }
    let post = canvas.expand(18.0);
    for corner in [post.left_top(), post.right_top(), post.left_bottom(), post.right_bottom()] {
        let r = Rect::from_center_size(corner, egui::Vec2::splat(64.0));
        p.image(
            look.turnbuckle.id(),
            r,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    canvas
}
