// Agent play-test driver: one real browser page per player at the same table,
// each controlled through a small local HTTP API (see README.md here). Every
// action goes through the real interface (taps, typing, drags, buttons), so
// automated players exercise the game the way people do.
//
//   node harness/agentplay/driver.mjs <room> <players> [apiBasePort]
//   e.g. node harness/agentplay/driver.mjs agent-test 3 9111
import { chromium } from 'playwright';
import http from 'node:http';
import { mkdir, appendFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { serve } from '../serve.mjs';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const [room = 'agent-test', count = '3', base = '9111'] = process.argv.slice(2);
const N = +count, BASE = +base, PORT = BASE - 10, RELAY = BASE - 9;
const OUT = here('./out/');
await mkdir(OUT, { recursive: true });
const LOG = `${OUT}log.txt`;
const log = async (who, msg) => { const line = `${new Date().toISOString().slice(11, 19)} ${who}: ${msg}`; console.log(line); await appendFile(LOG, line + '\n'); };

const srv = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({ args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] });
const url = `http://localhost:${PORT}/?signal=ws://localhost:${RELAY}#${room}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function player(k) {
  const ctx = await browser.newContext({ viewport: { width: 900, height: 980 } });
  // Skip one-time tips so toasts don't cover the board in screenshots.
  await ctx.addInitScript(() => { localStorage.setItem('shamrock:tip:guess', '1'); localStorage.setItem('shamrock:tip:write', '1'); });
  const page = await ctx.newPage();
  const tag = `P${k + 1}`;
  page.on('pageerror', (e) => log(tag, `PAGE ERROR ${e.message}`));
  page.on('console', (m) => { if (m.type() === 'error') log(tag, `console error ${m.text()}`); });
  await page.goto(url);
  await page.waitForFunction(() => window.__cl?.ui);
  const ev = (fn, a) => page.evaluate(fn, a);
  const origin = () => ev(() => { const r = document.getElementById('c').getBoundingClientRect(); return [r.left, r.top]; });
  const at = async (xy) => { const o = await origin(); return [o[0] + xy[0], o[1] + xy[1]]; };
  let shots = 0;

  const state = async () => ev(() => {
    const u = window.__cl.ui(), $ = (id) => document.getElementById(id);
    const btn = (id) => { const b = $(id); return b && !b.hidden ? { label: b.textContent, enabled: !b.disabled } : null; };
    return {
      phase: u.phase, round: u.round, seated: u.seated, me: u.name ?? null,
      players: u.players.map((p) => ({ name: p.name, status: p.status, away: p.away, you: p.you })),
      cloverIndex: u.k, cloverCount: u.n, writerName: u.writerName ?? null, isMyClover: !!u.isWriter, solved: !!u.done,
      decoys: u.decoys, readyCount: u.readyCount,
      statusLine: $('hint').textContent, mainButton: btn('go'), otherButton: btn('alt'), sideButton: btn('side'),
      joinFormOpen: $('joinSheet').classList.contains('open'), toast: $('toast').classList.contains('show') ? $('toast').textContent : null,
      clover: window.__cl.agentView(),
    };
  });
  const key = (s) => JSON.stringify(s);

  const actions = {
    async join({ name }) {
      await page.waitForSelector('#joinSheet.open', { timeout: 5000 }).catch(() => {});
      await page.fill('#nameInput', name);
      await page.click('#joinBtn');
      return `joined as ${name}`;
    },
    async go() {
      const b = await page.$('#go');
      if (!b || await b.isHidden() || await b.isDisabled()) return 'main button is hidden or disabled';
      const label = await b.textContent();
      await b.click();
      return `pressed "${label}"`;
    },
    async other() {
      const b = await page.$('#alt');
      if (!b || await b.isHidden()) return 'no other button';
      const label = await b.textContent();
      await b.click();
      return `pressed "${label}"`;
    },
    async clue({ leaf, text }) {
      const d = ['top', 'right', 'bottom', 'left'].indexOf(leaf);
      if (d < 0) return 'leaf must be top, right, bottom or left';
      await ev(() => window.__cl.settle());
      const [x, y] = await at((await ev((d) => window.__cl.petal(d), d)).slice(0, 2));
      await page.mouse.click(x, y);
      await page.waitForSelector('#clue', { state: 'visible', timeout: 3000 });
      await page.fill('#clue', text);
      await page.keyboard.press('Enter');
      await sleep(150);
      // Enter moves on to the next empty leaf; close that without changes.
      if (await page.isVisible('#clue')) await page.keyboard.press('Escape');
      return `wrote "${text}" on the ${leaf} leaf`;
    },
    async decoys({ n }) {
      for (let i = 0; i < 8; i++) {
        const cur = (await ev(() => window.__cl.ui().decoys));
        if (cur === n) return `decoys set to ${n}`;
        const id = cur < n ? '#decUp' : '#decDown';
        if (!(await page.isVisible(id))) return 'decoy buttons are not shown (write all four clues first)';
        await page.click(id);
      }
      return `decoys now ${await ev(() => window.__cl.ui().decoys)}`;
    },
    async move({ card, slot }) {
      await ev(() => window.__cl.settle());
      const from = await at((await ev((i) => window.__cl.card(i), card)).slice(0, 2));
      let to;
      if (slot === 'tray') {
        const r = await ev(() => { const t = window.__cl.X; return null; });
        const box = await page.evaluate(() => { const c = document.getElementById('c').getBoundingClientRect(); return [c.left + c.width * 0.5, c.top + c.height - 60]; });
        to = box;
      } else to = await at((await ev((s) => window.__cl.slot(s), slot)).slice(0, 2));
      const m = page.mouse;
      await m.move(from[0], from[1]); await m.down();
      for (let s = 1; s <= 14; s++) await m.move(from[0] + (to[0] - from[0]) * s / 14, from[1] + (to[1] - from[1]) * s / 14);
      await m.up();
      await sleep(250);
      const where = (await ev(() => window.__cl.agentView())).cards[card]?.at;
      return `card ${card} is now at ${where}`;
    },
    async turn({ card, times = 1 }) {
      for (let t = 0; t < times; t++) {
        await ev(() => window.__cl.settle());
        const [x, y] = await at((await ev((i) => window.__cl.card(i), card)).slice(0, 2));
        await page.mouse.click(x, y);
        await sleep(120);
      }
      const c = (await ev(() => window.__cl.agentView())).cards[card];
      return `card ${card} now reads top/right/bottom/left: ${c?.words.join(' / ')}`;
    },
    async pick({ clover }) {
      if (!(await page.$('#hint.pick'))) return 'clover picker is only available while guessing';
      await page.click('#hint');
      await page.waitForSelector('#cloverSheet.open');
      await page.click(`#cloverList .clover-row:nth-child(${clover + 1})`);
      return `picked clover ${clover}`;
    },
    async screenshot() {
      const path = `${OUT}${tag}-${String(++shots).padStart(2, '0')}.png`;
      await page.screenshot({ path });
      return path;
    },
  };

  http.createServer(async (req, res) => {
    const u = new URL(req.url, 'http://x');
    const reply = (code, body) => { res.writeHead(code, { 'content-type': 'application/json' }); res.end(JSON.stringify(body, null, 1)); };
    try {
      if (req.method === 'GET' && u.pathname === '/state') {
        // ?wait=N: hold up to N seconds until something changes since ?since=<etag>.
        const wait = Math.min(+(u.searchParams.get('wait') ?? 0), 60), since = u.searchParams.get('since');
        const t0 = Date.now();
        let s = await state();
        while (wait && since && String(hash(key(s))) === since && Date.now() - t0 < wait * 1000) { await sleep(400); s = await state(); }
        return reply(200, { etag: String(hash(key(s))), ...s });
      }
      if (req.method === 'POST' && actions[u.pathname.slice(1)]) {
        let body = ''; for await (const c of req) body += c;
        const args = body ? JSON.parse(body) : {};
        const result = await actions[u.pathname.slice(1)](args);
        await log(tag, `${u.pathname.slice(1)} ${body} -> ${result}`);
        return reply(200, { result });
      }
      reply(404, { error: 'GET /state[?wait=N&since=ETAG] or POST /join /go /other /clue /decoys /move /turn /pick /screenshot' });
    } catch (e) {
      await log(tag, `ERROR ${u.pathname}: ${e.message}`);
      reply(500, { error: e.message.split('\n')[0] });
    }
  }).listen(BASE + k, '127.0.0.1');
  await log(tag, `API on http://127.0.0.1:${BASE + k}`);
}
const hash = (s) => { let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0; return h >>> 0; };

for (let k = 0; k < N; k++) await player(k);
await log('driver', `table ${room} ready with ${N} pages; game at ${url}`);
process.on('SIGTERM', async () => { await browser.close(); await srv.close(); process.exit(0); });
