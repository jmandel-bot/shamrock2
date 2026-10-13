# Shamrock: a peer-to-peer clover clue game in Rust → wasm

A So Clover!-style word game, dealt from a 220-card deck (`core/src/shamrock.json`). Cards are dealt whole from a draw-down deck, so none repeats until the deck runs out. Browsers connect directly over WebRTC: there is no
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
2. **Start with N players:** one press by anyone. The button names who will be dealt in (seated players at the table now); that roster is fixed for the round, and anyone who joins later is dealt in next round (they can help guess meanwhile).
3. **Write.** Everyone gets a private clover with 4 locked cards in random rotations. Tap a leaf, type a clue linking the two words facing it, then Enter; the board turns so the next empty leaf faces you. Bid how many decoy cards to mix in (− / +), then **Submit**. You can keep editing clues and decoys after submitting, right up until guessing starts. Clearing a clue un-submits, and tapping *Submitted ✓* withdraws. Drafts are shared state, so a takeover keeps them.
4. **A progress bar fills** one segment per dealt-in player as they submit. Guessing starts by itself only when **all** of them have submitted, whether they're online, offline or gone. Anyone can override with **Proceed with 3 of 5**. Every version of each player's clues is kept, so an edit that races Proceed resolves identically everywhere: whatever was in before the Proceed stamp counts. Clovers are guessed one at a time, in deal order, on a big shared board. Drag cards **anywhere**: loose "maybe" placements stay exactly where you leave them for everyone, and a card only snaps when dropped close to a spot (pushing any card already there back to where yours came from). The card touched last sits on top. A card within reach of a spot counts for **Check**. **Guess** boots any wrongly placed cards back to the tray, with a small shake and no commentary. Right cards stay. There's no scoring and no limit: keep guessing until all four are right, and the clover glows.
While guessing, **tap the status line** ("Sam · 1/3 · 1 decoy ▾") to pick which clover everyone guesses: any order, jump ahead or back (solved ones show their answer). **Next clover** goes to the next unsolved one, and the round ends when all are solved.

5. **Next clover**, then **Next round**. Seats and scores carry across rounds. The ⋯ menu has the invite link, switch player, watch, *Deal new cards*, *New game* (reset scores, keep whoever is here), and *Remove* for away players.

**Turning the board** is local to each player: tap ⟳ or the clover's round centre, or drag around it. Tapping a leaf never turns the board: while writing, it opens that leaf's editor upright on screen, and each leaf shows its two keywords ("ocean & storm"). Cards on the board turn with it; cards in the tray stay put. Card turns, moves and board turns all animate. Each clover starts upright.

**No header bar.** The ⋯ menu button sits at the left of the bottom strip; players are listed by name (with what they're doing, and *Remove* for away players) in the menu. In the lobby, or whenever you're connected to nobody, the strip has an **Invite** button (the phone's share sheet, else copy link). The **tray** fits every card of the current clover without overlap, three to a row (a column in landscape) at your card size. A 5–6 card clover gets two rows at full size; only a clover that really has 7–8 cards takes a third row and shrinks its board to make room.

**The bottom bar is one thin strip**: a one-line status (whose clover, how many are in), the controls the moment needs and the main button; while writing, a thin coloured bar along its top fills as players submit. How-to tips show once per device (the next touch on the board dismisses them) and stay in the ⋯ menu. The menu also has **Full screen** where the browser supports it (Android, desktop); on iPhone, Add to Home Screen opens it full screen via the web manifest.

**Pinch zoom** is the board's own, per device: two fingers (or ctrl+wheel / a trackpad pinch) zoom and pan the board area up to 4×, while the bottom bar and menu stay normal size, so you can't get stranded zoomed in. While zoomed, one finger on a card still plays, and one finger anywhere else pans the view (the board only turns via ⟳ then; a tap is still a tap); **1×** (top-left, shown while zoomed) resets it. A second finger cancels whatever the first started (a dragged card goes back, nothing turns). The page itself doesn't pinch-zoom: zooming the whole page past the dock left the board filling the screen with no way back.

**Your tables and past games.** Opening the site without a room (no `#…`) shows the home page: *Start a table*, with a suggested table name (easy to type and unlikely to collide, like `brave-otter-42`; ↻ for another, or type your own, used as typed but link-safe: "Family Night" → `family-night`) and your name, pre-filled with the last one you used anywhere; Start takes you there already seated. A table link you haven't joined opens a join sheet with the table and your usual name filled in: one tap on Join (editing the table name there switches tables). Then the tables you've played on this device (newest first, with the name you used; **Forget** drops one, and the name saved for it, from this device; its past games stay), and *Past games*. Every finished round is kept in this device's localStorage with its clovers, answers included: review each as a small clover (clues around the four answer cards, decoys listed, guesses taken), *Play these again* as solo play, or delete it. The ⋯ menu has *Past games…* and *All my tables* too. Nothing leaves the device.

**Light and dark.** The theme follows the device's light/dark setting; the ⋯ menu's *Theme* button cycles auto → light → dark (kept per device). Light is a pale sage table with white cards; dark is near-neutral charcoal with muted leaves and slightly dimmed cards so they don't glare. Rust draws the board from a palette per theme; every text pair in both passes WCAG AA (native test and harness).

**Finding each other.** Players meet through public Nostr relays (six large, widely run ones rather than Trystero's hashed pick of hobby relays). While a device is connected to nobody, the status line says *Looking for the others…*, and it re-announces from scratch after 12 s (backing off to 30 s) and as soon as the page returns to the foreground, which is what a manual reload used to fix.

**Card size** is per device: − / + in the ⋯ menu (narrow screens start larger). Positions on the board are shared; bigger cards just overlap more on your screen. At the largest size, cards in neighbouring spots touch edge to edge. In the tray, cards never grow past a third of the tray's width, so a saved big size can't make them overlap. At that limit + greys out and the menu says to pinch the board for more; − always shrinks what you see (it steps from the drawn size, not a bigger saved one).

**Presence** follows the WebRTC connections, not a heartbeat, so a phone that locks its screen or a backgrounded tab still counts as at the table.

**Layout:** the board grows to fill the screen. While guessing, a tray for loose cards sits below it (portrait) or beside it (landscape). Tray positions are stored relative to the tray, so they fit every screen.

## Sharing boards

At the end of a round, **Share boards** (also in the ⋯ menu) lists the round's clovers. Pick any (one for a single board, or a set) and copy the link. Everything travels in the link (`?b=`), so no server or file is involved. The boards are deflate-compressed before encryption: one clover is about 330 characters including the address, and four about 800. Links don't give answers away by accident: the Rust core scrambles each board (cards shuffled and randomly turned, the answer a separate card-and-turn line), and the JS shell only compresses that text and AES-GCM encrypts it under a key derived from the link's `#` fragment. (The device must decode it to play, so this guards against glances, not determined devtools digging.) Opening a share link is **solo play**: no room, no name, no connection to the old table or any new one. It goes straight into guessing the clovers in order under their writers' names, and Guess checks against the real answers. At the end, **Start your own table** opens a fresh room. Older links (JSON payloads) still open. The link includes the answers, so it's meant for people who weren't there.

## Importing an unfinished clover

Boards from elsewhere (e.g. read off a screenshot of an old game) become ordinary share links. Write the board as text: name, the four clues (top, right, bottom, left), then the answer cards for spots top-left, top-right, bottom-right and bottom-left (words clockwise from the top as they sit), then any decoys. Then call `window.__cl.shareFromText([text])` on any open page to get the link.

## Harness

```bash
(cd core && cargo test --release)   # 12 native properties, incl. 300-seed convergence fuzz
node harness/run.mjs                # 23 browser gates + metrics → harness/out/scorecard.json
```

The browser harness drives the real UI through a whole game: 5 players (one phone-sized), a watcher, a seat takeover from a tablet, two rounds, and a New game. Latest scorecard (headless Chromium, WebRTC over loopback):

| metric | value |
|---|---|
| gates | 46/46 |
| every deck word fits its card edge at full size | widest: camouflage 141 of 146 units |
| worst text contrast (canvas + page) | 6.5 : 1 (WCAG AA ≥ 4.5) |
| presses to join / write a clover + Submit | 1 / 6 |
| last Submit → guessing starts | ~60 ms |
| card drop visible on 6 other screens (p50 / p95) | ~18 / ~33 ms |
| late visitor gets full state | ~300 ms |
| frame CPU, wasm + Canvas2D (p95) | ~1.3 ms |
| card word size while guessing, desktop / phone (default · largest) | 23.6 / 15.6 · 18.4 px |
| board width on a 390 px phone | 96% of screen |
| wasm size / JS imports | 220 KB / 0 |

Bugs the harness caught this iteration: dropping onto an occupied spot stacked cards without snapping, so turns went to the wrong card. A snapped card drew *under* a loose one, so taps hit the loose card. And a CSS `display` rule overrode `hidden`, leaving the decoy stepper showing during guessing (found by looking at screenshots, not by a gate). Adding the card-size control then surfaced two flaky failures. Repeating runs turned them into real bugs. Presence was a heartbeat sent from the render loop, so a throttled background tab looked like a player who had left. And any device, even a watcher, could auto-start guessing based on its own partial view of who was connected. Presence is now connection-based, and auto-start waits for every seated player.

## Look and feel

The mark is four rounded cards set as a clover, one of them gold (the decoy). The palette is green felt, paper cards and that one warm gold accent. The board is a four-leaf clover: a heart-shaped leaf on each side carries its clue in Fraunces, and the card spots sit in the round centre. Card words are in Figtree. Both fonts ship with the site (`web/fonts`), so there are no third-party requests. Cards are 210 board units against a 26-unit word size, with each word set 86 units in from the centre, so even the deck's longest word can't reach the corners where the neighbouring words sit (gated against the whole deck in the real font). Motion is understated: cards deal in from the centre with a short stagger, and turns, moves and board spins ease. The leaves glow softly when a clover is finished, sheets slide up, and buttons give a little on press. All of it is switched off under `prefers-reduced-motion`. Text can't be selected outside inputs, so a long press on mobile doesn't open dictionary lookups. Every canvas text colour pair is checked for WCAG AA contrast in both test suites.
