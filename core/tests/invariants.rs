//! Native harness: properties the game must satisfy, independent of browsers.

use cloverleaf::app::{App, EV_NET};
use cloverleaf::model::*;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

fn app(seed: u64) -> App {
    let mut a = App::new(seed);
    (a.w, a.h, a.dpr) = (1200.0, 900.0, 1.0);
    a
}

/// Pull EV_NET packets out of an app's outbox.
fn drain(a: &mut App) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let b = std::mem::take(&mut a.outbox);
    let mut p = 0;
    while p < b.len() {
        let kind = b[p];
        let len = u32::from_le_bytes(b[p + 1..p + 5].try_into().unwrap()) as usize;
        if kind == EV_NET {
            out.push(b[p + 5..p + 5 + len].to_vec());
        }
        p += 5 + len;
    }
    out
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

fn slot_at(a: &App, s: usize) -> (f32, f32) {
    a.slot_screen(s)
}

#[test]
fn wire_roundtrip() {
    let s = Stamp { clock: 99, peer: 7 };
    let msgs = vec![
        Msg::Op(Op::Card(3, Card { loc: Loc::Table { x: 1.5, y: 2.5 }, rot: 3, held: 9, stamp: s })),
        Msg::Op(Op::Card(0, Card { loc: Loc::Slot(2), rot: 1, held: 0, stamp: s })),
        Msg::Op(Op::Card(4, Card { loc: Loc::Float { x: -3.0, y: 4.0 }, rot: 0, held: 1, stamp: s })),
        Msg::Op(Op::Clue(1, Clue { text: "fruit ☘ salad".into(), stamp: s })),
        Msg::Op(Op::Deal(Deal { seed: 1234, stamp: s })),
        Msg::Op(Op::Solution(Solution { slots: [(0, 1), (1, 2), (2, 3), (4, 0)], stamp: s })),
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
        let n = (r.next() % 64) as usize;
        let b: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        let _ = decode(&b);
    }
}

/// Random ops from 3 replicas, delivered in random order with duplicates.
#[test]
fn replicas_converge_under_reordering_and_duplication() {
    for seed in 0..300u64 {
        let mut r = Rng::new(seed);
        let mut reps: Vec<App> = (0..3).map(|k| app(seed * 10 + k + 1)).collect();
        let mut pending: Vec<(usize, Vec<u8>)> = Vec::new();
        for _ in 0..60 {
            let k = (r.next() % 3) as usize;
            let a = &mut reps[k];
            match r.next() % 8 {
                0 => a.action(1),
                1 => a.action(2),
                2 => a.action(3),
                3 => a.set_clue((r.next() % 4) as u8, &format!("clue{}", r.next() % 100)),
                4 => a.rotate_view(if r.next() % 2 == 0 { 1 } else { -1 }),
                _ => {
                    a.settle();
                    let i = (r.next() % 5) as usize;
                    let from = card_at(a, i);
                    let to = if r.next() % 3 == 0 {
                        ((r.next() % 1200) as f32, (r.next() % 900) as f32)
                    } else {
                        slot_at(a, (r.next() % 4) as usize)
                    };
                    drag(a, from, to);
                }
            }
            a.frame(0.0);
            for p in drain(a) {
                for dst in 0..3 {
                    if dst != k {
                        pending.push((dst, p.clone()));
                    }
                }
            }
            // Deliver a random subset now (out of order), occasionally twice.
            let mut i = 0;
            while i < pending.len() {
                if r.next() % 3 == 0 {
                    let (dst, p) = pending.swap_remove(i);
                    reps[dst].on_packet(&p);
                    if r.next() % 5 == 0 {
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
        let (cards, owner) = reps[0].shared.layout();
        for (s, o) in owner.iter().enumerate() {
            if let Some(i) = o {
                assert_eq!(cards[*i].loc, Loc::Slot(s as u8));
            }
        }
    }
}

#[test]
fn late_joiner_receives_full_state() {
    let mut a = app(1);
    a.set_clue(2, "hello");
    let (from, to) = (card_at(&a, 1), slot_at(&a, 3));
    drag(&mut a, from, to);
    drain(&mut a);
    let mut b = app(2);
    a.on_peer_join();
    for p in drain(&mut a) {
        b.on_packet(&p);
    }
    assert_eq!(a.shared.digest(), b.shared.digest());
    assert_eq!(b.shared.layout().0[1].loc, Loc::Slot(3));
}

#[test]
fn view_rotation_turns_board_cards_only() {
    let mut a = app(3);
    let (from, to) = (card_at(&a, 0), slot_at(&a, 0));
    drag(&mut a, from, to);
    let (from, to) = (card_at(&a, 1), (1000.0, 300.0));
    drag(&mut a, from, to);
    a.settle();
    let before: Vec<_> = (0..5).map(|i| a.card_screen(i)).collect();
    let digest = a.shared.digest();
    drain(&mut a);
    let (cx, cy) = a.screen().apply(CX, CY);

    a.rotate_view(1);
    a.settle();
    assert_eq!(a.shared.digest(), digest, "view rotation must not touch shared state");
    a.frame(0.0);
    assert!(drain(&mut a).is_empty(), "view rotation must not send");
    a.settle();
    for i in 0..5 {
        let (bx, by, ba) = before[i];
        let (x, y, ang) = a.card_screen(i);
        if matches!(a.shared.layout().0[i].loc, Loc::Slot(_)) {
            // Rotated 90° clockwise about the board centre.
            let (ex, ey) = (cx - (by - cy), cy + (bx - cx));
            assert!(approx(x, ex) && approx(y, ey), "card {i}: ({x},{y}) vs ({ex},{ey})");
            let d = (ang - ba - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
            assert!(d < 1e-3 || d > std::f32::consts::TAU - 1e-3, "card {i} angle");
        } else {
            assert!(approx(x, bx) && approx(y, by) && approx(ang, ba), "off-board card {i} moved");
        }
    }
    // Four quarter turns are the identity.
    for _ in 0..3 {
        a.rotate_view(1);
    }
    a.settle();
    for i in 0..5 {
        let (x, y, _) = a.card_screen(i);
        assert!(approx(x, before[i].0) && approx(y, before[i].1));
    }
}

#[test]
fn drop_maps_screen_position_to_board_slot_under_rotation() {
    let mut a = app(4);
    a.rotate_view(1);
    a.settle();
    // The slot that appears top-left on screen after a clockwise quarter
    // turn is board slot 3 (bottom-left).
    let top_left = (0..4)
        .min_by(|&s, &t| {
            let (sx, sy) = slot_at(&a, s);
            let (tx, ty) = slot_at(&a, t);
            (sx + sy).partial_cmp(&(tx + ty)).unwrap()
        })
        .unwrap();
    assert_eq!(top_left, 3);
    let (from, to) = (card_at(&a, 2), slot_at(&a, 3));
    drag(&mut a, from, to);
    assert_eq!(a.shared.layout().0[2].loc, Loc::Slot(3));
    // Orientation on screen is kept: unrotated table card → board rot = -q.
    assert_eq!(a.shared.layout().0[2].rot, 3);
}

#[test]
fn drop_on_occupied_slot_swaps() {
    let mut a = app(5);
    let (from, to) = (card_at(&a, 0), slot_at(&a, 1));
    drag(&mut a, from, to);
    let (from, to) = (card_at(&a, 1), slot_at(&a, 2));
    drag(&mut a, from, to);
    // Drag card 0 (slot 1) onto slot 2 → card 1 moves to slot 1.
    let (from, to) = (card_at(&a, 0), slot_at(&a, 2));
    drag(&mut a, from, to);
    let (cards, _) = a.shared.layout();
    assert_eq!(cards[0].loc, Loc::Slot(2));
    assert_eq!(cards[1].loc, Loc::Slot(1));
    // Table card onto an occupied slot → occupant goes to the table origin.
    let origin = card_at(&a, 4);
    let (from, to) = (origin, slot_at(&a, 2));
    drag(&mut a, from, to);
    let (cards, _) = a.shared.layout();
    assert_eq!(cards[4].loc, Loc::Slot(2));
    let (x, y) = card_at(&a, 0);
    assert!(approx(x, origin.0) && approx(y, origin.1));
}

#[test]
fn concurrent_claims_on_one_slot_resolve_identically() {
    let mut a = app(6);
    let mut b = app(7);
    let (from, to) = (card_at(&a, 0), slot_at(&a, 0));
    drag(&mut a, from, to);
    let (from, to) = (card_at(&b, 3), slot_at(&b, 0));
    drag(&mut b, from, to);
    let pa = drain(&mut a);
    let pb = drain(&mut b);
    pb.iter().for_each(|p| a.on_packet(p));
    pa.iter().for_each(|p| b.on_packet(p));
    assert_eq!(a.shared.digest(), b.shared.digest());
    let (cards, owner) = a.shared.layout();
    let w = owner[0].unwrap();
    assert!(w == 0 || w == 3);
    let loser = if w == 0 { 3 } else { 0 };
    assert!(matches!(cards[loser].loc, Loc::Table { .. }));
}

#[test]
fn tap_rotates_card_and_deal_resets() {
    let mut a = app(8);
    let p = card_at(&a, 2);
    a.pointer_down(p.0, p.1);
    a.pointer_up(p.0, p.1);
    assert_eq!(a.shared.layout().0[2].rot, 1);
    a.set_clue(0, "x");
    a.action(1);
    assert_eq!(a.shared.layout().0[2].rot, 0);
    assert_eq!(a.shared.clue(0), "");
}

#[test]
fn answer_check_round_trip() {
    let mut a = app(9);
    for s in 0..4 {
        let (from, to) = (card_at(&a, s), slot_at(&a, s));
        drag(&mut a, from, to);
    }
    a.action(2);
    assert!(a.shared.solution().is_some());
    a.action(3);
    assert!(a.shared.layout().1.iter().all(|o| o.is_none()));
}

#[test]
fn deal_words_are_distinct_and_deterministic() {
    let s = Shared::default();
    let w = s.words();
    let mut all: Vec<&str> = w.iter().flatten().copied().collect();
    assert_eq!(s.words(), w);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 20);
}

#[test]
fn portrait_layout_keeps_everything_on_screen_and_drops_correct() {
    let mut a = App::new(10);
    (a.w, a.h, a.dpr) = (390.0, 700.0, 3.0);
    assert!(a.portrait());
    for q in 0..4 {
        a.settle();
        for i in 0..NCARDS {
            let (x, y, _) = a.card_screen(i);
            assert!(x > 0.0 && x < a.w && y > 0.0 && y < a.h, "card {i} off screen at q={q}");
        }
        let s = q as usize;
        let (from, to) = (card_at(&a, s), slot_at(&a, s));
        drag(&mut a, from, to);
        assert_eq!(a.shared.layout().0[s].loc, Loc::Slot(s as u8));
        a.rotate_view(1);
    }
}
