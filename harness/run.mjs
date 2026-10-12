// Browser harness: real Chromium tabs, real WebRTC data channels (signaled via
// a local relay), real pointer and keyboard events through the real UI. Five
// players (one phone-sized) plus a watcher play two rounds. Prints a
// scorecard and writes harness/out/scorecard.json + screenshots. Exit code 1
// if any gate fails.
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

async function open(label, viewport = { width: 1280, height: 860 }) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${label}: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${label}: ${m.text()}`));
  await page.goto(url);
  await page.waitForFunction(() => window.__cl);
  const P = {
    label, page, ctx, viewport,
    ev: (fn, arg) => page.evaluate(fn, arg),
    digest: () => page.evaluate(() => window.__cl.digest()),
    ui: () => page.evaluate(() => window.__cl.ui()),
    card: (i) => page.evaluate((i) => window.__cl.card(i), i),
    slot: (s) => page.evaluate((s) => window.__cl.slot(s), s),
    board: (x, y) => page.evaluate(([x, y]) => window.__cl.board(x, y), [x, y]),
    petal: (d) => page.evaluate((d) => window.__cl.petal(d), d),
    hub: () => page.evaluate(() => window.__cl.hub()),
    quarter: () => page.evaluate(() => window.__cl.quarter()),
    settle: () => page.evaluate(() => window.__cl.settle()),
    presses: () => page.evaluate(() => window.__cl.stats.presses),
    seat: async () => (await P.ui()).seat,
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
    async joinViaSheet(name) {
      await page.waitForSelector('#joinSheet.open', { timeout: 5000 });
      await page.fill('#nameInput', name);
      await page.click('#joinBtn');
    },
    go: () => page.click('#go'),
    waitGoEnabled: () => page.waitForFunction(() => !document.getElementById('go').disabled && !document.getElementById('go').hidden),
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

/** A guesser drops card s on spot s (snaps) and turns it right, or a decoy into spot 0. */
async function solve(p, { decoyInSpot0 = false } = {}) {
  await p.settle();
  for (let s = 0; s < 4; s++) {
    if (cur(await p.digest()).mask & (1 << s)) continue;
    const card = decoyInSpot0 && s === 0 ? 4 : s;
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
  console.log('\n# build');
  const wasm = await readFile(here('../web/core.wasm'));
  const mod = await WebAssembly.compile(wasm);
  metric('wasm_bytes', wasm.length, ' B');
  const shim = (await readFile(here('../web/main.js'), 'utf8')).split('\n').filter((l) => l.trim() && !l.trim().startsWith('//')).length;
  metric('js_shim_loc', shim);
  gate('wasm has zero JS imports (no bindgen glue)', WebAssembly.Module.imports(mod).length === 0);

  // ---- join ------------------------------------------------------------------------
  console.log('\n# join (5 players + 1 watcher, WebRTC mesh)');
  let t0 = performance.now();
  const names = ['Ana', 'Ben', 'Cy', 'Dee', 'Eve'];
  const players = await Promise.all([
    ...names.slice(0, 4).map((n) => open(n)),
    open('Eve-phone', { width: 390, height: 844 }),
  ]);
  const [A, B, C, D, E] = players;
  await E.page.waitForSelector('#joinSheet.open', { timeout: 5000 });
  await E.page.screenshot({ path: `${OUT}0-join-phone.png` });
  const joinPresses = [];
  for (const [k, p] of players.entries()) {
    const p0 = await p.presses();
    await p.joinViaSheet(names[k]);
    joinPresses.push((await p.presses()) - p0);
  }
  const W = await open('Watcher');
  await W.page.waitForSelector('#joinSheet.open');
  await W.page.click('#watchBtn');
  await Promise.all([...players, W].map((p) => p.page.waitForFunction(() => window.__cl.ui().players.length === 5, null, { timeout: 30000 })));
  metric('join_5_players_ms', performance.now() - t0, ' ms');
  gate('first visit asks for a name; joining is one tap', Math.max(...joinPresses) === 1);
  await A.page.waitForFunction(() => window.__cl.ui().watchers === 1, null, { timeout: 20000 })
    .then(() => gate('everyone sees the watcher', true), () => gate('everyone sees the watcher', false));

  // ---- writing ----------------------------------------------------------------------
  console.log('\n# writing (everyone at once)');
  const startLabel = await A.page.textContent('#go');
  gate('starting names how many players will be dealt in', startLabel === 'Start with 5 players', startLabel);
  await A.go(); // Start
  await until([...players, W], (d) => d.phase === 'writing' && d.round === 1 && d.dealt.length === 5);
  gate('one press deals everyone in, and fixes the roster', true);
  gate("watchers don't get a clover", (await W.ev(() => window.__cl.card(0)))[0] < 0);

  // ---- legibility: contrast, size, no stray selection --------------------------------
  {
    const canvasPairs = await A.ev(() => window.__cl.palette());
    const domPairs = await A.ev(() => {
      const rgb = (c) => (c.match(/[\d.]+/g) || []).map(Number);
      const opaqueBg = (el) => {
        for (let e = el; e; e = e.parentElement) {
          const c = rgb(getComputedStyle(e).backgroundColor);
          if (c.length === 3 || c[3] === 1) return c.slice(0, 3);
        }
        return [20, 48, 31];
      };
      const hex = (c) => '#' + c.slice(0, 3).map((x) => Math.round(x).toString(16).padStart(2, '0')).join('');
      return ['#hint', '#go', '#decLabel']
        .map((sel) => document.querySelector(sel))
        .filter((el) => el && el.offsetParent)
        .map((el) => [el.id || el.className, hex(rgb(getComputedStyle(el).color)), hex(opaqueBg(el)), parseFloat(getComputedStyle(el).fontSize)]);
    });
    const lum = (h) => {
      const v = parseInt(h.slice(1), 16);
      const ch = (s) => { const c = ((v >> s) & 255) / 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0);
    };
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
    const all = [...canvasPairs, ...domPairs].map(([n, f, b]) => [n, ratio(f, b)]);
    const worst = all.reduce((m, p) => (p[1] < m[1] ? p : m));
    metric('min_contrast_ratio', worst[1]);
    gate('all text meets WCAG AA contrast (≥ 4.5:1)', worst[1] >= 4.5, `worst: ${worst[0]} ${worst[1].toFixed(2)}`);
    const smallest = Math.min(...domPairs.map((p) => p[3]));
    gate('no UI text smaller than 14 px', smallest >= 14, `${smallest}px`);
    // Every word in the deck fits its card edge at full size (no squeezing,
    // no corner collisions), measured in the shipped font.
    const deck = JSON.parse(await readFile(here('../core/src/shamrock.json'), 'utf8')).flat();
    const widths = await A.ev((w) => window.__cl.measure(w), deck);
    const budget = await A.ev(() => window.__cl.wordBudget());
    const widest = widths.reduce((m, w) => (w[1] > m[1] ? w : m));
    metric('widest_word_vs_budget', `${widest[0]} ${widest[1].toFixed(0)}/${budget.toFixed(0)}`);
    gate('every deck word fits its card edge at full size', widths.every((w) => w[1] <= budget), `${widest[0]} ${widest[1].toFixed(1)} of ${budget}`);
    const fontOk = await A.ev(() => document.fonts.check('700 26px Figtree') && document.fonts.check('600 26px Fraunces'));
    gate('brand fonts load from the site itself', fontOk);
    const sel = await E.ev(() => [getComputedStyle(document.getElementById('hint')).userSelect, getComputedStyle(document.getElementById('nameInput')).userSelect]);
    gate("bars can't be text-selected (no dictionary pop-ups); inputs can", sel[0] === 'none' && sel[1] !== 'none', sel.join(','));
  }

  // Tapping the side of a leaf opens its editor and never turns the board.
  {
    await B.settle();
    const q0 = await B.quarter();
    await B.tap((await B.board(150, 330)).slice(0, 2)); // lower-right part of the bottom leaf
    await B.page.waitForSelector('#clue', { state: 'visible' });
    await sleep(300);
    gate('tapping the side of a leaf edits it without turning the board', (await B.quarter()) === q0 && await B.page.isVisible('#clue'));
    const tf = await B.page.evaluate(() => getComputedStyle(document.getElementById('clue')).transform);
    const [a, b] = (tf.match(/-?[\d.e-]+/g) || [1, 0]).map(Number);
    gate("a leaf's editor lies along the leaf (bottom leaf: upside down)", Math.abs(Math.atan2(b, a)) > 3.0, tf);
    await B.page.keyboard.press('Escape');
  }

  const words = ['ocean', 'storm', 'royal', 'kitchen'];
  const pressCounts = [];
  await Promise.all(players.map(async (p, k) => {
    await p.settle();
    const p0 = await p.presses();
    await p.tap((await p.petal(0)).slice(0, 2));
    for (const w of words) {
      await p.page.waitForSelector('#clue', { state: 'visible' });
      await p.page.keyboard.type(`${w}${k}`);
      await p.page.keyboard.press('Enter');
    }
    if (k === 1) await p.page.click('#decUp'); // Ben bids 2 decoys
    await p.waitGoEnabled();
    pressCounts.push([(await p.presses()) - p0, k]);
    if (k !== 4) await p.go(); // Eve holds back
  }));
  await A.page.screenshot({ path: `${OUT}1-writing-ready.png` });
  await E.page.screenshot({ path: `${OUT}1-writing-phone.png` });
  const plain = pressCounts.filter(([, k]) => k !== 1).map(([n]) => n + 1);
  metric('presses_to_write_and_ready', Math.max(...plain));
  gate('a clover takes ≤ 6 presses (leaf, 4× Enter, Ready)', Math.max(...plain) <= 6, plain.join(','));
  await sleep(600);
  gate('guessing waits until everyone has submitted', (await A.digest()).phase === 'writing');
  await A.page.waitForFunction(() => document.querySelectorAll('#segs i.done').length === 4, null, { timeout: 3000 })
    .then(() => gate('the progress bar fills as people submit (4 of 5)', true), () => gate('the progress bar fills as people submit (4 of 5)', false));
  await A.page.waitForFunction(() => document.getElementById('hint').textContent.includes('Eve'), null, { timeout: 3000 }).catch(() => {});
  const label = await A.page.textContent('#hint');
  const proceed = await A.page.textContent('#alt');
  const eAlt = await E.page.evaluate(() => document.getElementById('alt').hidden);
  gate('progress names who is left; override reads "Proceed with 4 of 5"', label.includes('4/5') && label.includes('Eve') && proceed === 'Proceed with 4 of 5' && eAlt, `${label} | ${proceed} | Eve's own Proceed hidden: ${eAlt}`);
  // Submitted players can still edit.
  await A.settle();
  await A.tap((await A.petal(2)).slice(0, 2));
  await A.page.waitForSelector('#clue', { state: 'visible' });
  await A.page.fill('#clue', 'edited after submit');
  await A.page.keyboard.press('Enter');
  await until([B], (d) => d.subs.some((x) => x[2] === true && x.includes('edited after submit')))
    .then(() => gate('a submitted player can still edit; stays submitted', true), () => gate('a submitted player can still edit; stays submitted', false));
  t0 = performance.now();
  await E.go();
  await until([...players, W], (d) => d.phase === 'guessing:0', 10000);
  metric('last_ready_to_guessing_ms', performance.now() - t0, ' ms');
  const d1 = await A.digest();
  gate('all 5 clovers queued; decoy bids kept', d1.puzzles.length === 5 && d1.subs.some((s) => s[1] === 2));
  gate('clues typed via Enter-to-next-leaf arrive intact', d1.subs.every((s) => s.slice(3).every((c, d) => c.startsWith(words[d]) || c === 'edited after submit')));

  // ---- take over a seat from another device ------------------------------------------
  console.log('\n# takeover');
  const A2 = await open('Ana-tablet', { width: 820, height: 1180 });
  await A2.joinViaSheet('ana');
  await A2.page.waitForFunction(() => window.__cl.ui().seated);
  gate('choosing an existing name takes over that seat', (await A2.seat()) === (await A.seat()) && (await A2.ui()).players.length === 5);

  // ---- guessing ----------------------------------------------------------------------
  console.log('\n# guessing');
  const all = [...players, A2, W];
  const bySeat = async (seat) => { for (const p of players) if ((await p.seat()) === seat) return p; };
  let Wr = await bySeat(cur(await A.digest()).writer);
  let guessers = players.filter((p) => p !== Wr);
  const [G, H] = guessers.filter((p) => p !== E);

  // Legibility and fill, measured where it matters: while guessing (tray shown).
  for (const p of [G, E]) {
    const word = await p.ev(() => window.__cl.wordPx());
    const boardPx = (await p.ev(() => window.__cl.cardPx() / window.__cl.zoom())) / 210 * 864; // 2 × EXTENT
    const side = Math.min(p.viewport.width, p.viewport.height);
    const tag = p === E ? 'phone_390x844' : 'desktop_1280x860';
    metric(`word_px_${tag}`, word, ' px');
    metric(`board_fill_${tag}`, boardPx / side);
  }
  gate('card words ≥ 16 px on desktop', metrics.word_px_desktop_1280x860 >= 16);
  gate('card words ≥ 14 px on a phone (bigger cards by default there)', metrics.word_px_phone_390x844 >= 14);
  gate('the board fills the phone width (≥ 95%)', metrics.board_fill_phone_390x844 >= 0.95);

  // Card size: per device, display only.
  {
    const d0 = JSON.stringify(await E.digest());
    const w0 = await E.ev(() => window.__cl.wordPx());
    await E.page.click('#menuBtn');
    await E.page.waitForSelector('#menuSheet.open');
    for (let k = 0; k < 4; k++) await E.page.click('#sizeUp');
    await E.page.click('#closeMenu');
    const w1 = await E.ev(() => window.__cl.wordPx());
    metric('word_px_phone_max_size', w1, ' px');
    await sleep(200);
    gate('the card-size buttons enlarge cards on my screen only', w1 > w0 && JSON.stringify(await G.digest()) === d0
      && Math.abs((await G.ev(() => window.__cl.wordPx())) - metrics.word_px_desktop_1280x860) < 0.01, `${w0.toFixed(1)} → ${w1.toFixed(1)}`);
    // Even at the largest size, three tray cards fit across a phone without overlap.
    const px = await E.ev(() => window.__cl.cardPx());
    const tray = [];
    let onScreen = true;
    const cards = cur(await E.digest()).cards;
    for (let i = 0; i < cards.length; i++) {
      const [x, y] = await E.card(i);
      onScreen &&= x > 0 && x < 390 && y > 0 && y < 844;
      if (cards[i][0] === 'table') tray.push([x, y]);
    }
    const overlap = tray.some((a, i) => tray.some((b, j) => j > i && Math.abs(a[0] - b[0]) < px - 0.5 && Math.abs(a[1] - b[1]) < px - 0.5));
    gate('at the largest card size, tray cards on a phone still never overlap', !overlap && tray.length > 0, `${tray.length} in tray at ${px.toFixed(0)} px`);
    gate('at the largest card size every card is still on screen', onScreen);
  }

  const wd = JSON.stringify(await Wr.digest());
  await Wr.settle();
  await Wr.drag((await Wr.card(0)).slice(0, 2), (await Wr.slot(0)).slice(0, 2));
  gate("the clover's writer cannot move cards", JSON.stringify(await Wr.digest()) === wd);

  const lat = [];
  for (let k = 0; k < 6; k++) {
    await G.settle();
    const s = k % 4;
    await G.drag((await G.card(0)).slice(0, 2), (await G.slot(s)).slice(0, 2));
    const t = performance.now();
    await until(all.filter((p) => p !== G), new Function('d', `const c=d.puzzles[0].cards[0];return c[0]==='slot'&&c[1]===${s}&&c[3]===0`));
    lat.push(performance.now() - t);
  }
  metric('sync_latency_p50_ms', pct(lat, 0.5), ' ms');
  metric('sync_latency_p95_ms', pct(lat, 0.95), ' ms');
  gate('a drop shows up on 6 other screens, p95 < 150 ms', pct(lat, 0.95) < 150);

  // Tentative placement: off-centre near a spot, and loose on the board.
  await G.settle();
  await G.drag((await G.card(1)).slice(0, 2), (await G.board(140 + 45, 140 + 40)).slice(0, 2));
  await G.drag((await G.card(2)).slice(0, 2), (await G.board(-20, 230)).slice(0, 2));
  await until(all, (d) => { const c = d.puzzles[0].cards; return c[1][0] === 'float' && c[2][0] === 'float'; });
  const tc = cur(await H.digest()).cards;
  gate('tentative placements stay exactly where dropped, for everyone',
    Math.abs(tc[1][1] - 185) < 2 && Math.abs(tc[1][2] - 180) < 2 && Math.abs(tc[2][2] - 230) < 2, JSON.stringify(tc.slice(1, 3)));
  gate('a card near a spot counts toward a check', cur(await H.digest()).claim[2] === 1);
  await H.page.screenshot({ path: `${OUT}2-guessing-tentative.png` });
  await E.page.screenshot({ path: `${OUT}2-guessing-phone.png` });

  // Card turns animate.
  await G.page.evaluate(() => new Promise(requestAnimationFrame));
  await G.tap((await G.card(3)).slice(0, 2));
  const target = (await G.card(3))[2];
  const mid = await G.page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(window.__cl.shownAngle(3)))));
  await sleep(600);
  const end = await G.ev(() => window.__cl.shownAngle(3));
  const gap = (a, b) => Math.abs((((a - b + Math.PI) % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI) - Math.PI);
  gate('turning a card animates instead of jumping', gap(mid, target) > 0.15 && gap(end, target) < 0.02, `mid off by ${gap(mid, target).toFixed(2)} rad`);

  // Turning the board: hub tap. Local only; board cards turn, tray cards stay.
  await G.settle();
  const n = cur(await G.digest()).cards.length;
  const before = await Promise.all([...Array(n).keys()].map((i) => G.card(i)));
  const otherBefore = await Promise.all([...Array(n).keys()].map((i) => H.card(i)));
  const dBefore = JSON.stringify(await G.digest());
  gate('the board starts each clover upright', (await G.quarter()) % 4 === 0);
  const q0 = await G.quarter();
  const [cx, cy] = await G.hub();
  await G.tap([cx, cy]);
  await G.settle();
  const after = await Promise.all([...Array(n).keys()].map((i) => G.card(i)));
  let boardOk = true, trayOk = true;
  cur(await G.digest()).cards.forEach((c, i) => {
    const [x, y] = before[i];
    if (c[0] !== 'table') boardOk &&= near(after[i], [cx - (y - cy), cy + (x - cx)]);
    else trayOk &&= near(after[i], before[i]);
  });
  gate('tapping ⟳ turns my view a quarter', (await G.quarter()) === q0 + 1);
  gate('cards on the board (snapped or loose) turn with it', boardOk);
  gate('cards in the tray stay put', trayOk);
  await sleep(150);
  const otherAfter = await Promise.all([...Array(n).keys()].map((i) => H.card(i)));
  gate("turning my board doesn't move anything on other screens", otherBefore.every((p, i) => near(p, otherAfter[i])));
  gate('board rotation is not shared state', JSON.stringify(await H.digest()) === dBefore);

  // First try with a decoy in spot 0, then fix it.
  await solve(G, { decoyInSpot0: true });
  await G.waitGoEnabled();
  // Record shakes on another screen as they happen (they last 0.6 s).
  await H.ev(() => { window.__shook = 0; const t = () => { window.__shook = Math.max(window.__shook, window.__cl.X.probe_shaking());
    if (window.__shook === 0) requestAnimationFrame(t); }; requestAnimationFrame(t); });
  await G.go();
  await until(all, (d) => d.puzzles[0].attempts === 1);
  const c1 = cur(await B.digest());
  gate('a miss boots the wrong card to the tray; right ones stay', c1.mask === 0b1110 && c1.cards[4][0] === 'table', `mask ${c1.mask}`);
  // (The one-time how-to tip may still be up from when guessing began; nothing else.)
  const toastNow = await H.page.$eval('#toast', (t) => t.classList.contains('show') ? t.textContent : '');
  gate('a miss comes with no commentary', (!toastNow || toastNow === (await H.page.textContent('#tipText'))) && (await H.page.textContent('#go')) === 'Guess', toastNow);
  const shook = await H.page.waitForFunction(() => window.__shook > 0, null, { timeout: 3000 }).then(() => H.ev(() => window.__shook), () => 0);
  gate('booted cards shake on every screen (simple motion for a miss)', shook > 0, `shaking ${shook}`);
  await solve(H);
  await H.waitGoEnabled();
  await H.go();
  await until(all, (d) => d.puzzles[0].attempts === 2 && d.puzzles[0].mask === 15);
  gate('guessing keeps going until all four are right (no limit, no score)', (await A.digest()).gameScore === 0);
  await G.page.screenshot({ path: `${OUT}3-clover-done.png` });

  for (let k = 1; k < 5; k++) {
    await guessers[0].go(); // Next clover
    await until(all, new Function('d', `return d.phase==='guessing:${k}'`));
    Wr = await bySeat(cur(await A.digest()).writer);
    guessers = players.filter((p) => p !== Wr);
    const g = guessers.find((p) => p !== E);
    await solve(g);
    await g.waitGoEnabled();
    await g.go();
    await until(all, new Function('d', `return d.puzzles[${k}].attempts===1`));
  }
  await A.go(); // Finish round
  await until(all, (d) => d.phase === 'end');
  const r1 = await A.digest();
  gate('every clover solved, same state everywhere', r1.puzzles.every((p) => p.mask === 15) && (await same(all)));
  await A.page.screenshot({ path: `${OUT}5-end.png` });

  // ---- share boards: pick clovers, open the link at a fresh table ------------------------
  console.log('\n# share boards');
  await A.page.click('#side'); // Share boards
  await A.page.waitForSelector('#shareSheet.open');
  await A.page.click('#shareList .item:nth-child(2) input'); // leave one out
  await A.page.waitForFunction(() => !document.getElementById('shareCopy').disabled && document.getElementById('shareText').textContent.includes('?b='));
  const link = (await A.page.textContent('#shareText')).trim();
  // Nothing about the boards is readable from the link, raw or base64-decoded.
  {
    const payload = new URL(link).searchParams.get('b');
    const raw = Buffer.from(payload.replace(/-/g, '+').replace(/_/g, '/'), 'base64').toString('latin1').toLowerCase();
    const words = r1.subs.flatMap((x) => x.slice(3)).concat((await A.ev(() => window.__cl.X && [])) || []);
    const cardWords = await A.ev(() => { const d = window.__cl.digest(); return d.subs.length; });
    const leaks = words.filter((w) => w.length > 3 && (raw.includes(w.toLowerCase()) || link.toLowerCase().includes(w.toLowerCase())));
    gate("share links don't reveal clues or answers at a glance (encrypted)", leaks.length === 0 && cardWords > 0, leaks.join(','));
  }
  await A.page.screenshot({ path: `${OUT}7-share.png` });
  await A.page.click('#shareClose');
  metric('share_link_chars', link.length);
  gate('a four-clover share link stays under 1,200 characters (compressed)', link.length < 1200, `${link.length}`);
  // A share link is solo play: no room, no name, no peers; straight into the clovers.
  const sharedUrl = link.replace(/^https?:\/\/[^/]+\//, `http://localhost:${PORT}/`);
  const soloCtx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const solo = await soloCtx.newPage();
  solo.on('pageerror', (e) => errors.push(`solo: ${e.message}`));
  await solo.goto(sharedUrl);
  await solo.waitForFunction(() => window.__cl?.digest?.().phase === 'guessing:0', null, { timeout: 10000 });
  await sleep(1200);
  const fd = await solo.evaluate(() => window.__cl.digest());
  const soloUi = await solo.evaluate(() => ({ join: document.getElementById('joinSheet').classList.contains('open'),
    roster: document.getElementById('roster').offsetParent ? 'shown' : 'hidden', go: document.getElementById('go').textContent,
    remote: location.search.includes('signal') }));
  const srcClues = r1.subs.filter((x) => r1.puzzles.some((p, i) => i !== 1 && p.writer === x[0])).map((x) => x.slice(3).join('/'));
  const gotClues = fd.subs.map((x) => x.slice(3).join('/'));
  gate('a share link opens solo, straight into the chosen clovers (no room, no name)',
    !soloUi.join && soloUi.roster === 'hidden' && soloUi.go === 'Guess' && fd.puzzles.length === 4
    && srcClues.every((c) => gotClues.includes(c)), `${fd.puzzles.length} boards, join=${soloUi.join}, go=${soloUi.go}`);
  // Placing a shared board as its writer laid it out solves it first time.
  const Fr = { page: solo, ev: (fn, a) => solo.evaluate(fn, a), digest: () => solo.evaluate(() => window.__cl.digest()),
    card: (i) => solo.evaluate((i) => window.__cl.card(i), i), slot: (s) => solo.evaluate((s) => window.__cl.slot(s), s),
    settle: () => solo.evaluate(() => window.__cl.settle()),
    origin: () => solo.evaluate(() => { const r = document.getElementById('c').getBoundingClientRect(); return [r.left, r.top]; }) };
  Fr.drag = async (from, to) => { const o = await Fr.origin(), m = solo.mouse; await m.move(o[0] + from[0], o[1] + from[1]); await m.down();
    for (let k = 1; k <= 12; k++) await m.move(o[0] + from[0] + (to[0] - from[0]) * k / 12, o[1] + from[1] + (to[1] - from[1]) * k / 12); await m.up(); };
  Fr.tap = async (at) => { const o = await Fr.origin(); await solo.mouse.click(o[0] + at[0], o[1] + at[1]); };
  await solve(Fr);
  await solo.waitForFunction(() => !document.getElementById('go').disabled);
  await solo.click('#go');
  await solo.waitForFunction(() => window.__cl.digest().puzzles[0].attempts === 1, null, { timeout: 8000 });
  gate("a shared board checks against its writer's answer", (await solo.evaluate(() => window.__cl.digest())).puzzles[0].mask === 15);
  await solo.screenshot({ path: `${OUT}8-solo.png` });
  await soloCtx.close();

  // ---- next round keeps the table and the score -----------------------------------------
  console.log('\n# rounds and games');
  const nextLabel = await B.page.textContent('#go');
  await B.go(); // Next round
  await until(all, (d) => d.phase === 'writing' && d.round === 2);
  gate('next round button names the players', nextLabel === 'Next round with 5 players', nextLabel);
  const r2 = await C.digest();
  gate('next round: same players', r2.seats.length === 5);

  await D.ctx.close();
  // (Dee has left mid-writing; others would now see "Waiting for Dee (away)".)
  await A.page.waitForFunction(() => window.__cl.ui().players.find((p) => p.name === 'Dee')?.away, null, { timeout: 15000 })
    .then(() => gate('a player who leaves shows as away', true), () => gate('a player who leaves shows as away', false));
  await A.page.click('#menuBtn');
  await A.page.screenshot({ path: `${OUT}6-menu.png` });
  await A.page.click('#newGameBtn');
  const live = [A, B, C, E, A2, W];
  await until(live, (d) => d.phase === 'lobby' && d.round === 0);
  const g2 = await B.digest();
  gate("New game starts over with whoever is here", g2.seats.length === 4 && !g2.seats.some((s) => s[1] === 'Dee'), g2.seats.map((s) => s[1]).join(','));

  t0 = performance.now();
  const F = await open('Late');
  await F.page.waitForFunction((t) => JSON.stringify(window.__cl.digest()) === t, JSON.stringify(await A.digest()), { timeout: 30000 });
  metric('late_join_state_ms', performance.now() - t0, ' ms');
  gate('a late visitor receives the full table state', true);

  console.log('\n# perf');
  await sleep(1000);
  const fms = await A.ev(() => window.__cl.stats.frameMs.slice(-300));
  metric('frame_cpu_p50_ms', pct(fms, 0.5), ' ms');
  metric('frame_cpu_p95_ms', pct(fms, 0.95), ' ms');
  gate('frame CPU (wasm + paint) p95 < 4 ms', pct(fms, 0.95) < 4);
  gate('no page errors', errors.length === 0, errors.slice(0, 3).join(' | '));
} catch (e) {
  gate('harness completed', false, e.stack?.split('\n').slice(0, 3).join(' '));
  for (const ctx of browser.contexts()) {
    for (const pg of ctx.pages()) {
      const st = await pg.evaluate(() => ({ ui: window.__cl?.ui(), hint: document.getElementById('hint').textContent })).catch((x) => String(x));
      console.log('  state', JSON.stringify(st).slice(0, 300));
      await pg.screenshot({ path: `${OUT}fail-${browser.contexts().indexOf(ctx)}.png` }).catch(() => {});
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
