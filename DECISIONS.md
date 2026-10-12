# Cloverleaf — frontloaded decisions

Experiment: test the "Rust→wasm, no framework, agent hill-climbs a custom harness"
workflow on a small P2P multiplayer board game (a So Clover!-style clover board).

| # | Unknown | Decision | Why |
|---|---------|----------|-----|
| 1 | Who owns logic? | **Rust (`core/`) owns everything**: state, sync/merge, geometry, hit-testing, input handling, rendering. JS is a dumb platform shim. | That's the thesis being tested. |
| 2 | Rust↔JS bridge | **Raw `extern "C"` exports, no wasm-bindgen.** Shared linear memory: Rust writes a draw-command buffer, an outbox, and reads an inbox. | Fewest layers. One less toolchain. |
| 3 | Rendering | **Canvas2D command buffer** (xform / rrect / circle / text) interpreted by ~60 lines of JS. Not WebGPU. | The game is 5 cards and text. WebGPU text rendering would be most of the work and buy nothing here. |
| 4 | P2P transport | **WebRTC data channels via Trystero.** Default signaling is public Nostr relays (no server of our own). `?signal=ws://…` swaps in a self-hosted relay (used by the harness). `?net=local` = BroadcastChannel (same-browser tabs). | Trystero is the only non-Rust code with real logic, and it is vendored as a single bundled file. |
| 5 | Consistency model | **Per-register last-writer-wins with Lamport stamps (clock, peerId).** Registers: deal, 5 cards, 4 clues, saved answer. Full-state exchange on peer join. A new deal makes all older registers invisible (epoch filtering), so state converges without coordination. | No host. Merge is commutative, associative and idempotent, so it's provable by property tests. |
| 6 | Board rotation | **Each player has their own view rotation (local, quarter turns, animated).** Card placement is shared. Cards on the board are stored in *board coordinates* (slot index + rotation relative to the board), so they turn with your view. Cards off the board are stored in *world coordinates* and don't move. | This is the user's requirement, and keeping two coordinate frames makes it fall out naturally. |
| 7 | Slot conflicts | Two cards claiming the same slot: the newer stamp wins, and the loser is shown at a fixed "bumped" spot. A local drop onto an occupied slot **swaps** (the occupant goes to where the dragged card came from). | Deterministic, so every peer shows the same thing. |
| 8 | Game scope | Deal 5 cards (4 words each, one per edge), a 2×2 board with 4 clue petals, drag on/off, tap to rotate a card, edit clues in place, "save answer → scramble → check". Clue writers use the honor system (everyone can see the board). | Keeps the core So Clover! loop without private hands. |
| 9 | What "good" means | `harness/` produces a scorecard: correctness gates (convergence, rotation invariants, swap, late join, clue sync, conflict) plus numbers (sync latency, frame cost, wasm size, bytes per drag update, legibility in CSS px at desktop and phone sizes). | The thing the agent climbs. |
