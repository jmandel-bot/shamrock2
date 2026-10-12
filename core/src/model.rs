//! Shared, replicated game state.
//!
//! Every op carries the game and round it belongs to. A newer game resets
//! everything; a newer round (same game) resets round state but keeps seats
//! and the score ledger. Within a round each value is a register merged by a
//! monotone rule (LWW by Lamport stamp, or max for counters). `apply` is
//! commutative, associative and idempotent, so replicas converge regardless
//! of delivery order or duplication.

use crate::words::CARDS;
use std::collections::BTreeMap;

pub const NSLOTS: usize = 4;
pub const MAX_DECOYS: u8 = 4;
pub const MAX_CARDS: usize = NSLOTS + MAX_DECOYS as usize;

// Board geometry, in board units (cards are 150 units).
pub const CARD: f32 = 210.0;
pub const HALF: f32 = CARD / 2.0;
pub const SLOT_OFF: f32 = 140.0;
/// Radius of the clover's round centre, where the four spots are.
pub const BOARD_HALF: f32 = 300.0;
/// Clue leaves sit between these distances from the centre.
pub const LEAF_IN: f32 = 290.0;
pub const LEAF_OUT: f32 = 400.0;
pub const LEAF_HALF_W: f32 = 205.0;
/// Half-extent of everything on the board, leaves included.
pub const EXTENT: f32 = 432.0;
pub const HUB_R: f32 = 30.0;
/// Dropping within this distance of a free spot snaps into it.
pub const SNAP_R: f32 = 40.0;
/// A card within this distance of a spot counts as "in" it for a check.
pub const CLAIM_R: f32 = 100.0;

pub const MAX_TEXT: usize = 60;
pub const MAX_PLAYERS: usize = 12;
pub const PERFECT_SCORE: u8 = 6;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Stamp {
    pub clock: u64,
    pub peer: u32,
}

/// `Table` is a spot in the tray, normalised to 0..=1 along the tray's long
/// (x) and short (y) axes, so it fits every screen; it never turns with
/// anyone's view. `Slot` and `Float` are in board units and turn with the
/// board.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Loc {
    Table { x: f32, y: f32 },
    Slot(u8),
    Float { x: f32, y: f32 },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Card {
    pub loc: Loc,
    /// Clockwise quarter turns, relative to the frame `loc` lives in.
    pub rot: u8,
    /// Seat currently dragging this card (0 = nobody).
    pub held: u32,
    pub stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Round {
    pub seed: u32,
    /// 1-based round number within the game (0 = no round yet).
    pub n: u16,
    pub stamp: Stamp,
    /// Seats dealt into this round (fixed when the round starts).
    pub seats: [u32; MAX_PLAYERS],
    pub nseats: u8,
    /// Where this round starts dealing in the game's deck (each dealt-in
    /// player takes `MAX_CARDS` cards).
    pub deck: u32,
    /// 1 = this round guesses the imported clover (see `Custom`).
    pub custom: u8,
}

impl Round {
    pub fn players(&self) -> &[u32] {
        &self.seats[..(self.nseats as usize).min(MAX_PLAYERS)]
    }
    pub fn with_players(mut self, players: &[u32]) -> Round {
        let n = players.len().min(MAX_PLAYERS);
        self.seats = [0; MAX_PLAYERS];
        self.seats[..n].copy_from_slice(&players[..n]);
        self.nseats = n as u8;
        self
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Seat {
    pub name: String,
    pub active: bool,
    pub stamp: Stamp,
}

/// A writer's clues (drafts until `ready`) and decoy bid.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sub {
    pub clues: [String; NSLOTS],
    pub decoys: u8,
    pub ready: bool,
    pub stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Check {
    pub attempts: u8,
    /// Bit s set = spot s was right at the last check (and is now locked).
    pub mask: u8,
    pub stamp: Stamp,
}

impl Check {
    pub fn done(&self) -> bool {
        self.attempts >= 2 || self.mask == 0b1111
    }
    pub fn score(&self) -> u8 {
        match (self.attempts, self.mask) {
            (1, 0b1111) => PERFECT_SCORE,
            _ => self.mask.count_ones() as u8,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Body {
    /// Just announces the game / round.
    None,
    Seat(u32, Seat),
    Sub(u32, Sub),
    /// Writing is over; guessing begins.
    Start(Stamp),
    Card(u32, u8, Card),
    Check(u32, Check),
    /// Index of the clover being guessed (grow-only).
    Next(u16, Stamp),
    /// Points a writer's clover earned in round `n`.
    Ledger(u16, u32, u8, Stamp),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Op {
    pub game: Stamp,
    pub round: Round,
    pub body: Body,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Lobby,
    Writing,
    /// Index into `puzzles()`.
    Guessing(usize),
    End,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Shared {
    pub game: Stamp,
    pub seats: BTreeMap<u32, Seat>,
    /// (round n, writer) → (points, stamp).
    pub ledger: BTreeMap<(u16, u32), (u8, Stamp)>,
    pub round: Round,
    /// Every version of every writer's clues, keyed (writer, stamp). Keeping
    /// versions makes "what was submitted when guessing started" well
    /// defined even when edits and Proceed race.
    pub subs: BTreeMap<(u32, Stamp), Sub>,
    pub start: Option<Stamp>,
    pub cards: BTreeMap<(u32, u8), Card>,
    pub checks: BTreeMap<u32, Check>,
    pub next: (u16, Stamp),
}

/// A clover brought in from elsewhere (e.g. read off a screenshot of an
/// unfinished game), guessed as an ordinary clover. Every device in the room
/// loads the same file, so it behaves like shared state without being sent.
#[derive(Debug)]
pub struct Custom {
    /// Who wrote it, shown as the clover's author.
    pub name: String,
    /// Top, right, bottom, left.
    pub clues: [String; NSLOTS],
    /// Cards 0..4 are the answer (card s in spot s, words clockwise from the
    /// top as they sit there); the rest are decoys.
    pub cards: Vec<[&'static str; 4]>,
}

static CUSTOM: std::sync::OnceLock<Custom> = std::sync::OnceLock::new();

impl Custom {
    /// The imported clover's author gets a seat id no typed name can produce.
    pub fn writer(&self) -> u32 {
        seat_id(&format!("\u{1}{}", self.name))
    }

    /// Format: name, then four clues, then one card per line as `a|b|c|d`
    /// (at least four cards). Returns false (and keeps nothing) if malformed.
    pub fn load(text: &str) -> bool {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let Some(name) = lines.next() else { return false };
        let clues: Vec<String> = lines.by_ref().take(NSLOTS).map(|c| clamp_text(c, MAX_TEXT)).collect();
        let cards: Vec<[&'static str; 4]> = lines
            .take(MAX_CARDS)
            .filter_map(|l| {
                let w: Vec<&'static str> = l.split('|').map(|x| &*Box::leak(x.trim().to_string().into_boxed_str())).collect();
                w.try_into().ok()
            })
            .collect();
        if clues.len() < NSLOTS || cards.len() < NSLOTS {
            return false;
        }
        CUSTOM
            .set(Custom { name: clamp_text(name, 40), clues: core::array::from_fn(|i| clues[i].clone()), cards })
            .is_ok()
    }

    pub fn get() -> Option<&'static Custom> {
        CUSTOM.get()
    }
}

/// Seat id for a display name: names are identities, so choosing an
/// existing name on another device takes that seat over.
pub fn seat_id(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.trim().to_lowercase().bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// Everything about one writer's clover that is derived from the seed.
pub struct Deal {
    pub words: [[&'static str; 4]; MAX_CARDS],
    /// Rotation of card s in spot s (the answer). Card s belongs in spot s.
    pub sol_rot: [u8; NSLOTS],
    /// Shuffle order for tray spots, and starting rotations.
    pub home: [u8; MAX_CARDS],
    pub home_rot: [u8; MAX_CARDS],
}

/// The game's draw-down deck: card at deck position `pos`. Each pass
/// through the deck is a fresh shuffle seeded by the game, so cards don't
/// repeat until the deck runs out.
pub fn deck_card(game: Stamp, pos: u32) -> usize {
    let n = CARDS.len();
    let (pass, at) = (pos as usize / n, pos as usize % n);
    let mut rng = Rng::new(game.clock ^ ((game.peer as u64) << 32) ^ (pass as u64).wrapping_mul(0xa076_1d64_78bd_642f));
    let mut perm: Vec<usize> = (0..n).collect();
    for k in (1..n).rev() {
        perm.swap(k, rng.below(k + 1));
    }
    perm[at]
}

/// Tray spot `k` of `n`: rows of up to three along the tray's long axis,
/// so loose cards don't cover each other's words even on a phone.
pub fn tray_spot(k: usize, n: usize) -> (f32, f32) {
    const PER_ROW: usize = 3;
    let rows = n.div_ceil(PER_ROW).max(1);
    let (row, col) = (k / PER_ROW, k % PER_ROW);
    let in_row = (n - row * PER_ROW).min(PER_ROW);
    let x = 0.5 + (col as f32 - (in_row as f32 - 1.0) / 2.0) / (PER_ROW - 1) as f32;
    let y = if rows == 1 { 0.5 } else { row as f32 / (rows - 1) as f32 };
    (x, y)
}

pub fn slot_pos(s: usize) -> (f32, f32) {
    const P: [(f32, f32); NSLOTS] =
        [(-SLOT_OFF, -SLOT_OFF), (SLOT_OFF, -SLOT_OFF), (SLOT_OFF, SLOT_OFF), (-SLOT_OFF, SLOT_OFF)];
    P[s]
}

/// Board position of a card, if it is on the board.
pub fn board_pos(c: &Card) -> Option<(f32, f32)> {
    match c.loc {
        Loc::Slot(s) => Some(slot_pos(s as usize)),
        Loc::Float { x, y } => Some((x, y)),
        Loc::Table { .. } => None,
    }
}

impl Shared {
    /// Merge one op. Returns true if anything changed.
    pub fn apply(&mut self, op: &Op) -> bool {
        let mut changed = false;
        if op.game > self.game {
            *self = Shared { game: op.game, ..Shared::default() };
            changed = true;
        }
        if op.game != self.game {
            return changed;
        }
        // Game-level registers survive rounds.
        match &op.body {
            Body::Seat(id, s) => {
                if self.seats.get(id).is_none_or(|c| s.stamp > c.stamp) {
                    self.seats.insert(*id, s.clone());
                    changed = true;
                }
            }
            Body::Ledger(n, w, p, s) => {
                if self.ledger.get(&(*n, *w)).is_none_or(|c| *s > c.1) {
                    self.ledger.insert((*n, *w), (*p, *s));
                    changed = true;
                }
            }
            _ => {}
        }
        if op.round.stamp > self.round.stamp {
            self.round = op.round;
            self.subs.clear();
            self.start = None;
            self.cards.clear();
            self.checks.clear();
            self.next = Default::default();
            changed = true;
        }
        if op.round.stamp != self.round.stamp {
            return changed;
        }
        let newer = match &op.body {
            Body::Sub(w, s) => {
                self.subs.insert((*w, s.stamp), s.clone()).is_none()
            }
            Body::Start(s) => {
                let n = self.start.is_none_or(|c| *s > c);
                if n {
                    self.start = Some(*s);
                }
                n
            }
            Body::Card(w, i, c) => {
                let n = self.cards.get(&(*w, *i)).is_none_or(|o| c.stamp > o.stamp);
                if n {
                    self.cards.insert((*w, *i), *c);
                }
                n
            }
            Body::Check(w, c) => {
                let n = self.checks.get(w).is_none_or(|o| (c.attempts, c.stamp) > (o.attempts, o.stamp));
                if n {
                    self.checks.insert(*w, *c);
                }
                n
            }
            Body::Next(k, s) => {
                let n = (*k, *s) > self.next;
                if n {
                    self.next = (*k, *s);
                }
                n
            }
            _ => false,
        };
        changed || newer
    }

    /// Every stored register, for full-state sync with a new peer.
    pub fn all_ops(&self) -> Vec<Op> {
        let (g, r) = (self.game, self.round);
        let op = |body| Op { game: g, round: r, body };
        let mut v = vec![op(Body::None)];
        v.extend(self.seats.iter().map(|(id, s)| op(Body::Seat(*id, s.clone()))));
        v.extend(self.ledger.iter().map(|((n, w), (p, s))| op(Body::Ledger(*n, *w, *p, *s))));
        v.extend(self.subs.iter().map(|((w, _), s)| op(Body::Sub(*w, s.clone()))));
        v.extend(self.start.map(|s| op(Body::Start(s))));
        v.extend(self.cards.iter().map(|((w, i), c)| op(Body::Card(*w, *i, *c))));
        v.extend(self.checks.iter().map(|(w, c)| op(Body::Check(*w, *c))));
        if self.next.0 > 0 {
            v.push(op(Body::Next(self.next.0, self.next.1)));
        }
        v
    }

    /// Active seats, by id.
    pub fn players(&self) -> Vec<u32> {
        self.seats.iter().filter(|(_, s)| s.active).map(|(id, _)| *id).collect()
    }

    /// What `writer` was dealt this round: whole cards from the deck.
    pub fn deal(&self, writer: u32) -> Deal {
        let seat = self.round.players().iter().position(|p| *p == writer).unwrap_or(writer as usize % MAX_PLAYERS);
        let base = self.round.deck + (seat * MAX_CARDS) as u32;
        let cards: [usize; MAX_CARDS] = core::array::from_fn(|i| deck_card(self.game, base + i as u32));
        let mut rng = Rng::new(((self.round.seed as u64) << 32 | writer as u64) ^ 0x9e37_79b9_7f4a_7c15);
        let mut home: [u8; MAX_CARDS] = core::array::from_fn(|i| i as u8);
        for k in (1..MAX_CARDS).rev() {
            home.swap(k, rng.below(k + 1));
        }
        if let Some(c) = Custom::get().filter(|c| self.round.custom == 1 && c.writer() == writer) {
            // The imported answer is exactly as it was laid out: no rotation.
            return Deal {
                words: core::array::from_fn(|i| c.cards.get(i).copied().unwrap_or(CARDS[cards[i]])),
                sol_rot: [0; NSLOTS],
                home,
                home_rot: core::array::from_fn(|_| rng.below(4) as u8),
            };
        }
        Deal {
            words: core::array::from_fn(|c| CARDS[cards[c]]),
            sol_rot: core::array::from_fn(|_| rng.below(4) as u8),
            home,
            home_rot: core::array::from_fn(|_| rng.below(4) as u8),
        }
    }

    /// A writer's latest clues (while writing).
    pub fn latest(&self, w: u32) -> Option<&Sub> {
        self.subs.range((w, Stamp::default())..=(w, Stamp { clock: u64::MAX, peer: u32::MAX })).next_back().map(|(_, s)| s)
    }

    /// A writer's clues as they stood when guessing started (latest otherwise).
    pub fn effective(&self, w: u32) -> Option<&Sub> {
        match self.start {
            Some(start) => self.subs.range((w, Stamp::default())..(w, start)).next_back().map(|(_, s)| s),
            None => self.latest(w),
        }
    }

    pub fn is_ready(&self, w: u32) -> bool {
        self.effective(w).is_some_and(|s| s.ready)
    }

    /// Players in this round who are still seated.
    pub fn round_players(&self) -> Vec<u32> {
        self.round.players().iter().copied().filter(|p| self.seats.get(p).is_some_and(|s| s.active)).collect()
    }

    /// Writers whose clovers get guessed, in deal order: those who had
    /// submitted when guessing started.
    pub fn puzzles(&self) -> Vec<u32> {
        if self.start.is_none() {
            return Vec::new();
        }
        self.round.players().iter().copied().filter(|w| self.is_ready(*w)).collect()
    }

    pub fn phase(&self) -> Phase {
        if self.round.n == 0 {
            Phase::Lobby
        } else if self.start.is_none() {
            Phase::Writing
        } else {
            let k = self.next.0 as usize;
            if k < self.puzzles().len() { Phase::Guessing(k) } else { Phase::End }
        }
    }

    pub fn check(&self, writer: u32) -> Check {
        self.checks.get(&writer).copied().unwrap_or_default()
    }

    pub fn ncards(&self, writer: u32) -> usize {
        NSLOTS + self.effective(writer).map_or(1, |s| s.decoys.min(MAX_DECOYS)) as usize
    }

    pub fn home_card(&self, d: &Deal, n: usize, i: usize) -> Card {
        // Rank of card i among the n cards in play gives its shuffled spot.
        let k = (0..n).filter(|&j| d.home[j] < d.home[i]).count();
        let (x, y) = tray_spot(k, n);
        Card { loc: Loc::Table { x, y }, rot: d.home_rot[i], held: 0, stamp: Stamp::default() }
    }

    /// Effective cards of `writer`'s clover while it is being guessed (locked
    /// spots enforced, answer shown once done) and which card claims each
    /// spot for a check: the nearest within `CLAIM_R`.
    pub fn layout(&self, writer: u32) -> (Vec<Card>, [Option<usize>; NSLOTS]) {
        let d = self.deal(writer);
        let n = self.ncards(writer);
        let check = self.check(writer);
        let locked = |s: usize| check.done() || check.mask & (1 << s) != 0;
        let cards: Vec<Card> = (0..n)
            .map(|i| {
                if i < NSLOTS && locked(i) {
                    Card { loc: Loc::Slot(i as u8), rot: d.sol_rot[i], held: 0, stamp: Stamp::default() }
                } else if check.done() {
                    // Once done, decoys go back to the tray to show the answer.
                    self.home_card(&d, n, i)
                } else {
                    self.cards.get(&(writer, i as u8)).copied().unwrap_or_else(|| self.home_card(&d, n, i))
                }
            })
            .collect();
        let mut claim = [None::<usize>; NSLOTS];
        for (s, cl) in claim.iter_mut().enumerate() {
            if locked(s) {
                *cl = Some(s);
                continue;
            }
            let (sx, sy) = slot_pos(s);
            *cl = (0..n)
                .filter(|&i| !(i < NSLOTS && locked(i)))
                .filter_map(|i| {
                    let (x, y) = board_pos(&cards[i])?;
                    let dist = (x - sx).hypot(y - sy);
                    (dist <= CLAIM_R).then_some((dist.to_bits(), core::cmp::Reverse(cards[i].stamp), i))
                })
                .min()
                .map(|t| t.2);
        }
        (cards, claim)
    }

    pub fn round_score(&self, n: u16) -> u32 {
        self.ledger.range((n, 0)..=(n, u32::MAX)).map(|(_, (p, _))| *p as u32).sum()
    }

    pub fn game_score(&self) -> u32 {
        self.ledger.values().map(|(p, _)| *p as u32).sum()
    }

    pub fn seat_score(&self, seat: u32) -> u32 {
        self.ledger.iter().filter(|((_, w), _)| *w == seat).map(|(_, (p, _))| *p as u32).sum()
    }

    /// Canonical rendering of the shared state; equal digests == converged.
    pub fn digest(&self) -> String {
        let phase = match self.phase() {
            Phase::Lobby => "lobby".to_string(),
            Phase::Writing => "writing".to_string(),
            Phase::Guessing(k) => format!("guessing:{k}"),
            Phase::End => "end".to_string(),
        };
        let seats: Vec<String> =
            self.seats.iter().map(|(id, s)| format!("[{id},{},{}]", json_str(&s.name), s.active)).collect();
        let subs: Vec<String> = self
            .round
            .players()
            .iter()
            .filter_map(|w| self.effective(*w).map(|s| (w, s)))
            .map(|(w, s)| {
                let clues: Vec<String> = s.clues.iter().map(|c| json_str(c)).collect();
                format!("[{w},{},{},{}]", s.decoys, s.ready, clues.join(","))
            })
            .collect();
        let f = |v: f32| format!("{v:.2}");
        let puzzles: Vec<String> = self
            .puzzles()
            .iter()
            .map(|&w| {
                let (cards, claim) = self.layout(w);
                let c = self.check(w);
                let cards: Vec<String> = cards
                    .iter()
                    .map(|c| match c.loc {
                        Loc::Table { x, y } => format!("[\"table\",{},{},{},{}]", f(x), f(y), c.rot, c.held),
                        Loc::Slot(s) => format!("[\"slot\",{s},{},{}]", c.rot, c.held),
                        Loc::Float { x, y } => format!("[\"float\",{},{},{},{}]", f(x), f(y), c.rot, c.held),
                    })
                    .collect();
                let claim: Vec<String> = claim.iter().map(|o| o.map_or("null".into(), |i| i.to_string())).collect();
                format!(
                    "{{\"writer\":{w},\"attempts\":{},\"mask\":{},\"claim\":[{}],\"cards\":[{}]}}",
                    c.attempts,
                    c.mask,
                    claim.join(","),
                    cards.join(",")
                )
            })
            .collect();
        format!(
            "{{\"phase\":\"{phase}\",\"round\":{},\"seed\":{},\"dealt\":{:?},\"seats\":[{}],\"subs\":[{}],\"puzzles\":[{}],\"roundScore\":{},\"gameScore\":{}}}",
            self.round.n,
            self.round.seed,
            self.round.players(),
            seats.join(","),
            subs.join(","),
            puzzles.join(","),
            self.round_score(self.round.n),
            self.game_score()
        )
    }
}

pub fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch)
            }
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    /// SplitMix64.
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

// ---------------------------------------------------------------------------
// Wire format: a packet is a concatenation of records, each `[tag u8] body`.
// All integers little-endian. Op records start with the game stamp and the
// round (stamp, seed, n): 30 bytes.

const T_NONE: u8 = 1;
const T_SUB: u8 = 2;
const T_START: u8 = 3;
const T_CARD: u8 = 4;
const T_CHECK: u8 = 5;
const T_NEXT: u8 = 6;
const T_SEAT: u8 = 7;
const T_LEDGER: u8 = 8;
const T_HELLO: u8 = 20;
const T_CURSOR: u8 = 21;

#[derive(Clone, PartialEq, Debug)]
pub enum Msg {
    Op(Op),
    /// Presence heartbeat: a device and the seat it plays (0 = watching).
    Hello { device: u32, seat: u32 },
    /// kind: 0 hidden, 1 tray coords, 2 board coords.
    Cursor { device: u32, seat: u32, kind: u8, x: f32, y: f32 },
}

fn put_stamp(b: &mut Vec<u8>, s: Stamp) {
    b.extend_from_slice(&s.clock.to_le_bytes());
    b.extend_from_slice(&s.peer.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let s = clamp_text(s, MAX_TEXT);
    b.push(s.len() as u8);
    b.extend_from_slice(s.as_bytes());
}

fn put_f32s(b: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
}

pub fn encode(m: &Msg, b: &mut Vec<u8>) {
    match m {
        Msg::Op(Op { game, round, body }) => {
            b.push(match body {
                Body::None => T_NONE,
                Body::Sub(..) => T_SUB,
                Body::Start(..) => T_START,
                Body::Card(..) => T_CARD,
                Body::Check(..) => T_CHECK,
                Body::Next(..) => T_NEXT,
                Body::Seat(..) => T_SEAT,
                Body::Ledger(..) => T_LEDGER,
            });
            put_stamp(b, *game);
            put_stamp(b, round.stamp);
            b.extend_from_slice(&round.seed.to_le_bytes());
            b.extend_from_slice(&round.n.to_le_bytes());
            b.push(round.nseats);
            for p in round.players() {
                b.extend_from_slice(&p.to_le_bytes());
            }
            b.extend_from_slice(&round.deck.to_le_bytes());
            b.push(round.custom);
            match body {
                Body::None => {}
                Body::Seat(id, s) => {
                    b.extend_from_slice(&id.to_le_bytes());
                    put_stamp(b, s.stamp);
                    b.push(s.active as u8);
                    put_str(b, &s.name);
                }
                Body::Sub(w, s) => {
                    b.extend_from_slice(&w.to_le_bytes());
                    put_stamp(b, s.stamp);
                    b.push(s.decoys);
                    b.push(s.ready as u8);
                    for c in &s.clues {
                        put_str(b, c);
                    }
                }
                Body::Start(s) => put_stamp(b, *s),
                Body::Card(w, i, c) => {
                    b.extend_from_slice(&w.to_le_bytes());
                    b.push(*i);
                    put_stamp(b, c.stamp);
                    let (k, x, y) = match c.loc {
                        Loc::Table { x, y } => (0u8, x, y),
                        Loc::Slot(s) => (1, s as f32, 0.0),
                        Loc::Float { x, y } => (2, x, y),
                    };
                    b.push(k);
                    put_f32s(b, &[x, y]);
                    b.push(c.rot);
                    b.extend_from_slice(&c.held.to_le_bytes());
                }
                Body::Check(w, c) => {
                    b.extend_from_slice(&w.to_le_bytes());
                    put_stamp(b, c.stamp);
                    b.push(c.attempts);
                    b.push(c.mask);
                }
                Body::Next(k, s) => {
                    b.extend_from_slice(&k.to_le_bytes());
                    put_stamp(b, *s);
                }
                Body::Ledger(n, w, p, s) => {
                    b.extend_from_slice(&n.to_le_bytes());
                    b.extend_from_slice(&w.to_le_bytes());
                    b.push(*p);
                    put_stamp(b, *s);
                }
            }
        }
        Msg::Hello { device, seat } => {
            b.push(T_HELLO);
            b.extend_from_slice(&device.to_le_bytes());
            b.extend_from_slice(&seat.to_le_bytes());
        }
        Msg::Cursor { device, seat, kind, x, y } => {
            b.push(T_CURSOR);
            b.extend_from_slice(&device.to_le_bytes());
            b.extend_from_slice(&seat.to_le_bytes());
            b.push(*kind);
            put_f32s(b, &[*x, *y]);
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().ok()?);
        (v.is_finite() && v.abs() < 1e5).then_some(v)
    }
    fn stamp(&mut self) -> Option<Stamp> {
        Some(Stamp { clock: self.u64()?, peer: self.u32()? })
    }
    fn lt(&mut self, max: u8) -> Option<u8> {
        self.u8().filter(|v| *v < max)
    }
    fn bool(&mut self) -> Option<bool> {
        Some(self.lt(2)? == 1)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if n > MAX_TEXT {
            return None;
        }
        Some(core::str::from_utf8(self.take(n)?).ok()?.to_string())
    }

    fn msg(&mut self) -> Option<Msg> {
        let tag = self.u8()?;
        if tag == T_HELLO {
            return Some(Msg::Hello { device: self.u32()?, seat: self.u32()? });
        }
        if tag == T_CURSOR {
            let (device, seat) = (self.u32()?, self.u32()?);
            let kind = self.lt(3)?;
            return Some(Msg::Cursor { device, seat, kind, x: self.f32()?, y: self.f32()? });
        }
        let game = self.stamp()?;
        let (stamp, seed, n) = (self.stamp()?, self.u32()?, self.u16()?);
        let nseats = self.u8().filter(|n| (*n as usize) <= MAX_PLAYERS)?;
        let mut seats = [0u32; MAX_PLAYERS];
        for p in seats.iter_mut().take(nseats as usize) {
            *p = self.u32()?;
        }
        let round = Round { stamp, seed, n, seats, nseats, deck: self.u32()?, custom: self.lt(2)? };
        let body = match tag {
            T_NONE => Body::None,
            T_SEAT => {
                let id = self.u32()?;
                let stamp = self.stamp()?;
                let active = self.bool()?;
                Body::Seat(id, Seat { name: self.str()?, active, stamp })
            }
            T_SUB => {
                let w = self.u32()?;
                let stamp = self.stamp()?;
                let decoys = self.u8().filter(|d| *d <= MAX_DECOYS)?;
                let ready = self.bool()?;
                let mut clues: [String; NSLOTS] = Default::default();
                for c in clues.iter_mut() {
                    *c = self.str()?;
                }
                Body::Sub(w, Sub { clues, decoys, ready, stamp })
            }
            T_START => Body::Start(self.stamp()?),
            T_CARD => {
                let w = self.u32()?;
                let i = self.lt(MAX_CARDS as u8)?;
                let stamp = self.stamp()?;
                let k = self.u8()?;
                let (x, y) = (self.f32()?, self.f32()?);
                let loc = match k {
                    0 => Loc::Table { x: x.clamp(0.0, 1.0), y: y.clamp(0.0, 1.0) },
                    1 if (0.0..NSLOTS as f32).contains(&x) && x.fract() == 0.0 => Loc::Slot(x as u8),
                    2 => Loc::Float { x, y },
                    _ => return None,
                };
                let rot = self.lt(4)?;
                let held = self.u32()?;
                Body::Card(w, i, Card { loc, rot, held, stamp })
            }
            T_CHECK => {
                let w = self.u32()?;
                let stamp = self.stamp()?;
                let attempts = self.lt(3)?;
                let mask = self.lt(16)?;
                Body::Check(w, Check { attempts, mask, stamp })
            }
            T_NEXT => {
                let k = self.u16()?;
                Body::Next(k, self.stamp()?)
            }
            T_LEDGER => {
                let (n, w) = (self.u16()?, self.u32()?);
                let p = self.u8().filter(|p| *p <= PERFECT_SCORE)?;
                Body::Ledger(n, w, p, self.stamp()?)
            }
            _ => return None,
        };
        Some(Msg::Op(Op { game, round, body }))
    }
}

/// Decode a packet. Stops at the first malformed record (never panics).
pub fn decode(b: &[u8]) -> Vec<Msg> {
    let mut r = Reader { b, p: 0 };
    let mut out = Vec::new();
    while r.p < b.len() {
        match r.msg() {
            Some(m) => out.push(m),
            None => break,
        }
    }
    out
}

/// Trim and byte-truncate text on a char boundary.
pub fn clamp_text(s: &str, max: usize) -> String {
    let s = s.trim();
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
