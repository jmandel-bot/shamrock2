// Tools for AI assistants (WebMCP): the same moves a player makes, offered to
// a browser's built-in agent when the browser supports it, and to the agent
// play-test driver (harness/agentplay) through window.__cl.tools. Each tool
// acts through the same Rust entry points as touch input, and only sees what
// this player's own screen shows (answers stay hidden until a round ends).

const LEAVES = ['top', 'right', 'bottom', 'left'];
const RULES = 'Shamrock is a cooperative word game (So Clover! rules). Each player gets a clover: four cards in four spots '
  + '(0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left), each card with a word on every edge. The top clue links the top '
  + 'words of spots 0 and 1, right links the right words of spots 1 and 2, bottom links the bottom words of spots 2 and 3, '
  + 'left links the left words of spots 3 and 0. Everyone writes four clues for their own clover (traditionally one word, '
  + 'lowercase unless a name or title, linking both words), then the table guesses each clover in turn: its cards are shuffled '
  + 'and turned in a tray with decoy cards mixed in, and the guessers place and turn them, then check. Right spots lock; wrong '
  + 'cards bounce back. Card numbers are tray labels only and say nothing about the answer.';

/** The tool list. `g` is the page's game handle (see main.js). */
export function agentTools(g) {
  const view = () => g.agentView();
  const done = (result, ok = true) => ({ ok, result });
  const labelToIndex = (label) => view().cards.find((c) => c.card === label)?._i;
  const settle = () => g.X.settle();
  const point = (fn, ...a) => g.probe(fn, ...a).slice(0, 2);
  // A gesture into the Rust input path, in canvas coordinates.
  function drag(from, to) {
    g.X.pointer_down(...from);
    for (let s = 1; s <= 12; s++) g.X.pointer_move(from[0] + (to[0] - from[0]) * s / 12, from[1] + (to[1] - from[1]) * s / 12);
    g.X.pointer_up(...to);
    g.sync();
  }
  const card = { type: 'integer', minimum: 0, description: 'Card number as listed by get_table (a tray label)' };

  return [
    {
      name: 'get_table',
      description: `Read the table: phase (lobby, writing, guessing, end), players, and the clover on screen with its clues and cards. ${RULES}`,
      annotations: { readOnlyHint: true },
      inputSchema: { type: 'object', properties: {} },
      run() {
        const u = g.ui(), v = view();
        return {
          phase: u.phase, round: u.round, you: u.name ?? null, seated: u.seated,
          players: u.players.map((p) => ({ name: p.name, status: p.status, away: p.away, you: p.you })),
          status: g.status(),
          decoys: u.decoys, readyCount: u.readyCount,
          clover: {
            index: u.k, count: u.n, writer: u.writerName ?? null, isMine: !!u.isWriter, solved: !!u.done,
            clues: Object.fromEntries(LEAVES.map((l, d) => [l, v.clues[d] || null])),
            // While writing your own clover: the two words each leaf must link.
            leafWords: Object.fromEntries(LEAVES.map((l, d) => [l, v.leafWords[d]])),
            solvedSpots: v.solvedSlots,
            // words: [top, right, bottom, left] as the card is turned now. at:
            // "tray", "slot N", or "board" (set down outside any spot).
            cards: v.cards.map(({ _i, ...c }) => c),
          },
        };
      },
    },
    {
      name: 'join_table',
      description: 'Take a seat at this table under a name (or rejoin as yourself).',
      inputSchema: { type: 'object', properties: { name: { type: 'string', maxLength: 24 } }, required: ['name'] },
      run({ name }) { g.joinAs(String(name ?? '')); return done(g.ui().seated ? `seated as ${g.ui().name}` : 'could not join', g.ui().seated); },
    },
    {
      name: 'start_round',
      description: 'Deal a new round to everyone seated (from the lobby, or after a round ends).',
      inputSchema: { type: 'object', properties: {} },
      run() {
        const u = g.ui();
        if (!['lobby', 'end'].includes(u.phase)) return done(`a round is already under way (${u.phase})`, false);
        if (!u.dealable.length) return done('nobody is seated yet', false);
        g.act(1);
        return done(`dealt to ${u.dealable.join(', ')}`);
      },
    },
    {
      name: 'write_clue',
      description: 'Write the clue on one leaf of your own clover (writing phase). It should link both words that leaf faces (see leafWords in get_table).',
      inputSchema: { type: 'object', properties: { leaf: { enum: LEAVES }, text: { type: 'string', maxLength: 40 } }, required: ['leaf', 'text'] },
      run({ leaf, text }) {
        const d = LEAVES.indexOf(leaf);
        if (d < 0) return done('leaf must be top, right, bottom or left', false);
        if (g.ui().phase !== 'writing') return done('clues are written in the writing phase', false);
        g.X.set_clue(d, g.writeText(String(text ?? '')), 0);
        g.sync();
        const now = view().clues[d];
        return done(`${leaf} clue is now "${now}"`, now === String(text ?? '').trim());
      },
    },
    {
      name: 'set_decoys',
      description: 'Choose how many decoy cards (0-4) get mixed in when others guess your clover.',
      inputSchema: { type: 'object', properties: { count: { type: 'integer', minimum: 0, maximum: 4 } }, required: ['count'] },
      run({ count }) { g.X.set_decoys(Math.max(0, Math.min(4, count | 0))); g.sync(); return done(`decoys: ${g.ui().decoys}`); },
    },
    {
      name: 'submit_clover',
      description: 'Submit your four clues (writing phase). Guessing starts once everyone has submitted.',
      inputSchema: { type: 'object', properties: {} },
      run() {
        const u = g.ui();
        if (u.ready) return done('already submitted');
        if (!u.canReady) return done('write all four clues first', false);
        g.act(2);
        return done('submitted');
      },
    },
    {
      name: 'place_card',
      description: 'Move a card into a spot (0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left) or back to the tray, while guessing.',
      inputSchema: { type: 'object', properties: { card, spot: { oneOf: [{ type: 'integer', minimum: 0, maximum: 3 }, { const: 'tray' }] } }, required: ['card', 'spot'] },
      run({ card: label, spot }) {
        const i = labelToIndex(label);
        if (i === undefined) return done(`no card ${label}`, false);
        settle();
        const to = spot === 'tray' ? g.trayPoint() : point(g.X.probe_slot, spot);
        drag(point(g.X.probe_card, i), to);
        const at = view().cards.find((c) => c._i === i)?.at;
        return done(`card ${label} is at ${at}`, spot === 'tray' ? at === 'tray' : at === `slot ${spot}`);
      },
    },
    {
      name: 'turn_card',
      description: 'Turn a card a quarter clockwise, `times` times (its new top word is its old left word).',
      inputSchema: { type: 'object', properties: { card, times: { type: 'integer', minimum: 1, maximum: 3, default: 1 } }, required: ['card'] },
      run({ card: label, times = 1 }) {
        const i = labelToIndex(label);
        if (i === undefined) return done(`no card ${label}`, false);
        for (let t = 0; t < times; t++) { settle(); const p = point(g.X.probe_card, i); g.X.pointer_down(...p); g.X.pointer_up(...p); g.sync(); }
        return done(`card ${label} reads top/right/bottom/left: ${view().cards.find((c) => c._i === i)?.words.join(' / ')}`);
      },
    },
    {
      name: 'check_guess',
      description: 'Check the cards in the spots. Right spots lock; wrong cards go back to the tray.',
      inputSchema: { type: 'object', properties: {} },
      run() {
        const u = g.ui();
        if (!u.canCheck) return done(u.phase === 'guessing' ? 'fill all four spots first (and it cannot be your own clover)' : 'nothing to check', false);
        g.act(3);
        const v = view();
        return done(v.solvedSlots.length === 4 ? 'solved!' : `right so far: spots ${v.solvedSlots.join(', ') || 'none'}`);
      },
    },
    {
      name: 'next_clover',
      description: 'Once the clover on screen is solved, move on to the next one (or finish the round after the last).',
      inputSchema: { type: 'object', properties: {} },
      run() { if (!g.ui().done) return done('this clover is not solved yet', false); g.act(4); return done(`now: ${g.ui().phase}`); },
    },
    {
      name: 'choose_clover',
      description: 'While guessing, switch the table to another clover (numbered from 0 in the order they were dealt).',
      inputSchema: { type: 'object', properties: { clover: { type: 'integer', minimum: 0 } }, required: ['clover'] },
      run({ clover }) { g.X.jump(clover | 0); g.sync(); return done(`clover ${g.ui().k} of ${g.ui().n}`, g.ui().k === (clover | 0)); },
    },
    {
      name: 'get_recap',
      description: 'After a round ends: every clover with its clues, answer cards in spot order, decoys and how many guesses it took.',
      annotations: { readOnlyHint: true },
      inputSchema: { type: 'object', properties: {} },
      run() {
        if (g.ui().phase !== 'end') return done('the recap is available once the round has ended', false);
        return g.review().map((b) => {
          const [writer, ...rest] = b.text.split('\n');
          const cards = rest.slice(4).map((l) => l.split('|'));
          return { writer, clues: Object.fromEntries(LEAVES.map((l, d) => [l, rest[d]])), answer: cards.slice(0, 4), decoys: cards.slice(4), guesses: b.guesses, solved: b.solved };
        });
      },
    },
  ];
}

/** Offer the tools to the browser's agent, if it has one (WebMCP). The API
 *  is still settling: `document.modelContext` is current, with the earlier
 *  `navigator.modelContext` as a fallback. Does nothing elsewhere. */
export function offerTools(tools) {
  const mc = document.modelContext ?? navigator.modelContext;
  if (typeof mc?.registerTool !== 'function') return false;
  for (const t of tools) {
    const { run, ...spec } = t;
    try {
      Promise.resolve(mc.registerTool({
        ...spec,
        async execute(args) { return { content: [{ type: 'text', text: JSON.stringify(await run(args ?? {})) }] }; },
      })).catch(() => {});
    } catch {}
  }
  return true;
}
