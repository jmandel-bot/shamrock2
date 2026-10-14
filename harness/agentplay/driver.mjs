// Agent play-test driver: one real browser page per player at the same table,
// each controlled through a small local HTTP API (see README.md here). Moves
// go through the page's own agent tools (web/agent.js, also offered to
// browser assistants over WebMCP), which drive the same Rust input path as
// touch; the main buttons are clicked as a person would.
//
//   node harness/agentplay/driver.mjs <room> <players> [apiBasePort]
//   e.g. node harness/agentplay/driver.mjs agent-test 3 9111
import { chromium } from 'playwright';
import http from 'node:http';
import { mkdir, appendFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { serve } from '../serve.mjs';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const [room = 'agent-test', count = '3', base = '9111'] = process.argv.slice(2);
const N = +count, BASE = +base, PORT = BASE - 10, RELAY = BASE - 9;
const OUT = here('./out/');
await mkdir(OUT, { recursive: true });
const LOG = `${OUT}log.txt`;
await rm(`${OUT}events.jsonl`, { force: true });
const log = async (who, msg) => { const line = `${new Date().toISOString().slice(11, 19)} ${who}: ${msg}`; console.log(line); await appendFile(LOG, line + '\n'); };

const srv = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({ args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] });
const url = `http://localhost:${PORT}/?signal=ws://localhost:${RELAY}#${room}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// Table talk: what the players say to each other, as on the call people have
// while they play. Shared by every player's API; not part of the game.
const talk = [];
// Everything that happens, in order, for reading the game back (events.jsonl).
const t0 = Date.now();
const event = (e) => appendFile(`${OUT}events.jsonl`, JSON.stringify({ t: Date.now() - t0, ...e }) + '\n');

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
  let shots = 0;

  // Game actions go through the page's own agent tools (web/agent.js), the
  // same ones a browser's assistant gets over WebMCP. The main and other
  // buttons are pressed as a person would.
  const tool = (name, args = {}) => ev(([n, a]) => window.__cl.tools[n](a), [name, args]);
  const said = (r) => typeof r === 'object' && 'result' in r ? r.result : JSON.stringify(r);
  const state = async () => {
    const [t, dom] = await Promise.all([tool('get_table'), ev(() => {
      const $ = (id) => document.getElementById(id);
      const btn = (id) => { const b = $(id); return b && !b.hidden ? { label: b.textContent, enabled: !b.disabled } : null; };
      return { mainButton: btn('go'), otherButton: btn('alt'), joinFormOpen: $('joinSheet').classList.contains('open'),
        toast: $('toast').classList.contains('show') ? $('toast').textContent : null };
    })]);
    return { ...t, ...dom };
  };
  const key = (s) => JSON.stringify(s);
  const me = async () => (await tool('get_table')).you ?? tag;

  const actions = {
    join: async ({ name }) => said(await tool('join_table', { name })),
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
    clue: async ({ leaf, text }) => said(await tool('write_clue', { leaf, text })),
    decoys: async ({ n }) => said(await tool('set_decoys', { count: n })),
    move: async ({ card, slot, top }) => said(await tool('place_card', { card, spot: slot, top, clover: (await tool('get_table')).clover.index })),
    turn: async ({ card, times, top }) => said(await tool('turn_card', { card, times, top, clover: (await tool('get_table')).clover.index })),
    pick: async ({ clover }) => said(await tool('choose_clover', { clover })),
    recap: async () => tool('get_recap'),
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
      // The tools as an assistant would see them: names, descriptions, input schemas.
      if (req.method === 'GET' && u.pathname === '/tools') return reply(200, await ev(() => window.__cl.toolSpecs()));
      if (req.method === 'GET' && u.pathname === '/state') {
        // ?wait=N: hold up to N seconds until something changes since ?since=<etag>
        // (the table, or table talk). ?talk=K: return what was said from entry K on.
        const wait = Math.min(+(u.searchParams.get('wait') ?? 0), 60), since = u.searchParams.get('since');
        const heard = +(u.searchParams.get('talk') ?? talk.length);
        const start = Date.now();
        const look = async () => { const st = await state(); return [st, String(hash(key(st) + talk.length))]; };
        let [s, etag] = await look();
        while (wait && since && etag === since && Date.now() - start < wait * 1000) { await sleep(400); [s, etag] = await look(); }
        return reply(200, { etag, ...s, talk: talk.slice(heard), talkCount: talk.length });
      }
      // Table talk: POST /say {text}.
      if (req.method === 'POST' && u.pathname === '/say') {
        let body = ''; for await (const c of req) body += c;
        const text = String(JSON.parse(body || '{}').text ?? '').slice(0, 600);
        const who = await me();
        talk.push({ who, text });
        await log(tag, `says: ${text}`);
        await event({ who, kind: 'say', text });
        return reply(200, { result: 'said', talkCount: talk.length });
      }
      // Any agent tool by name: POST /tool/<name> with its arguments.
      if (req.method === 'POST' && u.pathname.startsWith('/tool/')) {
        let body = ''; for await (const c of req) body += c;
        const name = u.pathname.slice(6), args = body ? JSON.parse(body) : {};
        const result = await tool(name, args);
        await log(tag, `${u.pathname.slice(1)} ${body} -> ${said(result)}`);
        if (name !== 'get_table') {
          const t = await tool('get_table');
          await event({ who: t.you ?? tag, kind: 'tool', name, args, ok: result?.ok !== false, result: said(result), clover: t.clover?.index, phase: t.phase });
        }
        return reply(200, { result });
      }
      if (req.method === 'POST' && actions[u.pathname.slice(1)]) {
        let body = ''; for await (const c of req) body += c;
        const args = body ? JSON.parse(body) : {};
        const result = await actions[u.pathname.slice(1)](args);
        await log(tag, `${u.pathname.slice(1)} ${body} -> ${result}`);
        return reply(200, { result });
      }
      reply(404, { error: 'GET /state[?wait=N&since=ETAG], GET /tools, or POST /join /go /other /clue /decoys /move /turn /pick /recap /screenshot /tool/<name>' });
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
