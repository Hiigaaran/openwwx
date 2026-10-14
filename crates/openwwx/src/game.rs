//! The match screen: you against the computer.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, ScrollArea, Ui};
use rawdeal_ai::{Bot, RandomBot};
use rawdeal_core::{
    Action, CardDb, CardDef, CardId, Decision, DecisionKind, Engine, Event, GameState, PlayerId, PlayerView,
};

use crate::cards::{self, Look};
use crate::text;

/// Seconds between the computer's moves, so you can follow them.
const BOT_DELAY: f64 = 0.6;

pub enum Leave {
    Stay,
    /// Back to the start screen.
    Menu,
    /// Same Superstars and decks, new shuffle.
    Rematch,
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
    next_bot_move: f64,
}

impl Match {
    pub fn new(db: Arc<CardDb>, state: GameState, events: Vec<Event>, seed: u64) -> Self {
        let mut m = Match {
            engine: Engine::new(db),
            state,
            bot: RandomBot::new(seed ^ 0x5eed),
            me: PlayerId::ONE,
            log: vec![],
            selected: vec![],
            kind_menu: None,
            ringside: None,
            next_bot_move: 0.0,
        };
        m.record(&events);
        m
    }

    fn db(&self) -> &CardDb {
        self.engine.db()
    }

    fn view(&self) -> PlayerView {
        self.engine.view(&self.state, self.me)
    }

    fn def<'a>(&'a self, view: &PlayerView, card: CardId) -> Option<&'a CardDef> {
        view.def_of(card).map(|d| self.db().card(d))
    }

    fn record(&mut self, events: &[Event]) {
        let view = self.view();
        let name = |c: CardId| match self.def(&view, c) {
            Some(d) => format!("{} [F{} D{}]", d.title, d.fortitude, d.damage),
            None => "una carta".into(),
        };
        let lines: Vec<String> =
            events.iter().filter_map(|ev| text::event(&ev.redacted_for(self.me), self.me, &name)).collect();
        self.log.extend(lines);
    }

    fn act(&mut self, player: PlayerId, action: Action) {
        match self.engine.apply(&mut self.state, player, action) {
            Ok(events) => self.record(&events),
            Err(e) => self.log.push(format!("(acción rechazada: {e})")),
        }
        self.selected.clear();
        self.kind_menu = None;
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Leave {
        self.computer_moves(ui);
        let view = self.view();
        let legal = self.engine.legal_actions(&self.state, self.me);
        let mut leave = Leave::Stay;

        egui::Panel::right("log").default_size(320.0).show(ui, |ui| {
            ui.heading("Relato");
            ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                for line in &self.log {
                    ui.label(line);
                }
            });
        });
        egui::Panel::top("opponent").show(ui, |ui| {
            ui.add_space(4.0);
            self.player_bar(ui, &view, self.me.opponent());
            ui.horizontal(|ui| {
                for _ in 0..view.opponent().hand_count.min(12) {
                    cards::card(ui, None, cards::SMALL * 0.6, Look::Normal);
                }
            });
            ui.add_space(4.0);
        });
        egui::Panel::bottom("me").show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(l) = self.prompt(ui, &view, &legal) {
                leave = l;
            }
            ui.add_space(4.0);
            self.hand(ui, &view, &legal);
            self.player_bar(ui, &view, self.me);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.ring(ui, &view, self.me.opponent());
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("En juego:").strong());
                match view.in_play {
                    Some(p) => {
                        let def = self.def(&view, p.card);
                        cards::card(ui, def, cards::SIZE, Look::Normal);
                        let who = if p.player == self.me { "La juegas" } else { "La juega el rival" };
                        ui.label(format!("{who} como {}", text::kind(p.kind)));
                    }
                    None => {
                        ui.label("—");
                    }
                }
            });
            ui.separator();
            self.ring(ui, &view, self.me);
        });
        self.choose_cards_window(ui, &view);
        self.ringside_window(ui, &view);
        leave
    }

    fn computer_moves(&mut self, ui: &Ui) {
        let Some(Decision { player, .. }) = self.state.decision.clone() else { return };
        if player == self.me || self.state.is_over() {
            return;
        }
        let now = ui.input(|i| i.time);
        if now < self.next_bot_move {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(self.next_bot_move - now));
            return;
        }
        let legal = self.engine.legal_actions(&self.state, player);
        let action = self.bot.choose(&self.engine, &self.engine.view(&self.state, player), &legal);
        self.act(player, action);
        self.next_bot_move = now + BOT_DELAY;
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(BOT_DELAY));
    }

    fn player_bar(&mut self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
        let info = &view.players[p.index()];
        let star = self.db().superstar(info.superstar).clone();
        ui.horizontal(|ui| {
            let label = if p == self.me { "Tú" } else { "Rival" };
            ui.label(RichText::new(format!("{label}: {}", star.name)).strong().size(16.0)).on_hover_text(format!(
                "Hand Size {} · Superstar Value {}\n{}",
                star.hand_size, star.superstar_value, star.text
            ));
            ui.separator();
            ui.label(RichText::new(format!("Fortitude {}", info.fortitude)).color(Color32::from_rgb(255, 210, 90)));
            ui.separator();
            ui.label(format!("Arsenal {}", info.arsenal_count));
            ui.separator();
            ui.label(format!("Mano {}", info.hand_count));
            ui.separator();
            if ui.button(format!("Ringside {}", info.ringside.len())).clicked() {
                self.ringside = if self.ringside == Some(p) { None } else { Some(p) };
            }
            if info.backlash_count > 0 {
                ui.separator();
                ui.label(format!("Backlash {}", info.backlash_count));
            }
            if view.active == p && !self.state.is_over() {
                ui.separator();
                ui.label(
                    RichText::new(if p == self.me { "tu turno" } else { "su turno" })
                        .color(Color32::from_rgb(90, 220, 120)),
                );
            }
        });
    }

    fn ring(&self, ui: &mut Ui, view: &PlayerView, p: PlayerId) {
        let ring = &view.players[p.index()].ring;
        ui.label(format!("Ring de {}", if p == self.me { "tu Superstar" } else { "tu rival" }));
        ScrollArea::horizontal().id_salt(("ring", p.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ring.is_empty() {
                    ui.add_sized(cards::SMALL, egui::Label::new("(vacío)"));
                }
                for &c in ring {
                    cards::card(ui, self.def(view, c), cards::SMALL, Look::Normal);
                }
            });
        });
    }

    /// Your hand (and Backlash deck), with the cards you can use highlighted.
    fn hand(&mut self, ui: &mut Ui, view: &PlayerView, legal: &[Action]) {
        let mut clicked = None;
        ScrollArea::horizontal().id_salt("hand").show(ui, |ui| {
            ui.horizontal(|ui| {
                for &c in view.hand.iter().chain(&view.backlash) {
                    let usable = legal.iter().any(|a| card_of(a) == Some(c));
                    let look = if self.kind_menu == Some(c) {
                        Look::Selected
                    } else if usable {
                        Look::Playable
                    } else {
                        Look::Normal
                    };
                    if view.backlash.first() == Some(&c) {
                        ui.separator();
                        ui.label("Backlash:");
                    }
                    if cards::card(ui, self.def(view, c), cards::SIZE, look).clicked() && usable {
                        clicked = Some(c);
                    }
                }
            });
        });
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
        if let Some(outcome) = &self.state.outcome {
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
        let Some(decision) = view.decision.clone() else {
            ui.label(RichText::new("Esperando al rival…").italics());
            return None;
        };
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| match &decision.kind {
            DecisionKind::Main => {
                if let Some(card) = self.kind_menu {
                    let title = self.def(view, card).map(|d| d.title.clone()).unwrap_or_default();
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
                if ui.button("Terminar turno").clicked() {
                    chosen = Some(Action::EndTurn);
                }
            }
            DecisionKind::Reversal { card, .. } => {
                let title = self.def(view, *card).map(|d| d.title.clone()).unwrap_or_default();
                ui.label(
                    RichText::new(format!("El rival juega {title}. Puedes revertirla con una carta marcada.")).strong(),
                );
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
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ScrollArea::both().max_height(420.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for &c in &options {
                            let picked = self.selected.contains(&c);
                            let look = if picked { Look::Selected } else { Look::Playable };
                            if cards::card(ui, self.def(view, c), cards::SIZE, look).clicked() {
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

    fn ringside_window(&mut self, ui: &Ui, view: &PlayerView) {
        let Some(p) = self.ringside else { return };
        let mut open = true;
        let whose = if p == self.me { "Tu Ringside" } else { "Ringside del rival" };
        egui::Window::new(whose).open(&mut open).default_width(640.0).show(ui.ctx(), |ui| {
            ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &c in view.players[p.index()].ringside.iter().rev() {
                        cards::card(ui, self.def(view, c), cards::SMALL, Look::Normal);
                    }
                });
            });
        });
        if !open {
            self.ringside = None;
        }
    }
}

fn card_of(action: &Action) -> Option<CardId> {
    match action {
        Action::PlayCard { card, .. } | Action::Reverse { card } => Some(*card),
        _ => None,
    }
}
