//! Native harness: properties the game must satisfy, independent of browsers.

use cloverleaf::app::{App, EV_EDIT_CLUE, EV_NET};
use cloverleaf::model::*;

const TAU: f32 = std::f32::consts::TAU;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

fn app(seed: u64) -> App {
    let mut a = App::new(seed);
    (a.w, a.h, a.dpr) = (900.0, 1000.0, 1.0);
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

/// A table of players on a perfect network.
struct Table {
    apps: Vec<App>,
    t: f64,
}

impl Table {
    fn new(n: usize) -> Table {
        Table { apps: (0..n).map(|k| app(1000 + k as u64)).collect(), t: 0.0 }
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
                        a.on_packet(&p);
                    }
                }
            }
        }
    }
    fn digests_equal(&self) -> bool {
        let d = self.apps[0].shared.digest();
        self.apps.iter().all(|a| a.shared.digest() == d)
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

fn write_and_submit(a: &mut App, clues: [&str; 4]) {
    for (d, c) in clues.iter().enumerate() {
        a.set_clue(d as u8, c, false);
    }
    a.action(2);
}

/// Start a round on player 0, everyone writes and submits, pump until guessing.
fn table_in_guessing(n: usize) -> Table {
    let mut t = Table::new(n);
    t.pump();
    t.apps[0].action(1);
    t.pump();
    for k in 0..n {
        write_and_submit(&mut t.apps[k], ["sea", "sky", "fire", "dog"]);
        t.pump();
    }
    t
}

/// Place the current clover correctly from player `k`'s screen.
fn solve(t: &mut Table, k: usize, wrong_slot0: bool) {
    let a = &mut t.apps[k];
    a.settle();
    let w = a.scene().writer.unwrap();
    let d = deal(a.shared.round.seed, w);
    for s in 0..NSLOTS {
        let card = if wrong_slot0 && s == 0 { NSLOTS } else { s };
        if a.scene().locked & (1 << s) != 0 {
            continue;
        }
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
    let round = Round { seed: 42, stamp: Stamp { clock: 3, peer: 1 } };
    let op = |body| Msg::Op(Op { round, body });
    let msgs = vec![
        op(Body::None),
        op(Body::Sub(9, Sub { clues: ["a".into(), "fruit ☘ salad".into(), "".into(), "d".into()], decoys: 2, stamp: s })),
        op(Body::Start(s)),
        op(Body::Card(9, 5, Card { loc: Loc::Table { x: 1.5, y: 2.5 }, rot: 3, held: 9, stamp: s })),
        op(Body::Card(9, 0, Card { loc: Loc::Slot(2), rot: 1, held: 0, stamp: s })),
        op(Body::Card(9, 4, Card { loc: Loc::Float { x: -3.0, y: 4.0 }, rot: 0, held: 1, stamp: s })),
        op(Body::Check(9, Check { attempts: 1, mask: 0b0101, stamp: s })),
        op(Body::Next(3, s)),
        Msg::Hello { peer: 5, name: "Witty Otter".into() },
        Msg::Cursor { peer: 5, kind: 2, x: 10.0, y: -10.0 },
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
        let n = (r.next() % 96) as usize;
        let b: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        let _ = decode(&b);
    }
}

#[test]
fn full_game_three_players() {
    let mut t = table_in_guessing(3);
    assert!(t.digests_equal());
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0), "guessing starts once everyone submitted");
    let puzzles = t.apps[0].shared.puzzles();
    assert_eq!(puzzles.len(), 3);
    for k in 0..3 {
        let w = t.apps[0].shared.puzzles()[k];
        let guesser = t.apps.iter().position(|a| a.me != w).unwrap();
        let writer = t.apps.iter().position(|a| a.me == w).unwrap();
        // The writer can't move cards on their own clover.
        let before = t.apps[writer].shared.digest();
        let (from, to) = (card_at(&t.apps[writer], 0), t.apps[writer].slot_screen(0));
        drag(&mut t.apps[writer], from, to);
        assert_eq!(t.apps[writer].shared.digest(), before);
        assert!(!t.apps[writer].can_check());

        solve(&mut t, guesser, k == 1);
        t.pump();
        assert!(t.apps[guesser].can_check());
        t.apps[guesser].action(3);
        t.pump();
        let c = t.apps[0].shared.check(w);
        if k == 1 {
            // First try had a decoy in slot 0: slot 0 goes back, rest locked.
            assert_eq!((c.attempts, c.mask, c.done()), (1, 0b1110, false));
            let (cards, owner) = t.apps[0].shared.layout(w);
            assert!(owner[0].is_none());
            assert!(matches!(cards[NSLOTS].loc, Loc::Table { .. }));
            solve(&mut t, guesser, false);
            t.pump();
            t.apps[guesser].action(3);
            t.pump();
            let c = t.apps[0].shared.check(w);
            assert_eq!((c.attempts, c.mask, c.score()), (2, 0b1111, 4));
        } else {
            assert_eq!(c.score(), PERFECT_SCORE);
        }
        assert!(t.digests_equal());
        t.apps[writer].action(4);
        t.pump();
    }
    assert_eq!(t.apps[1].shared.phase(), Phase::End);
    assert_eq!(t.apps[2].shared.total_score(), 6 + 4 + 6);
    t.apps[2].action(1);
    t.pump();
    assert!(t.apps.iter().all(|a| a.shared.phase() == Phase::Writing && a.drafts.iter().all(|d| d.is_empty())));
}

#[test]
fn guessing_waits_for_everyone_present() {
    let mut t = Table::new(3);
    t.pump();
    t.apps[1].action(1);
    t.pump();
    write_and_submit(&mut t.apps[0], ["a", "b", "c", "d"]);
    write_and_submit(&mut t.apps[1], ["a", "b", "c", "d"]);
    t.pump();
    assert_eq!(t.apps[0].shared.phase(), Phase::Writing);
    // A player who leaves stops blocking once their presence expires.
    let gone = t.apps.pop().unwrap();
    for _ in 0..60 {
        t.t += 200.0;
        t.pump();
    }
    assert_eq!(t.apps[0].shared.phase(), Phase::Guessing(0));
    assert!(!t.apps[0].shared.puzzles().contains(&gone.me));
}

#[test]
fn incomplete_clues_cannot_be_submitted() {
    let mut t = Table::new(1);
    t.apps[0].action(1);
    t.apps[0].set_clue(0, "x", false);
    assert!(!t.apps[0].can_submit());
    t.apps[0].action(2);
    assert!(t.apps[0].shared.subs.is_empty());
}

#[test]
fn enter_moves_to_next_empty_leaf_and_faces_it() {
    let mut a = app(5);
    a.action(1);
    drain(&mut a);
    let top = a.petal_screen(0);
    tap(&mut a, top);
    let (_, ev) = drain(&mut a);
    assert!(ev.iter().any(|(k, b)| *k == EV_EDIT_CLUE && b[0] == 0));
    a.set_clue(0, "ocean", true);
    let (_, ev) = drain(&mut a);
    let next = ev.iter().find(|(k, _)| *k == EV_EDIT_CLUE).expect("opens next leaf");
    assert_eq!(next.1[0], 1);
    // Leaf 1 (right) is turned to the top: a quarter turn anticlockwise.
    assert_eq!(a.q.rem_euclid(4), 3);
    a.settle();
    let (x, y) = a.petal_screen(1);
    assert!(approx(x, top.0) && approx(y, top.1));
}

#[test]
fn turning_the_board_is_local_and_keeps_table_cards_still() {
    let mut t = table_in_guessing(2);
    let w = t.apps[0].shared.puzzles()[0];
    let g = t.apps.iter().position(|a| a.me != w).unwrap();
    {
        let a = &mut t.apps[g];
        a.settle();
        let (from, to) = (card_at(a, 0), a.slot_screen(0));
        drag(a, from, to);
    }
    t.pump();
    let a = &mut t.apps[g];
    let digest = a.shared.digest();
    let before: Vec<_> = (0..a.scene().cards.len()).map(|i| a.card_screen(i)).collect();
    let (cx, cy) = a.hub_screen();
    let hub = a.hub_screen();
    tap(a, hub);
    a.settle();
    assert_eq!(a.q, 1, "tapping the hub turns the board");
    a.frame(t.t + 1.0);
    assert!(drain(a).0.iter().all(|p| decode(p).iter().all(|m| !matches!(m, Msg::Op(_)))), "no shared ops");
    assert_eq!(a.shared.digest(), digest);
    for (i, b) in before.iter().enumerate() {
        let (x, y, ang) = a.card_screen(i);
        if matches!(a.scene().cards[i].loc, Loc::Slot(_)) {
            let (ex, ey) = (cx - (b.1 - cy), cy + (b.0 - cx));
            assert!(approx(x, ex) && approx(y, ey), "slotted card {i} turns with the board");
            let d = (ang - b.2 - std::f32::consts::FRAC_PI_2).rem_euclid(TAU);
            assert!(d < 1e-3 || d > TAU - 1e-3);
        } else {
            assert!(approx(x, b.0) && approx(y, b.1) && approx(ang, b.2), "table card {i} stays put");
        }
    }
    // Tapping empty board turns it too; four turns are the identity.
    // Tapping an empty slot turns it too; four turns are the identity.
    let empty = (0..NSLOTS).find(|&s| a.scene().owner[s].is_none()).unwrap();
    let at = a.slot_screen(empty);
    tap(a, at);
    assert_eq!(a.q, 2);
    for _ in 0..2 {
        let hub = a.hub_screen();
        tap(a, hub);
    }
    a.settle();
    for (i, b) in before.iter().enumerate() {
        let (x, y, _) = a.card_screen(i);
        assert!(approx(x, b.0) && approx(y, b.1));
    }
}

#[test]
fn dragging_around_the_board_spins_it_to_the_nearest_quarter() {
    let mut a = app(6);
    a.action(1);
    let (cx, cy) = a.hub_screen();
    let r = 330.0;
    // East of the board, sweeping clockwise to south.
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
    let mut a = app(7);
    a.action(1);
    a.settle();
    let before = a.scene().cards.iter().map(|c| c.rot).collect::<Vec<_>>();
    let at = card_at(&a, 2);
    tap(&mut a, at);
    assert_eq!(a.q, 1);
    assert_eq!(a.scene().cards.iter().map(|c| c.rot).collect::<Vec<_>>(), before);
}

#[test]
fn drop_maps_screen_position_to_board_slot_under_rotation() {
    let mut t = table_in_guessing(2);
    let w = t.apps[0].shared.puzzles()[0];
    let g = t.apps.iter().position(|a| a.me != w).unwrap();
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
    let rot_on_table = a.scene().cards[2].rot;
    let (from, to) = (card_at(a, 2), a.slot_screen(3));
    drag(a, from, to);
    let c = a.scene().cards[2];
    assert_eq!(c.loc, Loc::Slot(3));
    assert_eq!(c.rot, (rot_on_table + 3) % 4, "on-screen orientation is kept");
}

#[test]
fn concurrent_claims_on_one_slot_resolve_identically() {
    let mut t = table_in_guessing(3);
    let w = t.apps[0].shared.puzzles()[0];
    let gs: Vec<usize> = (0..3).filter(|&k| t.apps[k].me != w).collect();
    for (n, &k) in gs.iter().enumerate() {
        let a = &mut t.apps[k];
        a.settle();
        let (from, to) = (card_at(a, n), a.slot_screen(0));
        drag(a, from, to);
    }
    t.pump();
    assert!(t.digests_equal());
    let (_, owner) = t.apps[0].shared.layout(w);
    assert!(owner[0] == Some(0) || owner[0] == Some(1));
}

#[test]
fn late_joiner_receives_full_state() {
    let mut t = table_in_guessing(2);
    let w = t.apps[0].shared.puzzles()[0];
    let g = t.apps.iter().position(|a| a.me != w).unwrap();
    let (from, to) = (card_at(&t.apps[g], 1), t.apps[g].slot_screen(2));
    drag(&mut t.apps[g], from, to);
    t.pump();
    let mut late = app(77);
    t.apps[0].on_peer_join();
    for p in drain(&mut t.apps[0]).0 {
        late.on_packet(&p);
    }
    assert_eq!(late.shared.digest(), t.apps[0].shared.digest());
    assert_eq!(late.shared.phase(), Phase::Guessing(0));
}

#[test]
fn any_op_from_a_newer_round_resets() {
    let mut a = app(8);
    a.action(1);
    write_and_submit(&mut a, ["a", "b", "c", "d"]);
    let mut b = app(9);
    b.action(1);
    // b's round is newer only if its stamp wins; force it.
    b.shared.round.stamp.clock = 50;
    let round = b.shared.round;
    let mut p = Vec::new();
    encode(&Msg::Op(Op { round, body: Body::Next(0, Stamp { clock: 51, peer: b.me }) }), &mut p);
    a.on_packet(&p);
    assert_eq!(a.shared.round, round);
    assert!(a.shared.subs.is_empty());
    assert!(a.drafts.iter().all(|d| d.is_empty()));
}

/// Random play by 3 players, delivered out of order with duplicates.
#[test]
fn replicas_converge_under_reordering_and_duplication() {
    let (mut guessed, mut moved) = (0, 0);
    for seed in 0..120u64 {
        let mut r = Rng::new(seed);
        let mut reps: Vec<App> = (0..3).map(|k| app(seed * 10 + k + 1)).collect();
        let mut pending: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut now = 0.0;
        for _ in 0..80 {
            now += 50.0;
            let k = r.below(3);
            let a = &mut reps[k];
            a.frame(now);
            a.settle();
            match r.below(10) {
                0 => a.action(1),
                1 => write_and_submit(a, ["w", "x", "y", "z"]),
                2 => a.action(3),
                3 => a.action(4),
                4 => a.action(5),
                5 => a.rotate_view(if r.below(2) == 0 { 1 } else { -1 }),
                _ => {
                    let n = a.scene().cards.len();
                    if n > 0 {
                        let from = card_at(a, r.below(n));
                        let to = if r.below(3) == 0 {
                            (r.below(900) as f32, r.below(1000) as f32)
                        } else {
                            a.slot_screen(r.below(4))
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
                    reps[dst].on_packet(&p);
                    if r.below(5) == 0 {
                        reps[dst].on_packet(&p);
                    }
                } else {
                    i += 1;
                }
            }
        }
        for (dst, p) in pending.drain(..).rev() {
            reps[dst].on_packet(&p);
        }
        let d0 = reps[0].shared.digest();
        for a in &reps[1..] {
            assert_eq!(a.shared.digest(), d0, "seed {seed}");
        }
        guessed += (!reps[0].shared.puzzles().is_empty()) as u32;
        moved += (!reps[0].shared.cards.is_empty()) as u32;
        for w in reps[0].shared.puzzles() {
            let (cards, owner) = reps[0].shared.layout(w);
            for (s, o) in owner.iter().enumerate() {
                if let Some(i) = o {
                    assert_eq!(cards[*i].loc, Loc::Slot(s as u8));
                }
            }
        }
    }
    // The fuzz must actually reach the interesting states.
    assert!(guessed >= 30 && moved >= 30, "coverage: {guessed} guessed, {moved} with card moves");
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
fn everything_fits_on_a_phone() {
    let mut t = table_in_guessing(2);
    let w = t.apps[0].shared.puzzles()[0];
    let g = t.apps.iter().position(|a| a.me != w).unwrap();
    let a = &mut t.apps[g];
    (a.w, a.h) = (390.0, 680.0);
    for q in 0..4 {
        a.q = q;
        a.settle();
        for i in 0..a.scene().cards.len() {
            let (x, y, _) = a.card_screen(i);
            assert!(x > 0.0 && x < a.w && y > 0.0 && y < a.h, "card {i} off screen at q={q}");
        }
    }
}
