//! Local (per-player) state: identity, presence, clue drafts, view rotation,
//! pointer interaction and rendering. Rendering emits a flat f32 command
//! buffer that the JS shim replays onto a Canvas2D context.

use crate::model::*;
use core::f32::consts::{FRAC_PI_2, PI, TAU};
use std::collections::{BTreeMap, BTreeSet};

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
const CURSOR_TTL_MS: f64 = 4000.0;
const HELLO_EVERY_MS: f64 = 2000.0;
const PEER_TTL_MS: f64 = 7000.0;
pub const WORD_SIZE: f32 = 24.0;
const BOARD_HALF: f32 = 185.0;

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

fn turn(r: u8) -> f32 {
    r as f32 * FRAC_PI_2
}

const PALETTE: [u32; 8] =
    [0xe0457bff, 0x2f80edff, 0xf2994aff, 0x9b51e0ff, 0x27ae60ff, 0x00a3a3ff, 0xd4a017ff, 0xeb5757ff];

/// Fallback colour for a peer not (yet) in the roster.
pub fn peer_color(peer: u32) -> u32 {
    PALETTE[(peer.wrapping_mul(2654435761) >> 29) as usize]
}

pub fn default_name(peer: u32) -> String {
    const A: [&str; 16] = [
        "Brave", "Calm", "Clever", "Dizzy", "Eager", "Fancy", "Gentle", "Happy", "Jolly", "Lucky", "Mellow",
        "Nimble", "Quiet", "Sunny", "Swift", "Witty",
    ];
    const B: [&str; 16] = [
        "Badger", "Beaver", "Crane", "Ferret", "Finch", "Gecko", "Heron", "Koala", "Lemur", "Moose", "Otter",
        "Panda", "Puffin", "Robin", "Stoat", "Walrus",
    ];
    let h = peer.wrapping_mul(2246822519);
    format!("{} {}", A[(h >> 28) as usize], B[((h >> 24) & 15) as usize])
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Card(usize),
    Petal(u8),
    Hub,
    Board,
    Empty,
}

struct Press {
    sx: f32,
    sy: f32,
    target: Target,
    /// Card centre minus pointer, in world coords.
    grab: (f32, f32),
    origin: Option<Card>,
    moved: bool,
    /// (pointer angle about the board centre, view angle) at drag start.
    spin: Option<(f32, f32)>,
}

pub struct RemoteCursor {
    pub peer: u32,
    pub kind: u8,
    pub x: f32,
    pub y: f32,
    pub seen: f64,
}

/// What the board shows right now, for this player.
pub struct Scene {
    pub writer: Option<u32>,
    pub cards: Vec<Card>,
    pub owner: [Option<usize>; NSLOTS],
    pub words: Vec<[&'static str; 4]>,
    pub clues: [String; NSLOTS],
    /// Cards this player may move (index set).
    pub movable: Vec<bool>,
    pub locked: u8,
}

pub struct App {
    pub shared: Shared,
    pub me: u32,
    pub name: String,
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
    pub drafts: [String; NSLOTS],
    pub decoys: u8,
    editing: Option<u8>,
    /// Peer id → (name, last heard).
    pub peers: BTreeMap<u32, (String, f64)>,
    pub cursors: Vec<RemoteCursor>,
    my_cursor: (u8, f32, f32),
    cursor_dirty: bool,
    last_cursor_ms: f64,
    last_hello_ms: f64,
    dirty_cards: BTreeSet<(u32, u8)>,
    last_phase: Phase,
    pub now: f64,
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
            name: default_name(me),
            clock: 0,
            rng,
            q: 0,
            angle: 0.0,
            w: 900.0,
            h: 1000.0,
            dpr: 1.0,
            press: None,
            drafts: Default::default(),
            decoys: 1,
            editing: None,
            peers: BTreeMap::new(),
            cursors: Vec::new(),
            my_cursor: (0, 0.0, 0.0),
            cursor_dirty: false,
            last_cursor_ms: 0.0,
            last_hello_ms: f64::NEG_INFINITY,
            dirty_cards: BTreeSet::new(),
            last_phase: Phase::Lobby,
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
    fn commit(&mut self, body: Body) {
        let op = Op { round: self.shared.round, body };
        self.shared.apply(&op);
        self.send(&[Msg::Op(op)]);
    }

    /// Local card write; broadcast is coalesced to once per frame.
    fn set_card(&mut self, w: u32, i: usize, loc: Loc, rot: u8, held: u32) {
        let stamp = self.stamp();
        let op = Op { round: self.shared.round, body: Body::Card(w, i as u8, Card { loc, rot, held, stamp }) };
        self.shared.apply(&op);
        self.dirty_cards.insert((w, i as u8));
    }

    fn flush_cards(&mut self) {
        if self.dirty_cards.is_empty() {
            return;
        }
        let r = self.shared.round;
        let msgs: Vec<Msg> = std::mem::take(&mut self.dirty_cards)
            .into_iter()
            .filter_map(|k| self.shared.cards.get(&k).map(|c| Msg::Op(Op { round: r, body: Body::Card(k.0, k.1, *c) })))
            .collect();
        self.send(&msgs);
    }

    fn hello(&mut self) {
        self.last_hello_ms = self.now;
        let (peer, name) = (self.me, self.name.clone());
        self.send(&[Msg::Hello { peer, name }]);
    }

    // ---- network in ---------------------------------------------------------

    pub fn on_packet(&mut self, bytes: &[u8]) {
        for m in decode(bytes) {
            match m {
                Msg::Op(op) => {
                    let s = match &op.body {
                        Body::Sub(_, s) => s.stamp,
                        Body::Start(s) | Body::Next(_, s) => *s,
                        Body::Card(_, _, c) => c.stamp,
                        Body::Check(_, c) => c.stamp,
                        Body::None => op.round.stamp,
                    };
                    self.clock = self.clock.max(s.clock).max(op.round.stamp.clock);
                    let old_round = self.shared.round.stamp;
                    self.shared.apply(&op);
                    if self.shared.round.stamp != old_round {
                        self.new_round_local();
                    }
                }
                Msg::Hello { peer, name } if peer != self.me => {
                    self.peers.insert(peer, (name, self.now));
                }
                Msg::Cursor { peer, kind, x, y } if peer != self.me => {
                    let now = self.now;
                    self.peers.entry(peer).or_insert_with(|| (default_name(peer), now)).1 = now;
                    match self.cursors.iter_mut().find(|c| c.peer == peer) {
                        Some(c) => *c = RemoteCursor { peer, kind, x, y, seen: now },
                        None => self.cursors.push(RemoteCursor { peer, kind, x, y, seen: now }),
                    }
                }
                _ => {}
            }
        }
    }

    fn new_round_local(&mut self) {
        self.drafts = Default::default();
        self.editing = None;
        self.press = None;
        self.dirty_cards.clear();
    }

    pub fn on_peer_join(&mut self) {
        let mut msgs: Vec<Msg> = self.shared.all_ops().into_iter().map(Msg::Op).collect();
        msgs.push(Msg::Hello { peer: self.me, name: self.name.clone() });
        self.send(&msgs);
    }

    pub fn set_name(&mut self, name: &str) {
        let n = clamp_text(name, 24);
        self.name = if n.is_empty() { default_name(self.me) } else { n };
        self.hello();
    }

    /// Peers heard from recently (not including me).
    pub fn alive(&self) -> Vec<u32> {
        self.peers.iter().filter(|(_, (_, t))| self.now - t < PEER_TTL_MS).map(|(p, _)| *p).collect()
    }

    /// Everyone in this game, sorted (stable colours by rank).
    pub fn roster(&self) -> Vec<u32> {
        let mut v = self.alive();
        v.push(self.me);
        v.extend(self.shared.subs.keys());
        v.sort();
        v.dedup();
        v
    }

    pub fn color_of(&self, p: u32) -> u32 {
        match self.roster().iter().position(|&x| x == p) {
            Some(k) => PALETTE[k % PALETTE.len()],
            None => peer_color(p),
        }
    }

    pub fn name_of(&self, p: u32) -> String {
        if p == self.me {
            return self.name.clone();
        }
        self.peers.get(&p).map(|(n, _)| n.clone()).unwrap_or_else(|| default_name(p))
    }

    // ---- what is on the board -------------------------------------------------

    pub fn scene(&self) -> Scene {
        let s = &self.shared;
        let empty = || Scene {
            writer: None,
            cards: Vec::new(),
            owner: [None; NSLOTS],
            words: Vec::new(),
            clues: Default::default(),
            movable: Vec::new(),
            locked: 0,
        };
        match s.phase() {
            Phase::Lobby | Phase::End => empty(),
            Phase::Writing => {
                let d = deal(s.round.seed, self.me);
                Scene {
                    writer: Some(self.me),
                    cards: (0..NSLOTS)
                        .map(|i| Card { loc: Loc::Slot(i as u8), rot: d.sol_rot[i], held: 0, stamp: Stamp::default() })
                        .collect(),
                    owner: [Some(0), Some(1), Some(2), Some(3)],
                    words: d.words[..NSLOTS].to_vec(),
                    clues: self.drafts.clone(),
                    movable: vec![false; NSLOTS],
                    locked: 0b1111,
                }
            }
            Phase::Guessing(k) => {
                let w = s.puzzles()[k];
                let d = deal(s.round.seed, w);
                let (cards, owner) = s.layout(w);
                let check = s.check(w);
                let locked = if check.done() { 0b1111 } else { check.mask };
                let can = w != self.me && !check.done();
                Scene {
                    writer: Some(w),
                    movable: (0..cards.len()).map(|i| can && (i >= NSLOTS || locked & (1 << i) == 0)).collect(),
                    words: d.words[..cards.len()].to_vec(),
                    clues: s.subs.get(&w).map(|x| x.clues.clone()).unwrap_or_default(),
                    cards,
                    owner,
                    locked,
                }
            }
        }
    }

    // ---- geometry -----------------------------------------------------------

    pub fn scale(&self) -> f32 {
        (self.w / WORLD_W).min(self.h / WORLD_H)
    }

    /// World → CSS pixels.
    pub fn screen(&self) -> M {
        let k = self.scale();
        M::t(self.w / 2.0, self.h / 2.0).then(M::s(k)).then(M::t(-WORLD_W / 2.0, -WORLD_H / 2.0))
    }

    /// Board coords → world.
    pub fn board(&self) -> M {
        M::t(CX, CY).then(M::r(self.angle))
    }

    /// Card-local coords → world.
    pub fn card_frame(&self, c: &Card) -> M {
        let r = M::r(turn(c.rot));
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

    fn dragging_card(&self) -> Option<usize> {
        self.press.as_ref().filter(|p| p.moved).and_then(|p| match p.target {
            Target::Card(i) => Some(i),
            _ => None,
        })
    }

    /// Back-to-front draw order: slotted, table, floating, then my drag.
    fn draw_order(&self, cards: &[Card]) -> Vec<usize> {
        let dragging = self.dragging_card();
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
        let mut v: Vec<usize> = (0..cards.len()).collect();
        v.sort_by_key(|&i| rank(i));
        v
    }

    fn petal_frame_at(&self, d: u8, angle: f32) -> M {
        M::t(CX, CY).then(M::r(angle)).then(M::r(turn(d)))
    }

    fn petal_frame(&self, d: u8) -> M {
        self.petal_frame_at(d, self.angle)
    }

    fn hit(&self, wx: f32, wy: f32) -> Target {
        let (bx, by) = self.board().inv().apply(wx, wy);
        if bx.hypot(by) <= HUB_R + 6.0 {
            return Target::Hub;
        }
        let sc = self.scene();
        let card = self.draw_order(&sc.cards).into_iter().rev().find(|&i| {
            let (lx, ly) = self.card_frame(&sc.cards[i]).inv().apply(wx, wy);
            lx.abs() <= HALF && ly.abs() <= HALF
        });
        if let Some(i) = card.filter(|&i| sc.movable[i]) {
            return Target::Card(i);
        }
        if let Some(d) = (0..4).find(|&d| {
            let (x, y) = self.petal_frame(d).inv().apply(wx, wy);
            x.abs() <= 150.0 && (-240.0..=-165.0).contains(&y)
        }) {
            return Target::Petal(d);
        }
        if (bx.abs() <= BOARD_HALF && by.abs() <= BOARD_HALF) || card.is_some() {
            return Target::Board;
        }
        Target::Empty
    }

    // ---- input --------------------------------------------------------------

    pub fn pointer_down(&mut self, sx: f32, sy: f32) {
        let (wx, wy) = self.to_world(sx, sy);
        let target = self.hit(wx, wy);
        let (grab, origin) = match target {
            Target::Card(i) => {
                let c = self.scene().cards[i];
                let (cx, cy) = self.card_frame(&c).apply(0.0, 0.0);
                ((cx - wx, cy - wy), Some(c))
            }
            _ => ((0.0, 0.0), None),
        };
        self.press = Some(Press { sx, sy, target, grab, origin, moved: false, spin: None });
    }

    pub fn pointer_move(&mut self, sx: f32, sy: f32) {
        let (wx, wy) = self.to_world(sx, sy);
        let (bx, by) = self.board().inv().apply(wx, wy);
        if matches!(self.shared.phase(), Phase::Guessing(_)) {
            let over = bx.abs() <= BOARD_REACH && by.abs() <= BOARD_REACH;
            self.my_cursor = if over { (2, bx, by) } else { (1, wx, wy) };
            self.cursor_dirty = true;
        }

        let Some(p) = self.press.as_mut() else { return };
        if !p.moved && (sx - p.sx).hypot(sy - p.sy) < DRAG_THRESHOLD {
            return;
        }
        p.moved = true;
        let target = p.target;
        match target {
            Target::Card(i) => {
                let (cx, cy) = (wx + p.grab.0, wy + p.grab.1);
                self.drag_card(i, cx, cy);
            }
            _ => {
                // Dragging anywhere else spins my view of the board.
                let (psx, psy, spin) = (p.sx, p.sy, p.spin);
                let ang = |x: f32, y: f32| (y - CY).atan2(x - CX);
                let start = spin.unwrap_or_else(|| {
                    let (x0, y0) = self.screen().inv().apply(psx, psy);
                    (ang(x0, y0), self.angle)
                });
                let mut d = ang(wx, wy) - start.0;
                d = (d + PI).rem_euclid(TAU) - PI;
                self.angle = start.1 + d;
                // Keep the spin continuous across the ±π seam.
                if let Some(p) = self.press.as_mut() {
                    p.spin = Some((ang(wx, wy), self.angle));
                }
            }
        }
    }

    fn drag_card(&mut self, i: usize, cx: f32, cy: f32) {
        let sc = self.scene();
        let Some(w) = sc.writer else { return };
        let cur = sc.cards[i];
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
        self.set_card(w, i, loc, rot, me);
    }

    pub fn pointer_up(&mut self, sx: f32, sy: f32) {
        self.pointer_move(sx, sy);
        let Some(p) = self.press.take() else { return };
        if p.spin.is_some() {
            self.q = (self.angle / FRAC_PI_2).round() as i32;
            return;
        }
        match (p.target, p.moved) {
            (Target::Card(i), true) => self.drop_card(i, p.origin.unwrap()),
            (Target::Card(i), false) => {
                let sc = self.scene();
                let c = sc.cards[i];
                self.set_card(sc.writer.unwrap(), i, c.loc, (c.rot + 1) % 4, 0);
            }
            (Target::Petal(d), false) => {
                if self.shared.phase() == Phase::Writing {
                    self.edit_clue(d);
                } else {
                    self.face_petal(d);
                }
            }
            (Target::Hub | Target::Board, false) => self.q += 1,
            _ => {}
        }
        self.flush_cards();
    }

    fn drop_card(&mut self, i: usize, origin: Card) {
        let sc = self.scene();
        let Some(w) = sc.writer else { return };
        let c = sc.cards[i];
        let Loc::Float { x, y } = c.loc else {
            return self.set_card(w, i, c.loc, c.rot, 0);
        };
        let s = nearest_slot(x, y);
        if sc.locked & (1 << s) != 0 {
            // Already solved: bounce back.
            let rot = if matches!(origin.loc, Loc::Table { .. }) { quarter(c.rot as i32 + self.q) } else { c.rot };
            return self.set_card(w, i, origin.loc, rot, 0);
        }
        if let Some(k) = sc.owner[s as usize].filter(|&k| k != i) {
            // Swap: the occupant goes where the dragged card came from.
            let occ = sc.cards[k];
            match origin.loc {
                Loc::Slot(_) => self.set_card(w, k, origin.loc, occ.rot, 0),
                _ => self.set_card(w, k, origin.loc, quarter(occ.rot as i32 + self.q), 0),
            }
        }
        self.set_card(w, i, Loc::Slot(s), c.rot, 0);
    }

    /// Turn my view so petal `d` sits at the top, reading upright.
    pub fn face_petal(&mut self, d: u8) {
        let delta = quarter(-(d as i32) - self.q) as i32;
        self.q += if delta == 3 { -1 } else { delta };
    }

    pub fn edit_clue(&mut self, d: u8) {
        self.face_petal(d);
        self.editing = Some(d);
        let target = self.q as f32 * FRAC_PI_2;
        let f = self.screen().then(self.petal_frame_at(d, target)).then(M::t(0.0, -204.0));
        let (x, y) = f.apply(0.0, 0.0);
        let k = self.scale();
        let mut p = vec![d];
        for v in [x, y, 290.0 * k, 56.0 * k, f.angle()] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(self.drafts[d as usize].as_bytes());
        self.event(EV_EDIT_CLUE, &p);
    }

    /// Store a clue draft; with `advance`, open the next empty leaf.
    pub fn set_clue(&mut self, d: u8, text: &str, advance: bool) {
        if d as usize >= NSLOTS || self.shared.phase() != Phase::Writing {
            return;
        }
        self.drafts[d as usize] = clamp_text(text, MAX_CLUE_BYTES);
        self.editing = None;
        if advance && !self.drafts[d as usize].is_empty() {
            if let Some(n) = (1..4).map(|k| (d + k) % 4).find(|&n| self.drafts[n as usize].is_empty()) {
                self.edit_clue(n);
            }
        }
    }

    pub fn rotate_view(&mut self, dir: i32) {
        self.q += dir.signum();
    }

    pub fn can_submit(&self) -> bool {
        self.shared.phase() == Phase::Writing && self.drafts.iter().all(|c| !c.is_empty())
    }

    /// 1 start/play again, 2 submit clues, 3 check, 4 next clover, 5 start guessing now.
    pub fn action(&mut self, code: u32) {
        let phase = self.shared.phase();
        match code {
            1 if matches!(phase, Phase::Lobby | Phase::End) => {
                let round = Round { seed: self.rng.next() as u32, stamp: self.stamp() };
                self.shared.apply(&Op { round, body: Body::None });
                self.new_round_local();
                self.send(&[Msg::Op(Op { round, body: Body::None })]);
            }
            2 if self.can_submit() => {
                let stamp = self.stamp();
                let sub = Sub { clues: self.drafts.clone(), decoys: self.decoys.min(MAX_DECOYS), stamp };
                let me = self.me;
                self.commit(Body::Sub(me, sub));
            }
            3 => self.check(),
            4 => {
                if let Phase::Guessing(k) = phase {
                    let w = self.shared.puzzles()[k];
                    if self.shared.check(w).done() {
                        let s = self.stamp();
                        self.commit(Body::Next(k as u16 + 1, s));
                    }
                }
            }
            5 if phase == Phase::Writing && self.shared.subs.contains_key(&self.me) => {
                let s = self.stamp();
                self.commit(Body::Start(s));
            }
            _ => {}
        }
    }

    pub fn can_check(&self) -> bool {
        let sc = self.scene();
        match (self.shared.phase(), sc.writer) {
            (Phase::Guessing(_), Some(w)) => {
                w != self.me && !self.shared.check(w).done() && sc.owner.iter().all(|o| o.is_some())
            }
            _ => false,
        }
    }

    fn check(&mut self) {
        if !self.can_check() {
            return;
        }
        let sc = self.scene();
        let w = sc.writer.unwrap();
        let d = deal(self.shared.round.seed, w);
        let mut mask = 0u8;
        for s in 0..NSLOTS {
            let k = sc.owner[s].unwrap();
            if k == s && sc.cards[k].rot == d.sol_rot[s] {
                mask |= 1 << s;
            }
        }
        let prev = self.shared.check(w);
        let stamp = self.stamp();
        let check = Check { attempts: prev.attempts + 1, mask, stamp };
        // Wrong cards go back to the table for the second try.
        if !check.done() {
            let n = sc.cards.len();
            for s in 0..NSLOTS {
                if mask & (1 << s) == 0 {
                    let k = sc.owner[s].unwrap();
                    let h = self.shared.home_card(&d, n, k);
                    self.set_card(w, k, h.loc, h.rot, 0);
                }
            }
        }
        self.commit(Body::Check(w, check));
        self.flush_cards();
        let msg = match (check.done(), check.score()) {
            (true, PERFECT_SCORE) => "Perfect clover! +6".to_string(),
            (true, s) => format!("{s}/4 right. +{s}"),
            (false, s) => format!("{s}/4 right. Wrong cards went back; one more try."),
        };
        self.status(&msg);
    }

    // ---- frame --------------------------------------------------------------

    pub fn frame(&mut self, now: f64) {
        let dt = ((now - self.now) / 1000.0).clamp(0.0, 0.1) as f32;
        self.now = now;
        let spinning = self.press.as_ref().is_some_and(|p| p.spin.is_some());
        if !spinning {
            let target = self.q as f32 * FRAC_PI_2;
            let diff = target - self.angle;
            self.angle = if diff.abs() < 1e-3 { target } else { self.angle + diff * (1.0 - (-dt * 14.0).exp()) };
        }
        if now - self.last_hello_ms >= HELLO_EVERY_MS {
            self.hello();
        }
        self.maybe_start_guessing();
        let phase = self.shared.phase();
        if phase != self.last_phase {
            // New phase or new clover: spin back upright the short way.
            self.last_phase = phase;
            self.q = (self.q as f32 / 4.0).round() as i32 * 4;
        }
        self.flush_cards();
        if self.cursor_dirty && now - self.last_cursor_ms >= 33.0 {
            self.cursor_dirty = false;
            self.last_cursor_ms = now;
            let (kind, x, y) = self.my_cursor;
            let me = self.me;
            self.send(&[Msg::Cursor { peer: me, kind, x, y }]);
        }
        self.cursors.retain(|c| now - c.seen < CURSOR_TTL_MS);
        self.render();
    }

    /// Guessing starts on its own once every player present has submitted.
    fn maybe_start_guessing(&mut self) {
        if self.shared.phase() != Phase::Writing || !self.shared.subs.contains_key(&self.me) {
            return;
        }
        if self.alive().iter().all(|p| self.shared.subs.contains_key(p)) {
            let s = self.stamp();
            self.commit(Body::Start(s));
        }
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

    #[allow(clippy::too_many_arguments)]
    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: u32, stroke: u32, lw: f32) {
        self.cmds.extend_from_slice(&[C_RRECT, x, y, w, h, r, f32::from_bits(fill), f32::from_bits(stroke), lw]);
    }

    fn circle(&mut self, x: f32, y: f32, r: f32, fill: u32, stroke: u32, lw: f32) {
        self.cmds.extend_from_slice(&[C_CIRCLE, x, y, r, f32::from_bits(fill), f32::from_bits(stroke), lw]);
    }

    #[allow(clippy::too_many_arguments)]
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
        let sc = self.scene();
        let phase = self.shared.phase();

        self.xform(scr);
        self.rrect(0.0, 0.0, WORLD_W, WORLD_H, 24.0, 0x2a4d3bff, 0, 0.0);

        // Board: base + four clue leaves.
        self.xform(board);
        self.rrect(-BOARD_HALF, -BOARD_HALF, 2.0 * BOARD_HALF, 2.0 * BOARD_HALF, 36.0, 0x3f8f4fff, 0x2c6b39ff, 4.0);
        for d in 0..4u8 {
            let f = scr.then(self.petal_frame(d));
            self.xform(f);
            let editing = self.editing == Some(d);
            self.rrect(-150.0, -240.0, 300.0, 72.0, 36.0, if editing { 0x8fdc9bff } else { 0x5cb36aff }, 0x2c6b39ff, 3.0);
            let clue = sc.clues[d as usize].clone();
            if !clue.is_empty() {
                self.text(0.0, -204.0, 26.0, 0x10301aff, true, &clue, 270.0);
            } else if phase == Phase::Writing {
                self.text(0.0, -204.0, 18.0, 0xffffffaa, false, "tap to write a clue", 270.0);
            }
        }
        self.xform(board);
        for s in 0..NSLOTS {
            let (x, y) = slot_pos(s);
            let solved = sc.locked & (1 << s) != 0 && phase != Phase::Writing;
            let fill = if solved { 0x6fe08aaa } else { 0x00000022 };
            self.rrect(x - HALF - 2.0, y - HALF - 2.0, CARD + 4.0, CARD + 4.0, 14.0, fill, 0xffffff40, 2.0);
        }

        // Cards.
        let dragging = self.dragging_card();
        for i in self.draw_order(&sc.cards) {
            let c = sc.cards[i];
            let f = scr.then(self.card_frame(&c));
            if c.held != 0 || Some(i) == dragging {
                self.xform(M::t(6.0, 9.0).then(f));
                self.rrect(-HALF, -HALF, CARD, CARD, 14.0, 0x00000055, 0, 0.0);
            }
            self.xform(f);
            let (stroke, lw) = match c.held {
                0 if matches!(c.loc, Loc::Slot(s) if sc.locked & (1 << s) != 0) && phase != Phase::Writing => {
                    (0x2fa84fff, 5.0)
                }
                0 => (0xb9a77fff, 2.0),
                h if h == self.me => (0xffffffff, 4.0),
                h => (self.color_of(h), 6.0),
            };
            self.rrect(-HALF, -HALF, CARD, CARD, 14.0, 0xfbf3dcff, stroke, lw);
            self.circle(0.0, 0.0, 16.0, 0xefe2bdff, 0, 0.0);
            for s in 0..4u8 {
                self.xform(f.then(M::r(turn(s))));
                self.text(0.0, -51.0, WORD_SIZE, 0x2b2112ff, true, sc.words[i][s as usize], 132.0);
            }
        }

        // Hub: tap to turn the board.
        self.xform(board);
        self.circle(0.0, 0.0, HUB_R, 0x2c6b39ff, 0xeaf5ecff, 3.0);
        self.xform(scr.then(M::t(CX, CY)));
        self.text(0.0, 1.0, 30.0, 0xffffffff, true, "⟳", 40.0);

        // Remote cursors (guessing only: that's when we share a board).
        if matches!(phase, Phase::Guessing(_)) {
            for k in 0..self.cursors.len() {
                let RemoteCursor { peer, kind, x, y, .. } = self.cursors[k];
                let m = match kind {
                    1 => scr,
                    2 => board,
                    _ => continue,
                };
                self.xform(m.then(M::t(x, y)));
                let col = self.color_of(peer);
                self.circle(0.0, 0.0, 9.0, col, 0xffffffff, 3.0);
            }
        }
    }

    // ---- UI state for the HTML shell -------------------------------------------

    pub fn ui_json(&self) -> String {
        let s = &self.shared;
        let phase = s.phase();
        let sc = self.scene();
        let players = self.roster();
        let alive = self.alive();
        let writer = sc.writer.filter(|_| matches!(phase, Phase::Guessing(_)));
        let rows: Vec<String> = players
            .iter()
            .map(|&p| {
                let score = s.puzzles().contains(&p).then(|| s.check(p)).filter(|c| c.done()).map(|c| c.score());
                format!(
                    "{{\"id\":{p},\"name\":{},\"color\":\"#{:06x}\",\"you\":{},\"away\":{},\"sub\":{},\"writer\":{},\"score\":{}}}",
                    json_str(&self.name_of(p)),
                    self.color_of(p) >> 8,
                    p == self.me,
                    p != self.me && !alive.contains(&p),
                    s.subs.contains_key(&p),
                    writer == Some(p),
                    score.map_or("null".into(), |x| x.to_string()),
                )
            })
            .collect();
        let (pname, k, n) = match phase {
            Phase::Lobby => ("lobby", 0, 0),
            Phase::Writing => ("writing", 0, 0),
            Phase::Guessing(k) => ("guessing", k, s.puzzles().len()),
            Phase::End => ("end", 0, s.puzzles().len()),
        };
        let check = writer.map(|w| s.check(w)).unwrap_or_default();
        format!(
            "{{\"phase\":\"{pname}\",\"me\":{},\"name\":{},\"players\":[{}],\"submitted\":{},\"canSubmit\":{},\
             \"filled\":{},\"decoys\":{},\"waiting\":{},\"k\":{k},\"n\":{n},\"writer\":{},\"writerName\":{},\
             \"isWriter\":{},\"attempts\":{},\"done\":{},\"canCheck\":{},\"total\":{}}}",
            self.me,
            json_str(&self.name),
            rows.join(","),
            s.subs.contains_key(&self.me),
            self.can_submit(),
            self.drafts.iter().filter(|c| !c.is_empty()).count(),
            self.decoys,
            self.alive().iter().filter(|p| !s.subs.contains_key(p)).count(),
            writer.map_or("null".into(), |w| w.to_string()),
            writer.map_or("null".into(), |w| json_str(&self.name_of(w))),
            writer == Some(self.me),
            check.attempts,
            check.done(),
            self.can_check(),
            s.total_score(),
        )
    }

    // ---- harness probes ------------------------------------------------------

    /// Screen centre (CSS px) and on-screen angle of scene card `i`.
    pub fn card_screen(&self, i: usize) -> (f32, f32, f32) {
        let sc = self.scene();
        let Some(c) = sc.cards.get(i) else { return (-1.0, -1.0, 0.0) };
        let f = self.screen().then(self.card_frame(c));
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

    pub fn hub_screen(&self) -> (f32, f32) {
        self.screen().apply(CX, CY)
    }
}
