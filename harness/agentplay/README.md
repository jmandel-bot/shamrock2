# Agent play test

`driver.mjs` opens one real browser page per player at the same table and gives
each a local HTTP API. Game moves go through the page's own agent tools
(`web/agent.js`, the same ones offered to browser assistants over WebMCP),
which drive the same Rust input path as touch; the main buttons are clicked
as a person would.

    node harness/agentplay/driver.mjs <room> <players> [apiBasePort=9111]

Player k listens on `apiBasePort + k`. Logs and screenshots go to `out/`.

| Call | What it does |
|---|---|
| `GET /state[?wait=N&since=ETAG]` | The `get_table` tool's answer (phase, status, players, and the current clover: clues by leaf, each card's location — `tray`, `slot N`, or `board` when set down outside a spot — and words by edge as they face now, solved spots, and while writing the two words each leaf links), plus the main and other buttons. `wait` holds up to N s until something changes. |
| `GET /tools` | The agent tools' names, descriptions and input schemas. |
| `POST /join {name}` | Join the table. |
| `POST /go` | Press the main button (Start, Submit, Guess, Next clover…). |
| `POST /other` | Press the secondary button (e.g. Proceed). |
| `POST /clue {leaf, text}` | Write a leaf's clue (top/right/bottom/left). |
| `POST /decoys {n}` | Set the decoy count. |
| `POST /move {card, slot}` | Move a card to spot 0–3 (top-left, top-right, bottom-right, bottom-left) or `"tray"`. |
| `POST /turn {card, times}` | Turn a card a quarter clockwise, `times` times. |
| `POST /pick {clover}` | Choose which clover everyone guesses. |
| `POST /recap` | After the round: every clover's clues, answers, decoys and guesses. |
| `POST /tool/<name> {args}` | Call any agent tool by name (see `web/agent.js`). |
| `POST /screenshot` | Save a screenshot; returns its path. |

`node harness/agentplay/tools-test.mjs` plays a whole three-player round through the tools alone and checks the recap.

## Real models over MCP

`mcp-bridge.mjs` is a small MCP server that offers one player's page tools
(exactly as the page defines them) to an MCP client: what a browser assistant
gets over WebMCP. Two more tools stand in for what people have around a real
table: `wait_for_change`, a long-poll, and `say`, table talk (what players
would say on the call). `wait_for_change` and `get_table` return what others
said since the player last looked. `mcp-game.mjs` starts the driver and one
`claude -p` session per player, each allowed only those tools (no shell, no
files, no tactics guide: the rules come from the tool descriptions), then
prints the clovers, guesses, tool calls and tool errors. Players play as a real
table does: no turn order and nobody in charge; anyone moves any card and
anyone may check, ideally after agreeing; a clover's writer gives no hints.
Transcripts, each player's report, and `events.jsonl` (every line of talk and
every action, in order) go to `out/`. It spends model time, so it isn't part
of `run.mjs`.

    node harness/agentplay/mcp-game.mjs [players=3] [model=opus] [apiBasePort=9181]
    node harness/agentplay/mcp-game.mjs Fable:claude-fable-5-1,Opus:claude-opus-5-5,…   # a mixed table
