# Cloverleaf: a Rust→wasm P2P board game, built harness-first

A So Clover!-style game. Browsers connect directly over WebRTC: there is no
game server, and signaling goes through public Nostr relays via
[Trystero](https://github.com/dmotz/trystero). Each player rotates the board
in their own view, while card placement is shared. Cards on the board turn
with your view; cards off the board stay where they are.

This is an experiment in the workflow of "delete abstraction layers, write a
validation harness, let the agent hill-climb it". See [DECISIONS.md](DECISIONS.md)
for the up-front decisions.

**Play:** https://jmandel-bot.github.io/shamrock2/#pick-a-room-name (share the link; same room = same table)

## Layout

```
core/                Rust crate; all game logic. No dependencies, no wasm-bindgen.
  src/model.rs       replicated state: LWW registers + Lamport stamps, wire format
  src/app.rs         per-player view rotation, input, drag/drop, draw-command buffer
  src/lib.rs         raw extern "C" exports
  tests/             native property tests (convergence, rotation invariants, …)
web/                 static site: index.html, main.js (~180-line shim), core.wasm, vendored Trystero
harness/serve.mjs    static server + local signaling relay
harness/run.mjs      Playwright scorecard: real tabs, real WebRTC, real pointer events
```

## Run it

```bash

bun install            # Trystero + Playwright (only needed for build/harness)
./build.sh             # cargo → web/core.wasm, bundle Trystero into web/vendor/
node harness/serve.mjs # prints a URL
```

- `http://localhost:8787/#myroom` uses public Nostr relays for signaling, so it works across machines.
- `?signal=ws://localhost:8788#myroom` uses the bundled local relay.
- `?net=local#myroom` uses BroadcastChannel, for same-browser tabs with no WebRTC.

`web/` is fully static, so any static host works. Anyone with the same `#room` joins the table.

**How a game goes** (So Clover! rules, everyone at once):

1. **Join.** Your first visit asks for a name: tap Join, or *Just watch*. Names are identities, so picking a name that's already at the table takes over that seat (to switch devices, or to debug). Your device remembers your name for this room.
2. **Deal cards:** one press by anyone.
3. **Write.** Everyone gets a private clover with 4 locked cards in random rotations. Tap a leaf, type a clue linking the two words facing it, then Enter; the board turns so the next empty leaf faces you. Bid how many decoy cards to mix in (− / +), then tap **Ready**. Tapping Ready again, or editing a clue, un-readies you. Drafts are shared state, so a takeover keeps them.
4. **Guessing starts by itself** once every seated player is Ready. If someone has left, the ready players see "Waiting for Dee (away)" and tap *Start without them* (or remove them in the menu). Guessing never starts on its own while someone is missing: in a peer-to-peer mesh, a device that hasn't connected to a player yet can't tell "absent" from "not seen yet". Clovers are guessed one at a time on a big shared board. Drag cards **anywhere**: loose "maybe" placements stay exactly where you leave them for everyone, and a card only snaps when dropped close to a spot (pushing any card already there back to where yours came from). The card touched last sits on top. A card within reach of a spot counts for **Check**. Correct cards lock and wrong ones go back to the tray for one more try. A perfect first try scores 6; otherwise 1 per correct card.
5. **Next clover**, then **Next round**. Seats and scores carry across rounds. The ⋯ menu has the invite link, switch player, watch, *Deal new cards*, *New game* (reset scores, keep whoever is here), and *Remove* for away players.

**Turning the board** is local to each player: tap ⟳ at its centre, tap empty board, or drag around it. Cards on the board turn with it; cards in the tray stay put. Card turns, moves and board turns all animate. Each clover starts upright.

**Card size** is per device: − / + in the bottom bar (narrow screens start larger). Positions on the board are shared; bigger cards just overlap more on your screen. At the largest size, cards in neighbouring spots touch edge to edge.

**Presence** follows the WebRTC connections, not a heartbeat, so a phone that locks its screen or a backgrounded tab still counts as at the table.

**Layout:** the board grows to fill the screen. While guessing, a tray for loose cards sits below it (portrait) or beside it (landscape). Tray positions are stored relative to the tray, so they fit every screen.

## Harness

```bash
(cd core && cargo test --release)   # 12 native properties, incl. 300-seed convergence fuzz
node harness/run.mjs                # 23 browser gates + metrics → harness/out/scorecard.json
```

The browser harness drives the real UI through a whole game: 5 players (one phone-sized), a watcher, a seat takeover from a tablet, two rounds, and a New game. Latest scorecard (headless Chromium, WebRTC over loopback):

| metric | value |
|---|---|
| gates | 35/35 (5 consecutive runs) |
| presses to join / write a clover + Ready | 1 / 6 |
| last Ready → guessing starts | ~60 ms |
| card drop visible on 6 other screens (p50 / p95) | ~18 / ~33 ms |
| late visitor gets full state | ~300 ms |
| frame CPU, wasm + Canvas2D (p95) | ~1.3 ms |
| card word size while guessing, desktop / phone (default · largest) | 23.6 / 15.6 · 18.4 px |
| board width on a 390 px phone | 96% of screen |
| wasm size / JS imports | 220 KB / 0 |

Bugs the harness caught this iteration: dropping onto an occupied spot stacked cards without snapping, so turns went to the wrong card. A snapped card drew *under* a loose one, so taps hit the loose card. And a CSS `display` rule overrode `hidden`, leaving the decoy stepper showing during guessing (found by looking at screenshots, not by a gate). Adding the card-size control then surfaced two flaky failures. Repeating runs turned them into real bugs. Presence was a heartbeat sent from the render loop, so a throttled background tab looked like a player who had left. And any device, even a watcher, could auto-start guessing based on its own partial view of who was connected. Presence is now connection-based, and auto-start waits for every seated player.
