// An integration test with real models: one Claude Code session per player,
// each given only that player's page tools over MCP (mcp-bridge.mjs), the
// way a browser assistant would get them over WebMCP. No shell, no files, no
// tactics guide: the rules come from the tool descriptions. Costs a few
// dollars of model time, so it isn't part of run.mjs.
//
//   node harness/agentplay/mcp-game.mjs [players=3] [model=opus] [apiBasePort=9181]
//   node harness/agentplay/mcp-game.mjs Name:model,Name:model,… [apiBasePort]
//     (a mixed table: each player its own name and model)
//
// Needs the `claude` CLI, signed in. Writes each session's transcript and
// report to out/, and prints how the round went: clovers solved and guesses,
// tool calls per player, and tool errors.
import { spawn } from 'node:child_process';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const argv = process.argv.slice(2);
const mixed = argv[0]?.includes(':');
const [count = '3', model = 'opus', base = '9181'] = mixed ? [null, null, argv[1]] : argv;
const SEATS = mixed
  ? argv[0].split(',').map((p) => { const [name, m] = p.split(':'); return { name, model: m }; })
  : ['Ana', 'Ben', 'Cy', 'Dee', 'Eve', 'Fay'].slice(0, +count).map((name) => ({ name, model }));
const N = SEATS.length, BASE = +(base ?? 9181), NAMES = SEATS.map((p) => p.name);
const OUT = here('./out/');
await mkdir(OUT, { recursive: true });
const room = `mcp-${Date.now().toString(36)}`;

const driver = spawn('node', [here('./driver.mjs'), room, String(N), String(BASE)], { stdio: ['ignore', 'pipe', 'inherit'] });
await new Promise((ok, fail) => {
  driver.stdout.on('data', (d) => { if (String(d).includes('ready with')) ok(); });
  driver.on('exit', () => fail(new Error('driver exited')));
});
console.log(`table ${room} ready; starting ${SEATS.map((p) => `${p.name} (${p.model})`).join(', ')}`);

const prompt = (name) => `You are ${name}, one of ${N} players (${NAMES.join(', ')}) at a Shamrock table. Your only way to act is the shamrock tools; read their descriptions for the rules. Join the table as ${name}. ${NAMES[0]} starts the round once all ${N} are seated. Write your own clover well (clues your teammates can decode first try), then guess the others' clovers carefully. While others are writing or guessing, call wait_for_change rather than polling. For a clover that isn't yours, the guesser whose name comes first alphabetically (of ${[...NAMES].sort().join(', ')}) moves the cards and checks; the others wait unless nothing has changed for about 3 minutes. Keep playing until the phase is end. Then reply with a short report: your clues, how each clover you guessed went, and anything about the tools that was confusing, missing or wrong.`;

const sessions = NAMES.map(async (name, k) => {
  const cfg = `${OUT}mcp-${name}.json`;
  await writeFile(cfg, JSON.stringify({ mcpServers: { shamrock: { command: 'node', args: [here('./mcp-bridge.mjs'), String(BASE + k)] } } }));
  const args = ['-p', prompt(name), '--model', SEATS[k].model, '--tools', '', '--strict-mcp-config', '--mcp-config', cfg,
    '--allowedTools', 'mcp__shamrock', '--output-format', 'stream-json', '--verbose'];
  const child = spawn('claude', args, { cwd: OUT, stdio: ['ignore', 'pipe', 'pipe'] });
  let out = '';
  child.stdout.on('data', (d) => { out += d; });
  const timer = setTimeout(() => child.kill(), 40 * 60_000);
  await new Promise((ok) => child.on('exit', ok));
  clearTimeout(timer);
  await writeFile(`${OUT}mcp-${name}.jsonl`, out);
  // Tally the transcript: tool calls, tool errors, the final report.
  const calls = {}; let errors = 0, result = null;
  for (const line of out.split('\n')) {
    let m; try { m = JSON.parse(line); } catch { continue; }
    for (const c of Array.isArray(m.message?.content) ? m.message.content : []) {
      if (m.type === 'assistant' && c.type === 'tool_use') { const t = c.name.split('__').pop(); calls[t] = (calls[t] ?? 0) + 1; }
      if (m.type === 'user' && c.type === 'tool_result' && c.is_error) errors++;
    }
    if (m.type === 'result') result = m;
  }
  await writeFile(`${OUT}mcp-${name}-report.md`, result?.result ?? '(no report)');
  return { name, model: SEATS[k].model, calls, errors, turns: result?.num_turns, usd: result?.total_cost_usd, report: result?.result ?? '(no report)' };
});
const players = await Promise.all(sessions);

const recap = await fetch(`http://127.0.0.1:${BASE}/recap`, { method: 'POST' }).then((r) => r.json()).then((j) => j.result).catch(() => null);
driver.kill('SIGTERM');

console.log('\n# clovers');
if (Array.isArray(recap)) for (const c of recap) console.log(`${c.solved ? 'solved' : 'NOT solved'} in ${c.guesses}: ${c.writer} — ${Object.values(c.clues).map((x) => `${x.clue} (${x.linked.join(' + ')})`).join(', ')}`);
else console.log(`no recap: ${JSON.stringify(recap)}`);
console.log('\n# players');
for (const p of players) console.log(`${p.name} (${p.model}): ${p.turns} turns, $${p.usd?.toFixed(2)}, ${p.errors} tool errors, calls ${JSON.stringify(p.calls)}`);
console.log(`\nReports: ${OUT}mcp-<name>-report.md; driver log: ${OUT}log.txt`);
const ok = Array.isArray(recap) && recap.length === N && recap.every((c) => c.solved);
console.log(ok ? '\nevery clover solved' : '\nround not completed');
process.exit(ok ? 0 : 1);
