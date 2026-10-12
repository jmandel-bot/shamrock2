//! Native harness: properties the game must satisfy, independent of browsers.

use cloverleaf::app::{App, EV_EDIT_CLUE, EV_NET, ZOOM_MAX, ZOOM_MIN};
use cloverleaf::model::*;

const TAU: f32 = std::f32::consts::TAU;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

fn app(seed: u64) -> App {
    let mut a = App::new(seed);
    (a.w, a.h, a.dpr) = (1000.0, 800.0, 1.0);
    a
}

/// Split an outbox into (net packets, other events).
fn drain(a: &mut App) -> (Vec<Vec<u8>>, Vec<(u8, Vec<u8>)>) {
    let (mut net, mut ev) = (Vec::new(), Vec::new());
    let b = std::mem::take(&mut a.outbox);
    let mut p = 0;
    while p < b.len() {
        let kind = b[p];
        let len = u32::from_le_bytes(b[p + 1..p + 5].try_into().unwrap()) as usize;
        let body = b[p + 5..p + 5 + len].to_vec();
        if kind == EV_NET { net.push(body) } else { ev.push((kind, body)) }
        p += 5 + len;
    }
    (net, ev)
}

/// Devices on a perfect network.
struct Table {
    apps: Vec<App>,
    t: f64,
}

impl Table {
    /// `n` devices, each seated under its own name.
    fn new(n: usize) -> Table {
        let mut t = Table { apps: (0..n).map(|k| app(1000 + k as u64)).collect(), t: 0.0 };
        for k in 0..n {
            t.apps[k].join(&format!("Player {k}"));
            t.pump();
        }
        t
    }
    fn add(&mut self, a: App) -> usize {
        self.apps.push(a);
        self.apps[0].on_peer_join();
        self.pump();
        self.apps.len() - 1
    }
    fn pump(&mut self) {
        for _ in 0..8 {
            self.t += 16.0;
            let mut sent = Vec::new();
            for (k, a) in self.apps.iter_mut().enumerate() {
                a.frame(self.t);
                for p in drain(a).0 {
                    sent.push((k, p));
                }
            }
            if sent.is_empty() {
                break;
            }
            for (from, p) in sent {
                for (k, a) in self.apps.iter_mut().enumerate() {
                    if k != from {
                        a.on_packet(&p, from as u32 + 1);
                    }
                }
            }
        }
    }
    fn digests_equal(&self) -> bool {
        let d = self.apps[0].shared.digest();
        self.apps.iter().all(|a| a.shared.digest() == d)
    }
    fn idx_of_seat(&self, seat: u32) -> usize {
        self.apps.iter().position(|a| a.seat == Some(seat)).unwrap()
    }
    fn guesser_for(&self, w: u32) -> usize {
        self.apps.iter().position(|a| a.seat.is_some() && a.seat != Some(w)).unwrap()
    }
    fn writer_now(&self) -> u32 {
        match self.apps[0].shared.phase() {
            Phase::Guessing(k) => self.apps[0].shared.puzzles()[k],
            p => panic!("not guessing: {p:?}"),
        }
    }
}

fn tap(a: &mut App, at: (f32, f32)) {
    a.pointer_down(at.0, at.1);
    a.pointer_up(at.0, at.1);
}

fn drag(a: &mut App, from: (f32, f32), to: (f32, f32)) {
    a.pointer_down(from.0, from.1);
    for k in 1..=10 {
        let t = k as f32 / 10.0;
        a.pointer_move(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
    }
    a.pointer_up(to.0, to.1);
}

fn card_at(a: &App, i: usize) -> (f32, f32) {
    let (x, y, _) = a.card_screen(i);
    (x, y)
}

fn write(a: &mut App, clues: [&str; 4]) {
    for (d, c) in clues.iter().enumerate() {
        a.set_clue(d as u8, c, false);
    }
}

fn write_and_ready(a: &mut App) {
    write(a, ["sea", "sky", "fire", "dog"]);
    a.action(2);
}

/// Everyone seated writes and taps Ready; pump until guessing.
fn table_in_guessing(n: usize) -> Table {
    let mut t = Table::new(n);
    t.apps[0].action(1);
    t.pump();
    for k in 0..n {
        write_and_ready(&mut t.apps[k]);
        t.pump();
    }
    t
}

/// Place the current clover correctly from device `k`'s screen.
fn solve(t: &mut Table, k: usize, decoy_in_spot0: bool) {
    let a = &mut t.apps[k];
    a.settle();
    let w = a.scene().writer.unwrap();
    let d = deal(a.shared.round.seed, w);
    for s in 0..NSLOTS {
        if a.scene().locked & (1 << s) != 0 {
            continue;
        }
        let card = if decoy_in_spot0 && s == 0 { NSLOTS } else { s };
        let (from, to) = (card_at(a, card), a.slot_screen(s));
        drag(a, from, to);
        let want = if card == s { d.sol_rot[s] } else { a.scene().cards[card].rot };
        while a.scene().cards[card].rot != want {
            let at = card_at(a, card);
            tap(a, at);
        }
    }
}

#[test]
fn wire_roundtrip() {
    let s = Stamp { clock: 99, peer: 7 };
    let game = Stamp { clock: 1, peer: 2 };
    let round = Round { seed: 42, n: 3, stamp: Stamp { clock: 3, peer: 1 }, ..Round::default() }.with_players(&[9, 4, 7]);
    let op = |body| Msg::Op(Op { game, round, body });
    let msgs = vec![
        op(Body::None),
        op(Body::Seat(9, Seat { name: "Ana ☘".into(), active: true, stamp: s })),
        op(Body::Sub(9, Sub { clues: ["a".into(), "fruit".into(), "".into(), "d".into()], decoys: 2, ready: true, stamp: s })),
        op(Body::Start(s)),
        op(Body::Card(9, 5, Card { loc: Loc::Table { x: 0.25, y: 1.0 }, rot: 3, held: 9, stamp: s })),
        op(Body::Card(9, 0, Card { loc: Loc::Slot(2), rot: 1, held: 0, stamp: s })),
        op(Body::Card(9, 4, Card { loc: Loc::Float { x: -3.0, y: 4.0 }, rot: 0, held: 1, stamp: s })),
        op(Body::Check(9, Check { attempts: 1, mask: 0b0101, stamp: s })),
        op(Body::Next(3, s)),
        op(Body::Ledger(3, 9, 6, s)),
        Msg::Hello { device: 5, seat: 9 },
        Msg::Cursor { device: 5, seat: 9, kind: 2, x: 10.0, y: -10.0 },
    ];
    let mut b = Vec::new();
    for m in &msgs {
        encode(m, &mut b);
    }
    assert_eq!(decode(&b), msgs);
}

#[test]
fn decode_never_panics_on_garbage() {
    let mut r = Rng::new(42);
    for _ in 0..20_000 {
        let n = r.below(120);
        let b: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        let _ = decode(&b);
    }
}

#[test]
fn full_game_two_rounds_three_players() {
    let mut t = table_in_guessing(3);
    assert!(t.digests_equal());
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0), "guessing starts once everyone is ready");
    assert_eq!(t.apps[0].shared.puzzles().len(), 3);
    for k in 0..3 {
        let w = t.writer_now();
        let (g, wi) = (t.guesser_for(w), t.idx_of_seat(w));
        // The writer can't move their own cards.
        let before = t.apps[wi].shared.digest();
        let (from, to) = (card_at(&t.apps[wi], 0), t.apps[wi].slot_screen(0));
        drag(&mut t.apps[wi], from, to);
        assert_eq!(t.apps[wi].shared.digest(), before);
        assert!(!t.apps[wi].can_check());

        solve(&mut t, g, k == 1);
        t.pump();
        assert!(t.apps[g].can_check());
        t.apps[g].action(3);
        t.pump();
        let c = t.apps[0].shared.check(w);
        if k == 1 {
            assert_eq!((c.attempts, c.mask, c.done()), (1, 0b1110, false));
            let (cards, claim) = t.apps[0].shared.layout(w);
            assert!(claim[0].is_none());
            assert!(matches!(cards[NSLOTS].loc, Loc::Table { .. }), "wrong card went back to the tray");
            solve(&mut t, g, false);
            t.pump();
            t.apps[g].action(3);
            t.pump();
            assert_eq!(t.apps[0].shared.check(w).score(), 4);
        } else {
            assert_eq!(c.score(), PERFECT_SCORE);
        }
        assert!(t.digests_equal());
        t.apps[wi].action(4);
        t.pump();
    }
    assert_eq!(t.apps[1].shared.phase(), Phase::End);
    assert_eq!(t.apps[2].shared.game_score(), 16);
    // Next round keeps the table and the running score.
    t.apps[2].action(1);
    t.pump();
    let s = &t.apps[0].shared;
    assert_eq!((s.phase(), s.round.n, s.players().len(), s.game_score()), (Phase::Writing, 2, 3, 16));
    assert_eq!(s.round_score(2), 0);
}

#[test]
fn guessing_waits_for_an_explicit_ready() {
    let mut t = Table::new(2);
    t.apps[0].action(1);
    t.pump();
    write(&mut t.apps[0], ["a", "b", "c", "d"]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Writing, "all clues written isn't enough");
    t.apps[0].action(2);
    t.apps[0].action(2); // un-ready
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Writing);
    t.apps[0].action(2);
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0));
}

#[test]
fn submitted_players_can_keep_editing_until_guessing_starts() {
    let mut t = Table::new(2);
    t.apps[0].action(1);
    t.pump();
    let me = t.apps[0].seat.unwrap();
    write_and_ready(&mut t.apps[0]);
    t.pump();
    t.apps[0].set_clue(2, "changed", false);
    t.apps[0].set_decoys(3);
    t.pump();
    let s = &t.apps[1].shared;
    assert!(s.is_ready(me), "editing keeps you submitted");
    assert_eq!(s.latest(me).unwrap().clues[2], "changed");
    // Clearing a clue does un-submit (it can't be guessed half-empty).
    t.apps[0].set_clue(1, "", false);
    t.pump();
    assert!(!t.apps[1].shared.is_ready(me));
    t.apps[0].set_clue(1, "back", false);
    t.apps[0].action(2);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    let s = &t.apps[1].shared;
    assert_eq!(s.phase(), Phase::Guessing(0));
    assert_eq!(s.effective(me).unwrap().clues, ["sea".to_string(), "back".into(), "changed".into(), "dog".into()]);
    assert_eq!(s.ncards(me), 7);
}

#[test]
fn an_edit_racing_proceed_resolves_the_same_everywhere() {
    let mut t = Table::new(3);
    t.apps[0].action(1);
    t.pump();
    let me = t.apps[0].seat.unwrap();
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    // Concurrently: player 0 edits while player 1 proceeds without player 2.
    t.apps[0].set_clue(0, "late edit", false);
    t.apps[1].action(5);
    let (e, p) = (drain(&mut t.apps[0]).0, drain(&mut t.apps[1]).0);
    // Deliver in opposite orders to the third device and back.
    for x in e.iter().chain(p.iter()) {
        t.apps[2].on_packet(x, 9);
    }
    for x in p.iter() {
        t.apps[0].on_packet(x, 2);
    }
    for x in e.iter() {
        t.apps[1].on_packet(x, 1);
    }
    t.pump();
    assert!(t.digests_equal());
    let c = t.apps[0].shared.effective(me).unwrap().clues[0].clone();
    assert!(c == "sea" || c == "late edit");
    assert_eq!(t.apps[2].shared.puzzles().len(), 2);
}

#[test]
fn starting_fixes_the_roster_late_joiners_wait_for_next_round() {
    let mut t = Table::new(2);
    t.apps[0].action(1);
    t.pump();
    let k = t.add(app(42));
    t.apps[k].join("Latecomer");
    t.pump();
    let late = t.apps[k].seat.unwrap();
    assert!(!t.apps[0].shared.round.players().contains(&late));
    assert!(t.apps[k].scene().cards.is_empty(), "no clover this round");
    assert!(t.apps[k].ui_json().contains("\"inRound\":false"));
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    assert_eq!(t.apps[k].shared.phase(), Phase::Guessing(0), "the latecomer doesn't block");
    // They can guess, and they're dealt in next round.
    let w = t.writer_now();
    assert!(t.apps[k].scene().movable.iter().any(|m| *m) || w == late);
    for _ in 0..2 {
        let w = t.writer_now();
        let g = t.guesser_for(w);
        solve(&mut t, g, false);
        t.apps[g].action(3);
        t.pump();
        t.apps[g].action(4);
        t.pump();
    }
    t.apps[k].action(1);
    t.pump();
    assert!(t.apps[0].shared.round.players().contains(&late));
}

#[test]
fn auto_proceed_ignores_who_is_online() {
    let mut t = Table::new(3);
    t.apps[0].action(1);
    t.pump();
    // Player 2 submits, then drops off entirely.
    write_and_ready(&mut t.apps[2]);
    t.pump();
    let _gone = t.apps.pop().unwrap();
    for a in t.apps.iter_mut() {
        a.on_link_lost(3);
    }
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0), "all three submitted, offline or not");
    assert_eq!(t.apps[0].shared.puzzles().len(), 3);
}

#[test]
fn decoy_bid_sets_cards_in_play() {
    let mut t = Table::new(2);
    t.apps[0].action(1);
    t.pump();
    t.apps[0].set_decoys(3);
    write_and_ready(&mut t.apps[0]);
    t.apps[1].set_decoys(0);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    let s = &t.apps[1].shared;
    assert_eq!(s.ncards(t.apps[0].seat.unwrap()), 7);
    assert_eq!(s.ncards(t.apps[1].seat.unwrap()), 4);
}

#[test]
fn away_players_block_until_someone_starts_without_them() {
    let mut t = Table::new(3);
    t.apps[1].action(1);
    t.pump();
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    // Going quiet (a backgrounded tab) is not leaving.
    for _ in 0..60 {
        t.t += 200.0;
        for k in 0..2 {
            t.apps[k].frame(t.t);
            drain(&mut t.apps[k]);
        }
    }
    assert_eq!(t.apps[0].shared.phase(), Phase::Writing, "a silent player still blocks");
    // Even a dropped connection doesn't auto-skip anyone (another device
    // may simply not have connected to them yet)...
    let gone = t.apps.pop().unwrap();
    for a in t.apps.iter_mut() {
        a.on_link_lost(3);
    }
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Writing);
    assert!(t.apps[0].ui_json().contains("\"ready\":false,\"away\":true"), "but they are shown as away");
    // ...a ready player decides.
    t.apps[0].action(5);
    t.pump();
    assert_eq!(t.apps[1].shared.phase(), Phase::Guessing(0));
    assert!(!t.apps[0].shared.puzzles().contains(&gone.seat.unwrap()));
}

#[test]
fn a_device_that_cant_see_everyone_never_starts_early() {
    let mut t = Table::new(3);
    t.apps[0].action(1);
    t.pump();
    // A watcher who has only heard from two of the three players.
    let mut w = app(321);
    w.watch();
    t.apps[0].on_peer_join();
    for p in drain(&mut t.apps[0]).0 {
        w.on_packet(&p, 1);
    }
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    for p in [0, 1] {
        t.apps[p].on_peer_join();
        for pk in drain(&mut t.apps[p]).0 {
            w.on_packet(&pk, p as u32 + 1);
        }
    }
    w.frame(t.t + 5.0);
    assert_eq!(w.shared.phase(), Phase::Writing);
}

#[test]
fn choosing_an_existing_name_takes_over_the_seat() {
    let mut t = Table::new(2);
    t.apps[0].action(1);
    t.pump();
    write(&mut t.apps[0], ["one", "two", "", ""]);
    t.pump();
    let mut phone = app(99);
    phone.on_packet(&{
        t.apps[0].on_peer_join();
        drain(&mut t.apps[0]).0.concat()
    }, 0);
    let k = t.add(phone);
    t.apps[k].join("player 0"); // names are case-insensitive identities
    t.pump();
    assert_eq!(t.apps[k].seat, t.apps[0].seat);
    assert_eq!(t.apps[0].shared.players().len(), 2, "no new seat created");
    let top = t.apps[k].petal_screen(0);
    tap(&mut t.apps[k], top);
    let (_, ev) = drain(&mut t.apps[k]);
    let edit = ev.iter().find(|(kind, _)| *kind == EV_EDIT_CLUE).expect("editor opens");
    assert_eq!(&edit.1[21..], b"one", "drafts follow the seat");
}

#[test]
fn watchers_cant_move_cards_and_dont_block() {
    let mut t = Table::new(2);
    let mut w = app(55);
    w.watch();
    let wi = t.add(w);
    t.apps[0].action(1);
    t.pump();
    assert!(t.apps[wi].scene().cards.is_empty(), "watchers don't get a clover");
    write_and_ready(&mut t.apps[0]);
    write_and_ready(&mut t.apps[1]);
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0));
    let before = t.apps[wi].shared.digest();
    t.apps[wi].settle();
    let (from, to) = (card_at(&t.apps[wi], 0), t.apps[wi].slot_screen(0));
    drag(&mut t.apps[wi], from, to);
    assert_eq!(t.apps[wi].shared.digest(), before);
}

#[test]
fn new_game_resets_scores_and_keeps_whoever_is_here() {
    let mut t = table_in_guessing(3);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    solve(&mut t, g, false);
    t.apps[g].action(3);
    t.pump();
    assert_eq!(t.apps[0].shared.game_score(), 6);
    let gone = t.apps.pop().unwrap();
    for a in t.apps.iter_mut() {
        a.on_link_lost(3);
    }
    t.pump();
    t.apps[0].action(7);
    t.pump();
    let s = &t.apps[1].shared;
    assert_eq!((s.game_score(), s.phase(), s.round.n), (0, Phase::Lobby, 0));
    assert_eq!(s.players().len(), 2);
    assert!(!s.players().contains(&gone.seat.unwrap()));
    assert!(t.digests_equal());
}

#[test]
fn cards_stay_where_dropped_unless_near_a_spot() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    let a = &mut t.apps[g];
    a.settle();
    let (sx, sy) = slot_pos(0);
    // Tentative: near spot 0 but not on it.
    let to = a.board_screen(sx + 45.0, sy + 40.0);
    let from = card_at(a, 0);
    drag(a, from, to);
    let c = a.scene().cards[0];
    match c.loc {
        Loc::Float { x, y } => assert!(approx(x, sx + 45.0) && approx(y, sy + 40.0)),
        l => panic!("expected free placement, got {l:?}"),
    }
    assert_eq!(a.scene().claim[0], Some(0), "close enough to count for a check");
    // Far from any spot: placed, not claimed.
    let to = a.board_screen(0.0, -230.0);
    let from = card_at(a, 1);
    drag(a, from, to);
    assert!(matches!(a.scene().cards[1].loc, Loc::Float { .. }));
    assert!(a.scene().claim.iter().all(|c| *c != Some(1)));
    // Very close: snaps.
    let (x2, y2) = slot_pos(2);
    let to = a.board_screen(x2 + 15.0, y2 - 10.0);
    let from = card_at(a, 2);
    drag(a, from, to);
    assert_eq!(a.scene().cards[2].loc, Loc::Slot(2));
    // Two cards may share a spot loosely while people think; nearest claims it.
    let to = a.board_screen(x2 + 60.0, y2 + 20.0);
    let from = card_at(a, 3);
    drag(a, from, to);
    assert_eq!(a.scene().claim[2], Some(2));
    // Snapping onto an occupied spot pushes the occupant back where the
    // dragged card came from.
    let origin = a.scene().cards[3].loc;
    let from = card_at(a, 3);
    let to = a.slot_screen(2);
    drag(a, from, to);
    assert_eq!(a.scene().cards[3].loc, Loc::Slot(2));
    assert_eq!(a.scene().cards[2].loc, origin);
}

#[test]
fn turning_the_board_is_local_and_keeps_tray_cards_still() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    {
        let a = &mut t.apps[g];
        a.settle();
        let (from, to) = (card_at(a, 0), a.slot_screen(0));
        drag(a, from, to);
        let to = a.board_screen(-40.0, 200.0);
        let from = card_at(a, 1);
        drag(a, from, to);
    }
    t.pump();
    let a = &mut t.apps[g];
    let digest = a.shared.digest();
    let n = a.scene().cards.len();
    let before: Vec<_> = (0..n).map(|i| a.card_screen(i)).collect();
    let (cx, cy) = a.hub_screen();
    tap(a, (cx, cy));
    assert_eq!(a.q, 1, "tapping the hub turns the board");
    a.settle();
    a.frame(t.t + 1.0);
    assert!(drain(a).0.iter().all(|p| decode(p).iter().all(|m| !matches!(m, Msg::Op(_)))), "no shared ops");
    assert_eq!(a.shared.digest(), digest);
    for (i, b) in before.iter().enumerate() {
        let (x, y, ang) = a.card_screen(i);
        if board_pos(&a.scene().cards[i]).is_some() {
            let (ex, ey) = (cx - (b.1 - cy), cy + (b.0 - cx));
            assert!(approx(x, ex) && approx(y, ey), "board card {i} turns with the board");
            let d = (ang - b.2 - std::f32::consts::FRAC_PI_2).rem_euclid(TAU);
            assert!(d < 1e-3 || d > TAU - 1e-3);
        } else {
            assert!(approx(x, b.0) && approx(y, b.1) && approx(ang, b.2), "tray card {i} stays put");
        }
    }
}

#[test]
fn card_turns_and_board_turns_animate() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    let a = &mut t.apps[g];
    a.settle();
    let mut now = t.t;
    a.frame(now);
    let start = a.shown_angle(0);
    let at = card_at(a, 0);
    tap(a, at);
    now += 16.0;
    a.frame(now);
    let target = a.card_screen(0).2;
    let mid = a.shown_angle(0);
    let gap = |x: f32| (x - target + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
    assert!(gap(mid).abs() > 0.2 && gap(mid).abs() < gap(start).abs(), "card is mid-turn after one frame");
    for _ in 0..60 {
        now += 16.0;
        a.frame(now);
    }
    assert!(gap(a.shown_angle(0)).abs() < 1e-2, "card settles");
    a.rotate_view(1);
    now += 16.0;
    a.frame(now);
    assert!(a.angle > 0.1 && a.angle < 1.4, "board is mid-turn after one frame");
}

#[test]
fn enter_moves_to_next_empty_leaf_and_faces_it() {
    let mut t = Table::new(1);
    let a = &mut t.apps[0];
    a.action(1);
    a.frame(1000.0);
    drain(a);
    a.settle();
    let top = a.petal_screen(0);
    tap(a, top);
    let (_, ev) = drain(a);
    assert!(ev.iter().any(|(k, b)| *k == EV_EDIT_CLUE && b[0] == 0));
    a.set_clue(0, "ocean", true);
    let (_, ev) = drain(a);
    let next = ev.iter().find(|(k, _)| *k == EV_EDIT_CLUE).expect("opens next leaf");
    assert_eq!(next.1[0], 1);
    assert_eq!(a.q.rem_euclid(4), 3);
    a.settle();
    let (x, y) = a.petal_screen(1);
    assert!(approx(x, top.0) && approx(y, top.1));
}

#[test]
fn dragging_around_the_board_spins_it_to_the_nearest_quarter() {
    let mut t = Table::new(1);
    let a = &mut t.apps[0];
    a.action(1);
    let (cx, cy) = a.hub_screen();
    let r = 330.0;
    a.pointer_down(cx + r, cy);
    for k in 1..=20 {
        let th = (k as f32 / 20.0) * 1.45;
        a.pointer_move(cx + r * th.cos(), cy + r * th.sin());
    }
    a.pointer_up(cx + r * 1.45f32.cos(), cy + r * 1.45f32.sin());
    assert_eq!(a.q, 1);
}

#[test]
fn writer_taps_on_locked_cards_turn_the_board() {
    let mut t = Table::new(1);
    let a = &mut t.apps[0];
    a.action(1);
    a.settle();
    let before = a.scene().cards.iter().map(|c| c.rot).collect::<Vec<_>>();
    let at = card_at(a, 2);
    tap(a, at);
    assert_eq!(a.q, 1);
    assert_eq!(a.scene().cards.iter().map(|c| c.rot).collect::<Vec<_>>(), before);
}

#[test]
fn drop_in_a_turned_view_lands_in_the_right_spot() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    let a = &mut t.apps[g];
    a.rotate_view(1);
    a.settle();
    let top_left = (0..4)
        .min_by(|&s, &u| {
            let (sx, sy) = a.slot_screen(s);
            let (ux, uy) = a.slot_screen(u);
            (sx + sy).partial_cmp(&(ux + uy)).unwrap()
        })
        .unwrap();
    assert_eq!(top_left, 3);
    let rot_in_tray = a.scene().cards[2].rot;
    let (from, to) = (card_at(a, 2), a.slot_screen(3));
    drag(a, from, to);
    let c = a.scene().cards[2];
    assert_eq!(c.loc, Loc::Slot(3));
    assert_eq!(c.rot, (rot_in_tray + 3) % 4, "on-screen orientation is kept");
}

#[test]
fn concurrent_drops_on_one_spot_resolve_identically() {
    let mut t = table_in_guessing(3);
    let w = t.writer_now();
    let gs: Vec<usize> = (0..3).filter(|&k| t.apps[k].seat != Some(w)).collect();
    for (n, &k) in gs.iter().enumerate() {
        let a = &mut t.apps[k];
        a.settle();
        let (from, to) = (card_at(a, n), a.slot_screen(0));
        drag(a, from, to);
    }
    t.pump();
    assert!(t.digests_equal());
    let (_, claim) = t.apps[0].shared.layout(w);
    assert!(claim[0] == Some(0) || claim[0] == Some(1));
}

#[test]
fn late_joiner_receives_full_state() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    let (from, to) = (card_at(&t.apps[g], 1), t.apps[g].slot_screen(2));
    drag(&mut t.apps[g], from, to);
    t.pump();
    let mut late = app(77);
    t.apps[0].on_peer_join();
    for p in drain(&mut t.apps[0]).0 {
        late.on_packet(&p, 0);
    }
    assert_eq!(late.shared.digest(), t.apps[0].shared.digest());
}

#[test]
fn newer_rounds_reset_round_state_but_keep_seats_and_scores() {
    let mut s = Shared::default();
    let g = Stamp { clock: 1, peer: 1 };
    let r1 = Round { seed: 1, n: 1, stamp: Stamp { clock: 2, peer: 1 }, ..Round::default() };
    let st = |c| Stamp { clock: c, peer: 1 };
    s.apply(&Op { game: g, round: r1, body: Body::Seat(5, Seat { name: "a".into(), active: true, stamp: st(3) }) });
    s.apply(&Op { game: g, round: r1, body: Body::Sub(5, Sub { ready: true, stamp: st(4), ..Sub::default() }) });
    s.apply(&Op { game: g, round: r1, body: Body::Ledger(1, 5, 6, st(5)) });
    let r2 = Round { seed: 2, n: 2, stamp: st(6), ..Round::default() };
    s.apply(&Op { game: g, round: r2, body: Body::None });
    assert!(s.subs.is_empty());
    assert_eq!((s.players(), s.game_score()), (vec![5], 6));
    // A late op from round 1 doesn't resurrect anything.
    s.apply(&Op { game: g, round: r1, body: Body::Sub(5, Sub { stamp: st(9), ..Sub::default() }) });
    assert!(s.subs.is_empty());
    // A newer game wipes the lot.
    s.apply(&Op { game: st(10), round: Round::default(), body: Body::None });
    assert!(s.seats.is_empty() && s.ledger.is_empty());
}

/// Random play by 3 devices, delivered out of order with duplicates.
#[test]
fn replicas_converge_under_reordering_and_duplication() {
    let (mut guessed, mut moved) = (0, 0);
    for seed in 0..120u64 {
        let mut r = Rng::new(seed);
        let mut reps: Vec<App> = (0..3).map(|k| app(seed * 10 + k + 1)).collect();
        let mut pending: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut now = 0.0;
        for step in 0..90 {
            now += 50.0;
            let k = r.below(3);
            let a = &mut reps[k];
            a.frame(now);
            a.settle();
            match if step < 3 { 99 } else { r.below(12) } {
                99 | 0 => a.join(["Ann", "Bo", "Cy"][r.below(3)]),
                1 => a.action(1),
                2 => write_and_ready(a),
                3 => a.action(3),
                4 => a.action(4),
                5 => a.action(if r.below(8) == 0 { 7 } else { 5 }),
                6 => a.set_decoys(r.below(5) as u8),
                7 => a.rotate_view(if r.below(2) == 0 { 1 } else { -1 }),
                _ => {
                    let n = a.scene().cards.len();
                    if n > 0 {
                        let from = card_at(a, r.below(n));
                        let to = match r.below(3) {
                            0 => (r.below(1000) as f32, r.below(800) as f32),
                            1 => a.board_screen(r.below(600) as f32 - 300.0, r.below(600) as f32 - 300.0),
                            _ => a.slot_screen(r.below(4)),
                        };
                        drag(a, from, to);
                    }
                }
            }
            a.frame(now + 1.0);
            for p in drain(a).0 {
                for dst in 0..3 {
                    if dst != k {
                        pending.push((dst, p.clone()));
                    }
                }
            }
            let mut i = 0;
            while i < pending.len() {
                if r.below(3) == 0 {
                    let (dst, p) = pending.swap_remove(i);
                    reps[dst].on_packet(&p, 0);
                    if r.below(5) == 0 {
                        reps[dst].on_packet(&p, 0);
                    }
                } else {
                    i += 1;
                }
            }
        }
        for (dst, p) in pending.drain(..).rev() {
            reps[dst].on_packet(&p, 0);
        }
        let d0 = reps[0].shared.digest();
        for a in &reps[1..] {
            assert_eq!(a.shared.digest(), d0, "seed {seed}");
        }
        guessed += (!reps[0].shared.puzzles().is_empty()) as u32;
        moved += (!reps[0].shared.cards.is_empty()) as u32;
    }
    assert!(guessed >= 20 && moved >= 20, "coverage: {guessed} guessed, {moved} with card moves");
}

#[test]
fn deals_are_distinct_deterministic_and_per_writer() {
    let a = deal(1, 10);
    let b = deal(1, 10);
    let c = deal(1, 11);
    assert_eq!(a.words, b.words);
    assert_ne!(a.words, c.words);
    let mut all: Vec<&str> = a.words.iter().flatten().copied().collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), MAX_CARDS * 4);
}

#[test]
fn board_fills_the_screen_and_everything_fits() {
    for (w, h) in [(390.0, 700.0), (1280.0, 760.0), (768.0, 1000.0)] {
        let mut t = table_in_guessing(2);
        let wr = t.writer_now();
        let g = t.guesser_for(wr);
        let a = &mut t.apps[g];
        (a.w, a.h) = (w, h);
        let board_px = 2.0 * EXTENT * a.scale();
        assert!(board_px >= 0.6 * w.min(h), "{w}x{h}: board is {board_px}px");
        for q in 0..4 {
            a.q = q;
            a.settle();
            for i in 0..a.scene().cards.len() {
                let (x, y, _) = a.card_screen(i);
                assert!(x > 0.0 && x < w && y > 0.0 && y < h, "{w}x{h}: card {i} off screen at q={q}");
            }
        }
    }
}

#[test]
fn card_size_is_local_and_hit_testing_follows_it() {
    let mut t = table_in_guessing(2);
    let w = t.writer_now();
    let g = t.guesser_for(w);
    let (o, a) = {
        let (head, tail) = t.apps.split_at_mut(g.max(1));
        if g == 0 { (&mut tail[0], &mut head[0]) } else { (&mut head[0], &mut tail[0]) }
    };
    for a in [&mut *a, &mut *o] {
        (a.w, a.h) = (390.0, 700.0);
        a.settle();
    }
    let digest = a.shared.digest();
    let k = a.scale();
    a.set_zoom(9.0);
    assert!((a.zoom - ZOOM_MAX).abs() < 1e-6, "clamped");
    assert_eq!(a.shared.digest(), digest, "zoom is not shared state");
    // A spot in the board corner: zoomed cards overlap neighbours at most edge to edge.
    assert!(CARD * ZOOM_MAX <= 2.0 * SLOT_OFF + 0.01);
    // A tap near the zoomed card's corner (outside its unzoomed size) still hits it.
    let (from, to) = (card_at(a, 0), a.slot_screen(1));
    drag(a, from, to);
    let (cx, cy) = a.slot_screen(1);
    let corner = HALF * k * (1.0 + ZOOM_MAX) / 2.0;
    let rot = a.scene().cards[0].rot;
    tap(a, (cx + corner, cy + corner));
    assert_eq!(a.scene().cards[0].rot, (rot + 1) % 4);
    // Every card stays on screen at the largest size.
    for i in 0..a.scene().cards.len() {
        let (x, y, _) = a.card_screen(i);
        assert!(x > 0.0 && x < a.w && y > 0.0 && y < a.h, "card {i} off screen");
    }
    // Another player at a different size sees the same positions on the board.
    o.set_zoom(ZOOM_MIN);
    t.pump();
    assert!(t.digests_equal());
}
