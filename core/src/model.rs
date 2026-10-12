//! Shared, replicated game state. Every field is a last-writer-wins register
//! ordered by a Lamport `Stamp`; a newer `Deal` hides all older registers.
//! `apply` is commutative, associative and idempotent, so replicas converge
//! regardless of delivery order or duplication.

use crate::words::WORDS;

pub const NCARDS: usize = 5;
pub const NSLOTS: usize = 4;
pub const WORLD_W: f32 = 1200.0;
pub const WORLD_H: f32 = 900.0;
pub const CX: f32 = WORLD_W / 2.0;
pub const CY: f32 = WORLD_H / 2.0;
pub const CARD: f32 = 150.0;
pub const HALF: f32 = CARD / 2.0;
pub const SLOT_OFF: f32 = 79.0;
/// Half-size of the square (in board coords) that counts as "over the board".
pub const BOARD_REACH: f32 = 205.0;
pub const MAX_CLUE_BYTES: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Stamp {
    pub clock: u64,
    pub peer: u32,
}

/// Where a card is. `Table` is in world coords and never rotates with anyone's
/// view; `Slot` and `Float` are in board coords and turn with the board.
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

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Clue {
    pub text: String,
    pub stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Solution {
    /// (card, rot) for each slot.
    pub slots: [(u8, u8); NSLOTS],
    pub stamp: Stamp,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Deal {
    pub seed: u32,
    pub stamp: Stamp,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Op {
    Deal(Deal),
    Card(u8, Card),
    Clue(u8, Clue),
    Solution(Solution),
}

impl Op {
    pub fn stamp(&self) -> Stamp {
        match self {
            Op::Deal(d) => d.stamp,
            Op::Card(_, c) => c.stamp,
            Op::Clue(_, c) => c.stamp,
            Op::Solution(s) => s.stamp,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Shared {
    pub deal: Deal,
    pub cards: [Card; NCARDS],
    pub clues: [Clue; NSLOTS],
    pub solution: Solution,
}

pub fn table_home(i: usize) -> (f32, f32) {
    (170.0, 110.0 + 170.0 * i as f32)
}

/// Where a card that lost a slot conflict is shown.
pub fn bump_home(i: usize) -> (f32, f32) {
    (WORLD_W - 170.0, 110.0 + 170.0 * i as f32)
}

pub fn slot_pos(s: usize) -> (f32, f32) {
    const P: [(f32, f32); NSLOTS] = [
        (-SLOT_OFF, -SLOT_OFF),
        (SLOT_OFF, -SLOT_OFF),
        (SLOT_OFF, SLOT_OFF),
        (-SLOT_OFF, SLOT_OFF),
    ];
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

pub fn default_card(i: usize) -> Card {
    let (x, y) = table_home(i);
    Card { loc: Loc::Table { x, y }, rot: 0, held: 0, stamp: Stamp::default() }
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            deal: Deal { seed: 0x5eed, stamp: Stamp::default() },
            cards: core::array::from_fn(default_card),
            clues: Default::default(),
            solution: Solution::default(),
        }
    }
}

impl Shared {
    /// Merge one register write. Returns true if anything changed.
    pub fn apply(&mut self, op: &Op) -> bool {
        match op {
            Op::Deal(d) if d.stamp > self.deal.stamp => self.deal = *d,
            Op::Card(i, c) if c.stamp > self.cards[*i as usize].stamp => self.cards[*i as usize] = *c,
            Op::Clue(i, c) if c.stamp > self.clues[*i as usize].stamp => self.clues[*i as usize] = c.clone(),
            Op::Solution(s) if s.stamp > self.solution.stamp => self.solution = *s,
            _ => return false,
        }
        true
    }

    /// Every stored register, for full-state sync with a new peer.
    pub fn all_ops(&self) -> Vec<Op> {
        let mut v = vec![Op::Deal(self.deal), Op::Solution(self.solution)];
        for (i, c) in self.cards.iter().enumerate() {
            v.push(Op::Card(i as u8, *c));
        }
        for (i, c) in self.clues.iter().enumerate() {
            v.push(Op::Clue(i as u8, c.clone()));
        }
        v
    }

    fn live(&self, s: Stamp) -> bool {
        s > self.deal.stamp
    }

    pub fn card(&self, i: usize) -> Card {
        if self.live(self.cards[i].stamp) { self.cards[i] } else { default_card(i) }
    }

    pub fn clue(&self, i: usize) -> &str {
        if self.live(self.clues[i].stamp) { &self.clues[i].text } else { "" }
    }

    pub fn solution(&self) -> Option<[(u8, u8); NSLOTS]> {
        self.live(self.solution.stamp).then_some(self.solution.slots)
    }

    /// Effective cards with slot conflicts resolved (newest claim wins).
    pub fn layout(&self) -> ([Card; NCARDS], [Option<usize>; NSLOTS]) {
        let mut cards: [Card; NCARDS] = core::array::from_fn(|i| self.card(i));
        let mut owner = [None::<usize>; NSLOTS];
        for (i, c) in cards.iter().enumerate() {
            if let Loc::Slot(s) = c.loc {
                let s = s as usize;
                match owner[s] {
                    Some(j) if cards[j].stamp > c.stamp => {}
                    _ => owner[s] = Some(i),
                }
            }
        }
        for (i, c) in cards.iter_mut().enumerate() {
            if let Loc::Slot(s) = c.loc {
                if owner[s as usize] != Some(i) {
                    let (x, y) = bump_home(i);
                    c.loc = Loc::Table { x, y };
                }
            }
        }
        (cards, owner)
    }

    /// Four words per card (top, right, bottom, left in card-local terms).
    pub fn words(&self) -> [[&'static str; 4]; NCARDS] {
        let mut rng = Rng::new(self.deal.seed as u64 ^ 0x9e37_79b9_7f4a_7c15);
        let mut idx: Vec<usize> = (0..WORDS.len()).collect();
        for k in 0..NCARDS * 4 {
            let j = k + (rng.next() as usize) % (idx.len() - k);
            idx.swap(k, j);
        }
        core::array::from_fn(|c| core::array::from_fn(|s| WORDS[idx[c * 4 + s]]))
    }

    /// Canonical rendering of the effective state; equal digests == converged.
    pub fn digest(&self) -> String {
        let (cards, _) = self.layout();
        let mut out = format!("{{\"seed\":{},\"cards\":[", self.deal.seed);
        for (i, c) in cards.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let loc = match c.loc {
                Loc::Table { x, y } => format!("\"table\",{x:.1},{y:.1}"),
                Loc::Slot(s) => format!("\"slot\",{s}"),
                Loc::Float { x, y } => format!("\"float\",{x:.1},{y:.1}"),
            };
            out += &format!("[{loc},{},{}]", c.rot, c.held);
        }
        out += "],\"clues\":[";
        for i in 0..NSLOTS {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            for ch in self.clue(i).chars() {
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
        }
        out += &format!("],\"answer\":{}}}", self.solution().is_some());
        out
    }
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
}

// ---------------------------------------------------------------------------
// Wire format: a packet is a concatenation of records, each `[tag u8] body`.
// All integers little-endian.

const T_CARD: u8 = 1;
const T_CLUE: u8 = 2;
const T_DEAL: u8 = 3;
const T_CURSOR: u8 = 4;
const T_SOLUTION: u8 = 5;

#[derive(Clone, PartialEq, Debug)]
pub enum Msg {
    Op(Op),
    /// kind: 0 hidden, 1 world coords, 2 board coords.
    Cursor { peer: u32, kind: u8, x: f32, y: f32 },
}

fn put_stamp(b: &mut Vec<u8>, s: Stamp) {
    b.extend_from_slice(&s.clock.to_le_bytes());
    b.extend_from_slice(&s.peer.to_le_bytes());
}

pub fn encode(m: &Msg, b: &mut Vec<u8>) {
    match m {
        Msg::Op(Op::Card(i, c)) => {
            b.push(T_CARD);
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
        Msg::Op(Op::Clue(i, c)) => {
            b.push(T_CLUE);
            b.push(*i);
            put_stamp(b, c.stamp);
            let t = c.text.as_bytes();
            b.push(t.len() as u8);
            b.extend_from_slice(t);
        }
        Msg::Op(Op::Deal(d)) => {
            b.push(T_DEAL);
            put_stamp(b, d.stamp);
            b.extend_from_slice(&d.seed.to_le_bytes());
        }
        Msg::Op(Op::Solution(s)) => {
            b.push(T_SOLUTION);
            put_stamp(b, s.stamp);
            for (c, r) in s.slots {
                b.push(c);
                b.push(r);
            }
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

    fn msg(&mut self) -> Option<Msg> {
        Some(match self.u8()? {
            T_CARD => {
                let i = self.lt(NCARDS as u8)?;
                let stamp = self.stamp()?;
                let k = self.u8()?;
                let (x, y) = (self.f32()?, self.f32()?);
                let loc = match k {
                    0 => Loc::Table { x, y },
                    1 if x >= 0.0 && x < NSLOTS as f32 && x.fract() == 0.0 => Loc::Slot(x as u8),
                    2 => Loc::Float { x, y },
                    _ => return None,
                };
                let rot = self.lt(4)?;
                let held = self.u32()?;
                Msg::Op(Op::Card(i, Card { loc, rot, held, stamp }))
            }
            T_CLUE => {
                let i = self.lt(NSLOTS as u8)?;
                let stamp = self.stamp()?;
                let n = self.u8()? as usize;
                if n > MAX_CLUE_BYTES {
                    return None;
                }
                let text = core::str::from_utf8(self.take(n)?).ok()?.to_string();
                Msg::Op(Op::Clue(i, Clue { text, stamp }))
            }
            T_DEAL => {
                let stamp = self.stamp()?;
                Msg::Op(Op::Deal(Deal { stamp, seed: self.u32()? }))
            }
            T_SOLUTION => {
                let stamp = self.stamp()?;
                let mut slots = [(0, 0); NSLOTS];
                for s in slots.iter_mut() {
                    *s = (self.lt(NCARDS as u8)?, self.lt(4)?);
                }
                Msg::Op(Op::Solution(Solution { slots, stamp }))
            }
            T_CURSOR => {
                let peer = self.u32()?;
                let kind = self.lt(3)?;
                Msg::Cursor { peer, kind, x: self.f32()?, y: self.f32()? }
            }
            _ => return None,
        })
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
pub fn clamp_clue(s: &str) -> String {
    let mut end = s.len().min(MAX_CLUE_BYTES);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim().to_string()
}
