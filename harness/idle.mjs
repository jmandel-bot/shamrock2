// What an idle table costs: three players in the middle of guessing, nobody
// touching anything. Measures CPU per page (Chrome's own task accounting),
// CPU of every browser process (the OS's), and frames run and painted; then
// the same during a card drag, so idling doesn't cost smoothness; then a page
// whose animation frames never fire (as in a background tab), to check it
// still talks to the table.
//
//   node harness/idle.mjs            (IDLE_S=20 sets the idle window)
import { chromium } from 'playwright';
import { readFileSync, readdirSync } from 'node:fs';
import { serve } from './serve.mjs';

const PORT = 8931, RELAY = 8932, room = `idle-${Date.now().toString(36)}`;
const IDLE_S = Number(process.env.IDLE_S ?? 20);
const srv = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? undefined, args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 30000) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { if (await fn()) return true; await sleep(150); } return false; };

// Counts animation frames run and canvas clears (one per paint), whichever
// build is under test. `noFrames` makes animation frames never fire.
const counters = (noFrames) => {
  window.__rafs = 0; window.__paints = 0;
  const raf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = noFrames ? () => 0 : (cb) => raf((t) => { window.__rafs++; cb(t); });
  const fill = CanvasRenderingContext2D.prototype.fillRect;
  CanvasRenderingContext2D.prototype.fillRect = function (...a) { window.__paints++; return fill.apply(this, a); };
  localStorage.setItem('shamrock:tip:guess', '1'); localStorage.setItem('shamrock:tip:write', '1'); localStorage.setItem('shamrock:name', '');
};

async function player(name, noFrames = false) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3 });
  await ctx.addInitScript(counters, noFrames);
  const page = await ctx.newPage();
  page.on('pageerror', (e) => console.log(`${name}: ${e.message}`));
  await page.goto(`http://localhost:${PORT}/?signal=ws://localhost:${RELAY}&fresh#${room}`);
  await page.waitForFunction(() => window.__cl?.tools);
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Performance.enable');
  const tool = (n, a = {}) => page.evaluate(([n, a]) => window.__cl.tools[n](a), [n, a]);
  return { name, page, cdp, tool, table: () => tool('get_table') };
}

// CPU seconds of this process's descendants (the browser and its helpers).
function treeCpu() {
  const kids = new Map();
  for (const d of readdirSync('/proc').filter((d) => /^\d+$/.test(d))) {
    try {
      const f = readFileSync(`/proc/${d}/stat`, 'utf8').slice(readFileSync(`/proc/${d}/stat`, 'utf8').lastIndexOf(')') + 2).split(' ');
      kids.set(Number(d), { ppid: Number(f[1]), cpu: (Number(f[11]) + Number(f[12])) / 100 });
    } catch {}
  }
  let total = 0;
  const walk = (pid) => { for (const [p, k] of kids) if (k.ppid === pid) { total += k.cpu; walk(p); } };
  walk(process.pid);
  return total;
}

const metrics = async (p) => {
  const { metrics } = await p.cdp.send('Performance.getMetrics');
  const m = Object.fromEntries(metrics.map((x) => [x.name, x.value]));
  const c = await p.page.evaluate(() => ({ rafs: window.__rafs, paints: window.__paints, sent: window.__cl.stats.sentPackets }));
  return { task: m.TaskDuration, script: m.ScriptDuration, ...c };
};
const diff = (a, b) => Object.fromEntries(Object.keys(a).map((k) => [k, b[k] - a[k]]));

const P = [];
for (const n of ['Ana', 'Ben', 'Cy']) P.push(await player(n));
const out = {};
try {
  if (!(await until(async () => (await Promise.all(P.map((p) => p.page.evaluate(() => window.__cl.peerCount())))).every((n) => n === 2), 45000))) throw new Error('pages did not connect');
  for (const p of P) await p.tool('join_table', { name: p.name });
  await until(async () => (await Promise.all(P.map((p) => p.table()))).every((t) => t.players.length === 3));
  await P[0].tool('start_round');
  await until(async () => (await Promise.all(P.map((p) => p.table()))).every((t) => t.phase === 'writing'));
  for (const p of P) {
    for (const leaf of ['top', 'right', 'bottom', 'left']) await p.tool('write_clue', { leaf, text: `${p.name} ${leaf}` });
    await p.tool('set_decoys', { count: 1 });
    await p.tool('submit_clover');
  }
  if (!(await until(async () => (await Promise.all(P.map((p) => p.table()))).every((t) => t.phase === 'guessing')))) throw new Error('guessing did not start');
  await sleep(3000);

  // Idle.
  const a = await Promise.all(P.map(metrics)), cpu0 = treeCpu();
  await sleep(IDLE_S * 1000);
  const b = await Promise.all(P.map(metrics)), cpu1 = treeCpu();
  const per = a.map((x, i) => diff(x, b[i]));
  const avg = (k) => per.reduce((s, d) => s + d[k], 0) / per.length;
  out.idle = {
    seconds: IDLE_S,
    pageCpuPct: +(100 * avg('task') / IDLE_S).toFixed(2),
    pageScriptPct: +(100 * avg('script') / IDLE_S).toFixed(2),
    browserCpuPct: +(100 * (cpu1 - cpu0) / IDLE_S).toFixed(1),
    framesPerS: +(avg('rafs') / IDLE_S).toFixed(1),
    paintsPerS: +(avg('paints') / IDLE_S).toFixed(1),
    packetsSentPerS: +(avg('sent') / IDLE_S).toFixed(2),
  };

  // A drag: a card from the tray, around the board, over two seconds.
  const g = P[0];
  const [cx, cy] = await g.page.evaluate(() => window.__cl.card(0));
  const d0 = await metrics(g), t0 = Date.now();
  await g.page.mouse.move(cx, cy);
  await g.page.mouse.down();
  for (let k = 1; k <= 60; k++) {
    await g.page.mouse.move(195 + 120 * Math.cos(k / 10), 330 + 120 * Math.sin(k / 10));
    await sleep(33);
  }
  await g.page.mouse.up();
  const secs = (Date.now() - t0) / 1000, dd = diff(d0, await metrics(g));
  out.drag = { seconds: +secs.toFixed(1), framesPerS: +(dd.rafs / secs).toFixed(1), paintsPerS: +(dd.paints / secs).toFixed(1) };

  // A page whose animation frames never fire, as in a background tab.
  const D = await player('Dee', true);
  await until(async () => (await D.page.evaluate(() => window.__cl.peerCount())) === 3, 30000);
  await sleep(1000);
  const h0 = await metrics(D);
  await sleep(10000);
  const h1 = await metrics(D);
  const others = await Promise.all(P.map((p) => p.page.evaluate(() => window.__cl.peerCount())));
  out.noFrames = {
    seconds: 10,
    packetsSent: h1.sent - h0.sent,
    seesGuessing: (await D.table()).phase === 'guessing',
    othersSeeIt: others.every((n) => n === 3),
  };
} finally {
  console.log(JSON.stringify(out, null, 2));
  await browser.close();
  await srv.close?.();
  process.exit(0);
}
