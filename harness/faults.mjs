// Fault scenarios in real browsers: what the Rust network simulator can't
// reach (the page's connection handling, reload and restore, the status
// line, real WebRTC and signaling). Each scenario is its own table with its
// own browser contexts (separate storage, like separate phones); moves go
// through the page's agent tools. Faults:
//   - the page's ?chaos hook: `cut` silences a page both ways while its
//     connections stay up (a phone in a tunnel), `delayMs` lags what arrives;
//   - the signaling relay going down and coming back;
//   - pages reloading, closing mid-gesture, and every page going away.
//
//   node harness/faults.mjs            exit code 1 if any check fails
import { chromium } from 'playwright';
import { serve } from './serve.mjs';

const PORT = 8941, RELAY = 8942;
const srv = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({ args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] });
const fails = [], errors = [];
const check = (ok, what, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}${detail ? `  — ${detail}` : ''}`); if (!ok) fails.push(what); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 15000) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { try { if (await fn()) return true; } catch {} await sleep(150); } return false; };
const url = (room, extra = '') => `http://localhost:${PORT}/?signal=ws://localhost:${RELAY}&chaos&rejoin=2000${extra}#${room}`;

/** A player: their own browser context (storage), one page at the table. */
async function player(name, room, { join = true } = {}) {
  const ctx = await browser.newContext({ viewport: { width: 900, height: 900 } });
  await ctx.addInitScript(() => { for (const k of ['guess', 'write']) localStorage.setItem(`shamrock:tip:${k}`, '1'); });
  const p = { name, ctx };
  p.open = async (extra = '&fresh') => {
    p.page = await ctx.newPage();
    p.page.on('pageerror', (e) => errors.push(`${name}: ${e.message}`));
    await p.page.goto(url(room, extra));
    await p.page.waitForFunction(() => window.__cl?.tools);
  };
  p.tool = (t, a = {}) => p.page.evaluate(([t, a]) => window.__cl.tools[t](a), [t, a]);
  p.table = () => p.tool('get_table');
  p.digest = () => p.page.evaluate(() => JSON.stringify(window.__cl.digest()));
  p.chaos = (c) => p.page.evaluate((c) => Object.assign(window.__cl.chaos, c), c);
  p.status = () => p.page.textContent('#hint');
  await p.open();
  if (join) await p.tool('join_table', { name });
  return p;
}
const live = (ps) => ps.filter((p) => !p.page.isClosed());
const agree = async (ps) => { const d = await Promise.all(live(ps).map((p) => p.digest())); return d.every((x) => x === d[0]); };
const seatedEverywhere = async (ps, n) => (await Promise.all(live(ps).map((p) => p.table()))).every((t) => t.players.length === n && t.seated);
const writeAll = async (p, word, submit = true) => {
  await until(async () => (await p.table()).phase === 'writing', 10000); // the deal reaches this screen first
  for (const leaf of ['top', 'right', 'bottom', 'left']) await p.tool('write_clue', { leaf, text: `${word} ${leaf}` });
  if (submit) await p.tool('submit_clover');
};
async function table(room, names) {
  const ps = [];
  for (const n of names) ps.push(await player(n, room));
  await until(() => seatedEverywhere(ps, names.length), 30000);
  return ps;
}
const done = async (ps) => { for (const p of ps) await p.ctx.close().catch(() => {}); };

// ONLY=text runs just the scenarios whose title contains it.
async function scenario(title, fn) {
  if (process.env.ONLY && !title.includes(process.env.ONLY)) return;
  console.log(`\n# ${title}`);
  try { await fn(); } catch (e) { check(false, `${title}: completed`, e.message.split('\n')[0]); }
}

await scenario('everyone joins at once, before anyone has connected', async () => {
  const room = `storm-${Date.now().toString(36)}`;
  // Four phones open the link together and sit down straight away.
  const ps = await Promise.all(['Ana', 'Ben', 'Cy', 'Dee'].map((n) => player(n, room)));
  check(await until(() => seatedEverywhere(ps, 4), 30000), 'all four end up seated on every screen');
  check(await until(() => agree(ps)), 'and every screen agrees');
  await ps[2].tool('start_round');
  for (const p of ps) await writeAll(p, p.name.toLowerCase());
  check(await until(async () => (await Promise.all(ps.map((p) => p.table()))).every((t) => t.phase === 'guessing'), 20000), 'a round dealt and submitted reaches guessing everywhere');
  await done(ps);
});

await scenario('a phone in a tunnel: cut off with its connections still up', async () => {
  const room = `tunnel-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben', 'Cy']);
  const [ana, ben, cy] = ps;
  await ana.tool('start_round');
  await until(async () => (await cy.table()).phase === 'writing');
  await cy.chaos({ cut: true });
  // Both sides carry on: Ana and Ben write and submit, Cy writes in the tunnel.
  await writeAll(ana, 'ana');
  await writeAll(ben, 'ben');
  await writeAll(cy, 'cy', false);
  await sleep(1500);
  check((await ana.table()).phase === 'writing', 'guessing waits for the player who is cut off');
  // Out of the tunnel. No connection opened or closed, so no full sync was
  // triggered: the table hashes in presence pings have to find the drift.
  await cy.chaos({ cut: false });
  const t0 = Date.now();
  check(await until(() => agree(ps), 20000), 'once back, every screen agrees again', `${((Date.now() - t0) / 1000).toFixed(1)} s`);
  const view = await ana.tool('get_table');
  check(view.readyCount === 2 && view.phase === 'writing', "Ana sees Cy's table as it is (two submitted, Cy still writing)", JSON.stringify({ phase: view.phase, ready: view.readyCount }));
  await cy.tool('submit_clover');
  check(await until(async () => (await Promise.all(ps.map((p) => p.table()))).every((t) => t.phase === 'guessing')), 'Cy submits and guessing starts everywhere');
  await done(ps);
});

await scenario('a laggy connection (600 ms behind)', async () => {
  const room = `lag-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben', 'Cy']);
  await ps[1].chaos({ delayMs: 600 });
  await ps[0].tool('start_round');
  for (const p of ps) await writeAll(p, p.name.toLowerCase());
  check(await until(async () => (await Promise.all(ps.map((p) => p.table()))).every((t) => t.phase === 'guessing'), 20000), 'a whole writing phase completes with one player lagging');
  // Guess the first clover from the laggy screen: the tools see what it sees.
  const lag = ps[1], t = await lag.table();
  if (!t.clover.isMine) {
    const labels = await lag.page.evaluate(() => window.__cl.agentView().cards.filter((c) => c._i < 4).sort((a, b) => a._i - b._i).map((c) => c.card));
    for (let s = 0; s < 4; s++) await lag.tool('place_card', { card: labels[s], spot: s });
  }
  check(await until(() => agree(ps), 15000), 'moves from the laggy screen reach everyone, and agree');
  await done(ps);
});

await scenario('reloading mid-writing', async () => {
  const room = `reload-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben', 'Cy']);
  const [ana, ben] = ps;
  await ana.tool('start_round');
  await until(async () => (await ben.table()).phase === 'writing');
  await ben.tool('write_clue', { leaf: 'top', text: 'kept' });
  await ben.tool('write_clue', { leaf: 'left', text: 'also kept' });
  await sleep(500);
  // A plain reload (no ?fresh): the page restores its save and rejoins as Ben.
  await ben.page.goto(url(room));
  await ben.page.waitForFunction(() => window.__cl?.tools);
  check(await until(async () => (await ben.table()).you === 'Ben' && (await ben.table()).seated, 10000), 'Ben is back in his seat without being asked');
  const clues = (await ben.table()).clover.clues;
  check(clues.top === 'kept' && clues.left === 'also kept', 'his half-written clues are still there', JSON.stringify(clues));
  check(await until(() => agree(ps)), 'and every screen agrees');
  await done(ps);
});

await scenario('everyone closes the table, then comes back', async () => {
  const room = `gone-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben', 'Cy']);
  await ps[0].tool('start_round');
  for (const p of ps) await writeAll(p, p.name.toLowerCase());
  await until(async () => (await ps[0].table()).phase === 'guessing');
  const before = await ps[0].digest();
  await sleep(2500); // past a save
  for (const p of ps) await p.page.close({ runBeforeUnload: true });
  // Only Cy comes back at first: no one else holds the table.
  await ps[2].open('');
  const t = await ps[2].table();
  check(t.phase === 'guessing' && t.players.length === 3, 'the first one back finds the table from their own save', JSON.stringify({ phase: t.phase, players: t.players.length }));
  await ps[0].open('');
  check(await until(async () => (await ps[0].digest()) === (await ps[2].digest())), 'the next one back agrees with them');
  check((await ps[0].digest()) === before, 'and nothing was lost');
  await done(ps);
});

await scenario('the signaling relay goes down mid-game', async () => {
  const room = `relay-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben']);
  await ps[0].tool('start_round');
  await srv.stopRelay();
  // Connected players keep playing over WebRTC.
  for (const p of ps) await writeAll(p, p.name.toLowerCase());
  check(await until(async () => (await Promise.all(ps.map((p) => p.table()))).every((t) => t.phase === 'guessing'), 15000), 'players already connected keep playing without the relay');
  // Someone new can't find the table until it's back.
  const cy = await player('Cy', room, { join: false });
  await sleep(3000);
  check((await cy.table()).players.length === 0, 'a newcomer meanwhile sees nobody', (await cy.status()).trim());
  check(/waiting|reconnecting/i.test(await cy.status()), 'and is told so in the status line', (await cy.status()).trim());
  await srv.startRelay();
  check(await until(async () => (await cy.table()).players.length === 2, 30000), 'when the relay is back, the newcomer finds the table (retrying on its own)');
  check(await until(() => agree([...ps, cy])), 'and receives the whole table');
  await done([...ps, cy]);
});

await scenario('a phone dies mid-drag', async () => {
  const room = `drag-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben', 'Cy']);
  await ps[0].tool('start_round');
  for (const p of ps) await writeAll(p, p.name.toLowerCase());
  await until(async () => (await Promise.all(ps.map((p) => p.table()))).every((t) => t.phase === 'guessing'));
  const writer = (await ps[0].table()).clover.writer;
  const [g, other] = ps.filter((p) => p.name !== writer);
  // A real finger on a real card, moved but never lifted.
  const at = await g.page.evaluate(() => { window.__cl.settle(); const r = document.getElementById('c').getBoundingClientRect(); const c = window.__cl.card(0); return [r.left + c[0], r.top + c[1]]; });
  await g.page.mouse.move(at[0], at[1]);
  await g.page.mouse.down();
  for (let k = 1; k <= 8; k++) await g.page.mouse.move(at[0] + 10 * k, at[1] + 6 * k);
  check(await until(() => other.page.evaluate(() => Object.keys(window.__cl.holds()).length > 0), 3000), 'the other guesser sees the card held');
  await g.ctx.close();
  check(await until(() => other.page.evaluate(() => Object.keys(window.__cl.holds()).length === 0), 8000), 'when that phone dies, the hold goes away');
  const labels = await other.page.evaluate(() => window.__cl.agentView().cards.filter((c) => c._i === 0).map((c) => c.card));
  const r = await other.tool('place_card', { card: labels[0], spot: 0 });
  check(r.ok, 'and the card can be moved by someone else', r.result);
  await done(ps);
});

await scenario('left alone, then joined again', async () => {
  const room = `alone-${Date.now().toString(36)}`;
  const ps = await table(room, ['Ana', 'Ben']);
  await ps[1].page.close();
  check(await until(async () => /waiting|reconnecting/i.test(await ps[0].status()), 10000), 'the one left behind is told they are on their own', (await ps[0].status()).trim());
  await ps[1].open('');
  check(await until(async () => !/waiting|reconnecting/i.test(await ps[0].status()), 15000), 'and the notice clears when someone is back', (await ps[0].status()).trim());
  check(await until(() => agree(ps)), 'with the table in agreement');
  await done(ps);
});

check(errors.length === 0, 'no page errors', errors.slice(0, 3).join(' | '));
await browser.close();
await srv.close();
console.log(fails.length ? `\n${fails.length} failed` : '\nall passed');
process.exit(fails.length ? 1 : 0);
