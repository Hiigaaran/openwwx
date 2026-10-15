//! What the board shows while the engine's events are being animated.
//!
//! The engine applies a whole action at once and reports what happened as a
//! list of events. The app replays those events one by one on a [`Display`]
//! (its own copy of the visible zones), so cards can fly from zone to zone
//! instead of jumping to where they end up.

use std::collections::HashMap;

use eframe::egui::{Pos2, Rect};
use rawdeal_core::{CardId, PlayerId, PlayerView, Zone};

/// The visible state of one player's areas. Cards you cannot see are `None`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Side {
    pub arsenal: usize,
    pub hand: Vec<Option<CardId>>,
    pub backlash: Vec<Option<CardId>>,
    pub ring: Vec<CardId>,
    pub ringside: Vec<CardId>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Display {
    pub sides: [Side; 2],
    pub in_play: Option<(CardId, PlayerId)>,
}

impl Display {
    pub fn from_view(view: &PlayerView) -> Self {
        let side = |p: PlayerId| {
            let info = &view.players[p.index()];
            let mine = p == view.me;
            Side {
                arsenal: info.arsenal_count,
                hand: match (mine, &view.opponent_hand) {
                    (true, _) => view.hand.iter().map(|&c| Some(c)).collect(),
                    (false, Some(cards)) => cards.iter().map(|&c| Some(c)).collect(),
                    (false, None) => vec![None; info.hand_count],
                },
                backlash: if mine {
                    view.backlash.iter().map(|&c| Some(c)).collect()
                } else {
                    vec![None; info.backlash_count]
                },
                ring: info.ring.clone(),
                ringside: info.ringside.clone(),
            }
        };
        Display { sides: [side(PlayerId::ONE), side(PlayerId::TWO)], in_play: view.in_play.map(|p| (p.card, p.player)) }
    }

    /// Takes `card` out of `zone`. Hidden cards are taken from the top (or any
    /// unknown card in a hand).
    pub fn take(&mut self, p: PlayerId, zone: Zone, card: Option<CardId>) {
        let side = &mut self.sides[p.index()];
        let from_list = |list: &mut Vec<Option<CardId>>| {
            let i = card
                .and_then(|c| list.iter().position(|&x| x == Some(c)))
                .or_else(|| list.iter().rposition(|x| x.is_none()));
            match i {
                Some(i) => {
                    list.remove(i);
                    true
                }
                None => list.pop().is_some(),
            }
        };
        match zone {
            Zone::Arsenal => side.arsenal = side.arsenal.saturating_sub(1),
            Zone::Hand => {
                if !from_list(&mut side.hand) {
                    from_list(&mut side.backlash);
                }
            }
            Zone::Backlash => {
                if !from_list(&mut side.backlash) {
                    from_list(&mut side.hand);
                }
            }
            Zone::Ring => side.ring.retain(|&c| Some(c) != card),
            Zone::Ringside => match card {
                Some(c) => side.ringside.retain(|&x| x != c),
                None => {
                    side.ringside.pop();
                }
            },
            Zone::InPlay => self.in_play = None,
        }
    }

    /// Takes a card being played from wherever it is in `p`'s hand or Backlash.
    pub fn take_played(&mut self, p: PlayerId, card: CardId) {
        let side = &self.sides[p.index()];
        let zone = if side.backlash.contains(&Some(card)) { Zone::Backlash } else { Zone::Hand };
        self.take(p, zone, Some(card));
    }

    pub fn put(&mut self, p: PlayerId, zone: Zone, card: Option<CardId>) {
        let side = &mut self.sides[p.index()];
        match zone {
            Zone::Arsenal => side.arsenal += 1,
            Zone::Hand => side.hand.push(card),
            Zone::Backlash => side.backlash.push(card),
            Zone::Ring => side.ring.extend(card),
            Zone::Ringside => side.ringside.extend(card),
            Zone::InPlay => self.in_play = card.map(|c| (c, p)),
        }
    }
}

/// Where things were drawn in the last frame, so animations know where to
/// start and end.
pub struct Layout {
    pub cards: HashMap<CardId, Rect>,
    pub arsenal: [Rect; 2],
    pub ringside: [Rect; 2],
    pub hand: [Rect; 2],
    pub ring: [Rect; 2],
    pub bar: [Rect; 2],
    pub in_play: Rect,
    pub board: Rect,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            cards: HashMap::new(),
            arsenal: [Rect::NOTHING; 2],
            ringside: [Rect::NOTHING; 2],
            hand: [Rect::NOTHING; 2],
            ring: [Rect::NOTHING; 2],
            bar: [Rect::NOTHING; 2],
            in_play: Rect::NOTHING,
            board: Rect::NOTHING,
        }
    }
}

impl Layout {
    /// Where a card goes when it lands in `zone`.
    pub fn target(&self, p: PlayerId, zone: Zone, d: &Display, size: eframe::egui::Vec2) -> Rect {
        let i = p.index();
        let after_last = |ids: &mut dyn Iterator<Item = CardId>, area: Rect| {
            let last = ids.filter_map(|c| self.cards.get(&c)).last();
            match last {
                Some(r) => Rect::from_min_size(Pos2::new(r.right() + 8.0, r.top()), size),
                None => Rect::from_min_size(area.min, size),
            }
        };
        match zone {
            Zone::Arsenal | Zone::Backlash => self.arsenal[i],
            Zone::Ringside => self.ringside[i],
            Zone::InPlay => self.in_play,
            Zone::Ring => after_last(&mut d.sides[i].ring.iter().copied(), self.ring[i]),
            Zone::Hand => after_last(&mut d.sides[i].hand.iter().flatten().copied(), self.hand[i]),
        }
    }

    /// Where `card` is now, or else where its zone is.
    pub fn source(&self, p: PlayerId, zone: Zone, card: Option<CardId>, size: eframe::egui::Vec2) -> Rect {
        if let Some(r) = card.and_then(|c| self.cards.get(&c)) {
            return *r;
        }
        let i = p.index();
        match zone {
            Zone::Arsenal | Zone::Backlash => self.arsenal[i],
            Zone::Ringside => self.ringside[i],
            Zone::InPlay => self.in_play,
            Zone::Ring => Rect::from_min_size(self.ring[i].min, size),
            Zone::Hand => Rect::from_center_size(self.hand[i].center(), size),
        }
    }
}

/// A card moving across the board.
pub struct Fly {
    pub card: Option<CardId>,
    pub from: Rect,
    pub to: Rect,
    /// Reddish tint, for cards being reversed.
    pub hurt: bool,
}

/// One blocking step of the replay.
pub struct Step {
    pub start: f64,
    pub duration: f64,
    pub fly: Option<Fly>,
    /// Cards shown large in the middle of the board (a revealed card, the
    /// opponent's hand).
    pub show: Vec<CardId>,
    /// Applied to the [`Display`] when the step ends.
    pub land: Option<(PlayerId, Zone, Option<CardId>)>,
}

/// Big text over the board.
pub struct Banner {
    pub text: String,
    pub color: eframe::egui::Color32,
    pub start: f64,
    pub duration: f64,
}

/// Small text floating up from a point ("-1").
pub struct Float {
    pub text: String,
    pub at: Pos2,
    pub start: f64,
}

pub fn ease(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (1.0 - (1.0 - t).powi(3)) as f32
}
