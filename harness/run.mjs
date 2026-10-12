// Browser harness: real Chromium tabs, real WebRTC data channels (signaled via
// a local relay), real pointer and keyboard events, five players playing a
// whole game. Prints a scorecard and writes harness/out/scorecard.json +
// screenshots. Exit code 1 if any gate fails.
import { chromium } from 'playwright';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { serve } from './serve.mjs';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const OUT = here('./out/');
const PORT = 8797, RELAY = 8798;
const room = `h${Date.now().toString(36)}`;
const url = `http://localhost:${PORT}/?signal=ws://localhost:${RELAY}#${room}`;

const metrics = {};
const gates = [];
const gate = (name, ok, detail = '') => {
  gates.push({ name, ok: !!ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
};
const metric = (name, value, unit = '') => {
  metrics[name] = value;
  console.log(`      ${name} = ${typeof value === 'number' ? +value.toFixed(2) : value}${unit}`);
};
const pct = (xs, p) => { const s = [...xs].sort((a, b) => a - b); return s[Math.min(s.length - 1, Math.floor(p * s.length))]; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const near = (a, b, tol = 1.5) => Math.abs(a[0] - b[0]) < tol && Math.abs(a[1] - b[1]) < tol;

await mkdir(OUT, { recursive: true });
const server = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM ?? undefined,
  args: ['--disable-features=WebRtcHideLocalIpsWithMdns'],
});
const errors = [];

async function player(name, viewport = { width: 1280, height: 860 }) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${name}: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${name}: ${m.text()}`));
  await page.goto(url);
  await page.waitForFunction(() => window.__cl);
  const P = {
    name, page, ctx,
    me: await page.evaluate(() => window.__cl.me),
    ev: (fn, arg) => page.evaluate(fn, arg),
    digest: () => page.evaluate(() => window.__cl.digest()),
    ui: () => page.evaluate(() => window.__cl.ui()),
    card: (i) => page.evaluate((i) => window.__cl.card(i), i),
    slot: (s) => page.evaluate((s) => window.__cl.slot(s), s),
    petal: (d) => page.evaluate((d) => window.__cl.petal(d), d),
    hub: () => page.evaluate(() => window.__cl.hub()),
    quarter: () => page.evaluate(() => window.__cl.quarter()),
    settle: () => page.evaluate(() => window.__cl.settle()),
    presses: () => page.evaluate(() => window.__cl.stats.presses),
    async origin() {
      return page.evaluate(() => { const r = document.getElementById('c').getBoundingClientRect(); return [r.left, r.top]; });
    },
    async drag(from, to, { steps = 12 } = {}) {
      const o = await P.origin(), m = page.mouse;
      await m.move(o[0] + from[0], o[1] + from[1]);
      await m.down();
      for (let k = 1; k <= steps; k++) {
        const t = k / steps;
        await m.move(o[0] + from[0] + (to[0] - from[0]) * t, o[1] + from[1] + (to[1] - from[1]) * t);
      }
      await m.up();
    },
    async tap(at) {
      const o = await P.origin();
      await page.mouse.click(o[0] + at[0], o[1] + at[1]);
    },
  };
  return P;
}

/** Wait until `pred(digest)` holds on every page; returns ms elapsed. */
async function until(players, pred, timeout = 8000) {
  const t0 = performance.now();
  await Promise.all(players.map((p) => p.page.waitForFunction(
    ([src]) => new Function('d', `return (${src})(d)`)(window.__cl.digest()), [pred.toString()], { timeout, polling: 'raf' })));
  return performance.now() - t0;
}
const same = async (players) => {
  const ds = await Promise.all(players.map((p) => p.digest()));
  return ds.every((d) => JSON.stringify(d) === JSON.stringify(ds[0]));
};
const cur = (d) => d.puzzles[+d.phase.split(':')[1]];

/** A guesser places card s in slot s and turns it right (or a decoy into slot 0). */
async function solve(p, { decoyInSlot0 = false } = {}) {
  await p.settle();
  for (let s = 0; s < 4; s++) {
    const d = cur(await p.digest());
    if (d.mask & (1 << s)) continue;
    const card = decoyInSlot0 && s === 0 ? 4 : s;
    await p.drag((await p.card(card)).slice(0, 2), (await p.slot(s)).slice(0, 2));
    if (card !== s) continue;
    const want = await p.ev((s) => window.__cl.answerRot(s), s);
    for (let k = 0; k < 4; k++) {
      const c = cur(await p.digest()).cards[s];
      if (c[0] === 'slot' && c[2] === want) break;
      await p.tap((await p.card(s)).slice(0, 2));
    }
  }
}

try {
  // ---- static properties -------------------------------------------------------
  console.log('\n# build');
  const wasm = await readFile(here('../web/core.wasm'));
  const mod = await WebAssembly.compile(wasm);
  metric('wasm_bytes', wasm.length, ' B');
  const shim = (await readFile(here('../web/main.js'), 'utf8')).split('\n').filter((l) => l.trim() && !l.trim().startsWith('//')).length;
  metric('js_shim_loc', shim);
  gate('wasm has zero JS imports (no bindgen glue)', WebAssembly.Module.imports(mod).length === 0);

  // ---- connect ------------------------------------------------------------------
  console.log('\n# connect (5 players, WebRTC mesh)');
  let t0 = performance.now();
  const players = await Promise.all([
    ...['A', 'B', 'C', 'D'].map((n) => player(n)),
    player('E-phone', { width: 390, height: 844 }),
  ]);
  const [A, B, C, D, E] = players;
  await Promise.all(players.map((p) => p.page.waitForFunction(() => window.__cl.ui().players.length === 5, null, { timeout: 30000 })));
  metric('connect_5_players_ms', performance.now() - t0, ' ms');
  gate('5 players see each other in the roster', true);

  // ---- writing ----------------------------------------------------------------------
  console.log('\n# writing (everyone at once)');
  await A.page.click('#go');
  await until(players, (d) => d.phase === 'writing');
  gate('one press starts the game for everyone', true);
  await A.page.screenshot({ path: `${OUT}1-writing.png` });

  const pressCounts = [];
  const words = ['ocean', 'storm', 'royal', 'kitchen'];
  let lastSubmit = 0;
  await Promise.all(players.map(async (p, k) => {
    await p.settle();
    const p0 = await p.presses();
    await p.tap((await p.petal(0)).slice(0, 2));
    for (const w of words) {
      await p.page.waitForSelector('#clue', { state: 'visible' });
      await p.page.keyboard.type(`${w}${k}`);
      await p.page.keyboard.press('Enter');
    }
    await p.page.waitForFunction(() => !document.getElementById('go').disabled);
    await p.page.click('#go');
    lastSubmit = performance.now();
    pressCounts.push((await p.presses()) - p0);
  }));
  metric('presses_to_write_and_submit_max', Math.max(...pressCounts));
  gate('writing a clover takes ≤ 6 presses (tap leaf, 4× Enter, Submit)', Math.max(...pressCounts) <= 6, pressCounts.join(','));
  const toGuess = await until(players, (d) => d.phase === 'guessing:0', 10000).then(() => performance.now() - lastSubmit);
  metric('last_submit_to_guessing_ms', toGuess, ' ms');
  gate('guessing starts by itself once everyone submitted', true);
  gate('all 5 clovers queued for guessing', (await A.digest()).puzzles.length === 5);
  const subs = (await A.digest()).subs;
  gate('clues typed via Enter-to-next-leaf arrive intact', subs.every((s) => s.slice(2).every((c, d) => c.startsWith(words[d]))));

  // ---- guessing clover 0 -------------------------------------------------------------
  console.log('\n# guessing');
  const byId = (id) => players.find((p) => p.me === id);
  const writerOf = async () => byId(cur(await A.digest()).writer);
  let W = await writerOf();
  let guessers = players.filter((p) => p !== W);
  const [G, H] = guessers.filter((p) => p !== E);

  const wd = JSON.stringify(await W.digest());
  await W.settle();
  await W.drag((await W.card(0)).slice(0, 2), (await W.slot(0)).slice(0, 2));
  gate("the clover's writer cannot move cards", JSON.stringify(await W.digest()) === wd);

  // Live sync of a guesser's drop.
  const lat = [];
  for (let k = 0; k < 6; k++) {
    await G.settle();
    const s = k % 4;
    await G.drag((await G.card(0)).slice(0, 2), (await G.slot(s)).slice(0, 2));
    const t = performance.now();
    await until(players.filter((p) => p !== G), new Function('d', `const c=d.puzzles[0].cards[0];return c[0]==='slot'&&c[1]===${s}&&c[3]===0`));
    lat.push(performance.now() - t);
  }
  metric('sync_latency_p50_ms', pct(lat, 0.5), ' ms');
  metric('sync_latency_p95_ms', pct(lat, 0.95), ' ms');
  gate('a drop shows up for all 4 others, p95 < 150 ms', pct(lat, 0.95) < 150);

  // Turning the board: tap the hub. Local only; slotted cards turn, table cards stay.
  await G.settle();
  const n = cur(await G.digest()).cards.length;
  const before = await Promise.all([...Array(n).keys()].map((i) => G.card(i)));
  const otherBefore = await Promise.all([...Array(n).keys()].map((i) => H.card(i)));
  const dBefore = JSON.stringify(await G.digest());
  gate('the board starts each clover upright', (await G.quarter()) % 4 === 0, `q=${await G.quarter()}`);
  const q0 = await G.quarter();
  const [cx, cy] = await G.hub();
  await G.tap([cx, cy]);
  await G.settle();
  const after = await Promise.all([...Array(n).keys()].map((i) => G.card(i)));
  const d0 = await G.digest();
  let boardOk = true, tableOk = true;
  cur(d0).cards.forEach((c, i) => {
    const [x, y] = before[i];
    if (c[0] === 'slot') boardOk &&= near(after[i], [cx - (y - cy), cy + (x - cx)]);
    else tableOk &&= near(after[i], before[i]);
  });
  gate('tapping ⟳ on the board turns my view a quarter', (await G.quarter()) === q0 + 1);
  gate('cards on the board turn with it', boardOk);
  gate('cards off the board stay put', tableOk);
  await sleep(150);
  const otherAfter = await Promise.all([...Array(n).keys()].map((i) => H.card(i)));
  gate("turning my board doesn't move anything on other screens", otherBefore.every((p, i) => near(p, otherAfter[i])));
  gate('board rotation is not shared state', JSON.stringify(await H.digest()) === dBefore);
  await G.page.screenshot({ path: `${OUT}2-guessing-turned.png` });

  // A drop in my turned view lands in the right board slot for everyone.
  const slotsG = await Promise.all([0, 1, 2, 3].map((s) => G.slot(s)));
  const topLeft = slotsG.map((p, s) => [p[0] + p[1], s]).sort((a, b) => a[0] - b[0])[0][1];
  await G.drag((await G.card(2)).slice(0, 2), slotsG[topLeft].slice(0, 2));
  await until(players, new Function('d', `const c=d.puzzles[0].cards[2];return c[0]==='slot'&&c[1]===${topLeft}`));
  gate('a drop in a turned view lands in the right slot for all', topLeft === (((-(q0 + 1)) % 4) + 4) % 4, `top-left = slot ${topLeft}`);

  // Two guessers drop different cards on one slot at the same moment.
  await H.settle();
  const [fg, fh] = [(await G.card(1)).slice(0, 2), (await H.card(3)).slice(0, 2)];
  const [tg, th] = [(await G.slot(1)).slice(0, 2), (await H.slot(1)).slice(0, 2)];
  await Promise.all([G.drag(fg, tg), H.drag(fh, th)]);
  await sleep(400);
  const dc = cur(await A.digest());
  gate('simultaneous drops on one slot converge to one winner', (await same(players)) && dc.cards.filter((c) => c[0] === 'slot' && c[1] === 1).length === 1);

  // First try with a decoy in slot 0, then fix it.
  await solve(G, { decoyInSlot0: true });
  await G.page.waitForFunction(() => !document.getElementById('go').disabled);
  await G.page.click('#go');
  await until(players, (d) => d.puzzles[0].attempts === 1);
  const c1 = cur(await B.digest());
  gate('check 1: 3 right are locked, the wrong card goes back', c1.mask === 0b1110 && c1.cards[4][0] === 'table', `mask ${c1.mask}`);
  await solve(H);
  await H.page.waitForFunction(() => !document.getElementById('go').disabled);
  await H.page.click('#go');
  await until(players, (d) => d.puzzles[0].attempts === 2);
  gate('check 2 completes the clover for everyone (+4)', (await A.ui()).players.find((p) => p.id === W.me).score === 4);
  await G.page.screenshot({ path: `${OUT}3-clover-done.png` });

  // ---- remaining clovers: first-try perfect ------------------------------------------
  for (let k = 1; k < 5; k++) {
    await guessers[0].page.click('#go'); // Next clover
    await until(players, new Function('d', `return d.phase==='guessing:${k}'`));
    W = await writerOf();
    const g = players.find((p) => p !== W && p !== E);
    await solve(g);
    await g.page.waitForFunction(() => !document.getElementById('go').disabled);
    await g.page.click('#go');
    await until(players, new Function('d', `return d.puzzles[${k}].attempts===1`));
    guessers = players.filter((p) => p !== W);
  }
  await E.page.screenshot({ path: `${OUT}4-phone-guessing.png` });
  await A.page.click('#go'); // See results
  await until(players, (d) => d.phase === 'end');
  const total = (await A.digest()).score;
  gate('final score agrees everywhere (4 + 4×6 = 28)', total === 28 && (await same(players)), `score ${total}`);
  await A.page.screenshot({ path: `${OUT}5-end.png` });

  // ---- late join / leave ---------------------------------------------------------------
  console.log('\n# late join / leave');
  t0 = performance.now();
  const F = await player('F');
  const target = JSON.stringify(await A.digest());
  await F.page.waitForFunction((t) => JSON.stringify(window.__cl.digest()) === t, target, { timeout: 30000 });
  metric('late_join_state_ms', performance.now() - t0, ' ms');
  gate('late joiner receives the full game state', true);
  await F.ctx.close();
  await D.ctx.close();
  let leaveOk = true;
  try { await A.page.waitForFunction(() => window.__cl.ui().players.filter((p) => !p.away).length === 4, null, { timeout: 15000 }); } catch { leaveOk = false; }
  gate('players who leave are shown as away', leaveOk);
  await A.page.click('#go'); // Play again
  await until([A, B, C, E], (d) => d.phase === 'writing' && d.subs.length === 0);
  gate('Play again starts a fresh round for everyone', true);

  // ---- perf + legibility ----------------------------------------------------------------
  console.log('\n# perf + legibility');
  await sleep(1500);
  const fms = await A.ev(() => window.__cl.stats.frameMs.slice(-300));
  metric('frame_cpu_p50_ms', pct(fms, 0.5), ' ms');
  metric('frame_cpu_p95_ms', pct(fms, 0.95), ' ms');
  gate('frame CPU (wasm + paint) p95 < 4 ms', pct(fms, 0.95) < 4);
  const desk = await A.ev(() => window.__cl.wordPx());
  const ph = await E.ev(() => window.__cl.wordPx());
  metric('word_px_desktop_1280x860', desk, ' px');
  metric('word_px_phone_390x844', ph, ' px');
  metric('card_px_phone_390x844', await E.ev(() => window.__cl.cardPx()), ' px');
  gate('card words legible on desktop (>= 14 px)', desk >= 14);
  gate('card words legible on phone (>= 10 px)', ph >= 10);
  gate('cards are touch-sized on phone (>= 44 px)', (await E.ev(() => window.__cl.cardPx())) >= 44);
  gate('no page errors', errors.length === 0, errors.slice(0, 3).join(' | '));
} catch (e) {
  gate('harness completed', false, e.stack?.split('\n').slice(0, 3).join(' '));
  // Leave evidence for whoever (or whatever) debugs this next.
  for (const ctx of browser.contexts()) {
    for (const [k, pg] of ctx.pages().entries()) {
      const st = await pg.evaluate(() => ({ ui: window.__cl?.ui(), clue: document.getElementById('clue').style.display, active: document.activeElement?.id })).catch((x) => String(x));
      console.log('  state', JSON.stringify(st).slice(0, 400));
      await pg.screenshot({ path: `${OUT}fail-${browser.contexts().indexOf(ctx)}-${k}.png` }).catch(() => {});
    }
  }
} finally {
  await browser.close();
  await server.close();
}

const passed = gates.filter((g) => g.ok).length;
console.log(`\n${passed}/${gates.length} gates passed`);
await writeFile(`${OUT}scorecard.json`, JSON.stringify({ at: new Date().toISOString(), passed, total: gates.length, gates, metrics }, null, 2));
process.exit(passed === gates.length ? 0 : 1);
