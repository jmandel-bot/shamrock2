//! The game on an unreliable network: devices on a mesh of links with
//! latency and jitter, links that drop and reconnect, partitions that heal,
//! devices that go offline, reload (and rejoin under their saved name 1.5 s
//! later, as the page does) or close for good, all while players act at
//! random. After the chaos the network heals and goes quiet, and every run
//! must satisfy the properties at the bottom: one state everywhere, nobody
//! connected loses their seat, nothing is wiped without a New game, no card
//! stays "held" by a hand that has gone, and the round can still be played
//! to its end.
//!
//! The model follows the real transport (Trystero over WebRTC): each pair of
//! devices has at most one connection; a packet is broadcast on every open
//! connection, delivered in order on each, and lost if that connection closes
//! first; when a connection opens both ends call `on_peer_join` (a full state
//! sync), and when it closes both call `on_link_lost`.
//!
//! `SEED=123 cargo test --release --test network -- --nocapture` replays one
//! run with its event log.

use cloverleaf::app::{App, EV_NET};
use cloverleaf::model::*;
use std::collections::{BTreeMap, BTreeSet};

const TICK: f64 = 20.0;
const NAMES: [&str; 5] = ["Ann", "Bo", "Cy", "Dee", "Eve"];

fn app(seed: u64) -> App {
    let mut a = App::new(seed);
    (a.w, a.h, a.dpr) = (1000.0, 800.0, 1.0);
    a
}

/// Network packets from an outbox (other events are dropped).
fn packets(a: &mut App) -> Vec<Vec<u8>> {
    let b = std::mem::take(&mut a.outbox);
    let (mut out, mut p) = (Vec::new(), 0);
    while p < b.len() {
        let len = u32::from_le_bytes(b[p + 1..p + 5].try_into().unwrap()) as usize;
        if b[p] == EV_NET {
            out.push(b[p + 5..p + 5 + len].to_vec());
        }
        p += 5 + len;
    }
    out
}

struct Dev {
    app: Option<App>,
    /// The name this device's page has saved (it rejoins under it on reload).
    name: Option<String>,
    join_at: Option<f64>,
    online: bool,
    /// Mid-drag: the finger is down and moving.
    dragging: bool,
    /// What the page last saved to localStorage (every 2 s), restored on reload.
    saved: Vec<u8>,
}

struct Packet {
    at: f64,
    to: usize,
    conn: u32,
    bytes: Vec<u8>,
}

/// How rough the network is.
#[derive(Clone, Copy)]
struct Rough {
    /// One-way latency range, ms.
    latency: (f64, f64),
    /// Chance per tick that some fault happens.
    faults: u32, // out of 1000
    /// Chance per tick that some player acts.
    acts: u32, // out of 1000
}

struct Sim {
    devs: Vec<Dev>,
    /// Open connections: (i, j) with i < j → connection id.
    links: BTreeMap<(usize, usize), u32>,
    /// Pairs that can't reach each other (a partition).
    blocked: BTreeSet<(usize, usize)>,
    inflight: Vec<Packet>,
    /// Last scheduled delivery per direction, to keep each connection in order.
    last_at: BTreeMap<(usize, usize), f64>,
    next_conn: u32,
    now: f64,
    r: Rng,
    rough: Rough,
    seed: u64,
    log: Vec<String>,
    new_games: u32,
    loads: u64,
}

fn pair(i: usize, j: usize) -> (usize, usize) {
    (i.min(j), i.max(j))
}

impl Sim {
    fn new(seed: u64, n: usize, rough: Rough) -> Sim {
        let devs = (0..n)
            .map(|k| Dev { app: Some(app(seed * 100 + k as u64 + 1)), name: None, join_at: None, online: true, dragging: false, saved: Vec::new() })
            .collect();
        Sim {
            devs,
            links: BTreeMap::new(),
            blocked: BTreeSet::new(),
            inflight: Vec::new(),
            last_at: BTreeMap::new(),
            next_conn: 1,
            now: 1000.0,
            r: Rng::new(seed ^ 0x5eed),
            rough,
            seed,
            log: Vec::new(),
            new_games: 0,
            loads: 0,
        }
    }

    fn note(&mut self, s: String) {
        if std::env::var("SEED").is_ok() {
            println!("{:>7.0} {s}", self.now);
        }
        self.log.push(format!("{:.0} {s}", self.now));
    }

    fn live(&self, k: usize) -> bool {
        self.devs[k].app.is_some() && self.devs[k].online
    }

    /// Send everything device `k` has queued on each of its open connections.
    fn flush(&mut self, k: usize) {
        let Some(a) = self.devs[k].app.as_mut() else { return };
        let out = packets(a);
        if out.is_empty() {
            return;
        }
        let peers: Vec<(usize, u32)> = self
            .links
            .iter()
            .filter_map(|(&(i, j), &c)| if i == k { Some((j, c)) } else if j == k { Some((i, c)) } else { None })
            .collect();
        for bytes in out {
            for &(to, conn) in &peers {
                let (lo, hi) = self.rough.latency;
                let lat = lo + (hi - lo) * (self.r.below(1000) as f64 / 1000.0);
                let at = (self.now + lat).max(*self.last_at.get(&(k, to)).unwrap_or(&0.0));
                self.last_at.insert((k, to), at);
                self.inflight.push(Packet { at, to, conn, bytes: bytes.clone() });
            }
        }
    }

    fn connect(&mut self, i: usize, j: usize) {
        let p = pair(i, j);
        if i == j || self.links.contains_key(&p) || !self.live(i) || !self.live(j) || self.blocked.contains(&p) {
            return;
        }
        let conn = self.next_conn;
        self.next_conn += 1;
        self.links.insert(p, conn);
        self.note(format!("link {i}-{j} up (#{conn})"));
        for k in [i, j] {
            self.devs[k].app.as_mut().unwrap().on_peer_join();
            self.flush(k);
        }
    }

    fn disconnect(&mut self, i: usize, j: usize) {
        if let Some(conn) = self.links.remove(&pair(i, j)) {
            self.note(format!("link {i}-{j} down (#{conn})"));
            for k in [i, j] {
                if let Some(a) = self.devs[k].app.as_mut() {
                    a.on_link_lost(conn);
                }
            }
        }
    }

    fn drop_all_links(&mut self, k: usize) {
        for j in 0..self.devs.len() {
            self.disconnect(k, j);
        }
    }

    fn tick(&mut self) {
        self.now += TICK;
        for k in 0..self.devs.len() {
            if self.devs[k].join_at.is_some_and(|t| t <= self.now) && self.live(k) {
                self.devs[k].join_at = None;
                let name = self.devs[k].name.clone().unwrap();
                self.note(format!("dev {k} rejoins as {name}"));
                self.devs[k].app.as_mut().unwrap().join(&name);
            }
            if self.live(k) {
                let now = self.now;
                self.devs[k].app.as_mut().unwrap().frame(now);
                if (now / TICK) as u64 % 100 == k as u64 {
                    self.devs[k].saved = self.devs[k].app.as_ref().unwrap().snapshot();
                }
            }
        }
        // Deliver what has arrived, in time order, on connections still open.
        self.inflight.sort_by(|a, b| a.at.total_cmp(&b.at));
        let due = self.inflight.iter().take_while(|p| p.at <= self.now).count();
        let open: BTreeSet<u32> = self.links.values().copied().collect();
        for p in self.inflight.drain(..due).collect::<Vec<_>>() {
            if open.contains(&p.conn) {
                if let Some(a) = self.devs[p.to].app.as_mut() {
                    a.on_packet(&p.bytes, p.conn);
                }
            }
        }
        for k in 0..self.devs.len() {
            self.flush(k);
        }
        if std::env::var("SEED").is_ok() {
            let games: Vec<String> = self
                .devs
                .iter()
                .map(|d| d.app.as_ref().map_or("-".into(), |a| format!("{}.{}/r{}", a.shared.game.clock, a.shared.game.peer % 100, a.shared.round.n)))
                .collect();
            let g = games.join(" ");
            if self.log.last().is_none_or(|l| !l.ends_with(&g)) {
                self.note(format!("games {g}"));
            }
        }
    }

    /// Connections form between reachable online devices, after a while.
    fn reconnect_some(&mut self, chance: u32) {
        let n = self.devs.len();
        for i in 0..n {
            for j in i + 1..n {
                if self.r.below(1000) < chance as usize {
                    self.connect(i, j);
                }
            }
        }
    }

    fn act(&mut self, k: usize) {
        let r = self.r.below(100);
        let dev = &mut self.devs[k];
        let a = dev.app.as_mut().unwrap();
        if dev.dragging {
            // Finish (or abandon) the drag first.
            if r < 70 {
                let (x, y) = (self.r.below(1000) as f32, self.r.below(800) as f32);
                a.pointer_up(x, y);
            } else {
                a.pointer_cancel();
            }
            dev.dragging = false;
            return;
        }
        let what = match r {
            0..=7 => {
                let name = NAMES[self.r.below(NAMES.len())].to_string();
                a.join(&name);
                dev.name = Some(name.clone());
                format!("join as {name}")
            }
            8..=13 => {
                a.action(1);
                "start / next round".into()
            }
            14..=27 => {
                for d in 0..4u8 {
                    a.set_clue(d, ["sea", "sky", "fire", "dog", "ice"][self.r.below(5)], false);
                }
                a.action(2);
                "write + toggle ready".into()
            }
            28..=31 => {
                a.set_clue(self.r.below(4) as u8, ["", "moon", "tea"][self.r.below(3)], false);
                "edit a clue".into()
            }
            32..=35 => {
                a.action(5);
                "proceed".into()
            }
            36..=44 => {
                a.action(3);
                "guess".into()
            }
            45..=50 => {
                a.action(4);
                "next clover".into()
            }
            51..=53 => {
                a.jump(self.r.below(4));
                "jump".into()
            }
            54..=55 => {
                a.set_decoys(self.r.below(5) as u8);
                "decoys".into()
            }
            56 => {
                if self.r.below(3) == 0 {
                    a.action(7);
                    self.new_games += 1;
                    "NEW GAME".into()
                } else {
                    a.action(6);
                    "deal new cards".into()
                }
            }
            57..=64 => {
                // Pick up a card and start dragging (finished on a later turn).
                let n = a.scene().cards.len();
                if n == 0 {
                    return;
                }
                a.settle();
                let (x, y, _) = a.card_screen(self.r.below(n));
                a.pointer_down(x, y);
                a.pointer_move(x + 30.0, y + 30.0);
                dev.dragging = true;
                "drag starts".into()
            }
            _ => {
                // A whole drag, often onto a spot, sometimes a tap (turn).
                let n = a.scene().cards.len();
                if n == 0 {
                    return;
                }
                a.settle();
                let i = self.r.below(n);
                let (x, y, _) = a.card_screen(i);
                let to = if self.r.below(4) == 0 { (x, y) } else { a.slot_screen(self.r.below(4)) };
                a.pointer_down(x, y);
                for s in 1..=6 {
                    let t = s as f32 / 6.0;
                    a.pointer_move(x + (to.0 - x) * t, y + (to.1 - y) * t);
                }
                a.pointer_up(to.0, to.1);
                "drag / tap a card".into()
            }
        };
        self.note(format!("dev {k}: {what}"));
    }

    fn fault(&mut self) {
        let n = self.devs.len();
        let k = self.r.below(n);
        match self.r.below(10) {
            0 | 1 => {
                let j = self.r.below(n);
                self.disconnect(k, j);
            }
            2 => {
                // Partition: a random side can't reach the rest.
                let side: Vec<bool> = (0..n).map(|_| self.r.below(2) == 0).collect();
                self.note(format!("partition {side:?}"));
                for i in 0..n {
                    for j in i + 1..n {
                        if side[i] != side[j] {
                            self.blocked.insert((i, j));
                            self.disconnect(i, j);
                        }
                    }
                }
            }
            3 | 4 => {
                self.note("partition heals".into());
                self.blocked.clear();
            }
            5 if self.devs[k].app.is_some() && self.devs[k].online => {
                self.note(format!("dev {k} offline"));
                self.drop_all_links(k);
                self.devs[k].online = false;
            }
            6 if self.devs[k].app.is_some() && !self.devs[k].online => {
                self.note(format!("dev {k} back online"));
                self.devs[k].online = true;
            }
            7 | 8 if self.devs[k].app.is_some() => {
                // Reload: a fresh page; a saved name rejoins 1.5 s later.
                self.note(format!("dev {k} reloads"));
                self.drop_all_links(k);
                // Every page load gets its own device id, as crypto random does.
                self.loads += 1;
                let seed = self.seed * 100_000 + 1000 + self.loads;
                // The page saves as it unloads, so the reload restores it all.
                self.devs[k].saved = self.devs[k].app.as_ref().unwrap().snapshot();
                let d = &mut self.devs[k];
                let mut fresh = app(seed);
                fresh.on_packet(&d.saved, 0);
                d.app = Some(fresh);
                d.online = true;
                d.dragging = false;
                d.join_at = d.name.as_ref().map(|_| self.now + 1500.0);
            }
            9 if self.devs.iter().filter(|d| d.app.is_some()).count() > 2 && self.r.below(4) == 0 => {
                self.note(format!("dev {k} closes for good"));
                self.drop_all_links(k);
                self.devs[k].app = None;
            }
            _ => {}
        }
    }

    fn apps(&self) -> impl Iterator<Item = (usize, &App)> {
        self.devs.iter().enumerate().filter_map(|(k, d)| d.app.as_ref().filter(|_| d.online).map(|a| (k, a)))
    }

    /// Everything heals: no partition, every online device connected, and
    /// 2 s pass with nobody acting.
    fn heal(&mut self) {
        self.heal_for(100);
    }

    fn heal_for(&mut self, ticks: u32) {
        self.blocked.clear();
        for d in &mut self.devs {
            if d.app.is_some() {
                d.online = true;
            }
        }
        for _ in 0..ticks {
            self.reconnect_some(1000);
            self.tick();
        }
    }

    fn fail(&self, what: &str) -> ! {
        let tail: Vec<&String> = self.log.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        panic!(
            "seed {}: {what}\n  last events:\n    {}\n  replay: SEED={} cargo test --release --test network -- --nocapture",
            self.seed,
            tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n    "),
            self.seed
        );
    }

    fn digest_of(&self, k: usize) -> String {
        let sh = &self.devs[k].app.as_ref().unwrap().shared;
        format!("{:?}/{} {}", sh.game, sh.gen, sh.digest())
    }
}

/// What happened across all runs (to check the chaos really got exercised).
#[derive(Default, Debug)]
struct Coverage {
    runs: u32,
    guessing_reached: u32,
    reloads: u32,
    partitions: u32,
    finished: u32,
}

/// One run: set up a table, play through chaos, heal, check, then play the
/// round out cooperatively and check it ends.
fn run(seed: u64, rough: Rough, cov: &mut Coverage) {
    let n = 3 + (seed % 3) as usize;
    let mut s = Sim::new(seed, n, rough);

    // Setup on a good network: everyone connects, sits down, a round is dealt.
    s.reconnect_some(1000);
    for _ in 0..20 {
        s.tick();
    }
    for k in 0..n {
        s.devs[k].name = Some(NAMES[k].into());
        s.devs[k].app.as_mut().unwrap().join(NAMES[k]);
        s.tick();
    }
    for _ in 0..20 {
        s.tick();
    }
    s.devs[0].app.as_mut().unwrap().action(1);
    for _ in 0..20 {
        s.tick();
    }
    let game0 = s.devs[0].app.as_ref().unwrap().shared.game;

    // Chaos.
    let mut guessed = false;
    for _ in 0..600 {
        if s.r.below(1000) < s.rough.faults as usize {
            s.fault();
        }
        s.reconnect_some(30);
        if s.r.below(1000) < s.rough.acts as usize {
            let k = s.r.below(n);
            if s.live(k) {
                s.act(k);
            }
        }
        s.tick();
        guessed |= s.apps().any(|(_, a)| matches!(a.shared.phase(), Phase::Guessing(_)));
    }
    cov.guessing_reached += guessed as u32;
    cov.reloads += s.log.iter().filter(|l| l.contains("reloads")).count() as u32;
    cov.partitions += s.log.iter().filter(|l| l.contains("partition [")).count() as u32;

    // Every hand lets go; then heal and go quiet.
    for d in &mut s.devs {
        if let (Some(a), true) = (d.app.as_mut(), d.dragging) {
            a.pointer_cancel();
            d.dragging = false;
        }
    }
    // Long enough for drifted devices to notice (two pings) and resync.
    s.heal_for(600);

    // 1. One state everywhere.
    let live: Vec<usize> = s.apps().map(|(k, _)| k).collect();
    let d0 = s.digest_of(live[0]);
    for &k in &live[1..] {
        if s.digest_of(k) != d0 {
            let cards = |k: usize| format!("{:?}", s.devs[k].app.as_ref().unwrap().shared.cards);
            s.fail(&format!("devices {} and {k} disagree after healing:\n  {}\n  {}\n  {}\n  {}", live[0], d0, s.digest_of(k), cards(live[0]), cards(k)));
        }
    }
    // 2. Nobody connected has lost their seat (or their name).
    for &k in &live {
        let a = s.devs[k].app.as_ref().unwrap();
        if let Some(id) = a.seat {
            match a.shared.seats.get(&id) {
                Some(seat) if seat.active => {}
                _ => s.fail(&format!("device {k} thinks it is seated but the table has no seat for it")),
            }
        }
        if s.devs[k].name.is_some() && s.devs[k].join_at.is_none() && a.seat.is_none() {
            s.fail(&format!("device {k} saved a name but isn't seated"));
        }
    }
    // 3. Nothing wiped without a New game.
    let game = s.devs[live[0]].app.as_ref().unwrap().shared.game;
    if s.new_games == 0 && game != game0 {
        s.fail("the table's game was replaced though nobody pressed New game");
    }
    // 4. No card is still drawn as held once every hand has let go.
    for &k in &live {
        let a = s.devs[k].app.as_ref().unwrap();
        if let Some(i) = a.scene().cards.iter().position(|c| c.held != 0) {
            s.fail(&format!("device {k} still shows card {i} as held after everyone let go"));
        }
    }

    // 5. The round can be played to its end: whoever is seated writes and
    // submits; if it still waits (on someone gone), Proceed; if nobody dealt
    // in is left to submit, deal again. Then solve every clover, moving on.
    let a0 = live[0];
    for _ in 0..3 {
        match s.devs[a0].app.as_ref().unwrap().shared.phase() {
            Phase::Lobby => s.devs[a0].app.as_mut().unwrap().action(1),
            Phase::Writing => {}
            _ => break,
        }
        s.heal();
        for &k in &live {
            let a = s.devs[k].app.as_mut().unwrap();
            if a.shared.phase() == Phase::Writing && a.seat.is_some_and(|me| a.shared.round_players().contains(&me)) {
                for d in 0..4u8 {
                    a.set_clue(d, "clue", false);
                }
                if !a.shared.is_ready(a.seat.unwrap()) {
                    a.action(2);
                }
            }
        }
        s.heal();
        let seated = live.iter().copied().find(|&k| s.devs[k].app.as_ref().unwrap().seat.is_some()).unwrap_or(a0);
        if s.devs[seated].app.as_ref().unwrap().shared.phase() == Phase::Writing {
            s.devs[seated].app.as_mut().unwrap().action(5);
            s.heal();
        }
        if s.devs[seated].app.as_ref().unwrap().shared.phase() == Phase::Writing {
            // Nobody dealt in is here to submit: what "Deal again" does.
            s.devs[seated].app.as_mut().unwrap().action(6);
            s.heal();
        }
    }
    for _ in 0..12 {
        let phase = s.devs[a0].app.as_ref().unwrap().shared.phase();
        let Phase::Guessing(_) = phase else { break };
        let w = s.devs[a0].app.as_ref().unwrap().scene().writer.unwrap();
        let Some(g) = live.iter().copied().find(|&k| {
            let a = s.devs[k].app.as_ref().unwrap();
            a.seat.is_some() && a.seat != Some(w)
        }) else {
            break; // nobody left to guess (only the writer is seated)
        };
        let a = s.devs[g].app.as_mut().unwrap();
        solve(a);
        a.action(3);
        s.heal();
        let a = s.devs[g].app.as_mut().unwrap();
        a.action(4);
        s.heal();
    }
    let phase = s.devs[a0].app.as_ref().unwrap().shared.phase();
    let seats: BTreeSet<u32> = live.iter().filter_map(|&k| s.devs[k].app.as_ref().unwrap().seat).collect();
    let only_writer = seats.len() < 2;
    if phase != Phase::End && !only_writer {
        let a = s.devs[a0].app.as_ref().unwrap();
        let w = a.scene().writer;
        let detail = w.map(|w| {
            let c = a.shared.check(w);
            let sc = a.scene();
            let seats: Vec<_> = live.iter().map(|&k| (k, s.devs[k].app.as_ref().unwrap().seat)).collect();
            format!("writer {w} mask {:04b} attempts {} cards {:?} seats {seats:?} puzzles {:?}", c.mask, c.attempts,
                sc.cards.iter().map(|c| (c.loc, c.rot)).collect::<Vec<_>>(), a.shared.puzzles())
        });
        let sh = &a.shared;
        let writing = format!("round players {:?} ready {:?} seats {:?} present {:?}", sh.round_players(),
            sh.round_players().iter().map(|p| sh.is_ready(*p)).collect::<Vec<_>>(), seats, a.present());
        s.fail(&format!("the round could not be played to its end (stuck in {phase:?}): {detail:?} {writing}"));
    }
    cov.finished += (phase == Phase::End) as u32;
    let d0 = s.digest_of(live[0]);
    if live.iter().any(|&k| s.digest_of(k) != d0) {
        s.fail("devices disagree at the end of the round");
    }
    cov.runs += 1;
}

/// A point where a touch picks up card `i` (not one lying over it).
fn grab_point(a: &App, i: usize) -> Option<(f32, f32)> {
    let (x, y, _) = a.card_screen(i);
    let offs = [0.0, -20.0, 20.0, -40.0, 40.0, -55.0, 55.0];
    offs.iter().flat_map(|&dx| offs.iter().map(move |&dy| (x + dx, y + dy))).find(|&(px, py)| a.card_at_point(px, py) == Some(i))
}

fn drag_card(a: &mut App, from: (f32, f32), to: (f32, f32)) {
    a.pointer_down(from.0, from.1);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        a.pointer_move(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
    }
    a.pointer_up(to.0, to.1);
    a.settle();
}

/// Put the right cards in every open spot, turned right (from this screen).
fn solve(a: &mut App) {
    a.settle();
    let Some(w) = a.scene().writer else { return };
    let d = a.shared.deal(w);
    for s in 0..NSLOTS {
        if a.scene().locked & (1 << s) != 0 {
            continue;
        }
        // Clear whatever lies over the card, out to the board's edge.
        for corner in 0..6 {
            if grab_point(a, s).is_some() {
                break;
            }
            let (x, y, _) = a.card_screen(s);
            let Some(top) = a.card_at_point(x, y) else { break };
            let Some(p) = grab_point(a, top) else { break };
            let to = a.board_screen(if corner % 2 == 0 { -260.0 } else { 260.0 }, if corner / 2 % 2 == 0 { -260.0 } else { 260.0 });
            drag_card(a, p, to);
        }
        let Some(from) = grab_point(a, s) else { continue };
        let to = a.slot_screen(s);
        drag_card(a, from, to);
        for _ in 0..4 {
            if a.scene().cards[s].rot == d.sol_rot[s] {
                break;
            }
            let Some(p) = grab_point(a, s) else { break };
            a.pointer_down(p.0, p.1);
            a.pointer_up(p.0, p.1);
            a.settle();
        }
    }
}

const CALM: Rough = Rough { latency: (5.0, 40.0), faults: 15, acts: 120 };
const STORMY: Rough = Rough { latency: (20.0, 400.0), faults: 60, acts: 200 };

/// Seeds `from..from + 150` ($SEEDS for more, for a long soak), or just
/// $SEED (to replay a failure) if it's in range.
fn seeds(from: u64) -> Vec<u64> {
    let n: u64 = std::env::var("SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(150);
    match std::env::var("SEED") {
        Ok(s) => Some(s.parse().unwrap()).filter(|s| (from..from + n.max(150)).contains(s)).into_iter().collect(),
        Err(_) => (from..from + n).collect(),
    }
}

#[test]
fn the_table_survives_a_calm_but_flaky_network() {
    let mut cov = Coverage::default();
    for seed in seeds(0) {
        run(seed, CALM, &mut cov);
    }
    println!("calm: {cov:?}");
    if std::env::var("SEED").is_err() {
        assert!(cov.reloads > 150 && cov.partitions > 80 && cov.guessing_reached > 100 && cov.finished > 140, "coverage: {cov:?}");
    }
}

#[test]
fn the_table_survives_a_stormy_network() {
    let mut cov = Coverage::default();
    for seed in seeds(10_000) {
        run(seed, STORMY, &mut cov);
    }
    println!("stormy: {cov:?}");
    if std::env::var("SEED").is_err() {
        assert!(cov.reloads > 600 && cov.partitions > 300 && cov.guessing_reached > 100 && cov.finished > 140, "coverage: {cov:?}");
    }
}

// ---- the bugs the random runs found, as fixed scenarios ------------------------

/// Three devices on a good network, seated, with a round dealt.
fn table3() -> Sim {
    let mut s = Sim::new(7, 3, CALM);
    s.reconnect_some(1000);
    for k in 0..3 {
        s.devs[k].name = Some(NAMES[k].into());
        s.devs[k].app.as_mut().unwrap().join(NAMES[k]);
        s.tick();
    }
    s.heal();
    s.devs[0].app.as_mut().unwrap().action(1);
    s.heal();
    s
}

#[test]
fn a_change_that_missed_one_peer_still_reaches_it() {
    // Ann's link to Cy is down when she writes a clue; then Ann's tab closes.
    // Bo and Cy stayed connected throughout, so no full sync happens between
    // them: they must notice the difference and resync on their own.
    let mut s = table3();
    s.disconnect(0, 2);
    s.blocked.insert((0, 2));
    s.devs[0].app.as_mut().unwrap().set_clue(0, "only bo saw this", false);
    for _ in 0..10 {
        s.tick();
    }
    s.drop_all_links(0);
    s.devs[0].app = None;
    s.heal_for(400);
    assert_eq!(s.digest_of(1), s.digest_of(2));
}

#[test]
fn a_player_who_reloads_and_deals_alone_cannot_wipe_the_table() {
    // Cy reloads while cut off, sees an empty table, sits down and deals.
    // When the network heals, the table in play (further along) must win.
    let mut s = table3();
    for k in 0..3 {
        let a = s.devs[k].app.as_mut().unwrap();
        a.set_clue(0, "kept", false);
    }
    s.heal();
    let game = s.devs[0].app.as_ref().unwrap().shared.game;
    s.drop_all_links(2);
    s.blocked.extend([(0, 2), (1, 2)]);
    s.devs[2].saved = Vec::new(); // no save on this device
    s.devs[2].app = Some(app(999));
    s.devs[2].app.as_mut().unwrap().join("Cy");
    s.devs[2].app.as_mut().unwrap().action(1);
    s.heal();
    let a = s.devs[2].app.as_ref().unwrap();
    assert_eq!(a.shared.game, game, "Cy's lone deal replaced the table");
    assert!(a.seat.is_some_and(|me| a.shared.seats.get(&me).is_some_and(|x| x.active)), "Cy sits back down at the table");
    assert_eq!(s.digest_of(0), s.digest_of(2));
}

#[test]
fn a_table_survives_every_device_reloading() {
    let mut s = table3();
    s.devs[1].app.as_mut().unwrap().set_clue(2, "remember me", false);
    s.heal();
    let before = s.digest_of(0);
    // Every page reloads at once; each restores its own save.
    for k in 0..3 {
        s.drop_all_links(k);
        let saved = s.devs[k].app.as_ref().unwrap().snapshot();
        let mut fresh = app(500 + k as u64);
        fresh.on_packet(&saved, 0);
        s.devs[k].app = Some(fresh);
        let name = s.devs[k].name.clone().unwrap();
        s.devs[k].app.as_mut().unwrap().join(&name);
    }
    s.heal();
    assert_eq!(s.digest_of(0), before);
    assert_eq!(s.digest_of(1), before);
}

#[test]
fn a_card_held_by_a_phone_that_vanished_is_let_go() {
    let mut s = table3();
    // Into guessing: everyone submits.
    for k in 0..3 {
        let a = s.devs[k].app.as_mut().unwrap();
        for d in 0..4u8 {
            a.set_clue(d, "clue", false);
        }
        a.action(2);
        s.heal();
    }
    let w = s.devs[0].app.as_ref().unwrap().scene().writer.unwrap();
    let g = (0..3).find(|&k| s.devs[k].app.as_ref().unwrap().seat != Some(w)).unwrap();
    // The guesser picks up a card and their phone dies mid-drag.
    let a = s.devs[g].app.as_mut().unwrap();
    a.settle();
    let (x, y, _) = a.card_screen(0);
    a.pointer_down(x, y);
    a.pointer_move(x + 40.0, y + 40.0);
    s.tick();
    s.drop_all_links(g);
    s.devs[g].app = None;
    s.heal_for(300);
    for k in (0..3).filter(|&k| k != g) {
        assert!(s.devs[k].app.as_ref().unwrap().scene().cards.iter().all(|c| c.held == 0), "device {k} still shows a held card");
    }
}
