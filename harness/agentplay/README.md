# Agent play test

`driver.mjs` opens one real browser page per player at the same table and gives
each a local HTTP API. Every action goes through the real interface (tapping
leaves, typing, dragging and tapping cards, pressing buttons), so automated
players (e.g. LLM agents) exercise the game the way people do.

    node harness/agentplay/driver.mjs <room> <players> [apiBasePort=9111]

Player k listens on `apiBasePort + k`. Logs and screenshots go to `out/`.

| Call | What it does |
|---|---|
| `GET /state[?wait=N&since=ETAG]` | Phase, status line, buttons, players, and the current clover: clues (top, right, bottom, left), each card's location and words as they face now, solved spots, and while writing the two words each leaf links. `wait` holds up to N s until something changes. |
| `POST /join {name}` | Join the table. |
| `POST /go` | Press the main button (Start, Submit, Guess, Next clover…). |
| `POST /other` | Press the secondary button (e.g. Proceed). |
| `POST /clue {leaf, text}` | Tap a leaf (top/right/bottom/left) and type its clue. |
| `POST /decoys {n}` | Set the decoy count. |
| `POST /move {card, slot}` | Drag a card to spot 0–3 (top-left, top-right, bottom-right, bottom-left) or `"tray"`. |
| `POST /turn {card, times}` | Tap a card to turn it a quarter clockwise, `times` times. |
| `POST /pick {clover}` | Choose which clover everyone guesses. |
| `POST /screenshot` | Save a screenshot; returns its path. |
