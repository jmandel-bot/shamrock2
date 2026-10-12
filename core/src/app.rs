//! Local (per-device) state: identity, presence, view rotation, layout,
//! pointer interaction, animation and rendering. Rendering emits a flat f32
//! command buffer that the JS shim replays onto a Canvas2D context.

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
/// Heart-shaped clover leaf: tip at the origin, lobes toward -y.
const C_LEAF: f32 = 6.0;

// Text faces (the shim maps these to fonts).
const F_SANS_BOLD: u8 = 1;
const F_SERIF: u8 = 2;

// Palette. Every text/background pair here is checked for WCAG AA contrast
// by the harness (see `palette_json`).
const BG: u32 = 0x14301fff;
const TRAY: u32 = 0x1b3d2aff;
const TRAY_EDGE: u32 = 0x2b5a3eff;
const LEAF: u32 = 0xa8dbb0ff;
const LEAF_ACTIVE: u32 = 0xd6f1daff;
const LEAF_GLOW: u32 = 0xe9f8cfff;
const LEAF_EDGE: u32 = 0x2e6b42ff;
const CENTRE: u32 = 0x2e6b45ff;
const CENTRE_EDGE: u32 = 0x24573aff;
const PAPER: u32 = 0xfcf8eeff;
const PAPER_EDGE: u32 = 0xcdbf9aff;
const INK: u32 = 0x1c1710ff;
const CLUE_INK: u32 = 0x0b2414ff;
const KEYWORD_INK: u32 = 0x1d4a2bff;
const HUB: u32 = 0x173a27ff;
const GOLD: u32 = 0xf2c14eff;

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| -> u32 {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

const DRAG_THRESHOLD: f32 = 6.0;
const CURSOR_TTL_MS: f64 = 4000.0;
const HELLO_EVERY_MS: f64 = 2000.0;
pub const WORD_SIZE: f32 = 26.0;
/// Distance from a card's centre to the middle of each edge word. Deep
/// enough that a word up to `WORD_MAX_W` wide never reaches the corner
/// squares where the neighbouring edges' words sit, however long they are.
pub const WORD_INSET: f32 = 86.0;
pub const WORD_MAX_W: f32 = 2.0 * (WORD_INSET - WORD_SIZE / 2.0);
/// Tray depth (short axis) in board units: two rows of cards plus air.
const TRAY_UNITS: f32 = 480.0;
const PAD: f32 = 8.0;
/// Per-device card size range. Up to `ZOOM_TOUCH` cards in neighbouring
/// spots don't overlap; beyond it they may, if a player wants them that big.
pub const ZOOM_MIN: f32 = 0.7;
pub const ZOOM_TOUCH: f32 = 2.0 * SLOT_OFF / CARD;
pub const ZOOM_MAX: f32 = 1.6;

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

/// Shortest signed difference between two angles.
fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

const PALETTE: [u32; 8] =
    [0xe0457bff, 0x2f80edff, 0xf2994aff, 0x9b51e0ff, 0x27ae60ff, 0x00a3a3ff, 0xd4a017ff, 0xeb5757ff];

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
    /// Card centre minus pointer, in CSS px.
    grab: (f32, f32),
    /// Where the dragged card started (for swaps).
    origin: Option<Card>,
    moved: bool,
    /// (pointer angle about the board centre, view angle) while spinning.
    spin: Option<(f32, f32)>,
}

pub struct RemoteCursor {
    pub seat: u32,
    pub kind: u8,
    pub x: f32,
    pub y: f32,
    pub seen: f64,
}

/// Where things go on this screen.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// CSS px per board unit.
    pub k: f32,
    /// Board centre, CSS px.
    pub bc: (f32, f32),
    /// Tray rect (x, y, w, h) and whether its long axis is horizontal.
    pub tray: Option<(f32, f32, f32, f32, bool)>,
}

/// Displayed (animated) pose of a card: in board units when on the board,
/// in tray coords otherwise; angle in radians.
#[derive(Clone, Copy, Debug)]
struct Pose {
    board: bool,
    x: f32,
    y: f32,
    a: f32,
    /// Scale (deal-in grows cards from small).
    s: f32,
    /// Seconds left before this card starts moving (deal-in stagger).
    wait: f32,
}

/// What the board shows right now, for this device.
pub struct Scene {
    pub writer: Option<u32>,
    pub cards: Vec<Card>,
    pub claim: [Option<usize>; NSLOTS],
    pub words: Vec<[&'static str; 4]>,
    pub clues: [String; NSLOTS],
    pub movable: Vec<bool>,
    pub locked: u8,
}

pub struct App {
    pub shared: Shared,
    /// This device (Lamport tie-break, presence).
    pub device: u32,
    /// The seat this device plays, if any (None = watching).
    pub seat: Option<u32>,
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
    editing: Option<u8>,
    /// Device → (seat, link it arrives over). Presence follows the
    /// connection, not a timer, so a backgrounded tab (no frames, no
    /// heartbeats) still counts as here until its connection actually drops.
    pub peers: BTreeMap<u32, (u32, u32)>,
    pub cursors: BTreeMap<u32, RemoteCursor>,
    my_cursor: (u8, f32, f32),
    cursor_dirty: bool,
    last_cursor_ms: f64,
    last_hello_ms: f64,
    dirty_cards: BTreeSet<(u32, u8)>,
    last_phase: Phase,
    poses: BTreeMap<(u32, u8), Pose>,
    dt: f32,
    /// Local card size multiplier (display only; positions are shared).
    pub zoom: f32,
    /// Decorative motion on (off for prefers-reduced-motion).
    pub motion: bool,
    /// This room has an imported clover to open with (see `Custom`).
    pub custom_room: bool,
    /// Soft glow on the leaves after a clover is completed (1 → 0).
    glow: f32,
    last_done: Option<(u32, bool)>,
    /// (writer, guesses) last seen, to spot a guess made anywhere.
    last_guess: Option<(u32, u8)>,
    /// Cards booted by the last guess: when their shake started.
    shakes: BTreeMap<(u32, u8), f64>,
    solved_at: f64,
    pub now: f64,
    pub outbox: Vec<u8>,
    pub cmds: Vec<f32>,
    pub strs: Vec<u8>,
}

impl App {
    pub fn new(seed: u64) -> App {
        let mut rng = Rng::new(seed);
        let device = (rng.next() as u32).max(1);
        App {
            shared: Shared::default(),
            device,
            seat: None,
            clock: 0,
            rng,
            q: 0,
            angle: 0.0,
            w: 800.0,
            h: 1000.0,
            dpr: 1.0,
            press: None,
            editing: None,
            peers: BTreeMap::new(),
            cursors: BTreeMap::new(),
            my_cursor: (0, 0.0, 0.0),
            cursor_dirty: false,
            last_cursor_ms: 0.0,
            last_hello_ms: f64::NEG_INFINITY,
            dirty_cards: BTreeSet::new(),
            last_phase: Phase::Lobby,
            poses: BTreeMap::new(),
            dt: 0.0,
            zoom: 1.0,
            motion: true,
            custom_room: false,
            glow: 0.0,
            last_done: None,
            last_guess: None,
            shakes: BTreeMap::new(),
            solved_at: f64::NEG_INFINITY,
            now: 0.0,
            outbox: Vec::new(),
            cmds: Vec::new(),
            strs: Vec::new(),
        }
    }

    fn stamp(&mut self) -> Stamp {
        self.clock += 1;
        Stamp { clock: self.clock, peer: self.device }
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

    fn op(&self, body: Body) -> Op {
        Op { game: self.shared.game, round: self.shared.round, body }
    }

    /// Apply a locally-originated op and broadcast it.
    fn commit(&mut self, body: Body) {
        let op = self.op(body);
        self.shared.apply(&op);
        self.send(&[Msg::Op(op)]);
    }

    /// Local card write; broadcast is coalesced to once per frame.
    fn set_card(&mut self, w: u32, i: usize, loc: Loc, rot: u8, held: u32) {
        let stamp = self.stamp();
        let op = self.op(Body::Card(w, i as u8, Card { loc, rot, held, stamp }));
        self.shared.apply(&op);
        self.dirty_cards.insert((w, i as u8));
    }

    fn flush_cards(&mut self) {
        if self.dirty_cards.is_empty() {
            return;
        }
        let msgs: Vec<Msg> = std::mem::take(&mut self.dirty_cards)
            .into_iter()
            .filter_map(|k| self.shared.cards.get(&k).map(|c| Msg::Op(self.op(Body::Card(k.0, k.1, *c)))))
            .collect();
        self.send(&msgs);
    }

    fn hello(&mut self) {
        self.last_hello_ms = self.now;
        let (device, seat) = (self.device, self.seat.unwrap_or(0));
        self.send(&[Msg::Hello { device, seat }]);
    }

    // ---- network in ---------------------------------------------------------

    /// A packet from the peer on connection `link`.
    pub fn on_packet(&mut self, bytes: &[u8], link: u32) {
        for m in decode(bytes) {
            match m {
                Msg::Op(op) => {
                    let s = match &op.body {
                        Body::Sub(_, s) => s.stamp,
                        Body::Seat(_, s) => s.stamp,
                        Body::Start(s) | Body::Next(_, s) | Body::Ledger(_, _, _, s) => *s,
                        Body::Card(_, _, c) => c.stamp,
                        Body::Check(_, c) => c.stamp,
                        Body::None => op.round.stamp,
                    };
                    self.clock = self.clock.max(s.clock).max(op.round.stamp.clock).max(op.game.clock);
                    let before = (self.shared.game, self.shared.round.stamp);
                    self.shared.apply(&op);
                    if (self.shared.game, self.shared.round.stamp) != before {
                        self.new_round_local();
                    }
                }
                Msg::Hello { device, seat } if device != self.device => {
                    self.peers.insert(device, (seat, link));
                }
                Msg::Cursor { device, seat, kind, x, y } if device != self.device => {
                    let now = self.now;
                    self.peers.insert(device, (seat, link));
                    self.cursors.insert(device, RemoteCursor { seat, kind, x, y, seen: now });
                }
                _ => {}
            }
        }
    }

    fn new_round_local(&mut self) {
        self.editing = None;
        self.press = None;
        self.dirty_cards.clear();
        self.poses.clear();
    }

    /// The connection `link` closed: everyone who spoke over it is gone.
    pub fn on_link_lost(&mut self, link: u32) {
        let gone: Vec<u32> = self.peers.iter().filter(|(_, (_, l))| *l == link).map(|(d, _)| *d).collect();
        for d in gone {
            self.peers.remove(&d);
            self.cursors.remove(&d);
        }
    }

    pub fn on_peer_join(&mut self) {
        let mut msgs: Vec<Msg> = self.shared.all_ops().into_iter().map(Msg::Op).collect();
        msgs.push(Msg::Hello { device: self.device, seat: self.seat.unwrap_or(0) });
        self.send(&msgs);
    }

    // ---- seats ----------------------------------------------------------------

    /// Sit down as `name`. An existing name takes over that seat.
    pub fn join(&mut self, name: &str) {
        let name = clamp_text(name, 24);
        if name.is_empty() {
            return;
        }
        if self.shared.game == Stamp::default() {
            self.shared.game = self.stamp();
        }
        let id = seat_id(&name);
        let stamp = self.stamp();
        self.commit(Body::Seat(id, Seat { name, active: true, stamp }));
        self.seat = Some(id);
        self.hello();
    }

    pub fn watch(&mut self) {
        self.seat = None;
        self.hello();
    }

    /// Take a (usually absent) player out of the game.
    pub fn remove_seat(&mut self, id: u32) {
        if let Some(s) = self.shared.seats.get(&id).cloned() {
            let stamp = self.stamp();
            self.commit(Body::Seat(id, Seat { active: false, stamp, ..s }));
            if self.seat == Some(id) {
                self.watch();
            }
        }
    }

    /// Seats with a device here right now (including mine).
    pub fn present(&self) -> BTreeSet<u32> {
        let mut v: BTreeSet<u32> = self
            .peers
            .values()
            .filter(|(seat, _)| *seat != 0)
            .map(|(s, _)| *s)
            .collect();
        v.extend(self.seat);
        v
    }

    pub fn watchers(&self) -> usize {
        self.peers.values().filter(|(seat, _)| *seat == 0).count()
            + self.seat.is_none() as usize
    }

    pub fn color_of(&self, seat: u32) -> u32 {
        let players = self.shared.players();
        let k = players.iter().position(|&x| x == seat).unwrap_or(seat as usize);
        PALETTE[k % PALETTE.len()]
    }

    pub fn name_of(&self, seat: u32) -> String {
        self.shared.seats.get(&seat).map(|s| s.name.clone()).unwrap_or_else(|| "someone".into())
    }

    fn my_sub(&self) -> Sub {
        self.seat.and_then(|s| self.shared.latest(s).cloned()).unwrap_or_else(|| Sub { decoys: 1, ..Sub::default() })
    }

    fn seated_writing(&self) -> Option<u32> {
        self.seat.filter(|s| self.shared.phase() == Phase::Writing && self.shared.round_players().contains(s))
    }

    // ---- what is on the board -------------------------------------------------

    pub fn scene(&self) -> Scene {
        let s = &self.shared;
        let empty = || Scene {
            writer: None,
            cards: Vec::new(),
            claim: [None; NSLOTS],
            words: Vec::new(),
            clues: Default::default(),
            movable: Vec::new(),
            locked: 0,
        };
        match s.phase() {
            Phase::Lobby | Phase::End => empty(),
            Phase::Writing => {
                let Some(me) = self.seated_writing() else { return empty() };
                let d = s.deal(me);
                Scene {
                    writer: Some(me),
                    cards: (0..NSLOTS)
                        .map(|i| Card { loc: Loc::Slot(i as u8), rot: d.sol_rot[i], held: 0, stamp: Stamp::default() })
                        .collect(),
                    claim: [Some(0), Some(1), Some(2), Some(3)],
                    words: d.words[..NSLOTS].to_vec(),
                    clues: self.my_sub().clues,
                    movable: vec![false; NSLOTS],
                    locked: 0b1111,
                }
            }
            Phase::Guessing(k) => {
                let w = s.puzzles()[k];
                let d = s.deal(w);
                let (cards, claim) = s.layout(w);
                let check = s.check(w);
                let locked = if check.done() { 0b1111 } else { check.mask };
                let can = self.seat.is_some_and(|me| me != w) && !check.done();
                Scene {
                    writer: Some(w),
                    movable: (0..cards.len()).map(|i| can && (i >= NSLOTS || locked & (1 << i) == 0)).collect(),
                    words: d.words[..cards.len()].to_vec(),
                    clues: s.effective(w).map(|x| x.clues.clone()).unwrap_or_default(),
                    cards,
                    claim,
                    locked,
                }
            }
        }
    }

    // ---- geometry -----------------------------------------------------------

    /// Board fills the screen; while guessing, a tray for loose cards goes
    /// below (portrait) or beside (landscape) it.
    pub fn layout(&self) -> Layout {
        let (w, h) = (self.w, self.h);
        let ext = 2.0 * EXTENT;
        if !matches!(self.shared.phase(), Phase::Guessing(_)) {
            let k = ((w - 2.0 * PAD).min(h - 2.0 * PAD) / ext).max(0.05);
            return Layout { k, bc: (w / 2.0, h / 2.0), tray: None };
        }
        if h >= w {
            let k = ((w - 2.0 * PAD) / ext).min((h - 3.0 * PAD) / (ext + TRAY_UNITS)).max(0.05);
            let bh = ext * k;
            let tray_y = PAD + bh + PAD;
            Layout { k, bc: (w / 2.0, PAD + bh / 2.0), tray: Some((PAD, tray_y, w - 2.0 * PAD, h - tray_y - PAD, true)) }
        } else {
            let k = ((h - 2.0 * PAD) / ext).min((w - 3.0 * PAD) / (ext + TRAY_UNITS)).max(0.05);
            let bw = ext * k;
            // Keep the group centred; the tray gets at most ~3 card widths.
            let tray_w = (w - bw - 3.0 * PAD).min(3.2 * CARD * k).max(TRAY_UNITS * k);
            let left = ((w - bw - tray_w - PAD) / 2.0).max(PAD);
            Layout {
                k,
                bc: (left + bw / 2.0, h / 2.0),
                tray: Some((left + bw + PAD, PAD, tray_w, h - 2.0 * PAD, false)),
            }
        }
    }

    pub fn scale(&self) -> f32 {
        self.layout().k
    }

    /// Board units → CSS px.
    pub fn board_m(&self) -> M {
        let l = self.layout();
        M::t(l.bc.0, l.bc.1).then(M::s(l.k)).then(M::r(self.angle))
    }

    /// Tray coords (0..1 along long, short axis) → CSS px of a card centre.
    pub fn tray_px(&self, u: f32, v: f32) -> (f32, f32) {
        let l = self.layout();
        let Some((x, y, w, h, horiz)) = l.tray else { return (-1000.0, -1000.0) };
        let half = HALF * l.k * self.zoom + 4.0;
        let along = |o: f32, len: f32, t: f32| o + half + t.clamp(0.0, 1.0) * (len - 2.0 * half).max(0.0);
        if horiz { (along(x, w, u), along(y, h, v)) } else { (along(x, w, v), along(y, h, u)) }
    }

    /// CSS px → tray coords (clamped).
    pub fn px_tray(&self, px: f32, py: f32) -> (f32, f32) {
        let l = self.layout();
        let Some((x, y, w, h, horiz)) = l.tray else { return (0.5, 0.5) };
        let half = HALF * l.k * self.zoom + 4.0;
        let t = |o: f32, len: f32, p: f32| ((p - o - half) / (len - 2.0 * half).max(1.0)).clamp(0.0, 1.0);
        if horiz { (t(x, w, px), t(y, h, py)) } else { (t(y, h, py), t(x, w, px)) }
    }

    pub fn set_zoom(&mut self, z: f32) {
        self.zoom = if z.is_finite() { z.clamp(ZOOM_MIN, ZOOM_MAX) } else { 1.0 };
    }

    /// Card-local units → CSS px, for a card's resting place.
    pub fn card_m(&self, c: &Card) -> M {
        let r = M::r(turn(c.rot)).then(M::s(self.zoom));
        match c.loc {
            Loc::Table { x, y } => {
                let (px, py) = self.tray_px(x, y);
                M::t(px, py).then(M::s(self.scale())).then(r)
            }
            Loc::Slot(s) => {
                let (x, y) = slot_pos(s as usize);
                self.board_m().then(M::t(x, y)).then(r)
            }
            Loc::Float { x, y } => self.board_m().then(M::t(x, y)).then(r),
        }
    }

    fn pose_m(&self, p: &Pose) -> M {
        if p.board {
            self.board_m().then(M::t(p.x, p.y)).then(M::r(p.a)).then(M::s(self.zoom * p.s))
        } else {
            let (px, py) = self.tray_px(p.x, p.y);
            M::t(px, py).then(M::s(self.scale())).then(M::r(p.a)).then(M::s(self.zoom * p.s))
        }
    }

    fn target_pose(c: &Card) -> Pose {
        let a = turn(c.rot);
        match c.loc {
            Loc::Table { x, y } => Pose { board: false, x, y, a, s: 1.0, wait: 0.0 },
            Loc::Slot(s) => {
                let (x, y) = slot_pos(s as usize);
                Pose { board: true, x, y, a, s: 1.0, wait: 0.0 }
            }
            Loc::Float { x, y } => Pose { board: true, x, y, a, s: 1.0, wait: 0.0 },
        }
    }

    /// Re-express a pose in the other frame without moving it on screen.
    fn reframe(&self, p: Pose, board: bool) -> Pose {
        if p.board == board {
            return p;
        }
        let (px, py) = self.pose_m(&Pose { a: 0.0, ..p }).apply(0.0, 0.0);
        if board {
            let (x, y) = self.board_m().inv().apply(px, py);
            Pose { board, x, y, a: p.a - self.angle, ..p }
        } else {
            let (x, y) = self.px_tray(px, py);
            Pose { board, x, y, a: p.a + self.angle, ..p }
        }
    }

    fn dragging_card(&self) -> Option<usize> {
        self.press.as_ref().filter(|p| p.moved).and_then(|p| match p.target {
            Target::Card(i) => Some(i),
            _ => None,
        })
    }

    /// Back-to-front draw order: like a real table, the card touched last
    /// is on top; cards being held or dragged float above everything.
    fn draw_order(&self, cards: &[Card]) -> Vec<usize> {
        let dragging = self.dragging_card();
        let rank = |i: usize| -> u8 {
            match (Some(i) == dragging, cards[i].held) {
                (true, _) => 2,
                (false, 0) => 0,
                _ => 1,
            }
        };
        let mut v: Vec<usize> = (0..cards.len()).collect();
        v.sort_by_key(|&i| (rank(i), cards[i].stamp));
        v
    }

    fn leaf_m_at(&self, d: u8, angle: f32) -> M {
        let l = self.layout();
        M::t(l.bc.0, l.bc.1).then(M::s(l.k)).then(M::r(angle + turn(d)))
    }

    fn leaf_mid() -> f32 {
        -(LEAF_IN + LEAF_OUT) / 2.0
    }

    fn hit(&self, px: f32, py: f32) -> Target {
        let (bx, by) = self.board_m().inv().apply(px, py);
        if bx.hypot(by) <= HUB_R + 6.0 {
            return Target::Hub;
        }
        let sc = self.scene();
        let card = self.draw_order(&sc.cards).into_iter().rev().find(|&i| {
            let (lx, ly) = self.card_m(&sc.cards[i]).inv().apply(px, py);
            lx.abs() <= HALF && ly.abs() <= HALF
        });
        if let Some(i) = card.filter(|&i| sc.movable[i]) {
            return Target::Card(i);
        }
        let r = bx.hypot(by);
        if r <= BOARD_HALF || card.is_some() {
            return Target::Board;
        }
        // Anywhere on a leaf (outside the centre) belongs to that leaf.
        if let Some(d) = (0..4).find(|&d| {
            let (x, y) = self.leaf_m_at(d, self.angle).inv().apply(px, py);
            -y >= x.abs() && -y <= EXTENT + 10.0
        }) {
            return Target::Petal(d);
        }
        Target::Empty
    }

    // ---- input --------------------------------------------------------------

    pub fn pointer_down(&mut self, sx: f32, sy: f32) {
        let target = self.hit(sx, sy);
        let (grab, origin) = match target {
            Target::Card(i) => {
                let c = self.scene().cards[i];
                let (cx, cy) = self.card_m(&c).apply(0.0, 0.0);
                ((cx - sx, cy - sy), Some(c))
            }
            _ => ((0.0, 0.0), None),
        };
        self.press = Some(Press { sx, sy, target, grab, origin, moved: false, spin: None });
    }

    pub fn pointer_move(&mut self, sx: f32, sy: f32) {
        if matches!(self.shared.phase(), Phase::Guessing(_)) && self.seat.is_some() {
            let (bx, by) = self.board_m().inv().apply(sx, sy);
            let on_board = bx.abs() <= EXTENT && by.abs() <= EXTENT;
            self.my_cursor = if on_board {
                (2, bx, by)
            } else {
                let (u, v) = self.px_tray(sx, sy);
                (1, u, v)
            };
            self.cursor_dirty = true;
        }

        let Some(p) = self.press.as_mut() else { return };
        if !p.moved && (sx - p.sx).hypot(sy - p.sy) < DRAG_THRESHOLD {
            return;
        }
        p.moved = true;
        match p.target {
            Target::Card(i) => {
                let (cx, cy) = (sx + p.grab.0, sy + p.grab.1);
                self.drag_card(i, cx, cy);
            }
            _ => {
                // Dragging anywhere else spins my view of the board.
                let (psx, psy, spin) = (p.sx, p.sy, p.spin);
                let bc = self.layout().bc;
                let ang = |x: f32, y: f32| (y - bc.1).atan2(x - bc.0);
                let start = spin.unwrap_or((ang(psx, psy), self.angle));
                self.angle = start.1 + wrap(ang(sx, sy) - start.0);
                let a = self.angle;
                if let Some(p) = self.press.as_mut() {
                    p.spin = Some((ang(sx, sy), a));
                }
            }
        }
    }

    fn drag_card(&mut self, i: usize, cx: f32, cy: f32) {
        let sc = self.scene();
        let (Some(w), Some(me)) = (sc.writer, self.seat) else { return };
        let cur = sc.cards[i];
        let in_board_frame = !matches!(cur.loc, Loc::Table { .. });
        let (bx, by) = self.board_m().inv().apply(cx, cy);
        let tray_hit =
            self.layout().tray.is_some_and(|(x, y, tw, th, _)| cx >= x && cx <= x + tw && cy >= y && cy <= y + th);
        let to_board = !tray_hit && bx.abs() <= EXTENT && by.abs() <= EXTENT;
        // Keep the card's on-screen orientation when it changes frame.
        let rot = match (in_board_frame, to_board) {
            (false, true) => quarter(cur.rot as i32 - self.q),
            (true, false) => quarter(cur.rot as i32 + self.q),
            _ => cur.rot,
        };
        let loc = if to_board {
            Loc::Float { x: bx.clamp(HALF - EXTENT, EXTENT - HALF), y: by.clamp(HALF - EXTENT, EXTENT - HALF) }
        } else {
            let (u, v) = self.px_tray(cx, cy);
            Loc::Table { x: u, y: v }
        };
        self.set_card(w, i, loc, rot, me);
        // My own drag follows the finger exactly.
        let pose = Self::target_pose(&self.shared.cards[&(w, i as u8)]);
        self.poses.insert((w, i as u8), pose);
    }

    pub fn pointer_up(&mut self, sx: f32, sy: f32) {
        self.pointer_move(sx, sy);
        let Some(p) = self.press.take() else { return };
        if p.spin.is_some() {
            self.q = (self.angle / FRAC_PI_2).round() as i32;
            return;
        }
        match (p.target, p.moved) {
            (Target::Card(i), true) => self.drop_card(i, p.origin),
            (Target::Card(i), false) => {
                let sc = self.scene();
                let c = sc.cards[i];
                self.set_card(sc.writer.unwrap(), i, c.loc, (c.rot + 1) % 4, 0);
            }
            // Tapping a leaf never turns the board: it only edits (when writing).
            (Target::Petal(d), false) => {
                if self.seated_writing().is_some() {
                    self.edit_clue(d);
                }
            }
            (Target::Hub | Target::Board, false) => self.q += 1,
            _ => {}
        }
        self.flush_cards();
    }

    /// Cards stay exactly where they're dropped (so players can leave
    /// "maybe" placements), except very close to a spot, where they snap. A
    /// card already snapped there is pushed back to where this one came from.
    fn drop_card(&mut self, i: usize, origin: Option<Card>) {
        let sc = self.scene();
        let Some(w) = sc.writer else { return };
        let c = sc.cards[i];
        let Loc::Float { x, y } = c.loc else {
            return self.set_card(w, i, c.loc, c.rot, 0);
        };
        let spot = (0..NSLOTS).filter(|&s| sc.locked & (1 << s) == 0).find(|&s| {
            let (sx, sy) = slot_pos(s);
            (x - sx).hypot(y - sy) <= SNAP_R
        });
        let Some(s) = spot else {
            return self.set_card(w, i, c.loc, c.rot, 0);
        };
        let occupant = (0..sc.cards.len()).find(|&j| j != i && sc.cards[j].loc == Loc::Slot(s as u8));
        if let (Some(k), Some(o)) = (occupant, origin) {
            let occ = sc.cards[k];
            let rot = match o.loc {
                Loc::Table { .. } => quarter(occ.rot as i32 + self.q),
                _ => occ.rot,
            };
            self.set_card(w, k, o.loc, rot, 0);
        }
        self.set_card(w, i, Loc::Slot(s as u8), c.rot, 0);
    }

    /// Turn my view so leaf `d` sits at the top, reading upright.
    pub fn face_petal(&mut self, d: u8) {
        let delta = quarter(-(d as i32) - self.q) as i32;
        self.q += if delta == 3 { -1 } else { delta };
    }

    /// Open the editor over leaf `d`, lying along the leaf exactly like its
    /// clue is drawn (vertical for side leaves). The board doesn't turn.
    pub fn edit_clue(&mut self, d: u8) {
        self.editing = Some(d);
        let f = self.leaf_m_at(d, self.q as f32 * FRAC_PI_2).then(M::t(0.0, Self::leaf_mid()));
        let (x, y) = f.apply(0.0, 0.0);
        let angle = f.angle();
        let k = self.scale();
        let vertical = angle.sin().abs() > 0.7;
        let room = if vertical { self.h } else { self.w } - 16.0;
        let w = (2.0 * LEAF_HALF_W * k * 0.95).clamp(180.0_f32.min(room), room);
        let h = ((LEAF_OUT - LEAF_IN) * k * 0.6).max(48.0);
        let mut p = vec![d];
        for v in [x, y, w, h, angle] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(self.my_sub().clues[d as usize].as_bytes());
        self.event(EV_EDIT_CLUE, &p);
    }

    fn put_sub(&mut self, f: impl FnOnce(&mut Sub)) {
        let Some(me) = self.seated_writing() else { return };
        let mut sub = self.my_sub();
        f(&mut sub);
        sub.stamp = self.stamp();
        self.commit(Body::Sub(me, sub));
    }

    /// Save a clue (shared, so another device can take over); with
    /// `advance`, open the next empty leaf. Edits after submitting keep you
    /// submitted (until guessing starts), unless a clue is left empty.
    pub fn set_clue(&mut self, d: u8, text: &str, advance: bool) {
        if d as usize >= NSLOTS || self.seated_writing().is_none() {
            return;
        }
        let text = clamp_text(text, MAX_TEXT);
        self.editing = None;
        if self.my_sub().clues[d as usize] != text {
            self.put_sub(|s| {
                s.ready &= !text.is_empty();
                s.clues[d as usize] = text;
            });
        }
        let clues = self.my_sub().clues;
        if advance && !clues[d as usize].is_empty() {
            if let Some(n) = (1..4).map(|k| (d + k) % 4).find(|&n| clues[n as usize].is_empty()) {
                self.edit_clue(n);
            }
        }
    }

    pub fn set_decoys(&mut self, n: u8) {
        let n = n.min(MAX_DECOYS);
        if self.my_sub().decoys != n {
            self.put_sub(|s| s.decoys = n);
        }
    }

    pub fn rotate_view(&mut self, dir: i32) {
        self.q += dir.signum();
    }

    pub fn can_ready(&self) -> bool {
        self.seated_writing().is_some() && self.my_sub().clues.iter().all(|c| !c.is_empty())
    }

    /// 1 start / next round · 2 toggle ready · 3 check · 4 next clover ·
    /// 5 start guessing without the stragglers · 6 deal new cards · 7 new game.
    pub fn action(&mut self, code: u32) {
        let phase = self.shared.phase();
        match code {
            1 if matches!(phase, Phase::Lobby | Phase::End) => self.new_round(),
            6 => self.new_round(),
            2 if self.can_ready() => {
                let r = !self.my_sub().ready;
                self.put_sub(|s| s.ready = r);
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
            5 if phase == Phase::Writing && self.seat.is_some() && self.ready_count() > 0 => {
                let s = self.stamp();
                self.commit(Body::Start(s));
            }
            7 => self.new_game(),
            _ => {}
        }
    }

    /// Who a new round would deal in: seated players at the table now.
    pub fn dealable(&self) -> Vec<u32> {
        let here = self.present();
        self.shared.players().into_iter().filter(|p| here.contains(p)).collect()
    }

    fn ready_count(&self) -> usize {
        self.shared.round_players().iter().filter(|p| self.shared.is_ready(**p)).count()
    }

    fn new_round(&mut self) {
        let players = self.dealable();
        if players.is_empty() {
            return self.status("Join the table first.");
        }
        // A room with an imported clover opens by guessing it.
        let customs = Custom::all();
        if self.custom_room && self.shared.round.n == 0 && !customs.is_empty() {
            return self.start_custom(customs, players);
        }
        let prev = self.shared.round;
        // Draw down the deck: this round deals after the cards the last one used.
        let deck = if prev.n == 0 { 0 } else { prev.deck + (prev.nseats as usize * MAX_CARDS) as u32 };
        let round = Round { seed: self.rng.next() as u32, n: prev.n + 1, stamp: self.stamp(), deck, ..Round::default() }
            .with_players(&players);
        let op = Op { game: self.shared.game, round, body: Body::None };
        self.shared.apply(&op);
        self.new_round_local();
        self.send(&[Msg::Op(op)]);
    }

    /// Round 1 of a room with imported clovers: straight to guessing them.
    fn start_custom(&mut self, customs: Vec<&'static Custom>, players: Vec<u32>) {
        let mut seats: Vec<u32> = customs.iter().map(|c| c.writer()).collect();
        seats.extend(players);
        let round =
            Round { seed: self.rng.next() as u32, n: 1, stamp: self.stamp(), custom: 1, ..Round::default() }.with_players(&seats);
        let op = Op { game: self.shared.game, round, body: Body::None };
        self.shared.apply(&op);
        self.new_round_local();
        self.send(&[Msg::Op(op)]);
        for c in customs {
            let w = c.writer();
            // Their authors aren't at the table: an inactive seat carries the name.
            let stamp = self.stamp();
            self.commit(Body::Seat(w, Seat { name: c.name.clone(), active: false, stamp }));
            let stamp = self.stamp();
            let sub = Sub { clues: c.clues.clone(), decoys: (c.cards.len() - NSLOTS) as u8, ready: true, stamp };
            self.commit(Body::Sub(w, sub));
        }
        let s = self.stamp();
        self.commit(Body::Start(s));
    }

    /// This round's clovers selected by `mask`, each as a scrambled,
    /// shareable board (see `model::scramble`), separated by blank lines.
    pub fn export_boards(&mut self, mask: u32) -> String {
        let boards: Vec<String> = self
            .shared
            .puzzles()
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .filter_map(|(_, &w)| self.shared.export_board(w, &self.name_of(w)))
            .collect();
        boards.iter().map(|b| scramble(b, &mut self.rng)).collect::<Vec<_>>().join("\n\n")
    }

    /// Scramble boards given in answer order (e.g. typed in from a screenshot).
    pub fn scramble_boards(&mut self, text: &str) -> String {
        text.split("\n\n").map(|b| scramble(b.trim(), &mut self.rng)).collect::<Vec<_>>().join("\n\n")
    }

    /// Fresh scores and seats: everyone here now is dealt in.
    fn new_game(&mut self) {
        let here: Vec<(u32, String)> = self
            .present()
            .into_iter()
            .filter_map(|s| self.shared.seats.get(&s).map(|x| (s, x.name.clone())))
            .collect();
        let game = self.stamp();
        self.shared = Shared { game, ..Shared::default() };
        self.new_round_local();
        self.commit(Body::None);
        for (id, name) in here {
            let stamp = self.stamp();
            self.commit(Body::Seat(id, Seat { name, active: true, stamp }));
        }
    }

    pub fn can_check(&self) -> bool {
        let sc = self.scene();
        match (self.shared.phase(), sc.writer, self.seat) {
            (Phase::Guessing(_), Some(w), Some(me)) => {
                w != me && !self.shared.check(w).done() && sc.claim.iter().all(|o| o.is_some())
            }
            _ => false,
        }
    }

    /// Guess: wrong cards are booted back to the tray, right ones stay and
    /// lock. No scoring and no limit: keep guessing until all four are right.
    fn check(&mut self) {
        if !self.can_check() {
            return;
        }
        let sc = self.scene();
        let w = sc.writer.unwrap();
        let d = self.shared.deal(w);
        let prev = self.shared.check(w);
        let mut mask = prev.mask;
        for s in 0..NSLOTS {
            let k = sc.claim[s].unwrap();
            // Orientation is judged relative to the board.
            if k == s && sc.cards[k].rot == d.sol_rot[s] {
                mask |= 1 << s;
            }
        }
        let stamp = self.stamp();
        let check = Check { attempts: prev.attempts.saturating_add(1), mask, stamp };
        let n = sc.cards.len();
        for s in 0..NSLOTS {
            if mask & (1 << s) == 0 {
                let k = sc.claim[s].unwrap();
                let h = self.shared.home_card(&d, n, k);
                self.set_card(w, k, h.loc, h.rot, 0);
            }
        }
        self.commit(Body::Check(w, check));
        self.flush_cards();
    }

    // ---- frame --------------------------------------------------------------

    pub fn frame(&mut self, now: f64) {
        self.dt = ((now - self.now) / 1000.0).clamp(0.0, 0.1) as f32;
        self.now = now;
        let spinning = self.press.as_ref().is_some_and(|p| p.spin.is_some());
        if !spinning {
            let target = self.q as f32 * FRAC_PI_2;
            let diff = target - self.angle;
            self.angle = if diff.abs() < 1e-3 { target } else { self.angle + diff * (1.0 - (-self.dt * 12.0).exp()) };
        }
        if now - self.last_hello_ms >= HELLO_EVERY_MS {
            self.hello();
        }
        self.maybe_start_guessing();
        let phase = self.shared.phase();
        // A clover just got finished: let the leaves glow briefly.
        let done_now = match phase {
            Phase::Guessing(k) => {
                let w = self.shared.puzzles()[k];
                Some((w, self.shared.check(w).done()))
            }
            _ => None,
        };
        if let (Some((w, true)), Some((pw, false))) = (done_now, self.last_done) {
            if w == pw {
                self.glow = 1.0;
                self.solved_at = now;
            }
        }
        // A guess was made (here or by anyone): cards that just left the
        // board for the tray were booted, so give them a shake on the way.
        if let Phase::Guessing(k) = phase {
            let w = self.shared.puzzles()[k];
            let g = self.shared.check(w).attempts;
            if self.last_guess.is_some_and(|(pw, pg)| pw == w && g > pg) {
                let sc = self.scene();
                for (i, c) in sc.cards.iter().enumerate() {
                    let was_on_board = self.poses.get(&(w, i as u8)).is_some_and(|p| p.board);
                    if matches!(c.loc, Loc::Table { .. }) && was_on_board {
                        self.shakes.insert((w, i as u8), now);
                    }
                }
            }
            self.last_guess = Some((w, g));
        }
        self.shakes.retain(|_, t| now - *t < 600.0);
        self.last_done = done_now;
        self.glow = (self.glow - self.dt / 1.8).max(0.0);
        if phase != self.last_phase {
            // New phase or new clover: spin back upright the short way.
            self.last_phase = phase;
            self.q = (self.q as f32 / 4.0).round() as i32 * 4;
            self.poses.clear();
        }
        self.flush_cards();
        if self.cursor_dirty && now - self.last_cursor_ms >= 33.0 {
            self.cursor_dirty = false;
            self.last_cursor_ms = now;
            let (kind, x, y) = self.my_cursor;
            let (device, seat) = (self.device, self.seat.unwrap_or(0));
            self.send(&[Msg::Cursor { device, seat, kind, x, y }]);
        }
        self.cursors.retain(|_, c| now - c.seen < CURSOR_TTL_MS);
        self.render();
    }

    /// Guessing starts on its own only once everyone dealt into the round
    /// has submitted, online or not. This must not depend on who *this*
    /// device can see: in a mesh, a device that hasn't connected to someone
    /// yet would start without them. Anyone can override ("Proceed with 3 of
    /// 5").
    fn maybe_start_guessing(&mut self) {
        if self.shared.phase() != Phase::Writing {
            return;
        }
        let players = self.shared.round_players();
        if !players.is_empty() && players.iter().all(|p| self.shared.is_ready(*p)) {
            let s = self.stamp();
            self.commit(Body::Start(s));
        }
    }

    /// Skip animations (tests / harness).
    pub fn settle(&mut self) {
        self.angle = self.q as f32 * FRAC_PI_2;
        self.poses.clear();
        let sc = self.scene();
        if let Some(w) = sc.writer {
            for (i, c) in sc.cards.iter().enumerate() {
                self.poses.insert((w, i as u8), Self::target_pose(c));
            }
        }
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
    fn text(&mut self, x: f32, y: f32, size: f32, color: u32, font: u8, s: &str, maxw: f32) {
        let off = self.strs.len() as f32;
        self.strs.extend_from_slice(s.as_bytes());
        self.cmds.extend_from_slice(&[
            C_TEXT,
            x,
            y,
            size,
            f32::from_bits(color),
            font as f32,
            off,
            s.len() as f32,
            maxw,
        ]);
    }

    /// Ease displayed card poses toward where the cards really are. Cards
    /// that appear (a new clover) are dealt in from the centre, staggered.
    fn animate(&mut self, sc: &Scene) {
        let Some(w) = sc.writer else { return };
        let k = 1.0 - (-self.dt * 14.0).exp();
        let dragging = self.dragging_card();
        for (i, c) in sc.cards.iter().enumerate() {
            let key = (w, i as u8);
            let target = Self::target_pose(c);
            let pose = match self.poses.get(&key) {
                Some(p) if Some(i) != dragging => {
                    let p = self.reframe(*p, target.board);
                    if p.wait > 0.0 {
                        Pose { wait: p.wait - self.dt, ..p }
                    } else {
                        let close = (p.x - target.x).abs() + (p.y - target.y).abs() + (p.s - 1.0).abs() < 1e-3
                            && wrap(target.a - p.a).abs() < 1e-3;
                        if close {
                            target
                        } else {
                            Pose {
                                board: target.board,
                                x: p.x + (target.x - p.x) * k,
                                y: p.y + (target.y - p.y) * k,
                                a: p.a + wrap(target.a - p.a) * k,
                                s: p.s + (1.0 - p.s) * k,
                                wait: 0.0,
                            }
                        }
                    }
                }
                None if self.motion && Some(i) != dragging => {
                    Pose { board: true, x: 0.0, y: 0.0, a: target.a - 0.6, s: 0.35, wait: i as f32 * 0.06 }
                }
                _ => target,
            };
            self.poses.insert(key, pose);
        }
    }

    /// The two words leaf `d` must link, for the writer: the words on the
    /// cards either side of it that face it.
    fn keywords(&self, sc: &Scene, d: u8) -> Option<String> {
        if self.shared.phase() != Phase::Writing || sc.writer.is_none() || sc.cards.len() < NSLOTS {
            return None;
        }
        let word = |s: usize| {
            let side = (d as i32 - sc.cards[s].rot as i32).rem_euclid(4) as usize;
            sc.words[s][side]
        };
        Some(format!("{} & {}", word(d as usize), word((d as usize + 1) % NSLOTS)))
    }

    pub fn render(&mut self) {
        self.cmds.clear();
        self.strs.clear();
        self.cmds.extend_from_slice(&[C_CLEAR, f32::from_bits(BG)]);
        let sc = self.scene();
        let phase = self.shared.phase();
        let l = self.layout();
        self.animate(&sc);

        if let Some((x, y, w, h, _)) = l.tray {
            self.xform(M::ID);
            self.rrect(x, y, w, h, 18.0, TRAY, TRAY_EDGE, 1.5);
        }

        // The clover: four heart-shaped leaves (one clue each) round a centre.
        let board = self.board_m();
        let glow = if self.motion { self.glow * (0.6 + 0.4 * (self.now as f32 / 180.0).sin()) } else { 0.0 };
        let mid = Self::leaf_mid();
        for d in 0..4u8 {
            self.xform(self.leaf_m_at(d, self.angle));
            let fill = if self.editing == Some(d) { LEAF_ACTIVE } else { mix(LEAF, LEAF_GLOW, glow) };
            self.cmds.extend_from_slice(&[C_LEAF, 290.0, 430.0, f32::from_bits(fill), f32::from_bits(LEAF_EDGE), 4.0]);
        }
        self.xform(board);
        self.circle(0.0, 0.0, BOARD_HALF, CENTRE, CENTRE_EDGE, 4.0);
        for d in 0..4u8 {
            self.xform(self.leaf_m_at(d, self.angle));
            let clue = sc.clues[d as usize].clone();
            let keys = self.keywords(&sc, d);
            let maxw = 2.0 * LEAF_HALF_W - 30.0;
            match (clue.is_empty(), keys) {
                (false, Some(k)) => {
                    self.text(0.0, mid - 18.0, 48.0, CLUE_INK, F_SERIF, &clue, maxw);
                    self.text(0.0, mid + 30.0, 30.0, KEYWORD_INK, F_SANS_BOLD, &k, maxw);
                }
                (false, None) => self.text(0.0, mid, 54.0, CLUE_INK, F_SERIF, &clue, maxw),
                (true, Some(k)) => self.text(0.0, mid, 38.0, KEYWORD_INK, F_SANS_BOLD, &k, maxw),
                (true, None) => {}
            }
        }
        self.xform(board);
        for s in 0..NSLOTS {
            let (x, y) = slot_pos(s);
            let solved = sc.locked & (1 << s) != 0 && phase != Phase::Writing;
            let claimed = sc.claim[s].is_some() && matches!(phase, Phase::Guessing(_));
            let (fill, stroke) = match (solved, claimed) {
                (true, _) => (0x9be7a8aa, 0xffffff60),
                (false, true) => (0x0000002e, GOLD),
                _ => (0x0000002e, 0xffffff38),
            };
            self.rrect(x - HALF - 6.0, y - HALF - 6.0, CARD + 12.0, CARD + 12.0, 18.0, fill, stroke, 3.0);
        }

        // Cards.
        let dragging = self.dragging_card();
        let w = sc.writer.unwrap_or(0);
        for i in self.draw_order(&sc.cards) {
            let c = sc.cards[i];
            let pose = self.poses.get(&(w, i as u8)).copied().unwrap_or_else(|| Self::target_pose(&c));
            let mut f = self.pose_m(&pose);
            if self.motion {
                // Booted: a quick, decaying shake as it heads back.
                if let Some(t0) = self.shakes.get(&(w, i as u8)) {
                    let t = ((self.now - t0) / 1000.0) as f32;
                    let dx = (t * 48.0).sin() * 9.0 * (1.0 - t / 0.6).max(0.0);
                    f = M::t(dx, 0.0).then(f);
                }
                // Solved: one gentle pulse.
                let t = ((self.now - self.solved_at) / 1000.0) as f32;
                if (0.0..0.6).contains(&t) && matches!(c.loc, Loc::Slot(_)) {
                    let k = 1.0 + 0.07 * (t / 0.6 * PI).sin();
                    let (cx, cy) = f.apply(0.0, 0.0);
                    f = M::t(cx, cy).then(M::s(k)).then(M::t(-cx, -cy)).then(f);
                }
            }
            if c.held != 0 || Some(i) == dragging {
                self.xform(M::t(5.0, 8.0).then(f));
                self.rrect(-HALF, -HALF, CARD, CARD, 18.0, 0x00000055, 0, 0.0);
            }
            self.xform(f);
            let solved = matches!(c.loc, Loc::Slot(s) if sc.locked & (1 << s) != 0) && phase != Phase::Writing;
            let (stroke, lw) = match c.held {
                0 if solved => (0x2fa84fff, 6.0),
                0 => (PAPER_EDGE, 2.0),
                h if Some(h) == self.seat => (0xffffffff, 4.0),
                h => (self.color_of(h), 7.0),
            };
            self.rrect(-HALF, -HALF, CARD, CARD, 18.0, PAPER, stroke, lw);
            self.circle(0.0, 0.0, 11.0, 0xeee4c8ff, 0, 0.0);
            for s in 0..4u8 {
                self.xform(f.then(M::r(turn(s))));
                self.text(0.0, -WORD_INSET, WORD_SIZE, INK, F_SANS_BOLD, sc.words[i][s as usize], WORD_MAX_W);
            }
        }

        // Hub: tap to turn the board.
        self.xform(board);
        self.circle(0.0, 0.0, HUB_R, HUB, PAPER, 3.0);
        self.xform(M::t(l.bc.0, l.bc.1).then(M::s(l.k)));
        self.text(0.0, 1.0, 32.0, PAPER, F_SANS_BOLD, "⟳", 44.0);

        // Other guessers' pointers.
        if matches!(phase, Phase::Guessing(_)) {
            let cursors: Vec<(u32, u8, f32, f32)> = self.cursors.values().map(|c| (c.seat, c.kind, c.x, c.y)).collect();
            for (seat, kind, x, y) in cursors {
                if seat == 0 {
                    continue;
                }
                let m = match kind {
                    1 => {
                        let (px, py) = self.tray_px(x, y);
                        M::t(px, py)
                    }
                    2 => self.board_m().then(M::t(x, y)).then(M::s(1.0 / l.k)),
                    _ => continue,
                };
                let col = self.color_of(seat);
                self.xform(m);
                self.circle(0.0, 0.0, 7.0, col, 0xffffffff, 2.5);
            }
        }
    }

    // ---- UI state for the HTML shell -------------------------------------------

    pub fn ui_json(&self) -> String {
        let s = &self.shared;
        let phase = s.phase();
        let sc = self.scene();
        let present = self.present();
        let writer = sc.writer.filter(|_| matches!(phase, Phase::Guessing(_)));
        let players = s.players();
        let rows: Vec<String> = players
            .iter()
            .map(|&p| {
                let status = match phase {
                    Phase::Writing if !s.round.players().contains(&p) => "next",
                    Phase::Writing if s.is_ready(p) => "ready",
                    Phase::Writing => "writing",
                    Phase::Guessing(_) if writer == Some(p) => "writer",
                    _ => "",
                };
                format!(
                    "{{\"id\":{p},\"name\":{},\"color\":\"#{:06x}\",\"you\":{},\"away\":{},\"status\":\"{status}\"}}",
                    json_str(&self.name_of(p)),
                    self.color_of(p) >> 8,
                    self.seat == Some(p),
                    !present.contains(&p),
                )
            })
            .collect();
        let (pname, k, n) = match phase {
            Phase::Lobby => ("lobby", 0, 0),
            Phase::Writing => ("writing", 0, 0),
            Phase::Guessing(k) => ("guessing", k, s.puzzles().len()),
            Phase::End => ("end", 0, s.puzzles().len()),
        };
        // The round's progress: one entry per dealt-in player.
        let progress: Vec<String> = s
            .round_players()
            .iter()
            .map(|&p| {
                format!(
                    "{{\"name\":{},\"color\":\"#{:06x}\",\"ready\":{},\"away\":{}}}",
                    json_str(&self.name_of(p)),
                    self.color_of(p) >> 8,
                    s.is_ready(p),
                    !present.contains(&p)
                )
            })
            .collect();
        let boards: Vec<String> = s
            .puzzles()
            .iter()
            .map(|&w| format!("{{\"name\":{},\"solved\":{}}}", json_str(&self.name_of(w)), s.check(w).done()))
            .collect();
        let deal_names: Vec<String> = self.dealable().iter().map(|p| json_str(&self.name_of(*p))).collect();
        let sub = self.my_sub();
        let check = writer.map(|w| s.check(w)).unwrap_or_default();
        let seated = self.seat.is_some_and(|me| players.contains(&me));
        let in_round = self.seat.is_some_and(|me| s.round.players().contains(&me));
        format!(
            "{{\"phase\":\"{pname}\",\"seat\":{},\"seated\":{seated},\"name\":{},\"players\":[{}],\"watchers\":{},\
             \"inRound\":{in_round},\"ready\":{},\"canReady\":{},\"filled\":{},\"decoys\":{},\"progress\":[{}],\"readyCount\":{},\"dealable\":[{}],\
             \"k\":{k},\"n\":{n},\"writerName\":{},\"writerDecoys\":{},\"isWriter\":{},\"attempts\":{},\"done\":{},\
             \"canCheck\":{},\"claimed\":{},\"round\":{},\"boards\":[{}]}}",
            self.seat.map_or("null".into(), |x| x.to_string()),
            self.seat.map_or("null".into(), |x| json_str(&self.name_of(x))),
            rows.join(","),
            self.watchers(),
            sub.ready,
            self.can_ready(),
            sub.clues.iter().filter(|c| !c.is_empty()).count(),
            sub.decoys,
            progress.join(","),
            self.ready_count(),
            deal_names.join(","),
            writer.map_or("null".into(), |w| json_str(&self.name_of(w))),
            writer.map_or(0, |w| s.ncards(w) - NSLOTS),
            writer.is_some() && writer == self.seat,
            check.attempts,
            check.done(),
            self.can_check(),
            sc.claim.iter().filter(|c| c.is_some()).count(),
            s.round.n,
            boards.join(","),
        )
    }

    /// Text/background colour pairs actually used, for contrast checks.
    pub fn palette_json() -> String {
        let pairs = [
            ("card words", INK, PAPER),
            ("clue on leaf", CLUE_INK, LEAF),
            ("clue on glowing leaf", CLUE_INK, LEAF_GLOW),
            ("clue on active leaf", CLUE_INK, LEAF_ACTIVE),
            ("keywords on leaf", KEYWORD_INK, LEAF),
            ("keywords on active leaf", KEYWORD_INK, LEAF_ACTIVE),
            ("hub glyph", PAPER, HUB),
        ];
        let rows: Vec<String> =
            pairs.iter().map(|(n, f, b)| format!("[\"{n}\",\"#{:06x}\",\"#{:06x}\"]", f >> 8, b >> 8)).collect();
        format!("[{}]", rows.join(","))
    }

    // ---- harness probes ------------------------------------------------------

    /// Screen centre (CSS px) and on-screen angle of scene card `i`, at rest.
    pub fn card_screen(&self, i: usize) -> (f32, f32, f32) {
        let sc = self.scene();
        let Some(c) = sc.cards.get(i) else { return (-1.0, -1.0, 0.0) };
        let f = self.card_m(c);
        let (x, y) = f.apply(0.0, 0.0);
        (x, y, f.angle())
    }

    pub fn slot_screen(&self, s: usize) -> (f32, f32) {
        let (x, y) = slot_pos(s);
        self.board_m().apply(x, y)
    }

    pub fn board_screen(&self, x: f32, y: f32) -> (f32, f32) {
        self.board_m().apply(x, y)
    }

    pub fn petal_screen(&self, d: u8) -> (f32, f32) {
        self.leaf_m_at(d, self.angle).apply(0.0, Self::leaf_mid())
    }

    pub fn hub_screen(&self) -> (f32, f32) {
        self.layout().bc
    }

    pub fn shaking(&self) -> usize {
        self.shakes.len()
    }

    /// Displayed (animated) on-screen angle of card `i`.
    pub fn shown_angle(&self, i: usize) -> f32 {
        let sc = self.scene();
        let w = sc.writer.unwrap_or(0);
        self.poses.get(&(w, i as u8)).map_or(f32::NAN, |p| self.pose_m(p).angle())
    }
}
