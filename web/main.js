// Platform shim: everything here is plumbing. Game rules, sync, layout,
// hit-testing, animation and drawing decisions live in core.wasm (Rust).

const params = new URLSearchParams(location.search);
if (!location.hash.slice(1)) location.hash = Math.random().toString(36).slice(2, 8);
const roomId = decodeURIComponent(location.hash.slice(1));

const $ = (id) => document.getElementById(id);
const canvas = $('c'), ctx = canvas.getContext('2d'), clueInput = $('clue');
const enc = new TextEncoder(), dec = new TextDecoder();
const remember = (key, value) => { try { value == null ? localStorage.removeItem(key) : localStorage.setItem(key, value); } catch {} };
const recall = (key) => { try { return localStorage.getItem(key); } catch { return null; } };

const { instance } = await WebAssembly.instantiateStreaming(fetch('core.wasm'), {});
const X = instance.exports;
const seed = crypto.getRandomValues(new Uint32Array(2));
const device = X.init(seed[0], seed[1]) >>> 0; // u32 comes back as i32
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');
X.set_motion(reduceMotion.matches ? 0 : 1);
reduceMotion.addEventListener?.('change', (e) => X.set_motion(e.matches ? 0 : 1));

const bytes = (ptr, len) => new Uint8Array(X.memory.buffer, ptr, len);
const writeInbox = (u8) => { bytes(X.inbox_reserve(u8.length), u8.length).set(u8); return u8.length; };
const writeText = (s) => writeInbox(enc.encode(s));
const scratchJson = (len) => JSON.parse(dec.decode(bytes(X.scratch_ptr(), len)));

// ---- canvas: replay the command buffer ------------------------------------

const SANS = 'Figtree, system-ui, -apple-system, sans-serif';
const FONTS = [
  (px) => `400 ${px}px ${SANS}`,
  (px) => `700 ${px}px ${SANS}`,
  (px) => `600 ${px}px Fraunces, Georgia, serif`,
];
// Fonts ship with the site (web/fonts). Repaint happens every frame, so
// canvas text switches over as soon as they're ready.
document.fonts?.load(`700 20px Figtree`).catch(() => {});
document.fonts?.load(`600 20px Fraunces`).catch(() => {});

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
      case 4: { // text x y size color face off len maxw
        ctx.font = FONTS[f[i + 5]](f[i + 3]);
        ctx.fillStyle = css(u[i + 4]);
        ctx.fillText(dec.decode(bytes(strs + f[i + 6], f[i + 7])), f[i + 1], f[i + 2], f[i + 8]);
        i += 9; break;
      }
      case 6: { // clover leaf w h fill stroke lw: tip at origin, lobes toward -y
        const w = f[i + 1], h = f[i + 2];
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.bezierCurveTo(-0.15 * w, -0.25 * h, -1.0 * w, -0.45 * h, -0.85 * w, -0.78 * h);
        ctx.bezierCurveTo(-0.72 * w, -1.02 * h, -0.2 * w, -1.03 * h, 0, -0.93 * h);
        ctx.bezierCurveTo(0.2 * w, -1.03 * h, 0.72 * w, -1.02 * h, 0.85 * w, -0.78 * h);
        ctx.bezierCurveTo(1.0 * w, -0.45 * h, 0.15 * w, -0.25 * h, 0, 0);
        ctx.fillStyle = css(u[i + 3]); ctx.fill();
        ctx.strokeStyle = css(u[i + 4]); ctx.lineWidth = f[i + 5]; ctx.stroke();
        i += 6; break;
      }
      case 5: ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.fillStyle = css(u[i + 1]); ctx.fillRect(0, 0, canvas.width, canvas.height); i += 2; break;
      default: throw new Error(`bad draw command ${f[i]} at ${i}`);
    }
  }
}

// ---- outbox: network packets, UI requests ---------------------------------

const stats = { sentPackets: 0, sentBytes: 0, recvPackets: 0, recvBytes: 0, frameMs: [], presses: 0 };
let net = null;

let toastTimer = 0;
function toast(msg, ms = 4500) {
  const t = $('toast');
  t.textContent = msg;
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), ms);
}

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
      toast(dec.decode(body));
    }
  }
}

// Presence follows connections: each peer connection gets a numeric link id.
const linkId = (s) => { let h = 0x811c9dc5; for (const c of String(s)) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193); return (h >>> 0) || 1; };

function receive(u8, link) {
  stats.recvPackets++; stats.recvBytes += u8.length;
  X.on_message(writeInbox(u8), link);
}

// ---- input ------------------------------------------------------------------

const local = (e) => { const r = canvas.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
canvas.addEventListener('pointerdown', (e) => { canvas.setPointerCapture(e.pointerId); X.pointer_down(...local(e)); });
canvas.addEventListener('pointermove', (e) => X.pointer_move(...local(e)));
canvas.addEventListener('pointerup', (e) => { stats.presses++; X.pointer_up(...local(e)); drainOutbox(); });
canvas.addEventListener('pointercancel', (e) => X.pointer_up(...local(e)));
canvas.addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement) return;
  if (e.key === 'q') X.rotate_view(-1);
  if (e.key === 'e' || e.key === ' ') X.rotate_view(1);
});
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
  if (commit) X.set_clue(idx, writeText(clueInput.value), advance ? 1 : 0);
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
  // Layout shifts must not interrupt typing.
  if (editing >= 0) { X.edit_clue(editing); drainOutbox(); }
}).observe(canvas);

// ---- joining, watching, menu ------------------------------------------------------

const nameKey = `shamrock:${roomId}:name`;

// A room can open with clovers brought in from elsewhere: a shared-boards
// link (?b=…, made by "Share boards"), or web/puzzles/<room>.json listed in
// web/puzzles/index.json (e.g. read off a screenshot). Everyone opening the
// same link loads the same boards.
const b64url = {
  enc: (text) => btoa(String.fromCharCode(...enc.encode(text))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''),
  dec: (s) => dec.decode(Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0))),
};
const customNames = [];
const loadBoard = (text) => { if (X.load_custom(writeText(text))) customNames.push(text.split('\n')[0]); };
try {
  if (params.get('b')) b64url.dec(params.get('b')).split('\n\n').forEach(loadBoard);
  const listed = await fetch('puzzles/index.json').then((r) => (r.ok ? r.json() : []));
  const r = listed.includes(roomId) ? await fetch(`puzzles/${encodeURIComponent(roomId)}.json`) : null;
  if (r?.ok) {
    const p = await r.json();
    loadBoard([p.name, ...p.clues, ...p.cards.map((c) => c.join('|'))].join('\n'));
  }
} catch {}

// "Share boards": pick clovers from this round → a link that opens a fresh
// table guessing them (answers included, for people who weren't here).
function shareLink(mask) {
  const len = X.export_boards(mask);
  const text = dec.decode(bytes(X.scratch_ptr(), len));
  const room = Math.random().toString(36).slice(2, 8);
  return `${location.origin}${location.pathname}?b=${b64url.enc(text)}#${room}`;
}
function openShare() {
  const s = ui();
  const list = $('shareList');
  list.replaceChildren(...s.boards.map((b, i) => {
    const row = document.createElement('label');
    row.className = 'item';
    row.innerHTML = `<input type="checkbox" checked data-i="${i}"> <span class="grow"></span>`;
    row.querySelector('span').textContent = `${b.name}'s clover`;
    return row;
  }));
  const update = () => {
    let mask = 0;
    list.querySelectorAll('input:checked').forEach((c) => (mask |= 1 << +c.dataset.i));
    $('shareText').textContent = mask ? shareLink(mask) : '';
    $('shareCopy').disabled = !mask;
  };
  list.onchange = update;
  update();
  closeSheets();
  $('shareSheet').classList.add('open');
}
$('shareCopy').onclick = async () => {
  try { await navigator.clipboard.writeText($('shareText').textContent); toast('Link copied'); }
  catch { getSelection().selectAllChildren($('shareText')); toast('Select the link above and copy it'); }
};
$('shareClose').onclick = () => closeSheets();
$('shareMenuBtn').onclick = openShare;
function joinAs(name) {
  name = name.trim();
  if (!name) return;
  X.join(writeText(name));
  drainOutbox();
  remember(nameKey, name);
  closeSheets();
  renderUi(true);
}
function watch() {
  X.watch();
  drainOutbox();
  remember(nameKey, '');
  closeSheets();
  renderUi(true);
}
const closeSheets = () => document.querySelectorAll('.overlay').forEach((o) => o.classList.remove('open'));
function openJoin() {
  const s = ui();
  const others = s.players.filter((p) => !p.you);
  $('seatList').replaceChildren(
    ...(others.length ? [Object.assign(document.createElement('p'), { textContent: 'Or continue as:' })] : []),
    ...others.map((p) => {
      const b = document.createElement('button');
      b.type = 'button'; b.className = 'menuBtn';
      b.innerHTML = `<span class="chip${p.away ? ' away' : ''}"><i style="background:${p.color}"></i>${esc(p.name)}</span>${p.away ? ' <small>away</small>' : ''}`;
      b.onclick = () => joinAs(p.name);
      return b;
    }));
  $('nameInput').value = s.name ?? recall(nameKey) ?? '';
  $('joinSheet').classList.add('open');
  setTimeout(() => $('nameInput').focus(), 50);
}
$('joinForm').onsubmit = (e) => { e.preventDefault(); stats.presses++; joinAs($('nameInput').value); };
$('watchBtn').onclick = watch;
$('menuBtn').onclick = () => {
  const s = ui();
  $('linkText').textContent = location.href;
  $('playerList').replaceChildren(...s.players.filter((p) => p.away && !p.you).map((p) => {
    const row = document.createElement('div');
    row.className = 'item';
    const label = Object.assign(document.createElement('span'), { className: 'grow', textContent: `${p.name} is away` });
    const rm = Object.assign(document.createElement('button'), { textContent: 'Remove' });
    rm.onclick = () => { X.remove_seat(p.id); drainOutbox(); row.remove(); };
    row.append(label, rm);
    return row;
  }));
  $('watchMenuBtn').hidden = !s.seated;
  $('menuSheet').classList.add('open');
};
$('closeMenu').onclick = closeSheets;
document.querySelectorAll('.overlay').forEach((o) => o.addEventListener('click', (e) => e.target === o && closeSheets()));
$('copyLink').onclick = async () => {
  try { await navigator.clipboard.writeText(location.href); toast('Invite link copied'); }
  catch { getSelection().selectAllChildren($('linkText')); toast('Select the link above and copy it'); }
};
$('switchBtn').onclick = () => { closeSheets(); openJoin(); };
$('watchMenuBtn').onclick = watch;
$('redealBtn').onclick = () => { closeSheets(); act(6); };
$('newGameBtn').onclick = () => { closeSheets(); act(7); };

// Card size is per device (display only): bigger cards just overlap more.
const zoomKey = 'shamrock:zoom';
let zoom = X.set_zoom(+(recall(zoomKey) ?? (Math.min(innerWidth, innerHeight) < 520 ? 1.25 : 1)));
const setZoom = (z) => { stats.presses++; zoom = X.set_zoom(z); remember(zoomKey, zoom); };
$('sizeDown').onclick = () => setZoom(zoom - 0.15);
$('sizeUp').onclick = () => setZoom(zoom + 0.15);

const setDecoys = (d) => { X.set_decoys(Math.max(0, Math.min(4, d))); drainOutbox(); renderUi(true); };
$('decDown').onclick = () => { stats.presses++; setDecoys(ui().decoys - 1); };
$('decUp').onclick = () => { stats.presses++; setDecoys(ui().decoys + 1); };

// ---- HTML controls mirror the Rust UI state ------------------------------------

const ui = () => scratchJson(X.ui());
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const names = (xs) => xs.length <= 2 ? xs.join(' and ') : `${xs.slice(0, -1).join(', ')} and ${xs.at(-1)}`;
let lastUi = '';

function renderUi(force = false) {
  const s = ui();
  const key = JSON.stringify(s);
  if (!force && key === lastUi) return;
  lastUi = key;

  const mark = { writing: '✎', ready: '✓', writer: '★' };
  $('roster').replaceChildren(...s.players.map((p) => {
    const chip = document.createElement('span');
    chip.className = `chip${p.you ? ' you' : ''}${p.status === 'writer' ? ' writer' : ''}${p.away ? ' away' : ''}`;
    chip.innerHTML = `<i style="background:${p.color}"></i>${esc(p.name)} ${mark[p.status] ?? ''}`;
    chip.title = p.away ? `${p.name} is away` : p.name;
    return chip;
  }), ...(s.watchers ? [Object.assign(document.createElement('span'), { className: 'chip', textContent: `👀 ${s.watchers}` })] : []));

  let hint = '', go = null, alt = null, side = null;
  const decoys = s.phase === 'writing' && s.inRound;
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  const dealLine = s.dealable.length ? `Deals in ${esc(names(s.dealable))}; anyone who joins later is dealt in next round.` : '';
  if (!s.seated) {
    hint = !s.players.length ? 'Nobody is playing yet. Join to start a table.'
      : `Watching${s.phase === 'guessing' ? ` <b>${esc(s.writerName ?? '')}</b>'s clover` : ''}.`;
    go = ['Join the game', true, openJoin];
  } else switch (s.phase) {
    case 'lobby':
      if (customNames.length && s.round === 0) {
        const whose = names(customNames.map((n) => `<b>${esc(n)}</b>'s`));
        hint = `This table opens with ${whose} clover${customNames.length > 1 ? 's' : ''}. Send the invite link (⋯ menu), then start guessing together.`;
        go = ['Start guessing', s.dealable.length > 0, () => act(1)];
        break;
      }
      hint = `Send the invite link (⋯ menu). ${dealLine}`;
      go = [`Start with ${plural(s.dealable.length, 'player')}`, s.dealable.length > 0, () => act(1)];
      break;
    case 'writing':
      if (!s.inRound) {
        hint = "You joined after the deal, so you're in next round. You'll help guess this round's clovers.";
      } else if (!s.ready) {
        hint = s.filled < 4
          ? `<b>Your secret clover.</b> Tap a leaf and write a clue linking the two words facing it (${s.filled}/4). Enter jumps to the next leaf.`
          : 'Choose how many decoy cards to mix in, then <b>Submit</b>.';
        go = ['Submit', s.canReady, () => act(2)];
      } else {
        hint = 'Submitted. You can still change clues or decoys until guessing starts.';
        go = ['Submitted ✓', true, () => act(2), 'on'];
      }
      if (s.readyCount > 0 && s.readyCount < s.progress.length) {
        const label = innerWidth < 480 ? `Proceed · ${s.readyCount}/${s.progress.length}` : `Proceed with ${s.readyCount} of ${s.progress.length}`;
        alt = [label, () => act(5)];
      }
      break;
    case 'guessing': {
      const what = `<b>${esc(s.writerName)}</b>'s clover · ${s.k + 1} of ${s.n} · ${plural(s.writerDecoys, 'decoy')}`;
      if (s.done) {
        hint = `${what} · done`;
        go = [s.k + 1 < s.n ? 'Next clover' : 'Finish round', true, () => act(4)];
      } else if (s.isWriter) {
        hint = `${what}. It's yours: no hints while they guess!`;
        go = ['Guessing…', false];
      } else {
        hint = `${what}. Drag cards anywhere; they snap when close to a spot. Tap a card to turn it. Fill all four spots to guess.`;
        go = ['Guess', s.canCheck, () => act(3)];
      }
      break;
    }
    case 'end':
      hint = `Round ${s.round} is done. ${dealLine}`;
      go = [`Next round with ${plural(s.dealable.length, 'player')}`, s.dealable.length > 0, () => act(1)];
      side = ['Share boards', openShare];
      break;
  }
  // Submission progress, building up to guessing.
  const prog = $('progress');
  prog.hidden = s.phase !== 'writing';
  if (!prog.hidden) {
    $('segs').replaceChildren(...s.progress.map((p) => {
      const seg = document.createElement('i');
      seg.className = `${p.ready ? 'done' : ''}${p.away ? ' away' : ''}`;
      seg.style.color = p.color;
      seg.title = `${p.name}: ${p.ready ? 'submitted' : 'writing'}${p.away ? ' (away)' : ''}`;
      return seg;
    }));
    const left = s.progress.filter((p) => !p.ready).map((p) => p.name + (p.away ? ' (away)' : ''));
    $('progLabel').textContent = `${s.readyCount} of ${s.progress.length} submitted${left.length && left.length <= 2 ? ` · waiting for ${names(left)}` : ''}`;
  }
  $('hint').innerHTML = hint;
  const goBtn = $('go');
  goBtn.textContent = go?.[0] ?? '';
  goBtn.hidden = !go;
  goBtn.disabled = !go?.[1];
  goBtn.onclick = go?.[2] ?? null;
  goBtn.classList.toggle('on', go?.[3] === 'on');
  const sideBtn = $('side');
  sideBtn.hidden = !side;
  if (side) { sideBtn.textContent = side[0]; sideBtn.onclick = side[1]; }
  $('shareMenuBtn').hidden = !s.boards.length;
  const altBtn = $('alt');
  altBtn.hidden = !alt;
  if (alt) { altBtn.textContent = alt[0]; altBtn.onclick = alt[1]; }
  $('decoys').hidden = !decoys;
  $('size').hidden = !(s.phase === 'guessing' || (s.phase === 'writing' && s.seated));
  $('decLabel').textContent = `${s.decoys} decoy${s.decoys === 1 ? '' : 's'}`;
}

// ---- network ----------------------------------------------------------------

const peers = new Set();
const joined = (id) => { peers.add(id); X.on_peer_join(); drainOutbox(); };
const left = (id) => { peers.delete(id); X.on_link_lost(linkId(id)); };

async function connect() {
  const mode = params.get('net');
  if (mode === 'local') {
    // Same-browser tabs; handy offline. Same shape as the WebRTC path.
    const bc = new BroadcastChannel(`shamrock:${roomId}`);
    const hello = (to) => bc.postMessage({ hello: device, to, from: device });
    bc.onmessage = ({ data }) => {
      if (data.hello) {
        if (!peers.has(data.hello)) { joined(data.hello); hello(data.hello); }
      } else if (data.bye) left(data.bye);
      else receive(new Uint8Array(data.bytes), linkId(data.from));
    };
    addEventListener('pagehide', () => bc.postMessage({ bye: device }));
    hello();
    return { send: (u8) => bc.postMessage({ bytes: u8, from: device }) };
  }
  const signal = params.get('signal');
  const lib = signal ? await import('./vendor/trystero-ws.js') : await import('./vendor/trystero-nostr.js');
  const config = { appId: 'shamrock-p2p' };
  if (signal) config.relayConfig = { urls: [signal] };
  const room = lib.joinRoom(config, roomId);
  const action = room.makeAction('m');
  action.onMessage = (data, { peerId }) => receive(new Uint8Array(data), linkId(peerId));
  room.onPeerJoin = joined;
  room.onPeerLeave = left;
  addEventListener('pagehide', () => room.leave());
  return { send: (u8) => peers.size && action.send(u8) };
}

connect().then((n) => { net = n; }, (e) => toast(`Can't connect: ${e.message}`, 10000));

// Returning to this room re-takes your seat; a first visit asks. Wait a
// moment first so we've heard the table's state (and existing names).
const saved = params.has('fresh') ? null : recall(nameKey);
if (saved) setTimeout(() => joinAs(saved), 1500);
else if (saved === null) setTimeout(() => { if (!ui().seated) openJoin(); }, 900);

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

const probe = (fn, ...a) => { fn(...a); return Array.from(new Float32Array(X.memory.buffer, X.probe_ptr(), 3)); };
window.__cl = {
  device, X, stats, peers, ui, joinAs, watch,
  digest: () => scratchJson(X.digest()),
  card: (i) => probe(X.probe_card, i),
  slot: (s) => probe(X.probe_slot, s),
  board: (x, y) => probe(X.probe_board, x, y),
  petal: (d) => probe(X.probe_petal, d),
  hub: () => probe(X.probe_hub),
  shownAngle: (i) => X.probe_shown_angle(i),
  quarter: () => X.view_quarter(),
  wordPx: () => X.probe_word_px(),
  cardPx: () => X.probe_card_px(),
  zoom: () => zoom,
  settle: () => X.settle(),
  answerRot: (s) => X.answer_rot(s),
  palette: () => scratchJson(X.palette()),
  wordBudget: () => X.word_budget(),
  measure: async (words) => {
    await document.fonts.load('700 26px Figtree');
    const c = document.createElement('canvas').getContext('2d');
    c.font = FONTS[1](26);
    return words.map((w) => [w, c.measureText(w).width]);
  },
};
