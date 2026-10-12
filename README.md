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

1. **Start game:** one press by anyone.
2. **Write.** Everyone gets a private clover: 4 locked cards in random rotations. Tap a leaf, type a clue that links the two words facing it, then press Enter. The board turns so the next empty leaf faces you. After the 4th leaf, press **Submit** (optionally choose 0–4 extra decoy cards first).
3. **Guessing starts by itself** once every connected player has submitted. Clovers are guessed one at a time. Everyone except the writer shares one board: drag cards on, tap a card to turn it, and press **Check**. Correct cards lock and wrong ones go back for one more try. A perfect first try is worth 6 points; otherwise you get 1 point per correct card.
4. **Next clover**, then **Play again**.

**Turning the board** is local to each player: tap ⟳ at the board's center, tap any empty part of the board, drag around it, or press Q/E. Cards on the board turn with it; cards off the board stay put. Each clover starts upright.

## Harness

```bash
(cd core && cargo test --release)   # 12 native properties, incl. 300-seed convergence fuzz
node harness/run.mjs                # 23 browser gates + metrics → harness/out/scorecard.json
```

The browser harness plays a whole 5-player game, including one phone-sized player. Latest scorecard (headless Chromium, WebRTC over loopback):

| metric | value |
|---|---|
| gates | 28/28 |
| presses to write + submit a clover | 6 (tap leaf, 4× Enter, Submit) |
| last submit → guessing starts | ~30 ms |
| card drop visible to 4 peers (p50 / p95) | 5 / 12 ms |
| late joiner gets full state | ~300 ms |
| frame CPU, wasm + Canvas2D (p95) | 0.3 ms |
| wasm size / JS imports | 169 KB / 0 |
| card word size, desktop / phone | 17.6 / 10.4 px |

Bugs the harness caught in this iteration: Chrome fires a late `blur` on a hidden input, which closed the next leaf after Enter. The phone roster re-wrapping resized the canvas and closed the editor mid-word. Player IDs came back as signed i32 across the wasm boundary. And writing left the board turned, so guessing began at whatever angle writing ended on.
