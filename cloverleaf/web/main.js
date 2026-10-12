// Platform shim: everything here is plumbing. Game logic, sync, geometry,
// hit-testing and drawing decisions live in core.wasm (Rust).

const params = new URLSearchParams(location.search);
if (!location.hash.slice(1)) location.hash = Math.random().toString(36).slice(2, 8);
const roomId = decodeURIComponent(location.hash.slice(1));

const $ = (id) => document.getElementById(id);
const canvas = $('c'), ctx = canvas.getContext('2d'), clueInput = $('clue');
const enc = new TextEncoder(), dec = new TextDecoder();

const { instance } = await WebAssembly.instantiateStreaming(fetch('core.wasm'), {});
const X = instance.exports;
const seed = crypto.getRandomValues(new Uint32Array(2));
const me = X.init(seed[0], seed[1]);

const bytes = (ptr, len) => new Uint8Array(X.memory.buffer, ptr, len);
const writeInbox = (u8) => bytes(X.inbox_reserve(u8.length), u8.length).set(u8);

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

const stats = { sentPackets: 0, sentBytes: 0, recvPackets: 0, recvBytes: 0, frameMs: [] };
let net = null;

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
      $('status').textContent = dec.decode(body);
    }
  }
}

function receive(u8) {
  stats.recvPackets++; stats.recvBytes += u8.length;
  writeInbox(u8);
  X.on_message(u8.length);
}

// ---- input ------------------------------------------------------------------

const local = (e) => { const r = canvas.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
canvas.addEventListener('pointerdown', (e) => { canvas.setPointerCapture(e.pointerId); X.pointer_down(...local(e)); });
canvas.addEventListener('pointermove', (e) => X.pointer_move(...local(e)));
canvas.addEventListener('pointerup', (e) => X.pointer_up(...local(e)));
canvas.addEventListener('pointercancel', (e) => X.pointer_up(...local(e)));
canvas.addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('keydown', (e) => {
  if (e.target === clueInput) return;
  if (e.key === 'q') X.rotate_view(-1);
  if (e.key === 'e') X.rotate_view(1);
});
$('rotl').onclick = () => X.rotate_view(-1);
$('rotr').onclick = () => X.rotate_view(1);
document.querySelectorAll('[data-act]').forEach((b) => (b.onclick = () => X.action(+b.dataset.act)));
$('copy').onclick = async () => {
  await navigator.clipboard?.writeText(location.href).catch(() => {});
  $('status').textContent = `Invite link: ${location.href}`;
};

let editing = -1;
function openClueEditor(idx, x, y, w, h, angle, text) {
  editing = idx;
  Object.assign(clueInput.style, {
    display: 'block', left: `${x - w / 2}px`, top: `${y - h / 2}px`, width: `${w}px`, height: `${h}px`,
    transform: `rotate(${angle}rad)`,
  });
  clueInput.value = text;
  clueInput.focus();
  clueInput.select();
}
function closeClueEditor(commit) {
  if (editing < 0) return;
  if (commit) {
    writeInbox(enc.encode(clueInput.value));
    X.set_clue(editing, enc.encode(clueInput.value).length);
  }
  editing = -1;
  clueInput.style.display = 'none';
}
clueInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') closeClueEditor(true);
  if (e.key === 'Escape') closeClueEditor(false);
});
clueInput.addEventListener('blur', () => closeClueEditor(true));

new ResizeObserver(() => {
  const r = canvas.getBoundingClientRect(), dpr = devicePixelRatio || 1;
  canvas.width = Math.round(r.width * dpr);
  canvas.height = Math.round(r.height * dpr);
  X.resize(r.width, r.height, dpr);
  closeClueEditor(true);
}).observe(canvas);

// ---- network ----------------------------------------------------------------

const peers = new Set();
function showPeers(state) {
  $('net').textContent = state ?? `room ${roomId} · ${peers.size + 1} player${peers.size ? 's' : ''}`;
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

function tick(t) {
  const t0 = performance.now();
  const n = X.frame(t);
  paint(n);
  const dt = performance.now() - t0;
  if (stats.frameMs.push(dt) > 600) stats.frameMs.shift();
  drainOutbox();
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);

// ---- harness hooks (read-only probes + a settle switch) ------------------------

const probe = (fn, i) => { fn(i); return Array.from(new Float32Array(X.memory.buffer, X.probe_ptr(), 3)); };
window.__cl = {
  me, X, stats, peers,
  digest: () => { const n = X.digest(); return JSON.parse(dec.decode(bytes(X.scratch_ptr(), n))); },
  card: (i) => probe(X.probe_card, i),
  slot: (s) => probe(X.probe_slot, s),
  petal: (d) => probe(X.probe_petal, d),
  wordPx: () => X.probe_word_px(),
  cardPx: () => X.probe_card_px(),
  settle: () => X.settle(),
};
