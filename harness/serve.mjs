// Static server for web/ plus a self-hosted Trystero WebSocket signaling relay.
// Only signaling goes through the relay; game data flows over WebRTC.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createWsRelayServer } from '@trystero-p2p/ws-relay/server';

const root = fileURLToPath(new URL('../web/', import.meta.url));
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css' };

export async function serve({ port = 8787, relayPort = 8788, quiet = false } = {}) {
  const http = createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(/^(\.\.[/\\])+/, '');
    const file = join(root, path.endsWith('/') ? `${path}index.html` : path);
    try {
      const body = await readFile(file);
      res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store' });
      res.end(body);
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  await new Promise((r) => http.listen(port, '127.0.0.1', r));
  let relay = createWsRelayServer({ port: relayPort, host: '127.0.0.1' });
  await relay.ready;
  if (!quiet) {
    console.log(`game:   http://localhost:${port}/?signal=ws://localhost:${relayPort}#myroom`);
    console.log(`        (open in several windows; omit ?signal=… to use public Nostr relays instead)`);
  }
  return {
    close: async () => { http.close(); await relay?.close(); },
    // For fault tests: the signaling relay goes down (players already
    // connected keep playing over WebRTC; nobody new can find the table).
    stopRelay: async () => { await relay?.close(); relay = null; },
    startRelay: async () => { relay = createWsRelayServer({ port: relayPort, host: '127.0.0.1' }); await relay.ready; },
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  serve({ port: +(process.env.PORT ?? 8787), relayPort: +(process.env.RELAY_PORT ?? 8788) });
}
