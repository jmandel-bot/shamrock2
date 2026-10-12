//! Shared, replicated game state.
//!
//! Every per-round op carries the `Round` it belongs to, so a replica that
//! hears about a newer round from any op resets and follows; ops from older
//! rounds are dropped. Within a round each value is a register merged by a
//! monotone rule (LWW by Lamport stamp, or max for counters). `apply` is
//! therefore commutative, associative and idempotent: replicas converge
//! regardless of delivery order or duplication.

use crate::words::WORDS;
use std::collections::BTreeMap;

pub const NSLOTS: usize = 4;
pub const MAX_DECOYS: u8 = 4;
pub const MAX_CARDS: usize = NSLOTS + MAX_DECOYS as usize;
pub const WORLD_W: f32 = 900.0;
pub const WORLD_H: f32 = 1000.0;
pub const CX: f32 = WORLD_W / 2.0;
pub const CY: f32 = 320.0;
pub const CARD: f32 = 150.0;
pub const HALF: f32 = CARD / 2.0;
pub const SLOT_OFF: f32 = 79.0;
/// Half-size of the square (in board coords) that counts as "over the board".
pub const BOARD_REACH: f32 = 205.0;
pub const HUB_R: f32 = 30.0;
pub const MAX_CLUE_BYTES: usize = 60;
pub const PERFECT_SCORE: u32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Stamp {
    pub clock: u64,
    pub peer: u32,
}

/// `Table` is in world coords and never turns with anyone's view; `Slot` and
/// `Float` are in board coords and turn with the board.
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
    /// Peer currently dragging this card (0 = nobody).
    pub held: u32,
    pub stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Round {
    pub seed: u32,
    pub stamp: Stamp,
}

/// A writer's submitted clues.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sub {
    pub clues: [String; NSLOTS],
    pub decoys: u8,
    pub stamp: Stamp,
}

/// Result of the guessers' checks on one clover.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Check {
    pub attempts: u8,
    /// Bit s set = slot s was right at the last check (and is now locked).
    pub mask: u8,
    pub stamp: Stamp,
}

impl Check {
    pub fn done(&self) -> bool {
        self.attempts >= 2 || self.mask == 0b1111
    }
    pub fn score(&self) -> u32 {
        match (self.attempts, self.mask) {
            (1, 0b1111) => PERFECT_SCORE,
            _ => self.mask.count_ones(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Body {
    /// Just announces the round.
    None,
    Sub(u32, Sub),
    /// Writing is over; guessing begins.
    Start(Stamp),
    Card(u32, u8, Card),
    Check(u32, Check),
    /// Index of the clover being guessed (grow-only).
    Next(u16, Stamp),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Op {
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
    pub round: Round,
    pub subs: BTreeMap<u32, Sub>,
    pub start: Option<Stamp>,
    pub cards: BTreeMap<(u32, u8), Card>,
    pub checks: BTreeMap<u32, Check>,
    pub next: (u16, Stamp),
}

/// Everything about one writer's clover that is derived from the seed.
pub struct Deal {
    pub words: [[&'static str; 4]; MAX_CARDS],
    /// Rotation of card s in slot s (the answer). Card s belongs in slot s.
    pub sol_rot: [u8; NSLOTS],
    /// Shuffled table spot and rotation for each card when guessing starts.
    pub home: [u8; MAX_CARDS],
    pub home_rot: [u8; MAX_CARDS],
}

pub fn deal(seed: u32, writer: u32) -> Deal {
    let mut rng = Rng::new(((seed as u64) << 32 | writer as u64) ^ 0x9e37_79b9_7f4a_7c15);
    let mut idx: Vec<usize> = (0..WORDS.len()).collect();
    for k in 0..MAX_CARDS * 4 {
        let j = k + rng.below(idx.len() - k);
        idx.swap(k, j);
    }
    let mut home: [u8; MAX_CARDS] = core::array::from_fn(|i| i as u8);
    for k in (1..MAX_CARDS).rev() {
        home.swap(k, rng.below(k + 1));
    }
    Deal {
        words: core::array::from_fn(|c| core::array::from_fn(|s| WORDS[idx[c * 4 + s]])),
        sol_rot: core::array::from_fn(|_| rng.below(4) as u8),
        home,
        home_rot: core::array::from_fn(|_| rng.below(4) as u8),
    }
}

/// Table spot `k` of `n`, in rows of up to five below the board.
pub fn table_spot(k: usize, n: usize) -> (f32, f32) {
    let (row, col) = (k / 5, k % 5);
    let in_row = (n - row * 5).min(5);
    (CX + (col as f32 - (in_row as f32 - 1.0) / 2.0) * 172.0, 715.0 + row as f32 * 175.0)
}

pub fn slot_pos(s: usize) -> (f32, f32) {
    const P: [(f32, f32); NSLOTS] =
        [(-SLOT_OFF, -SLOT_OFF), (SLOT_OFF, -SLOT_OFF), (SLOT_OFF, SLOT_OFF), (-SLOT_OFF, SLOT_OFF)];
    P[s]
}

/// Slot whose quadrant contains board point (x, y).
pub fn nearest_slot(x: f32, y: f32) -> u8 {
    match (x >= 0.0, y >= 0.0) {
        (false, false) => 0,
        (true, false) => 1,
        (true, true) => 2,
        (false, true) => 3,
    }
}

impl Shared {
    /// Merge one op. Returns true if anything changed.
    pub fn apply(&mut self, op: &Op) -> bool {
        let mut changed = false;
        if op.round.stamp > self.round.stamp {
            *self = Shared { round: op.round, ..Shared::default() };
            changed = true;
        }
        if op.round.stamp != self.round.stamp {
            return changed;
        }
        let newer = match &op.body {
            Body::None => false,
            Body::Sub(w, s) => {
                let n = self.subs.get(w).is_none_or(|c| s.stamp > c.stamp);
                if n {
                    self.subs.insert(*w, s.clone());
                }
                n
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
        };
        changed || newer
    }

    /// Every stored register, for full-state sync with a new peer.
    pub fn all_ops(&self) -> Vec<Op> {
        let r = self.round;
        let mut v = vec![Op { round: r, body: Body::None }];
        v.extend(self.subs.iter().map(|(w, s)| Op { round: r, body: Body::Sub(*w, s.clone()) }));
        v.extend(self.start.map(|s| Op { round: r, body: Body::Start(s) }));
        v.extend(self.cards.iter().map(|((w, i), c)| Op { round: r, body: Body::Card(*w, *i, *c) }));
        v.extend(self.checks.iter().map(|(w, c)| Op { round: r, body: Body::Check(*w, *c) }));
        if self.next.0 > 0 {
            v.push(Op { round: r, body: Body::Next(self.next.0, self.next.1) });
        }
        v
    }

    /// Writers whose clovers get guessed, in order. Only clues submitted
    /// before guessing started count.
    pub fn puzzles(&self) -> Vec<u32> {
        let Some(start) = self.start else { return Vec::new() };
        let mut v: Vec<(Stamp, u32)> =
            self.subs.iter().filter(|(_, s)| s.stamp < start).map(|(w, s)| (s.stamp, *w)).collect();
        v.sort();
        v.into_iter().map(|(_, w)| w).collect()
    }

    pub fn phase(&self) -> Phase {
        if self.round.stamp == Stamp::default() {
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
        NSLOTS + self.subs.get(&writer).map_or(1, |s| s.decoys.min(MAX_DECOYS)) as usize
    }

    pub fn home_card(&self, d: &Deal, n: usize, i: usize) -> Card {
        // Rank of card i among the n cards in play gives its shuffled spot.
        let k = (0..n).filter(|&j| d.home[j] < d.home[i]).count();
        let (x, y) = table_spot(k, n);
        Card { loc: Loc::Table { x, y }, rot: d.home_rot[i], held: 0, stamp: Stamp::default() }
    }

    /// Effective cards of `writer`'s clover while it is being guessed, with
    /// locked (already-correct) slots enforced, conflicts resolved (newest
    /// claim wins, losers go home) and the answer shown once done.
    pub fn layout(&self, writer: u32) -> (Vec<Card>, [Option<usize>; NSLOTS]) {
        let d = deal(self.round.seed, writer);
        let n = self.ncards(writer);
        let check = self.check(writer);
        let solved = |s: usize| Card { loc: Loc::Slot(s as u8), rot: d.sol_rot[s], held: 0, stamp: Stamp::default() };
        let locked = |s: usize| check.done() || check.mask & (1 << s) != 0;
        let mut cards: Vec<Card> = (0..n)
            .map(|i| {
                if i < NSLOTS && locked(i) {
                    solved(i)
                } else {
                    self.cards.get(&(writer, i as u8)).copied().unwrap_or_else(|| self.home_card(&d, n, i))
                }
            })
            .collect();
        let mut owner = [None::<usize>; NSLOTS];
        for s in 0..NSLOTS {
            if locked(s) {
                owner[s] = Some(s);
            }
        }
        for (i, c) in cards.iter().enumerate() {
            if let Loc::Slot(s) = c.loc {
                let s = s as usize;
                if locked(s) {
                    continue;
                }
                match owner[s] {
                    Some(j) if cards[j].stamp > c.stamp => {}
                    _ => owner[s] = Some(i),
                }
            }
        }
        for (i, c) in cards.iter_mut().enumerate() {
            if let Loc::Slot(s) = c.loc {
                if owner[s as usize] != Some(i) {
                    *c = self.home_card(&d, n, i);
                }
            }
        }
        (cards, owner)
    }

    pub fn total_score(&self) -> u32 {
        self.puzzles().iter().map(|w| self.check(*w)).filter(|c| c.done()).map(|c| c.score()).sum()
    }

    /// Canonical rendering of the shared state; equal digests == converged.
    pub fn digest(&self) -> String {
        let phase = match self.phase() {
            Phase::Lobby => "lobby".to_string(),
            Phase::Writing => "writing".to_string(),
            Phase::Guessing(k) => format!("guessing:{k}"),
            Phase::End => "end".to_string(),
        };
        let mut out = format!("{{\"phase\":\"{phase}\",\"seed\":{},\"subs\":[", self.round.seed);
        let subs: Vec<String> = self
            .subs
            .iter()
            .map(|(w, s)| format!("[{w},{},{}]", s.decoys, s.clues.iter().map(|c| json_str(c)).collect::<Vec<_>>().join(",")))
            .collect();
        out += &subs.join(",");
        out += "],\"puzzles\":[";
        let puzzles: Vec<String> = self
            .puzzles()
            .iter()
            .map(|&w| {
                let (cards, _) = self.layout(w);
                let c = self.check(w);
                let cards: Vec<String> = cards
                    .iter()
                    .map(|c| match c.loc {
                        Loc::Table { x, y } => format!("[\"table\",{x:.1},{y:.1},{},{}]", c.rot, c.held),
                        Loc::Slot(s) => format!("[\"slot\",{s},{},{}]", c.rot, c.held),
                        Loc::Float { x, y } => format!("[\"float\",{x:.1},{y:.1},{},{}]", c.rot, c.held),
                    })
                    .collect();
                format!("{{\"writer\":{w},\"attempts\":{},\"mask\":{},\"cards\":[{}]}}", c.attempts, c.mask, cards.join(","))
            })
            .collect();
        out += &puzzles.join(",");
        out += &format!("],\"score\":{}}}", self.total_score());
        out
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
// All integers little-endian. Per-round records start with the round
// (stamp + seed, 16 bytes).

const T_ROUND: u8 = 1;
const T_SUB: u8 = 2;
const T_START: u8 = 3;
const T_CARD: u8 = 4;
const T_CHECK: u8 = 5;
const T_NEXT: u8 = 6;
const T_HELLO: u8 = 7;
const T_CURSOR: u8 = 8;

#[derive(Clone, PartialEq, Debug)]
pub enum Msg {
    Op(Op),
    /// Presence heartbeat with display name.
    Hello { peer: u32, name: String },
    /// kind: 0 hidden, 1 world coords, 2 board coords.
    Cursor { peer: u32, kind: u8, x: f32, y: f32 },
}

fn put_stamp(b: &mut Vec<u8>, s: Stamp) {
    b.extend_from_slice(&s.clock.to_le_bytes());
    b.extend_from_slice(&s.peer.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let s = &s.as_bytes()[..s.len().min(MAX_CLUE_BYTES)];
    b.push(s.len() as u8);
    b.extend_from_slice(s);
}

pub fn encode(m: &Msg, b: &mut Vec<u8>) {
    match m {
        Msg::Op(Op { round, body }) => {
            let tag = match body {
                Body::None => T_ROUND,
                Body::Sub(..) => T_SUB,
                Body::Start(..) => T_START,
                Body::Card(..) => T_CARD,
                Body::Check(..) => T_CHECK,
                Body::Next(..) => T_NEXT,
            };
            b.push(tag);
            put_stamp(b, round.stamp);
            b.extend_from_slice(&round.seed.to_le_bytes());
            match body {
                Body::None => {}
                Body::Sub(w, s) => {
                    b.extend_from_slice(&w.to_le_bytes());
                    put_stamp(b, s.stamp);
                    b.push(s.decoys);
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
                    b.extend_from_slice(&x.to_le_bytes());
                    b.extend_from_slice(&y.to_le_bytes());
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
            }
        }
        Msg::Hello { peer, name } => {
            b.push(T_HELLO);
            b.extend_from_slice(&peer.to_le_bytes());
            put_str(b, name);
        }
        Msg::Cursor { peer, kind, x, y } => {
            b.push(T_CURSOR);
            b.extend_from_slice(&peer.to_le_bytes());
            b.push(*kind);
            b.extend_from_slice(&x.to_le_bytes());
            b.extend_from_slice(&y.to_le_bytes());
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
    fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if n > MAX_CLUE_BYTES {
            return None;
        }
        Some(core::str::from_utf8(self.take(n)?).ok()?.to_string())
    }

    fn msg(&mut self) -> Option<Msg> {
        let tag = self.u8()?;
        if tag == T_HELLO {
            return Some(Msg::Hello { peer: self.u32()?, name: self.str()? });
        }
        if tag == T_CURSOR {
            let peer = self.u32()?;
            let kind = self.lt(3)?;
            return Some(Msg::Cursor { peer, kind, x: self.f32()?, y: self.f32()? });
        }
        let round = Round { stamp: self.stamp()?, seed: self.u32()? };
        let body = match tag {
            T_ROUND => Body::None,
            T_SUB => {
                let w = self.u32()?;
                let stamp = self.stamp()?;
                let decoys = self.u8().filter(|d| *d <= MAX_DECOYS)?;
                let mut clues: [String; NSLOTS] = Default::default();
                for c in clues.iter_mut() {
                    *c = self.str()?;
                }
                Body::Sub(w, Sub { clues, decoys, stamp })
            }
            T_START => Body::Start(self.stamp()?),
            T_CARD => {
                let w = self.u32()?;
                let i = self.lt(MAX_CARDS as u8)?;
                let stamp = self.stamp()?;
                let k = self.u8()?;
                let (x, y) = (self.f32()?, self.f32()?);
                let loc = match k {
                    0 => Loc::Table { x, y },
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
            _ => return None,
        };
        Some(Msg::Op(Op { round, body }))
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

/// Byte-truncate a clue to the wire limit on a char boundary.
pub fn clamp_text(s: &str, max: usize) -> String {
    let s = s.trim();
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
