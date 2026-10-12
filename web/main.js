// Platform shim: everything here is plumbing. Game rules, sync, geometry,
// hit-testing and drawing decisions live in core.wasm (Rust).

const params = new URLSearchParams(location.search);
if (!location.hash.slice(1)) location.hash = Math.random().toString(36).slice(2, 8);
const roomId = decodeURIComponent(location.hash.slice(1));

const $ = (id) => document.getElementById(id);
const canvas = $('c'), ctx = canvas.getContext('2d'), clueInput = $('clue');
const enc = new TextEncoder(), dec = new TextDecoder();
const store = (area, key, make) => {
  try { return area.getItem(key) ?? (area.setItem(key, make()), area.getItem(key)); } catch { return make(); }
};

const { instance } = await WebAssembly.instantiateStreaming(fetch('core.wasm'), {});
const X = instance.exports;
// Same tab + same room = same player after a reload.
const seed = store(sessionStorage, `cloverleaf:${roomId}`, () => crypto.getRandomValues(new Uint32Array(2)).join(':')).split(':');
const me = X.init(+seed[0], +seed[1]) >>> 0; // u32 comes back as i32

const bytes = (ptr, len) => new Uint8Array(X.memory.buffer, ptr, len);
const writeInbox = (u8) => { bytes(X.inbox_reserve(u8.length), u8.length).set(u8); return u8.length; };
const scratchJson = (len) => JSON.parse(dec.decode(bytes(X.scratch_ptr(), len)));
const savedName = store(localStorage, 'cloverleaf:name', () => '');
if (savedName) X.set_name(writeInbox(enc.encode(savedName)));

// ---- canvas: replay the command buffer ------------------------------------

const colors = new Map();
const css = (c) => {
  let s = colors.get(c);
  if (!s) colors.set(c, (s = `rgba(${c >>> 24},${(c >>> 16) & 255},${(c >>> 8) & 255},${(c & 255) / 255})`));
  return s;
};

function paint(n) {
  const f = new Float32Array(X.memory.buffer, X.cmd_ptr(), n);
  const u = new Uint32Array(X.memory.buffer, X.cmd_ptr(), n);
  const strs = X.str_ptr();
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (let i = 0; i < n; ) {
    switch (f[i]) {
      case 1: ctx.setTransform(f[i + 1], f[i + 2], f[i + 3], f[i + 4], f[i + 5], f[i + 6]); i += 7; break;
      case 2: { // rrect x y w h r fill stroke lw
        ctx.beginPath(); ctx.roundRect(f[i + 1], f[i + 2], f[i + 3], f[i + 4], f[i + 5]);
        if (u[i + 6] & 255) { ctx.fillStyle = css(u[i + 6]); ctx.fill(); }
        if (u[i + 7] & 255) { ctx.strokeStyle = css(u[i + 7]); ctx.lineWidth = f[i + 8]; ctx.stroke(); }
        i += 9; break;
      }
      case 3: { // circle x y r fill stroke lw
        ctx.beginPath(); ctx.arc(f[i + 1], f[i + 2], f[i + 3], 0, Math.PI * 2);
        if (u[i + 4] & 255) { ctx.fillStyle = css(u[i + 4]); ctx.fill(); }
        if (u[i + 5] & 255) { ctx.strokeStyle = css(u[i + 5]); ctx.lineWidth = f[i + 6]; ctx.stroke(); }
        i += 7; break;
      }
      case 4: { // text x y size color bold off len maxw
        ctx.font = `${f[i + 5] ? 600 : 400} ${f[i + 3]}px system-ui, sans-serif`;
        ctx.fillStyle = css(u[i + 4]);
        ctx.fillText(dec.decode(bytes(strs + f[i + 6], f[i + 7])), f[i + 1], f[i + 2], f[i + 8]);
        i += 9; break;
      }
      case 5: ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.fillStyle = css(u[i + 1]); ctx.fillRect(0, 0, canvas.width, canvas.height); i += 2; break;
      default: throw new Error(`bad draw command ${f[i]} at ${i}`);
    }
  }
}

// ---- outbox: network packets, UI requests ---------------------------------

const stats = { sentPackets: 0, sentBytes: 0, recvPackets: 0, recvBytes: 0, frameMs: [], presses: 0 };
let net = null;
let flash = '', flashUntil = 0;

function drainOutbox() {
  const len = X.outbox_len();
  if (!len) return;
  const out = bytes(X.outbox_ptr(), len).slice();
  X.outbox_clear();
  const dv = new DataView(out.buffer);
  for (let p = 0; p < len; ) {
    const kind = out[p], n = dv.getUint32(p + 1, true), body = out.subarray(p + 5, p + 5 + n);
    p += 5 + n;
    if (kind === 1) {
      stats.sentPackets++; stats.sentBytes += n;
      net?.send(body);
    } else if (kind === 2) {
      const b = new DataView(body.buffer, body.byteOffset, body.byteLength);
      const [x, y, w, h, angle] = [1, 5, 9, 13, 17].map((o) => b.getFloat32(o, true));
      openClueEditor(body[0], x, y, w, h, angle, dec.decode(body.subarray(21)));
    } else if (kind === 3) {
      flash = dec.decode(body); flashUntil = performance.now() + 6000;
    }
  }
}

function receive(u8) {
  stats.recvPackets++; stats.recvBytes += u8.length;
  X.on_message(writeInbox(u8));
}

// ---- input ------------------------------------------------------------------

const local = (e) => { const r = canvas.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
let downAt = null;
canvas.addEventListener('pointerdown', (e) => { canvas.setPointerCapture(e.pointerId); downAt = local(e); X.pointer_down(...downAt); });
canvas.addEventListener('pointermove', (e) => X.pointer_move(...local(e)));
canvas.addEventListener('pointerup', (e) => { stats.presses++; X.pointer_up(...local(e)); drainOutbox(); });
canvas.addEventListener('pointercancel', (e) => X.pointer_up(...local(e)));
canvas.addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement) return;
  if (e.key === 'q') X.rotate_view(-1);
  if (e.key === 'e' || e.key === ' ') X.rotate_view(1);
});
$('copy').onclick = async () => {
  await navigator.clipboard?.writeText(location.href).catch(() => {});
  flash = `Invite link copied: ${location.href}`; flashUntil = performance.now() + 5000;
};
$('decoys').onchange = (e) => X.set_decoys(+e.target.value);
const act = (code) => { stats.presses++; X.action(code); drainOutbox(); renderUi(true); };

let editing = -1;
function openClueEditor(idx, x, y, w, h, angle, text) {
  const reposition = editing === idx && clueInput.style.display === 'block';
  editing = idx;
  Object.assign(clueInput.style, {
    display: 'block', left: `${x - w / 2}px`, top: `${y - h / 2}px`, width: `${w}px`, height: `${h}px`,
    transform: `rotate(${angle}rad)`,
  });
  if (reposition) return; // keep what's being typed
  clueInput.value = text;
  clueInput.focus();
  clueInput.select();
}
function closeClueEditor(commit, advance = false) {
  if (editing < 0) return;
  const idx = editing;
  editing = -1;
  // Stay visible while Rust decides whether to open the next leaf: hiding a
  // focused input fires a late blur that would close the next leaf.
  if (commit) X.set_clue(idx, writeInbox(enc.encode(clueInput.value)), advance ? 1 : 0);
  drainOutbox();
  if (editing < 0) clueInput.style.display = 'none';
}
clueInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') { stats.presses++; closeClueEditor(true, true); }
  if (e.key === 'Escape') closeClueEditor(false);
});
clueInput.addEventListener('blur', () => closeClueEditor(true));

new ResizeObserver(() => {
  const r = canvas.getBoundingClientRect(), dpr = devicePixelRatio || 1;
  canvas.width = Math.round(r.width * dpr);
  canvas.height = Math.round(r.height * dpr);
  X.resize(r.width, r.height, dpr);
  // Layout shifts (e.g. the roster re-wrapping) must not interrupt typing.
  if (editing >= 0) { X.edit_clue(editing); drainOutbox(); }
}).observe(canvas);

// ---- HTML controls mirror the Rust UI state ------------------------------------

const ui = () => scratchJson(X.ui());
let lastUi = '';
function renderUi(force = false) {
  const s = ui();
  const key = JSON.stringify(s) + (performance.now() < flashUntil ? flash : '');
  if (!force && key === lastUi) return;
  lastUi = key;

  const roster = $('roster');
  if (!roster.contains(document.activeElement)) {
    roster.replaceChildren(...s.players.map((p) => {
      const chip = document.createElement('span');
      chip.className = 'chip' + (p.writer ? ' writer' : '') + (p.away ? ' away' : '');
      if (p.away) chip.title = 'Left the table';
      const dot = document.createElement('i');
      dot.style.background = p.color;
      chip.append(dot);
      if (p.you) {
        const input = document.createElement('input');
        input.id = 'myname'; input.value = p.name; input.title = 'Your name (click to change)';
        input.onchange = () => {
          try { localStorage.setItem('cloverleaf:name', input.value); } catch {}
          X.set_name(writeInbox(enc.encode(input.value)));
        };
        input.onkeydown = (e) => e.key === 'Enter' && input.blur();
        chip.append(input);
      } else chip.append(p.name);
      const mark = s.phase === 'writing' ? (p.sub ? ' ✓' : ' ✎') : p.writer ? ' ★' : p.score != null ? ` +${p.score}` : '';
      if (mark) chip.append(mark);
      return chip;
    }));
  }

  const go = $('go'), alt = $('alt');
  let hint = '', label = '', enabled = false, onGo = null, altLabel = '', onAlt = null;
  $('decoysWrap').style.display = s.phase === 'writing' && !s.submitted ? 'inline-flex' : 'none';
  switch (s.phase) {
    case 'lobby':
      hint = 'Share the invite link. Start when everyone is here (late joiners can still play).';
      [label, enabled, onGo] = ['Start game', true, () => act(1)];
      break;
    case 'writing':
      if (!s.submitted) {
        hint = s.filled === 0
          ? 'Your secret clover: tap a leaf and write one clue linking the two words that face it. Enter jumps to the next leaf.'
          : 'Keep going. Tap ⟳ or the board to turn it so words read upright.';
        [label, enabled, onGo] = [`Submit clues (${s.filled}/4)`, s.canSubmit, () => act(2)];
      } else {
        hint = s.waiting ? `Submitted. Waiting for ${s.waiting} player${s.waiting > 1 ? 's' : ''} to finish writing…` : 'Submitted. Starting…';
        [label, enabled] = ['Submitted ✓', false];
        if (s.waiting) [altLabel, onAlt] = ['Start guessing without them', () => act(5)];
      }
      break;
    case 'guessing': {
      const pos = `Clover ${s.k + 1} of ${s.n}`;
      if (s.done) {
        hint = `${pos} by ${s.writerName} is done.`;
        [label, enabled, onGo] = [s.k + 1 < s.n ? 'Next clover ▸' : 'See results ▸', true, () => act(4)];
      } else if (s.isWriter) {
        hint = `${pos} is yours. No hints! Watch the others guess.`;
        [label, enabled] = ['Guessing…', false];
      } else {
        hint = `${pos} by ${s.writerName}. Drag cards onto the board, tap a card to turn it, tap ⟳ to turn the board. ${s.attempts ? 'Last try!' : 'You get 2 tries.'}`;
        [label, enabled, onGo] = [s.attempts ? 'Check (last try)' : 'Check', s.canCheck, () => act(3)];
      }
      break;
    }
    case 'end':
      hint = `Game over: ${s.total} points from ${s.n} clover${s.n === 1 ? '' : 's'} (6 for a perfect first try).`;
      [label, enabled, onGo] = ['Play again', true, () => act(1)];
      break;
  }
  if (performance.now() < flashUntil) hint = flash;
  $('hint').textContent = hint;
  go.textContent = label; go.disabled = !enabled; go.onclick = onGo;
  alt.hidden = !altLabel; alt.textContent = altLabel; alt.onclick = onAlt;
  $('decoys').value = String(s.decoys);
}

// ---- network ----------------------------------------------------------------

const peers = new Set();
function showPeers(state) {
  $('net').textContent = state ?? `room “${roomId}” · ${peers.size + 1} connected`;
}
const joined = (id) => { peers.add(id); showPeers(); X.on_peer_join(); };
const left = (id) => { peers.delete(id); showPeers(); };

async function connect() {
  const mode = params.get('net');
  if (mode === 'local') {
    // Same-browser tabs; handy offline. Same shape as the WebRTC path.
    const bc = new BroadcastChannel(`cloverleaf:${roomId}`);
    const hello = (to) => bc.postMessage({ hello: me, to });
    bc.onmessage = ({ data }) => {
      if (data.hello) {
        if (!peers.has(data.hello)) { joined(data.hello); hello(data.hello); }
      } else if (data.bye) left(data.bye);
      else receive(new Uint8Array(data.bytes));
    };
    addEventListener('pagehide', () => bc.postMessage({ bye: me }));
    hello();
    return { send: (u8) => bc.postMessage({ bytes: u8 }) };
  }
  const signal = params.get('signal');
  const lib = signal ? await import('./vendor/trystero-ws.js') : await import('./vendor/trystero-nostr.js');
  const config = { appId: 'cloverleaf-rust-wasm-demo' };
  if (signal) config.relayConfig = { urls: [signal] };
  const room = lib.joinRoom(config, roomId);
  const action = room.makeAction('m');
  action.onMessage = (data) => receive(new Uint8Array(data));
  room.onPeerJoin = joined;
  room.onPeerLeave = left;
  addEventListener('pagehide', () => room.leave());
  return { send: (u8) => peers.size && action.send(u8) };
}

showPeers('connecting…');
connect().then((n) => { net = n; showPeers(); }, (e) => showPeers(`offline: ${e.message}`));

// ---- frame loop ---------------------------------------------------------------

let frames = 0;
function tick(t) {
  const t0 = performance.now();
  const n = X.frame(t);
  paint(n);
  const dt = performance.now() - t0;
  if (stats.frameMs.push(dt) > 600) stats.frameMs.shift();
  drainOutbox();
  if (++frames % 6 === 0) renderUi();
  requestAnimationFrame(tick);
}
renderUi(true);
requestAnimationFrame(tick);

// ---- harness hooks (read-only probes + a settle switch) ------------------------

const probe = (fn, i) => { fn(i); return Array.from(new Float32Array(X.memory.buffer, X.probe_ptr(), 3)); };
window.__cl = {
  me, X, stats, peers, ui,
  digest: () => scratchJson(X.digest()),
  card: (i) => probe(X.probe_card, i),
  slot: (s) => probe(X.probe_slot, s),
  petal: (d) => probe(X.probe_petal, d),
  hub: () => probe(X.probe_hub, 0),
  quarter: () => X.view_quarter(),
  wordPx: () => X.probe_word_px(),
  cardPx: () => X.probe_card_px(),
  settle: () => X.settle(),
  answerRot: (s) => X.answer_rot(s),
};
