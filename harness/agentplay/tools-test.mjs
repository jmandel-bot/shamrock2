// Plays a whole three-player round through the agent tools alone (web/agent.js,
// the WebMCP surface), then checks the recap. Exits non-zero on failure.
//
//   node harness/agentplay/tools-test.mjs
import { chromium } from 'playwright';
import { serve } from '../serve.mjs';

const PORT = 8921, RELAY = 8922, room = `tools-${Date.now().toString(36)}`;
const srv = await serve({ port: PORT, relayPort: RELAY, quiet: true });
const browser = await chromium.launch({ args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] });
const fails = [];
const check = (ok, what, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'} ${what}${ok ? '' : ` — ${detail}`}`); if (!ok) fails.push(what); };
const until = async (fn, ms = 15000) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { if (await fn()) return true; await new Promise((r) => setTimeout(r, 150)); } return false; };

const names = ['Ana', 'Ben', 'Cy'];
const P = [];
for (const name of names) {
  const ctx = await browser.newContext({ viewport: { width: 900, height: 980 } });
  await ctx.addInitScript(() => { localStorage.setItem('shamrock:tip:guess', '1'); localStorage.setItem('shamrock:tip:write', '1'); localStorage.setItem('shamrock:name', ''); });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => fails.push(`${name}: ${e.message}`));
  await page.goto(`http://localhost:${PORT}/?signal=ws://localhost:${RELAY}&fresh#${room}`);
  await page.waitForFunction(() => window.__cl?.tools);
  const tool = (n, a = {}) => page.evaluate(([n, a]) => window.__cl.tools[n](a), [n, a]);
  P.push({ name, page, tool, table: () => tool('get_table') });
}
try {
  const listed = await P[0].page.evaluate(() => Object.keys(window.__cl.tools));
  check(listed.length === 12 && listed.includes('get_table') && listed.includes('place_card'), 'the page offers its twelve tools', listed.join(','));

  // Let every page find the other two before anyone sits down.
  check(await until(async () => (await Promise.all(P.map((p) => p.page.evaluate(() => window.__cl.peerCount())))).every((n) => n === 2), 45000), 'the three pages connect');
  for (const p of P) await p.tool('join_table', { name: p.name });
  check(await until(async () => (await Promise.all(P.map((p) => p.table()))).every((t) => t.players.length === 3), 30000), 'three players seated through join_table');
  check((await P[1].tool('start_round')).ok !== false, 'start_round deals');
  check(await until(async () => (await Promise.all(P.map((p) => p.table()))).every((t) => t.phase === 'writing')), 'everyone is writing');

  // Answers stay hidden mid-round.
  check((await P[0].tool('get_recap')).ok === false, 'get_recap refuses before the round ends');

  for (const p of P) {
    const t = await p.table();
    check(Object.values(t.clover.leafWords).every((w) => / & /.test(w)), `${p.name} sees the two words on each leaf`, JSON.stringify(t.clover.leafWords));
    check(t.clover.solvedSpots.length === 0 && t.clover.cards.every((c) => typeof c.words.top === 'string' && typeof c.words.left === 'string'),
      `${p.name}: no solved spots while writing, and card words named by edge`, JSON.stringify(t.clover.cards[0]));
    for (const leaf of ['top', 'right', 'bottom', 'left']) await p.tool('write_clue', { leaf, text: `${p.name.toLowerCase()} ${leaf}` });
    await p.tool('set_decoys', { count: 1 });
    const r = await p.tool('submit_clover');
    check(r.ok, `${p.name} submits through submit_clover`, r.result);
  }
  check(await until(async () => (await P[0].table()).phase === 'guessing'), 'guessing begins once all submit');

  // Guess every clover: put the answer cards in their spots, then turn the
  // ones that bounce until they lock. (The test peeks at which card is which;
  // the tools themselves never say.)
  for (let k = 0; k < 3; k++) {
    await until(async () => (await P[0].table()).clover.index === k);
    if ((await P[0].table()).phase !== 'guessing') break;
    const t0 = await P[0].table();
    const g = P.find((p) => p.name !== t0.clover.writer);
    const labels = await g.page.evaluate(() => window.__cl.agentView().cards.filter((c) => c._i < 4).sort((a, b) => a._i - b._i).map((c) => c.card));
    const leaked = JSON.stringify(await g.table()).includes('"_i"');
    check(!leaked, 'get_table never exposes internal card order');
    if (k === 0) {
      // Placing with the word to put on top, twice (two players carrying out
      // the same plan): the card ends up turned once, not twice.
      const deck = (await g.table()).clover.cards.find((c) => c.card === labels[3]);
      const top = deck.words.left;
      const a = await g.tool('place_card', { card: labels[3], spot: 2, clover: k, top });
      const b = await g.tool('place_card', { card: labels[3], spot: 2, clover: k, top });
      const now = (await g.table()).clover.cards.find((c) => c.card === labels[3]);
      check(a.ok && b.ok && now.words.top === top && now.at === 'slot 2', 'place_card with top is safe to repeat: placed and turned once', `${a.result} | ${b.result}`);
      check(now.movedBy === g.name, 'each card says who last moved it', String(now.movedBy));
      await g.tool('place_card', { card: labels[3], spot: 'tray', clover: k });
    }
    for (let round = 0; round < 5; round++) {
      const t = await g.table();
      if (t.clover.solved) break;
      for (let s = 0; s < 4; s++) {
        if (t.clover.solvedSpots.includes(s)) continue;
        // A card that bounced comes back unturned, so turn it once more each try.
        if (round > 0) await g.tool('turn_card', { card: labels[s], times: round, clover: t.clover.index });
        const r = await g.tool('place_card', { card: labels[s], spot: s, clover: t.clover.index });
        if (!r.ok) check(false, 'place_card puts a card in a spot', r.result);
      }
      await g.tool('check_guess', { clover: t.clover.index });
      await until(async () => (await g.table()).clover.solvedSpots.length > t.clover.solvedSpots.length || (await g.table()).clover.solved, 3000);
    }
    const t = await g.table();
    check(t.clover.solved, `clover ${k + 1} (${t.clover.writer}'s) solved through place_card, turn_card and check_guess`, JSON.stringify(t.clover.solvedSpots));
    if (k === 0) {
      // Checking a clover someone just solved says so, rather than failing.
      const again = await g.tool('check_guess', { clover: t.clover.index });
      check(again.ok && /already solved/.test(again.result), 'checking a clover that was just solved says so', again.result);
      // Two players move on from the same clover: the second finds it done.
      const other = P.find((p) => p !== g);
      const first = await other.tool('next_clover', { from: t.clover.index });
      await until(async () => (await g.table()).clover.index !== t.clover.index);
      const second = await g.tool('next_clover', { from: t.clover.index });
      check(first.ok && second.ok && /already moved the table on/.test(second.result), 'next_clover after someone else moved on reports success, not "not solved"', `${first.result} | ${second.result}`);
      // A move still meant for the clover just left is refused, and moves nothing.
      const before = JSON.stringify((await g.table()).clover.cards);
      const late = await g.tool('turn_card', { card: labels[0], times: 1, clover: t.clover.index });
      check(!late.ok && /moved on/.test(late.result) && JSON.stringify((await g.table()).clover.cards) === before, 'a move for a clover the table has left is refused and changes nothing', late.result);
    } else {
      await g.tool('next_clover', { from: t.clover.index });
    }
  }
  check(await until(async () => (await P[0].table()).phase === 'end'), 'the round ends');
  const recap = await P[2].tool('get_recap');
  check(Array.isArray(recap) && recap.length === 3 && recap.every((c) => c.solved && c.answer.length === 4 && c.clues.top.clue.endsWith(' top') && c.clues.top.linked.every((w) => typeof w === 'string') && c.decoys.length === 1),
    'get_recap returns every clover with clues, answers and decoys', JSON.stringify(recap).slice(0, 300));

  // The recap sheet opens by itself, one clover after another, no expanders.
  const sheet = await P[0].page.waitForSelector('#recapSheet.open', { timeout: 5000 }).then(() => true, () => false);
  const rows = await P[0].page.$$eval('#recapList .mini', (els) => els.map((e) => e.querySelector('.who').textContent));
  check(sheet && rows.length === 3, 'the recap opens at the end of the round with every clover', rows.join(' | '));
  const overflow = await P[0].page.evaluate(() => { const l = document.getElementById('recapList'); return l.scrollHeight > l.clientHeight; });
  check(overflow, 'the recap list scrolls clover to clover');
  await P[0].page.screenshot({ path: new URL('./out/tools-recap.png', import.meta.url).pathname });
  await P[0].page.click('#recapClose');
  await P[0].page.click('#hint');
  check(await P[0].page.waitForSelector('#recapSheet.open', { timeout: 3000 }).then(() => true, () => false), 'tapping the status reopens the recap');

  // Every page goes away; the table comes back from the device's own save.
  {
    for (const p of P) await p.page.close({ runBeforeUnload: true });
    const again = await P[0].page.context().newPage();
    await again.goto(`http://localhost:${PORT}/?signal=ws://localhost:${RELAY}#${room}`);
    await again.waitForFunction(() => window.__cl?.tools);
    const t = await again.evaluate(() => window.__cl.tools.get_table({}));
    check(t.phase === 'end' && t.round === 1 && t.players.length === 3, 'with every page closed, a reopened page restores the table from this device', JSON.stringify({ phase: t.phase, round: t.round, players: t.players.length }));
  }
} finally {
  await browser.close();
  await srv.close();
}
console.log(fails.length ? `\n${fails.length} failed` : '\nall passed');
process.exit(fails.length ? 1 : 0);
