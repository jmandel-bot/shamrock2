//! Local (per-player) state: view rotation, pointer interaction, rendering.
//! Rendering emits a flat f32 command buffer that the JS shim replays onto a
//! Canvas2D context; nothing here knows about the DOM.

use crate::model::*;
use core::f32::consts::FRAC_PI_2;

// Outbox event kinds (to JS).
pub const EV_NET: u8 = 1;
pub const EV_EDIT_CLUE: u8 = 2;
pub const EV_STATUS: u8 = 3;

// Draw commands.
const C_XFORM: f32 = 1.0;
const C_RRECT: f32 = 2.0;
const C_CIRCLE: f32 = 3.0;
const C_TEXT: f32 = 4.0;
const C_CLEAR: f32 = 5.0;

const DRAG_THRESHOLD: f32 = 6.0;
pub const WORD_SIZE: f32 = 24.0;
const CURSOR_TTL_MS: f64 = 4000.0;

/// 2D affine matrix in canvas `setTransform(a, b, c, d, e, f)` order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl M {
    pub const ID: M = M { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
    pub fn t(x: f32, y: f32) -> M {
        M { e: x, f: y, ..M::ID }
    }
    /// Clockwise on screen (y points down).
    pub fn r(th: f32) -> M {
        let (s, c) = th.sin_cos();
        M { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }
    pub fn s(k: f32) -> M {
        M { a: k, d: k, ..M::ID }
    }
    /// self ∘ o (apply `o` first).
    pub fn then(self, o: M) -> M {
        M {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
    pub fn inv(self) -> M {
        let det = self.a * self.d - self.b * self.c;
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        M { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) }
    }
    pub fn angle(self) -> f32 {
        self.b.atan2(self.a)
    }
}

fn quarter(q: i32) -> u8 {
    q.rem_euclid(4) as u8
}

pub fn peer_color(peer: u32) -> u32 {
    const P: [u32; 8] = [
        0xe0457bff, 0x2f80edff, 0xf2994aff, 0x9b51e0ff, 0x27ae60ff, 0xeb5757ff, 0x00a3a3ff, 0xd4a017ff,
    ];
    P[(peer.wrapping_mul(2654435761) >> 29) as usize]
}

struct Press {
    sx: f32,
    sy: f32,
    card: Option<usize>,
    petal: Option<u8>,
    /// Card centre minus pointer, in world coords.
    grab: (f32, f32),
    origin: Card,
    moved: bool,
}

pub struct RemoteCursor {
    pub peer: u32,
    pub kind: u8,
    pub x: f32,
    pub y: f32,
    pub seen: f64,
}

pub struct App {
    pub shared: Shared,
    pub me: u32,
    clock: u64,
    rng: Rng,
    /// Target view rotation in quarter turns (local only).
    pub q: i32,
    /// Animated view angle in radians.
    pub angle: f32,
    pub w: f32,
    pub h: f32,
    pub dpr: f32,
    press: Option<Press>,
    pub cursors: Vec<RemoteCursor>,
    my_cursor: (u8, f32, f32),
    cursor_dirty: bool,
    last_cursor_ms: f64,
    dirty_cards: u8,
    check: Option<([bool; NSLOTS], f64)>,
    now: f64,
    pub outbox: Vec<u8>,
    pub cmds: Vec<f32>,
    pub strs: Vec<u8>,
}

impl App {
    pub fn new(seed: u64) -> App {
        let mut rng = Rng::new(seed);
        let me = (rng.next() as u32).max(1);
        App {
            shared: Shared::default(),
            me,
            clock: 0,
            rng,
            q: 0,
            angle: 0.0,
            w: 1200.0,
            h: 900.0,
            dpr: 1.0,
            press: None,
            cursors: Vec::new(),
            my_cursor: (0, 0.0, 0.0),
            cursor_dirty: false,
            last_cursor_ms: 0.0,
            dirty_cards: 0,
            check: None,
            now: 0.0,
            outbox: Vec::new(),
            cmds: Vec::new(),
            strs: Vec::new(),
        }
    }

    fn stamp(&mut self) -> Stamp {
        self.clock += 1;
        Stamp { clock: self.clock, peer: self.me }
    }

    // ---- outbox -----------------------------------------------------------

    fn event(&mut self, kind: u8, payload: &[u8]) {
        self.outbox.push(kind);
        self.outbox.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        self.outbox.extend_from_slice(payload);
    }

    fn send(&mut self, msgs: &[Msg]) {
        let mut b = Vec::new();
        for m in msgs {
            encode(m, &mut b);
        }
        self.event(EV_NET, &b);
    }

    fn status(&mut self, s: &str) {
        self.event(EV_STATUS, s.as_bytes());
    }

    /// Apply a locally-originated op and broadcast it.
    fn commit(&mut self, op: Op) {
        self.shared.apply(&op);
        self.send(&[Msg::Op(op)]);
    }

    /// Local card write; broadcast is coalesced to once per frame.
    fn set_card(&mut self, i: usize, loc: Loc, rot: u8, held: u32) {
        let stamp = self.stamp();
        self.shared.apply(&Op::Card(i as u8, Card { loc, rot, held, stamp }));
        self.dirty_cards |= 1 << i;
    }

    fn flush_cards(&mut self) {
        if self.dirty_cards == 0 {
            return;
        }
        let msgs: Vec<Msg> = (0..NCARDS)
            .filter(|i| self.dirty_cards & (1 << i) != 0)
            .map(|i| Msg::Op(Op::Card(i as u8, self.shared.cards[i])))
            .collect();
        self.dirty_cards = 0;
        self.send(&msgs);
    }

    // ---- network in ---------------------------------------------------------

    pub fn on_packet(&mut self, bytes: &[u8]) {
        for m in decode(bytes) {
            match m {
                Msg::Op(op) => {
                    self.clock = self.clock.max(op.stamp().clock);
                    self.shared.apply(&op);
                }
                Msg::Cursor { peer, kind, x, y } if peer != self.me => {
                    let now = self.now;
                    match self.cursors.iter_mut().find(|c| c.peer == peer) {
                        Some(c) => *c = RemoteCursor { peer, kind, x, y, seen: now },
                        None => self.cursors.push(RemoteCursor { peer, kind, x, y, seen: now }),
                    }
                }
                _ => {}
            }
        }
    }

    pub fn on_peer_join(&mut self) {
        let msgs: Vec<Msg> = self.shared.all_ops().into_iter().map(Msg::Op).collect();
        self.send(&msgs);
    }

    // ---- geometry -----------------------------------------------------------

    /// Tall screens show the whole (landscape) world turned a quarter turn so
    /// it fills the screen. Purely presentational: shared coords are unchanged.
    pub fn portrait(&self) -> bool {
        self.h > self.w * 1.15
    }

    /// World → CSS pixels.
    pub fn screen(&self) -> M {
        let k = self.scale();
        let fit = M::t(self.w / 2.0, self.h / 2.0).then(M::s(k));
        let fit = if self.portrait() { fit.then(M::r(FRAC_PI_2)) } else { fit };
        fit.then(M::t(-CX, -CY))
    }

    pub fn scale(&self) -> f32 {
        let (w, h) = if self.portrait() { (self.h, self.w) } else { (self.w, self.h) };
        (w / WORLD_W).min(h / WORLD_H)
    }

    /// Board coords → world.
    pub fn board(&self) -> M {
        M::t(CX, CY).then(M::r(self.angle))
    }

    fn quarter_turn(r: u8) -> f32 {
        r as f32 * FRAC_PI_2
    }

    /// Card-local coords → world.
    pub fn card_frame(&self, c: &Card) -> M {
        let r = M::r(Self::quarter_turn(c.rot));
        match c.loc {
            Loc::Table { x, y } => M::t(x, y).then(r),
            Loc::Slot(s) => {
                let (x, y) = slot_pos(s as usize);
                self.board().then(M::t(x, y)).then(r)
            }
            Loc::Float { x, y } => self.board().then(M::t(x, y)).then(r),
        }
    }

    pub fn to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        self.screen().inv().apply(sx, sy)
    }

    /// Back-to-front draw order: slotted, table, floating, then my drag.
    fn draw_order(&self, cards: &[Card; NCARDS]) -> Vec<usize> {
        let dragging = self.press.as_ref().filter(|p| p.moved).and_then(|p| p.card);
        let rank = |i: usize| -> u8 {
            if Some(i) == dragging {
                return 4;
            }
            match cards[i].loc {
                Loc::Slot(_) => 0,
                Loc::Table { .. } if cards[i].held == 0 => 1,
                _ => 2,
            }
        };
        let mut v: Vec<usize> = (0..NCARDS).collect();
        v.sort_by_key(|&i| rank(i));
        v
    }

    pub fn hit_card(&self, wx: f32, wy: f32) -> Option<usize> {
        let (cards, _) = self.shared.layout();
        self.draw_order(&cards).into_iter().rev().find(|&i| {
            let (lx, ly) = self.card_frame(&cards[i]).inv().apply(wx, wy);
            lx.abs() <= HALF && ly.abs() <= HALF
        })
    }

    fn petal_frame(&self, d: u8) -> M {
        self.board().then(M::r(Self::quarter_turn(d)))
    }

    pub fn hit_petal(&self, wx: f32, wy: f32) -> Option<u8> {
        (0..4).find(|&d| {
            let (x, y) = self.petal_frame(d).inv().apply(wx, wy);
            x.abs() <= 150.0 && (-240.0..=-165.0).contains(&y)
        })
    }

    // ---- input --------------------------------------------------------------

    pub fn pointer_down(&mut self, sx: f32, sy: f32) {
        let (wx, wy) = self.to_world(sx, sy);
        let card = self.hit_card(wx, wy);
        let (cards, _) = self.shared.layout();
        let (grab, origin) = match card {
            Some(i) => {
                let (cx, cy) = self.card_frame(&cards[i]).apply(0.0, 0.0);
                ((cx - wx, cy - wy), cards[i])
            }
            None => ((0.0, 0.0), default_card(0)),
        };
        let petal = if card.is_none() { self.hit_petal(wx, wy) } else { None };
        self.press = Some(Press { sx, sy, card, petal, grab, origin, moved: false });
    }

    pub fn pointer_move(&mut self, sx: f32, sy: f32) {
        let (wx, wy) = self.to_world(sx, sy);
        let (bx, by) = self.board().inv().apply(wx, wy);
        let over_board = bx.abs() <= BOARD_REACH && by.abs() <= BOARD_REACH;
        self.my_cursor = if over_board { (2, bx, by) } else { (1, wx, wy) };
        self.cursor_dirty = true;

        let Some(p) = self.press.as_mut() else { return };
        if !p.moved && (sx - p.sx).hypot(sy - p.sy) < DRAG_THRESHOLD {
            return;
        }
        p.moved = true;
        let Some(i) = p.card else { return };
        let (cx, cy) = (wx + p.grab.0, wy + p.grab.1);
        let cur = self.shared.layout().0[i];
        let in_board_frame = !matches!(cur.loc, Loc::Table { .. });
        let (bx, by) = self.board().inv().apply(cx, cy);
        let to_board = bx.abs() <= BOARD_REACH && by.abs() <= BOARD_REACH;
        // Keep the card's on-screen orientation when it changes frame.
        let rot = match (in_board_frame, to_board) {
            (false, true) => quarter(cur.rot as i32 - self.q),
            (true, false) => quarter(cur.rot as i32 + self.q),
            _ => cur.rot,
        };
        let loc = if to_board {
            Loc::Float { x: bx, y: by }
        } else {
            Loc::Table { x: cx.clamp(HALF, WORLD_W - HALF), y: cy.clamp(HALF, WORLD_H - HALF) }
        };
        let me = self.me;
        self.set_card(i, loc, rot, me);
    }

    pub fn pointer_up(&mut self, sx: f32, sy: f32) {
        self.pointer_move(sx, sy);
        let Some(p) = self.press.take() else { return };
        match (p.card, p.moved) {
            (Some(i), true) => self.drop_card(i, p.origin),
            (Some(i), false) => {
                let c = self.shared.layout().0[i];
                self.set_card(i, c.loc, (c.rot + 1) % 4, 0);
            }
            (None, false) => {
                if let Some(d) = p.petal {
                    self.request_clue_edit(d);
                }
            }
            (None, true) => {}
        }
        self.flush_cards();
    }

    fn drop_card(&mut self, i: usize, origin: Card) {
        let (cards, owner) = self.shared.layout();
        let c = cards[i];
        match c.loc {
            Loc::Float { x, y } => {
                let s = nearest_slot(x, y);
                if let Some(k) = owner[s as usize].filter(|&k| k != i) {
                    // Swap: the occupant goes where the dragged card came from.
                    let occ = cards[k];
                    match origin.loc {
                        Loc::Slot(_) => self.set_card(k, origin.loc, occ.rot, 0),
                        _ => {
                            let loc = match origin.loc {
                                Loc::Table { .. } => origin.loc,
                                _ => {
                                    let (x, y) = table_home(k);
                                    Loc::Table { x, y }
                                }
                            };
                            self.set_card(k, loc, quarter(occ.rot as i32 + self.q), 0);
                        }
                    }
                }
                self.set_card(i, Loc::Slot(s), c.rot, 0);
            }
            loc => self.set_card(i, loc, c.rot, 0),
        }
    }

    fn request_clue_edit(&mut self, d: u8) {
        let s = self.screen();
        let f = s.then(self.petal_frame(d)).then(M::t(0.0, -204.0));
        let (x, y) = f.apply(0.0, 0.0);
        let k = self.scale();
        let mut p = vec![d];
        for v in [x, y, 290.0 * k, 56.0 * k, f.angle()] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(self.shared.clue(d as usize).as_bytes());
        self.event(EV_EDIT_CLUE, &p);
    }

    pub fn set_clue(&mut self, d: u8, text: &str) {
        if d as usize >= NSLOTS {
            return;
        }
        let stamp = self.stamp();
        self.commit(Op::Clue(d, Clue { text: clamp_clue(text), stamp }));
    }

    pub fn rotate_view(&mut self, dir: i32) {
        self.q += dir.signum();
    }

    /// 1 deal, 2 save answer, 3 scramble, 4 check.
    pub fn action(&mut self, code: u32) {
        match code {
            1 => {
                let stamp = self.stamp();
                let seed = self.rng.next() as u32;
                self.commit(Op::Deal(Deal { seed, stamp }));
                self.check = None;
                self.status("New cards dealt.");
            }
            2 => {
                let (cards, owner) = self.shared.layout();
                if owner.iter().any(|o| o.is_none()) {
                    return self.status("Fill all four slots first.");
                }
                let slots = core::array::from_fn(|s| {
                    let k = owner[s].unwrap();
                    (k as u8, cards[k].rot)
                });
                let stamp = self.stamp();
                self.commit(Op::Solution(Solution { slots, stamp }));
                self.status("Answer saved. Scramble, then let the others solve it.");
            }
            3 => {
                let mut order: Vec<usize> = (0..NCARDS).collect();
                for k in (1..NCARDS).rev() {
                    order.swap(k, self.rng.next() as usize % (k + 1));
                }
                for (i, &home) in order.iter().enumerate() {
                    let (x, y) = table_home(home);
                    let rot = (self.rng.next() % 4) as u8;
                    self.set_card(i, Loc::Table { x, y }, rot, 0);
                }
                self.flush_cards();
                self.check = None;
            }
            4 => {
                let Some(sol) = self.shared.solution() else {
                    return self.status("No answer saved for this deal.");
                };
                let (cards, owner) = self.shared.layout();
                let ok: [bool; NSLOTS] = core::array::from_fn(|s| {
                    owner[s].is_some_and(|k| (k as u8, cards[k].rot) == sol[s])
                });
                let n = ok.iter().filter(|b| **b).count();
                self.check = Some((ok, self.now + 4000.0));
                let msg = if n == 4 { "All four correct! 🍀".to_string() } else { format!("{n}/4 correct.") };
                self.status(&msg);
            }
            _ => {}
        }
    }

    // ---- frame --------------------------------------------------------------

    pub fn frame(&mut self, now: f64) {
        let dt = ((now - self.now) / 1000.0).clamp(0.0, 0.1) as f32;
        self.now = now;
        let target = self.q as f32 * FRAC_PI_2;
        let diff = target - self.angle;
        self.angle = if diff.abs() < 1e-3 { target } else { self.angle + diff * (1.0 - (-dt * 14.0).exp()) };

        self.flush_cards();
        if self.cursor_dirty && now - self.last_cursor_ms >= 33.0 {
            self.cursor_dirty = false;
            self.last_cursor_ms = now;
            let (kind, x, y) = self.my_cursor;
            let me = self.me;
            self.send(&[Msg::Cursor { peer: me, kind, x, y }]);
        }
        self.cursors.retain(|c| now - c.seen < CURSOR_TTL_MS);
        if self.check.is_some_and(|(_, until)| now > until) {
            self.check = None;
        }
        self.render();
    }

    /// Skip the animation (tests / harness).
    pub fn settle(&mut self) {
        self.angle = self.q as f32 * FRAC_PI_2;
    }

    // ---- rendering ----------------------------------------------------------

    fn xform(&mut self, m: M) {
        let d = M::s(self.dpr).then(m);
        self.cmds.extend_from_slice(&[C_XFORM, d.a, d.b, d.c, d.d, d.e, d.f]);
    }

    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: u32, stroke: u32, lw: f32) {
        self.cmds.extend_from_slice(&[C_RRECT, x, y, w, h, r, f32::from_bits(fill), f32::from_bits(stroke), lw]);
    }

    fn circle(&mut self, x: f32, y: f32, r: f32, fill: u32, stroke: u32, lw: f32) {
        self.cmds.extend_from_slice(&[C_CIRCLE, x, y, r, f32::from_bits(fill), f32::from_bits(stroke), lw]);
    }

    fn text(&mut self, x: f32, y: f32, size: f32, color: u32, bold: bool, s: &str, maxw: f32) {
        let off = self.strs.len() as f32;
        self.strs.extend_from_slice(s.as_bytes());
        self.cmds.extend_from_slice(&[
            C_TEXT,
            x,
            y,
            size,
            f32::from_bits(color),
            if bold { 1.0 } else { 0.0 },
            off,
            s.len() as f32,
            maxw,
        ]);
    }

    pub fn render(&mut self) {
        self.cmds.clear();
        self.strs.clear();
        self.cmds.extend_from_slice(&[C_CLEAR, f32::from_bits(0x1f3b2dff)]);
        let scr = self.screen();
        let board = scr.then(self.board());

        // Table area.
        self.xform(scr);
        self.rrect(0.0, 0.0, WORLD_W, WORLD_H, 24.0, 0x2a4d3bff, 0, 0.0);

        // Board: base + four clue petals.
        self.xform(board);
        self.rrect(-185.0, -185.0, 370.0, 370.0, 36.0, 0x3f8f4fff, 0x2c6b39ff, 4.0);
        for d in 0..4u8 {
            let f = scr.then(self.petal_frame(d));
            self.xform(f);
            self.rrect(-150.0, -240.0, 300.0, 72.0, 36.0, 0x5cb36aff, 0x2c6b39ff, 3.0);
            let clue = self.shared.clue(d as usize).to_string();
            if clue.is_empty() {
                self.text(0.0, -204.0, 18.0, 0xffffff88, false, "tap to write a clue", 270.0);
            } else {
                self.text(0.0, -204.0, 26.0, 0x10301aff, true, &clue, 270.0);
            }
        }
        self.xform(board);
        let check = self.check.map(|c| c.0);
        for s in 0..NSLOTS {
            let (x, y) = slot_pos(s);
            let fill = match check {
                Some(ok) if ok[s] => 0x6fe08aaa,
                Some(_) => 0xff6b6baa,
                None => 0x00000022,
            };
            self.rrect(x - HALF - 2.0, y - HALF - 2.0, CARD + 4.0, CARD + 4.0, 14.0, fill, 0xffffff40, 2.0);
        }

        // Cards.
        let (cards, _) = self.shared.layout();
        let words = self.shared.words();
        let dragging = self.press.as_ref().filter(|p| p.moved).and_then(|p| p.card);
        for i in self.draw_order(&cards) {
            let c = cards[i];
            let f = scr.then(self.card_frame(&c));
            let lifted = c.held != 0 || Some(i) == dragging;
            if lifted {
                self.xform(M::t(6.0, 9.0).then(f));
                self.rrect(-HALF, -HALF, CARD, CARD, 14.0, 0x00000055, 0, 0.0);
            }
            self.xform(f);
            let (stroke, lw) = match c.held {
                0 => (0xb9a77fff, 2.0),
                h if h == self.me => (0xffffffff, 4.0),
                h => (peer_color(h), 6.0),
            };
            self.rrect(-HALF, -HALF, CARD, CARD, 14.0, 0xfbf3dcff, stroke, lw);
            self.circle(0.0, 0.0, 20.0, 0xefe2bdff, 0, 0.0);
            self.text(0.0, 1.0, 18.0, 0xa08c5cff, true, ["A", "B", "C", "D", "E"][i], 30.0);
            for s in 0..4u8 {
                self.xform(f.then(M::r(Self::quarter_turn(s))));
                self.text(0.0, -51.0, WORD_SIZE, 0x2b2112ff, true, words[i][s as usize], 132.0);
            }
        }

        // Remote cursors.
        for k in 0..self.cursors.len() {
            let RemoteCursor { peer, kind, x, y, .. } = self.cursors[k];
            let m = match kind {
                1 => scr,
                2 => board,
                _ => continue,
            };
            self.xform(m.then(M::t(x, y)));
            self.circle(0.0, 0.0, 9.0, peer_color(peer), 0xffffffff, 3.0);
        }
    }

    // ---- harness probes ------------------------------------------------------

    /// Screen centre (CSS px) and on-screen angle of card `i`.
    pub fn card_screen(&self, i: usize) -> (f32, f32, f32) {
        let c = self.shared.layout().0[i];
        let f = self.screen().then(self.card_frame(&c));
        let (x, y) = f.apply(0.0, 0.0);
        (x, y, f.angle())
    }

    pub fn slot_screen(&self, s: usize) -> (f32, f32) {
        let (x, y) = slot_pos(s);
        self.screen().then(self.board()).apply(x, y)
    }

    pub fn petal_screen(&self, d: u8) -> (f32, f32) {
        self.screen().then(self.petal_frame(d)).apply(0.0, -204.0)
    }
}
