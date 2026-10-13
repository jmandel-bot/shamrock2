// Minimal MCP stdio server exposing one Shamrock player's page tools (as a
// browser's WebMCP would), via the agent play-test driver's HTTP API.
//   node harness/agentplay/mcp-bridge.mjs <driverPlayerPort>
// mcp-game.mjs starts the driver and one Claude session per player on it.
import readline from 'node:readline';
const base = `http://127.0.0.1:${process.argv[2]}`;
const log = (m) => process.stderr.write(`[bridge] ${m}\n`);
const send = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');
const specs = await (await fetch(`${base}/tools`)).json();
// Not a page tool: a long-poll so a waiting player needn't spin on get_table.
const WAIT = { name: 'wait_for_change', description: 'Wait up to `seconds` (max 50) until something at the table changes (another player acts), then return the new table state like get_table. Use this instead of polling while others write or guess.',
  inputSchema: { type: 'object', properties: { seconds: { type: 'integer', minimum: 1, maximum: 50, default: 40 } } } };
let etag = null;
async function call(name, args) {
  if (name === 'wait_for_change') {
    const r = await (await fetch(`${base}/state?wait=${Math.min(50, args.seconds ?? 40)}${etag ? `&since=${etag}` : ''}`)).json();
    etag = r.etag; delete r.etag; delete r.mainButton; delete r.otherButton; delete r.joinFormOpen; delete r.toast;
    return r;
  }
  const r = await fetch(`${base}/tool/${name}`, { method: 'POST', body: JSON.stringify(args ?? {}) });
  const j = await r.json();
  if (name === 'get_table') { const s = await (await fetch(`${base}/state`)).json(); etag = s.etag; }
  return j.result ?? j;
}
const rl = readline.createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  let m; try { m = JSON.parse(line); } catch { return; }
  const reply = (result) => send({ jsonrpc: '2.0', id: m.id, result });
  try {
    if (m.method === 'initialize') return reply({ protocolVersion: m.params?.protocolVersion ?? '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'shamrock', version: '0.1' } });
    if (m.method === 'tools/list') return reply({ tools: [...specs, WAIT] });
    if (m.method === 'tools/call') {
      const out = await call(m.params.name, m.params.arguments ?? {});
      log(`${m.params.name} ${JSON.stringify(m.params.arguments ?? {})} -> ${JSON.stringify(out).slice(0, 160)}`);
      return reply({ content: [{ type: 'text', text: JSON.stringify(out) }], isError: out?.ok === false });
    }
    if (m.method === 'ping') return reply({});
    if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: `no method ${m.method}` } });
  } catch (e) { if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: String(e.message) } }); }
});
