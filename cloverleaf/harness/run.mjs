// Browser harness: real Chromium tabs, real WebRTC data channels (signaled via
// a local relay), real pointer events. Prints a scorecard and writes
// harness/out/scorecard.json + screenshots. Exit code 1 if any gate fails.
import { chromium } from 'playwright';
import { mkdir, readFile, writeFile, stat } from 'node:fs/promises';
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

await mkdir(OUT, { recursive: true });
const server = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM ?? undefined,
  args: ['--disable-features=WebRtcHideLocalIpsWithMdns'],
});
const errors = [];

async function player(name, viewport = { width: 1280, height: 800 }) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${name}: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${name}: ${m.text()}`));
  await page.goto(url);
  await page.waitForFunction(() => window.__cl);
  const origin = await page.evaluate(() => { const r = document.getElementById('c').getBoundingClientRect(); return [r.left, r.top]; });
  const P = {
    name, page, ctx,
    ev: (fn, arg) => page.evaluate(fn, arg),
    digest: () => page.evaluate(() => window.__cl.digest()),
    card: (i) => page.evaluate((i) => window.__cl.card(i), i),
    slot: (s) => page.evaluate((s) => window.__cl.slot(s), s),
    petal: (d) => page.evaluate((d) => window.__cl.petal(d), d),
    settle: () => page.evaluate(() => window.__cl.settle()),
    peers: () => page.evaluate(() => window.__cl.peers.size),
    async drag(from, to, { steps = 12, holdMidway } = {}) {
      const m = page.mouse;
      await m.move(origin[0] + from[0], origin[1] + from[1]);
      await m.down();
      for (let k = 1; k <= steps; k++) {
        const t = k / steps;
        await m.move(origin[0] + from[0] + (to[0] - from[0]) * t, origin[1] + from[1] + (to[1] - from[1]) * t);
        if (holdMidway && k === steps >> 1) await holdMidway();
      }
      await m.up();
    },
    async click(at) {
      await page.mouse.click(origin[0] + at[0], origin[1] + at[1]);
    },
  };
  return P;
}

/** Wait until `pred(digest)` holds on every given page; returns ms elapsed. */
async function until(players, pred, timeout = 5000) {
  const t0 = performance.now();
  await Promise.all(players.map((p) => p.page.waitForFunction(
    ([src]) => new Function('d', `return (${src})(d)`)(window.__cl.digest()), [pred.toString()], { timeout, polling: 'raf' })));
  return performance.now() - t0;
}
const same = async (players) => {
  const ds = await Promise.all(players.map((p) => p.digest()));
  return ds.every((d) => JSON.stringify(d) === JSON.stringify(ds[0]));
};
const near = (a, b, tol = 1.5) => Math.abs(a[0] - b[0]) < tol && Math.abs(a[1] - b[1]) < tol;

try {
  // ---- static properties -------------------------------------------------------
  console.log('\n# build');
  const wasm = await readFile(here('../web/core.wasm'));
  const mod = await WebAssembly.compile(wasm);
  metric('wasm_bytes', wasm.length, ' B');
  metric('wasm_imports', WebAssembly.Module.imports(mod).length);
  const shim = (await readFile(here('../web/main.js'), 'utf8')).split('\n').filter((l) => l.trim() && !l.trim().startsWith('//')).length;
  metric('js_shim_loc', shim);
  gate('wasm has zero JS imports (no bindgen glue)', WebAssembly.Module.imports(mod).length === 0);

  // ---- connect --------------------------------------------------------------------
  console.log('\n# connect (3 players, WebRTC)');
  let t0 = performance.now();
  const [A, B, C] = await Promise.all(['A', 'B', 'C'].map((n) => player(n)));
  await Promise.all([A, B, C].map((p) => p.page.waitForFunction(() => window.__cl.peers.size === 2, null, { timeout: 20000 })));
  metric('connect_3_players_ms', performance.now() - t0, ' ms');
  gate('all 3 players mesh-connected', true);
  gate('initial state identical', await same([A, B, C]));

  // ---- drag sync latency --------------------------------------------------------------
  console.log('\n# drag → remote sync');
  const lat = [];
  for (let k = 0; k < 10; k++) {
    const s = k % 4;
    const card = 0;
    const from = (await A.card(card)).slice(0, 2);
    const to = (await A.slot(s)).slice(0, 2);
    await A.drag(from, to);
    const t = performance.now();
    await until([B, C], new Function('d', `return d.cards[0][0]==='slot' && d.cards[0][1]===${s} && d.cards[0][3]===0`));
    lat.push(performance.now() - t);
  }
  metric('sync_latency_p50_ms', pct(lat, 0.5), ' ms');
  metric('sync_latency_p95_ms', pct(lat, 0.95), ' ms');
  gate('drop visible on peers p95 < 150 ms', pct(lat, 0.95) < 150);

  const before = await A.ev(() => ({ ...window.__cl.stats, frameMs: undefined }));
  let sawHeld = false;
  {
    const from = (await A.card(1)).slice(0, 2);
    await A.drag(from, [1000, 200], {
      steps: 30,
      holdMidway: async () => {
        await sleep(100);
        const d = await B.digest();
        sawHeld = d.cards[1][d.cards[1][0] === 'slot' ? 3 : 4] !== 0;
      },
    });
  }
  const after = await A.ev(() => ({ ...window.__cl.stats, frameMs: undefined }));
  gate('peers see a card mid-drag (held by someone)', sawHeld);
  metric('bytes_per_drag_packet', (after.sentBytes - before.sentBytes) / Math.max(1, after.sentPackets - before.sentPackets), ' B');
  await until([A, B, C], (d) => d.cards[1][0] === 'table' && d.cards[1][4] === 0);

  // ---- per-player rotation ---------------------------------------------------------
  console.log('\n# per-player board rotation');
  const shot0 = await B.page.screenshot({ path: `${OUT}B-before-rotate.png` });
  const dBefore = await B.digest();
  const scrB = await Promise.all([0, 1, 2, 3, 4].map((i) => B.card(i)));
  const scrA = await Promise.all([0, 1, 2, 3, 4].map((i) => A.card(i)));
  await B.page.click('#rotr');
  await B.settle();
  const scrB2 = await Promise.all([0, 1, 2, 3, 4].map((i) => B.card(i)));
  const scrA2 = await Promise.all([0, 1, 2, 3, 4].map((i) => A.card(i)));
  const [cx, cy] = await B.ev(() => { const b = window.__cl; const s = [0, 1, 2, 3].map(b.slot); return [s.reduce((a, p) => a + p[0], 0) / 4, s.reduce((a, p) => a + p[1], 0) / 4]; });
  let boardOk = true, tableOk = true;
  dBefore.cards.forEach((c, i) => {
    const [x, y] = scrB[i];
    if (c[0] === 'slot') boardOk &&= near(scrB2[i], [cx - (y - cy), cy + (x - cx)]);
    else tableOk &&= near(scrB2[i], scrB[i]);
  });
  gate('rotating my view turns on-board cards 90° about board centre', boardOk);
  gate('rotating my view leaves off-board cards where they were', tableOk);
  gate("rotating B's view does not move anything on A's screen", scrA.every((p, i) => near(p, scrA2[i])));
  await sleep(150);
  gate('view rotation is local (shared state unchanged everywhere)', JSON.stringify(await A.digest()) === JSON.stringify(dBefore) && await same([A, B, C]));
  await B.page.screenshot({ path: `${OUT}B-after-rotate.png` });

  // B drops a card onto the slot that is top-left on B's (rotated) screen.
  const slotsB = await Promise.all([0, 1, 2, 3].map((s) => B.slot(s)));
  const topLeft = slotsB.map((p, s) => [p[0] + p[1], s]).sort((a, b) => a[0] - b[0])[0][1];
  await B.drag((await B.card(2)).slice(0, 2), slotsB[topLeft].slice(0, 2));
  await until([A, C], new Function('d', `return d.cards[2][0]==='slot' && d.cards[2][1]===${topLeft}`));
  const onA = await A.card(2), slotsA = await Promise.all([0, 1, 2, 3].map((s) => A.slot(s)));
  gate('a drop in a rotated view lands in the right board slot for everyone', topLeft === 3 && near(onA, slotsA[3]), `B top-left = board slot ${topLeft}`);

  // ---- tap to rotate card ---------------------------------------------------------------
  const rotBefore = (await A.digest()).cards[3][3];
  await A.click((await A.card(3)).slice(0, 2));
  await until([B, C], new Function('d', `return d.cards[3][3]===${(rotBefore + 1) % 4}`));
  gate('tap rotates a card for everyone', true);

  // ---- concurrent conflict ------------------------------------------------------------------
  console.log('\n# conflicts');
  const [fa, fc] = [(await A.card(4)).slice(0, 2), (await C.card(3)).slice(0, 2)];
  const [ta, tc] = [(await A.slot(1)).slice(0, 2), (await C.slot(1)).slice(0, 2)];
  await Promise.all([A.drag(fa, ta), C.drag(fc, tc)]);
  await sleep(300);
  const conflictConverged = await same([A, B, C]);
  const dc = await A.digest();
  const inSlot1 = dc.cards.filter((c) => c[0] === 'slot' && c[1] === 1).length;
  gate('simultaneous drops on one slot converge, one winner', conflictConverged && inSlot1 === 1);

  // ---- clue editing ---------------------------------------------------------------------------
  console.log('\n# clues');
  await A.click((await A.petal(0)).slice(0, 2));
  const inputShown = await A.page.isVisible('#clue');
  await A.page.keyboard.type('fruit salad');
  await A.page.keyboard.press('Enter');
  const clueMs = await until([B, C], (d) => d.clues[0] === 'fruit salad');
  gate('tapping a petal opens an in-place editor', inputShown);
  gate('clue text syncs to peers', true);
  metric('clue_sync_ms', clueMs, ' ms');

  // ---- late joiner -----------------------------------------------------------------------------
  console.log('\n# late join / leave');
  t0 = performance.now();
  const D = await player('D');
  const target = JSON.stringify(await A.digest());
  await D.page.waitForFunction((t) => JSON.stringify(window.__cl.digest()) === t, target, { timeout: 20000 });
  metric('late_join_state_ms', performance.now() - t0, ' ms');
  gate('late joiner receives the full table state', true);
  await C.ctx.close();
  let leaveOk = true;
  try { await A.page.waitForFunction(() => window.__cl.peers.size === 2, null, { timeout: 15000 }); } catch { leaveOk = false; }
  gate('peer leave is noticed', leaveOk);

  // ---- answer flow ------------------------------------------------------------------------------
  await A.page.click('[data-act="1"]');
  await until([B, D], (d) => d.cards.every((c) => c[0] === 'table'));
  for (let s = 0; s < 4; s++) await A.drag((await A.card(s)).slice(0, 2), (await A.slot(s)).slice(0, 2));
  await A.page.click('[data-act="2"]');
  await until([B, D], (d) => d.answer);
  await A.page.click('[data-act="3"]');
  await until([B, D], (d) => d.cards.every((c) => c[0] === 'table'));
  await B.settle();
  // B places cards in its own rotated view; rotations are whatever the scramble left.
  for (let s = 0; s < 4; s++) await B.drag((await B.card(s)).slice(0, 2), (await B.slot(s)).slice(0, 2));
  const status = async () => (await B.page.textContent('#status')).trim();
  await B.page.click('[data-act="4"]');
  await B.page.waitForFunction(() => /correct/.test(document.getElementById('status').textContent), null, { timeout: 2000 }).catch(() => {});
  const firstCheck = await status();
  gate('check reports a score', /correct/.test(firstCheck), firstCheck);
  await B.page.screenshot({ path: `${OUT}B-check.png` });

  // ---- perf ----------------------------------------------------------------------------------------
  console.log('\n# perf + legibility');
  await sleep(1500);
  const fms = await A.ev(() => window.__cl.stats.frameMs.slice(-300));
  metric('frame_cpu_p50_ms', pct(fms, 0.5), ' ms');
  metric('frame_cpu_p95_ms', pct(fms, 0.95), ' ms');
  gate('frame CPU (wasm + paint) p95 < 4 ms', pct(fms, 0.95) < 4);

  const wordPx = (p) => p.ev(() => window.__cl.wordPx());
  const cardPx = (p) => p.ev(() => window.__cl.cardPx());
  const desk = await wordPx(A);
  metric('word_px_desktop_1280x800', desk, ' px');
  gate('card words legible on desktop (>= 12 px)', desk >= 12);
  const phone = await player('phone', { width: 390, height: 844 });
  const ph = await wordPx(phone);
  metric('word_px_phone_390x844', ph, ' px');
  metric('card_px_phone_390x844', await cardPx(phone), ' px');
  gate('card words legible on phone (>= 10 px)', ph >= 10);
  gate('cards are touch-sized on phone (>= 44 px)', (await cardPx(phone)) >= 44);
  // The phone player is in the same room: its portrait view must not disturb shared state.
  await phone.page.waitForFunction((t) => JSON.stringify(window.__cl.digest()) === t, JSON.stringify(await A.digest()), { timeout: 10000 })
    .then(() => gate('portrait player converges with everyone else', true), () => gate('portrait player converges with everyone else', false));
  await phone.page.screenshot({ path: `${OUT}phone.png` });
  await A.page.screenshot({ path: `${OUT}A-final.png` });

  gate('no page errors', errors.length === 0, errors.slice(0, 3).join(' | '));
} catch (e) {
  gate('harness completed', false, e.stack?.split('\n').slice(0, 3).join(' '));
} finally {
  await browser.close();
  await server.close();
}

const passed = gates.filter((g) => g.ok).length;
console.log(`\n${passed}/${gates.length} gates passed`);
await writeFile(`${OUT}scorecard.json`, JSON.stringify({ at: new Date().toISOString(), passed, total: gates.length, gates, metrics }, null, 2));
process.exit(passed === gates.length ? 0 : 1);
