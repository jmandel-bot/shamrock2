// Trystero 0.26.0 timestamps both announcements and subscription filters with
// the device clock. A slow clock makes ephemeral events expire at public relays.
// Keep this small, checked patch reproducible; never change Date globally.
import { readFile, realpath, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';

const source = await realpath('node_modules/@trystero-p2p/nostr/dist/index.mjs');
const code = await readFile(source, 'utf8');
const needle = 'const now = () => Math.floor(Date.now() / 1e3);';
if (code.split(needle).length !== 2) throw new Error('Review the Nostr clock patch for this Trystero version');
// Subscriptions also look back a while: discovery events are ephemeral (never
// stored), so this only stops them dropping a live event from a device whose
// clock is a few minutes behind this one's.
const since = ['since: now(),', 'const since = now();'];
if (since.some((s) => code.split(s).length !== 2)) throw new Error('Review the Nostr subscription patch for this Trystero version');
const patched = code.replace(needle, `let discoveryClock = Date.now;
export const setDiscoveryClock = (clock) => { discoveryClock = clock; };
const now = () => Math.floor(discoveryClock() / 1e3);
const SINCE_SLACK_S = 15 * 60;`)
  .replace(since[0], 'since: now() - SINCE_SLACK_S,')
  .replace(since[1], 'const since = now() - SINCE_SLACK_S;');
const temp = await mkdtemp(join(dirname(source), '.shamrock-build-'));
try {
  const entry = join(temp, 'index.mjs');
  await writeFile(entry, patched);
  execFileSync('bun', ['build', entry, '--target=browser', '--format=esm', '--minify', '--outfile', resolve('web/vendor/trystero-nostr.js')], { stdio: 'inherit' });
} finally { await rm(temp, { recursive: true, force: true }); }
