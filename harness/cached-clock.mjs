// Two phones whose copies of network.json came from a CDN cache, each with its
// own Age. Public relays accept events stamped minutes off (relay.primal.net
// and relay.snort.social took ones 6½ minutes ahead), but every subscription
// drops events stamped before its own `since`. So when the discovery clocks
// disagree, the device that is ahead never hears the other, and nobody meets.
// Field reports (Nov 2026) showed an "offset" growing 1 s per second and
// resetting: GitHub Pages' Age, added to a Date that is already the time served.
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { serve } from './serve.mjs';
import { serveNostr } from './nostr-relay.mjs';
import { randomUUID } from 'node:crypto';

const server = await serve({ port: 8877, relayPort: 8878, quiet: true });
const relay = await serveNostr({ port: 8879, maxClockSkewMs: 15 * 60000 });
const chrome = await chromium.launch({ executablePath: process.env.CHROMIUM ?? undefined });
const http = (ms) => new Date(ms).toUTCString();
// How the cache in front of the site reports its copy's age.
const caches = {
  'GitHub Pages (Date is when it was served)': (age) => ({ date: http(Date.now()), age: String(age) }),
  'a cache that keeps the origin Date': (age) => ({ date: http(Date.now() - age * 1000), age: String(age) }),
};
let failed = 0;
try {
  for (const [name, headers] of Object.entries(caches)) {
    const room = randomUUID(), contexts = [];
    try {
      const pages = await Promise.all([0, 400].map(async (age) => {
        const context = await chrome.newContext(); contexts.push(context);
        await context.route(/network\.json/, async (route) => {
          const res = await route.fetch();
          await route.fulfill({ response: res, headers: { ...res.headers(), ...headers(age), 'cache-control': 'max-age=600' } });
        });
        const page = await context.newPage();
        await page.goto(`http://localhost:8877/?relays=ws://localhost:8879#${room}`);
        await page.waitForFunction(() => window.__cl);
        return page;
      }));
      const met = await Promise.all(pages.map((p) => p.waitForFunction(() => __cl.peerCount() === 1, null, { timeout: 20000 }).then(() => true, () => false)));
      const offsets = await Promise.all(pages.map((p) => p.evaluate(async () => (await __cl.connectionReport()).current.settings.clockOffsetMs)));
      const ok = met.every(Boolean);
      if (!ok) failed++;
      console.log(`${ok ? 'PASS' : 'FAIL'}  copies 0 s and 400 s old, ${name}: ${ok ? 'they meet' : 'they never meet'} (clock offsets ${offsets.map((o) => `${o / 1000}s`).join(', ')})`);
    } finally { await Promise.all(contexts.map((c) => c.close())); }
  }
} finally {
  await chrome.close(); await relay.close(); await server.close();
}
assert.equal(failed, 0);
