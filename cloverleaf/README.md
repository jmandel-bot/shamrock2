# Cloverleaf: a Rust→wasm P2P board game, built harness-first

A So Clover!-style game. Browsers connect directly over WebRTC: there is no
game server, and signaling goes through public Nostr relays via
[Trystero](https://github.com/dmotz/trystero). Each player rotates the board
in their own view, while card placement is shared. Cards on the board turn
with your view; cards off the board stay where they are.

This is an experiment in the workflow of "delete abstraction layers, write a
validation harness, let the agent hill-climb it". See [DECISIONS.md](DECISIONS.md)
for the up-front decisions.

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
cd experiments/cloverleaf
bun install            # Trystero + Playwright (only needed for build/harness)
./build.sh             # cargo → web/core.wasm, bundle Trystero into web/vendor/
node harness/serve.mjs # prints a URL
```

- `http://localhost:8787/#myroom` uses public Nostr relays for signaling, so it works across machines.
- `?signal=ws://localhost:8788#myroom` uses the bundled local relay.
- `?net=local#myroom` uses BroadcastChannel, for same-browser tabs with no WebRTC.

`web/` is fully static, so any static host works. Anyone with the same `#room` joins the table.

**Controls:** drag cards on or off the board. Tap a card to rotate it. Tap a
petal to write a clue. Use ⟲/⟳ (or Q/E) to rotate your own view. To play, a clue
writer arranges 4 cards, writes clues, clicks **Save answer**, then **Scramble**.
Everyone else solves it and clicks **Check**.

## Harness

```bash
(cd core && cargo test --release)   # 12 native properties, incl. 300-seed convergence fuzz
node harness/run.mjs                # 23 browser gates + metrics → harness/out/scorecard.json
```

Latest scorecard (headless Chromium, 3–5 tabs, WebRTC over loopback):

| metric | value |
|---|---|
| gates | 23/23 |
| wasm size / JS imports | 108 KB / 0 |
| drop → visible on peers (p50 / p95) | 31 / 33 ms |
| clue sync | 29 ms |
| late joiner gets full state | ~300 ms |
| bytes per drag update | 23 B |
| frame CPU, wasm + Canvas2D (p95) | 0.6 ms |
| card word size, desktop / phone | 19.4 / 10.4 px |

The phone legibility gate failed at first (words were 6.8 px). Fixing it is the
one real hill-climb in this run. Portrait screens now show the whole table turned
a quarter turn, which is presentation-only and leaves shared coordinates alone.
