// Platform shim: everything here is plumbing. Game rules, sync, layout,
// hit-testing, animation and drawing decisions live in core.wasm (Rust).

const params = new URLSearchParams(location.search);
// No room in the link (and no shared boards): the home page, listing this
// device's tables and past games, with a button to start a new table.
const home = !location.hash.slice(1) && !params.has('b');
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
      case 1: // Rust's transform, under this device's zoom camera.
        ctx.setTransform(cam.s, 0, 0, cam.s, cam.x * devicePixelRatio, cam.y * devicePixelRatio);
        ctx.transform(f[i + 1], f[i + 2], f[i + 3], f[i + 4], f[i + 5], f[i + 6]); i += 7; break;
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
        // Too wide for its room: shrink the type evenly (not the canvas's
        // sideways squeeze), down to 60%; squeeze only past that.
        const text = dec.decode(bytes(strs + f[i + 6], f[i + 7]));
        let size = f[i + 3];
        ctx.font = FONTS[f[i + 5]](size);
        const wide = ctx.measureText(text).width;
        if (wide > f[i + 8]) { size = Math.max(size * 0.6, size * f[i + 8] / wide); ctx.font = FONTS[f[i + 5]](size); }
        ctx.fillStyle = css(u[i + 4]);
        ctx.fillText(text, f[i + 1], f[i + 2], f[i + 8]);
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

const stats = { joins: 0, sentPackets: 0, sentBytes: 0, recvPackets: 0, recvBytes: 0, frameMs: [], presses: 0 };
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

// Zoom is the app's own, per device, on the board area only: two fingers
// (or ctrl+wheel / a trackpad pinch) zoom and pan a camera over the canvas,
// so the dock and menu stay put and you can't get stuck zoomed in. Rust
// never knows: drawing goes through the camera and pointers come back
// through its inverse. One finger always plays. A second finger cancels
// whatever the first started, and nothing acts again until all have lifted.
const cam = { s: 1, x: 0, y: 0 };
const CAM_MAX = 4;
const view = () => canvas.getBoundingClientRect();
const local = (e) => { const r = view(); return [(e.clientX - r.left - cam.x) / cam.s, (e.clientY - r.top - cam.y) / cam.s]; };
function setCam(s, x, y) {
  const r = view();
  cam.s = Math.min(CAM_MAX, Math.max(1, s));
  // Keep the board area covered: no blank edges past the content.
  cam.x = Math.min(0, Math.max(r.width * (1 - cam.s), x));
  cam.y = Math.min(0, Math.max(r.height * (1 - cam.s), y));
  if (cam.s < 1.01) Object.assign(cam, { s: 1, x: 0, y: 0 });
  $('unzoom').hidden = cam.s === 1;
  if (editing >= 0) placeEditor();
}
/** Zoom by `k` about the canvas point (px, py), CSS px. */
const zoomAt = (k, px, py) => {
  const s = Math.min(CAM_MAX, Math.max(1, cam.s * k));
  setCam(s, px - (px - cam.x) * s / cam.s, py - (py - cam.y) * s / cam.s);
};
$('unzoom').onclick = () => setCam(1, 0, 0);

const fingers = new Map();
// While zoomed, one finger dragged from anywhere but a card pans the view
// (instead of spinning the board); a press that doesn't move is still a tap.
let pan = null; // { x, y, cx, cy, moved }
let pinch = null; // { d, mx, my, s, x, y } at the start of a two-finger gesture
let pinching = false;
const at = (e) => { const r = view(); return [e.clientX - r.left, e.clientY - r.top]; };
const spread = () => {
  const [a, b] = [...fingers.values()];
  return { d: Math.hypot(a[0] - b[0], a[1] - b[1]) || 1, mx: (a[0] + b[0]) / 2, my: (a[1] + b[1]) / 2 };
};
canvas.addEventListener('pointerdown', (e) => {
  try { canvas.setPointerCapture(e.pointerId); } catch {}
  fingers.set(e.pointerId, at(e));
  if (fingers.size === 2) {
    if (!pinching && !pan) { X.pointer_cancel(); drainOutbox(); }
    pan = null;
    pinching = true;
    pinch = { ...spread(), s: cam.s, x: cam.x, y: cam.y };
    return;
  }
  if (fingers.size > 2 || pinching) return;
  const [px, py] = at(e);
  if (cam.s > 1 && !X.on_card(...local(e))) { pan = { x: px, y: py, cx: cam.x, cy: cam.y, moved: false }; return; }
  X.pointer_down(...local(e));
});
canvas.addEventListener('pointermove', (e) => {
  if (!fingers.has(e.pointerId)) { if (e.pointerType === 'mouse') X.pointer_move(...local(e)); return; }
  fingers.set(e.pointerId, at(e));
  if (pan && !pinching) {
    const [px, py] = at(e);
    pan.moved ||= Math.hypot(px - pan.x, py - pan.y) > 8;
    if (pan.moved) setCam(cam.s, pan.cx + px - pan.x, pan.cy + py - pan.y);
    return;
  }
  if (!pinching) return X.pointer_move(...local(e));
  if (fingers.size !== 2 || !pinch) return;
  const now = spread(), s = pinch.s * now.d / pinch.d;
  // The board point under the fingers' midpoint follows the midpoint.
  setCam(s, now.mx - (pinch.mx - pinch.x) * s / pinch.s, now.my - (pinch.my - pinch.y) * s / pinch.s);
});
const lift = (e, act) => {
  if (!fingers.delete(e.pointerId)) return;
  if (pinching) {
    if (fingers.size === 2) pinch = { ...spread(), s: cam.s, x: cam.x, y: cam.y };
    if (!fingers.size) { pinching = false; pinch = null; }
    return;
  }
  if (pan) {
    // A pan that never moved was a tap (a leaf to edit, ⟳ to turn).
    const tap = act && !pan.moved;
    pan = null;
    if (!tap) return;
    X.pointer_down(...local(e));
  }
  if (act) { stats.presses++; X.pointer_up(...local(e)); } else X.pointer_cancel();
  drainOutbox();
};
canvas.addEventListener('pointerup', (e) => lift(e, true));
canvas.addEventListener('pointercancel', (e) => lift(e, false));
canvas.addEventListener('wheel', (e) => {
  // ctrl+wheel and trackpad pinches zoom the board; a plain wheel pans when zoomed.
  if (!e.ctrlKey && cam.s === 1) return;
  e.preventDefault();
  const [px, py] = at(e);
  if (e.ctrlKey) zoomAt(Math.exp(-e.deltaY / 200), px, py);
  else setCam(cam.s, cam.x - e.deltaX, cam.y - e.deltaY);
}, { passive: false });
// Safari's own page pinch: the board handles pinches itself.
for (const g of ['gesturestart', 'gesturechange']) document.addEventListener(g, (e) => e.preventDefault());
canvas.addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement) return;
  if (e.key === 'q') X.rotate_view(-1);
  if (e.key === 'e' || e.key === ' ') X.rotate_view(1);
});
const act = (code) => { stats.presses++; X.action(code); drainOutbox(); renderUi(true); };

let editing = -1;
let editorAt = null;
function placeEditor() {
  if (!editorAt) return;
  const [x, y, w, h, angle] = editorAt;
  // Under the zoom camera: centre moves with it, the box scales with it.
  Object.assign(clueInput.style, {
    display: 'block', left: `${cam.x + x * cam.s - w / 2}px`, top: `${cam.y + y * cam.s - h / 2}px`, width: `${w}px`, height: `${h}px`,
    transform: `rotate(${angle}rad) scale(${cam.s})`,
  });
}
function openClueEditor(idx, x, y, w, h, angle, text) {
  const reposition = editing === idx && clueInput.style.display === 'block';
  editing = idx;
  editorAt = [x, y, w, h, angle];
  placeEditor();
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
  setCam(cam.s, cam.x, cam.y);
  // Layout shifts must not interrupt typing.
  if (editing >= 0) { X.edit_clue(editing); drainOutbox(); }
}).observe(canvas);

// ---- joining, watching, menu ------------------------------------------------------

const nameKey = `shamrock:${roomId}:name`;

// Shared boards travel inside the link (?b=…, made by "Share boards") and
// open as solo play on whatever device follows it: no room, no name, no
// connection. Links must not give answers away by accident: Rust scrambles
// each board (cards shuffled and randomly turned, the answer on a separate
// line); here that text is only compressed and AES-GCM encrypted under a key
// derived from the link's #fragment. Every device has to decode it to play,
// so this stops accidental peeking, not determined devtools digging.
const solo = params.has('b');
const b64 = {
  enc: (u8) => btoa(String.fromCharCode(...u8)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''),
  dec: (s) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)),
};
const shareKey = async (salt) => crypto.subtle.importKey(
  'raw', await crypto.subtle.digest('SHA-256', enc.encode(`shamrock-share-v1:${salt}`)), 'AES-GCM', false, ['encrypt', 'decrypt']);
const squeeze = async (u8, mode) => new Uint8Array(await new Response(
  new Blob([u8]).stream().pipeThrough(mode === 'in' ? new CompressionStream('deflate-raw') : new DecompressionStream('deflate-raw'))).arrayBuffer());
const scratchText = (len) => dec.decode(bytes(X.scratch_ptr(), len));

/** Scrambled board text (from Rust) → link payload. Byte 2 = deflated text. */
async function seal(text, salt) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const data = new Uint8Array([2, ...(await squeeze(enc.encode(text), 'in'))]);
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, await shareKey(salt), data));
  return b64.enc(new Uint8Array([...iv, ...ct]));
}
/** Link payload → scrambled board texts, for Rust to load. Older links (JSON,
 *  plain or deflated) are rewritten into the same scrambled text form. */
async function unseal(payload, salt) {
  const raw = b64.dec(payload);
  const pt = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: raw.slice(0, 12) }, await shareKey(salt), raw.slice(12)));
  if (pt[0] === 2) return dec.decode(await squeeze(pt.slice(1), 'out')).split('\n\n');
  const json = pt[0] === 1 ? await squeeze(pt.slice(1), 'out') : pt;
  return JSON.parse(dec.decode(json)).map(({ n, c, k, a }) => [n, ...c, ...k.map((w) => w.join('|')), `@ ${a.flat().join(' ')}`].join('\n'));
}

// Card words are laid out per card from their real widths: measure every
// card word (deck and any imported clovers) once the card font is in.
let wordsMeasured = false;
async function measureWords() {
  await document.fonts?.load(`700 100px Figtree`).catch(() => {});
  const c = document.createElement('canvas').getContext('2d');
  c.font = FONTS[1](100);
  const list = scratchText(X.word_list()).split('\n');
  X.set_word_widths(writeText(list.map((w) => `${w}\t${c.measureText(w).width.toFixed(2)}`).join('\n')));
  wordsMeasured = true;
}

const customNames = [];
const loadBoard = (text) => { if (X.load_custom(writeText(text))) customNames.push(text.split('\n')[0]); };
if (solo) {
  try { (await unseal(params.get('b'), roomId)).forEach(loadBoard); }
  catch { setTimeout(() => toast("This shared-boards link is damaged; check it was copied in full.", 8000), 500); }
}
measureWords();

/** A link that plays `boards` (texts in answer order, e.g. from a screenshot). */
async function shareFromText(boards) {
  const scrambled = scratchText(X.scramble_boards(writeText(boards.join('\n\n'))));
  return linkFor(scrambled);
}
async function linkFor(scrambled) {
  const salt = Math.random().toString(36).slice(2, 8);
  return `${location.origin}${location.pathname}?b=${await seal(scrambled, salt)}#${salt}`;
}
const shareLink = async (mask) => linkFor(scratchText(X.export_boards(mask)));
const ownTable = () => { location.href = `${location.origin}${location.pathname}#${Math.random().toString(36).slice(2, 8)}`; };
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
  const update = async () => {
    let mask = 0;
    list.querySelectorAll('input:checked').forEach((c) => (mask |= 1 << +c.dataset.i));
    $('shareCopy').disabled = true;
    $('shareText').textContent = mask ? await shareLink(mask) : '';
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
  noteTable(name);
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
  // Everyone at the table, by name, with what they're up to.
  const doing = { writing: 'writing', ready: 'submitted', writer: 'their clover is up' };
  $('playerList').replaceChildren(...s.players.map((p) => {
    const row = document.createElement('div');
    row.className = 'who-row';
    const label = document.createElement('span');
    label.className = 'grow';
    label.innerHTML = `<b>${esc(p.name)}</b>${p.you ? ' (you)' : ''}<br><small>${p.away ? 'away' : doing[p.status] ?? 'here'}</small>`;
    row.append(label);
    if (p.away && !p.you) {
      const rm = Object.assign(document.createElement('button'), { textContent: 'Remove' });
      rm.onclick = () => { X.remove_seat(p.id); drainOutbox(); row.remove(); };
      row.append(rm);
    }
    return row;
  }), ...(s.watchers ? [Object.assign(document.createElement('div'), { className: 'who-row', textContent: `${s.watchers} watching` })] : []));
  $('watchMenuBtn').hidden = !s.seated;
  syncSize();
  $('menuSheet').classList.add('open');
};
$('closeMenu').onclick = closeSheets;
document.querySelectorAll('.overlay').forEach((o) => o.addEventListener('click', (e) => {
  if (e.target !== o || o.id === 'homeSheet') return;
  closeSheets();
  if (home) openHome();
}));
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
const setZoom = (z) => { stats.presses++; zoom = X.set_zoom(z); remember(zoomKey, zoom); syncSize(); };
// The size buttons step from what's drawn (a saved size can exceed what fits
// right now) and grey out when a step would change nothing; past the limit,
// pinch zoom takes over.
const SIZE_STEP = 0.15;
function sizeWouldChange(dz) {
  const was = X.card_zoom(), saved = zoom;
  X.set_zoom(was + dz);
  const changed = Math.abs(X.card_zoom() - was) > 1e-3;
  X.set_zoom(saved);
  return changed;
}
function syncSize() {
  const up = sizeWouldChange(SIZE_STEP), down = sizeWouldChange(-SIZE_STEP);
  $('sizeUp').disabled = !up;
  $('sizeDown').disabled = !down;
  $('sizeNote').hidden = up;
}
// Theme: follow the device's light/dark setting, or a choice made in the
// menu (kept per device). Rust draws the board in the matching palette.
const themeKey = 'shamrock:theme';
const systemDark = matchMedia('(prefers-color-scheme: dark)');
let themePref = recall(themeKey) ?? 'auto';
function applyTheme() {
  const dark = themePref === 'dark' || (themePref === 'auto' && systemDark.matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  X.set_theme(dark ? 1 : 0);
  document.querySelector('meta[name=theme-color]')?.setAttribute('content', getComputedStyle(document.documentElement).getPropertyValue('--bar').trim());
  $('themeBtn').textContent = `Theme: ${themePref}`;
}
systemDark.addEventListener('change', applyTheme);
$('themeBtn').onclick = () => {
  themePref = { auto: 'light', light: 'dark', dark: 'auto' }[themePref] ?? 'auto';
  remember(themeKey, themePref === 'auto' ? null : themePref);
  applyTheme();
};
applyTheme();

// Full screen where the browser allows it (Android, desktop; not iPhone,
// where "Add to Home Screen" gives the same via the manifest).
const fullBtn = $('fullBtn');
const fsEl = document.documentElement;
fullBtn.hidden = !(document.fullscreenEnabled && fsEl.requestFullscreen);
const syncFull = () => { fullBtn.textContent = document.fullscreenElement ? 'Exit full screen' : 'Full screen'; };
document.addEventListener('fullscreenchange', syncFull);
fullBtn.onclick = () => {
  closeSheets();
  (document.fullscreenElement ? document.exitFullscreen() : fsEl.requestFullscreen({ navigationUI: 'hide' })).catch(() => {});
};
syncFull();
$('sizeDown').onclick = () => setZoom(X.card_zoom() - SIZE_STEP);
$('sizeUp').onclick = () => setZoom(X.card_zoom() + SIZE_STEP);

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

  // One short line of status; how-to tips show once per device (see `tip`).
  let hint = '', go = null, alt = null, side = null, decoys = false;
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  const narrow = innerWidth < 480;
  const clover = s.n ? `<b>${esc(s.writerName ?? '')}</b> · ${s.k + 1}/${s.n} · ${plural(s.writerDecoys, 'decoy')}` : '';
  if (s.phase === 'guessing' && !(s.isWriter && !s.done) && (solo || s.seated)) tip('guess');
  if (solo) {
    if (s.phase === 'guessing' && s.done) {
      hint = `<b>${esc(s.writerName ?? '')}</b> · solved`;
      go = [s.k + 1 < s.n ? 'Next clover' : 'Done', true, () => act(4)];
    } else if (s.phase === 'guessing') {
      hint = clover;
      go = ['Guess', s.canCheck, () => act(3)];
    } else {
      hint = customNames.length ? 'All done' : 'No clovers in this link';
      go = ['Start your own table', true, ownTable];
    }
  } else if (!s.seated) {
    hint = !s.players.length ? "No one's playing yet"
      : s.phase === 'guessing' ? `Watching ${clover}` : 'Watching';
    go = ['Join', true, openJoin];
  } else switch (s.phase) {
    case 'lobby':
      if (customNames.length && s.round === 0) {
        hint = `Opens with ${names(customNames.map((n) => `<b>${esc(n)}</b>`))}`;
        go = ['Start guessing', s.dealable.length > 0, () => act(1)];
        break;
      }
      hint = s.dealable.length < 2 ? 'Invite friends' : esc(names(s.dealable));
      go = [narrow ? `Start (${s.dealable.length})` : `Start with ${plural(s.dealable.length, 'player')}`, s.dealable.length > 0, () => act(1)];
      break;
    case 'writing': {
      const left = s.progress.filter((p) => !p.ready).map((p) => p.name);
      const waiting = `${s.readyCount}/${s.progress.length} in${left.length && left.length <= 2 ? ` · waiting for ${esc(names(left))}` : ''}`;
      if (!s.inRound) {
        hint = `Next round for you · ${waiting}`;
      } else if (!s.ready) {
        tip('write');
        hint = s.filled < 4 ? `Your clover · clues ${s.filled}/4` : 'Decoys?';
        decoys = s.filled >= 4;
        go = ['Submit', s.canReady, () => act(2)];
      } else {
        hint = waiting;
        go = [narrow ? '✓' : 'Submitted ✓', true, () => act(2), 'on'];
      }
      if ((s.ready || !s.inRound) && s.readyCount > 0 && s.readyCount < s.progress.length) {
        alt = [narrow ? `Proceed · ${s.readyCount}/${s.progress.length}` : `Proceed with ${s.readyCount} of ${s.progress.length}`, () => act(5)];
      }
      break;
    }
    case 'guessing':
      if (s.done) {
        hint = `<b>${esc(s.writerName)}</b> · solved`;
        go = [s.k + 1 < s.n ? 'Next clover' : 'Finish round', true, () => act(4)];
      } else if (s.isWriter) {
        hint = 'Your clover · no hints!';
      } else {
        hint = clover;
        go = ['Guess', s.canCheck, () => act(3)];
      }
      break;
    case 'end':
      hint = `Round ${s.round} done`;
      go = [narrow ? `Next round (${s.dealable.length})` : `Next round with ${plural(s.dealable.length, 'player')}`, s.dealable.length > 0, () => act(1)];
      side = ['Share', openShare];
      break;
  }
  if (!solo && !home && s.phase === 'end') logRound(s);
  // Not connected to anyone: say so rather than look normal while broken.
  if (!solo && !peers.size) {
    const others = s.players.some((p) => !p.you);
    hint = `<span class="pulse"></span>${others ? 'Reconnecting…' : 'Waiting for others'}`;
  }
  // Submission progress: a thin bar along the top of the strip.
  const segs = $('segs');
  segs.hidden = s.phase !== 'writing';
  if (!segs.hidden) {
    segs.replaceChildren(...s.progress.map((p) => {
      const seg = document.createElement('i');
      seg.className = `${p.ready ? 'done' : ''}${p.away ? ' away' : ''}`;
      seg.style.color = p.color;
      seg.title = `${p.name}: ${p.ready ? 'submitted' : 'writing'}${p.away ? ' (away)' : ''}`;
      return seg;
    }));
  }
  $('hint').innerHTML = hint;
  $('hint').title = $('hint').textContent;
  // While guessing, the status names the clover; tap it to pick another.
  const picking = s.phase === 'guessing' && (solo ? s.n : s.boards.length) > 1;
  $('hint').classList.toggle('pick', picking);
  $('hint').onclick = picking ? openClovers : null;
  if (picking) $('hint').innerHTML += ' ▾';
  // Invite sits in the strip itself in the lobby, or whenever you're alone.
  $('invite').hidden = solo || home || !(s.phase === 'lobby' || !peers.size);
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
  $('decLabel').textContent = `${s.decoys} decoy${s.decoys === 1 ? '' : 's'}`;
}

// How-to tips: shown once per device as a toast that the next touch on the
// board dismisses, and kept in the ⋯ menu.
function tip(kind) {
  const TIPS = {
    write: 'Tap a leaf and write a clue linking the two words facing it. Enter jumps to the next leaf. Then choose decoys and Submit.',
    guess: 'Drag cards onto the spots (they snap when close), tap a card to turn it, then Guess. Wrong cards bounce back.',
  };
  $('tipText').textContent = TIPS[kind];
  const key = `shamrock:tip:${kind}`;
  if (recall(key)) return;
  remember(key, '1');
  setTimeout(() => toast(TIPS[kind], 9000), 300);
}
canvas.addEventListener('pointerdown', () => $('toast').classList.remove('show'));

// ---- choosing the clover, inviting ---------------------------------------------

function openClovers() {
  const s = ui();
  const list = $('cloverList');
  const names = solo ? customNames : s.boards.map((b) => b.name);
  list.replaceChildren(...names.map((name, i) => {
    const b = Object.assign(document.createElement('button'), { className: `clover-row${i === s.k ? ' now' : ''}` });
    const solved = s.boards[i]?.solved;
    b.innerHTML = `<span>${i + 1}. <b>${esc(name)}</b>'s clover</span><small>${i === s.k ? 'now' : solved ? 'solved ✓' : ''}</small>`;
    b.onclick = () => { stats.presses++; X.jump(i); drainOutbox(); closeSheets(); renderUi(true); };
    return b;
  }));
  $('cloverSheet').classList.add('open');
}
$('cloverClose').onclick = closeSheets;
$('invite').onclick = async () => {
  stats.presses++;
  const url = location.href.replace(/[?&]fresh\b[^&#]*/, '');
  try {
    if (navigator.share) return await navigator.share({ title: 'Shamrock', text: 'Join my Shamrock table', url });
  } catch (e) { if (e?.name === 'AbortError') return; }
  try { await navigator.clipboard.writeText(url); toast('Invite link copied'); }
  catch { toast(url, 9000); }
};

// ---- this device's tables and game log (localStorage only) ------------------------

const TABLES = 'shamrock:tables', LOG = 'shamrock:log', LOG_MAX = 60;
const readJson = (k, d) => { try { return JSON.parse(recall(k)) ?? d; } catch { return d; } };
const newRoom = () => Math.random().toString(36).slice(2, 8);
const tableUrl = (room) => `${location.origin}${location.pathname}${room ? `#${room}` : ''}`;
const goTo = (room) => { location.href = tableUrl(room); location.reload(); };
const when = (t) => (t ? new Date(t).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }) : 'earlier');
const guesses = (n) => `${n} guess${n === 1 ? '' : 'es'}`;
function noteTable(name) {
  const t = readJson(TABLES, {});
  t[roomId] = { name, at: Date.now() };
  remember(TABLES, JSON.stringify(t));
}
function myTables() {
  const t = readJson(TABLES, {});
  try { // Visits from before this list only left a saved name.
    for (let i = 0; i < localStorage.length; i++) {
      const m = localStorage.key(i).match(/^shamrock:(.+):name$/);
      if (m && !t[m[1]]) t[m[1]] = { name: recall(m[0]), at: 0 };
    }
  } catch {}
  return Object.entries(t).map(([room, v]) => ({ room, ...v })).sort((a, b) => b.at - a.at);
}
/** Keep a finished round's clovers (answers included) on this device. */
function logRound(s) {
  const boards = scratchJson(X.review());
  if (!boards.length) return;
  const id = boards.map((b) => b.text).join('\n\n');
  const log = readJson(LOG, []);
  if (log.some((e) => e.id === id)) return;
  log.unshift({ id, room: roomId, round: s.round, at: Date.now(), players: s.players.map((p) => p.name), boards });
  remember(LOG, JSON.stringify(log.slice(0, LOG_MAX)));
}
/** A small read-only clover: clues around four answer cards, decoys below. */
function cloverHtml(b) {
  const [name, ...rest] = b.text.split('\n');
  const clues = rest.slice(0, 4), cards = rest.slice(4).map((l) => l.split('|'));
  const card = (w) => `<div class="mc">${['t', 'r', 'b', 'l'].map((k, i) => `<span class="${k}">${esc(w?.[i] ?? '')}</span>`).join('')}</div>`;
  const [tl, tr, br, bl] = cards;
  return `<div class="mini">
    <div class="who"><b>${esc(name)}</b> · ${b.solved ? `solved in ${guesses(b.guesses)}` : 'not solved'}</div>
    <div class="mclover"><span class="ct">${esc(clues[0])}</span><span class="cr">${esc(clues[1])}</span>
      <div class="mgrid">${card(tl)}${card(tr)}${card(bl)}${card(br)}</div>
      <span class="cb">${esc(clues[2])}</span><span class="cl">${esc(clues[3])}</span></div>
    ${cards.length > 4 ? `<div class="decoys">Decoys: ${cards.slice(4).map((w) => esc(w.join(' · '))).join('; ')}</div>` : ''}
  </div>`;
}
function openLog() {
  const log = readJson(LOG, []);
  const list = $('logList');
  list.replaceChildren(...log.map((e, i) => {
    const d = document.createElement('details');
    d.className = 'logEntry';
    d.innerHTML = `<summary><b>${when(e.at)}</b> · round ${e.round} at #${esc(e.room)}<br><small>${esc(e.players.join(', '))} · ${e.boards.length} clover${e.boards.length === 1 ? '' : 's'}</small></summary>
      ${e.boards.map(cloverHtml).join('')}
      <div class="row"><button class="go replay">Play these again</button><button class="forget">Delete</button></div>`;
    d.querySelector('.replay').onclick = async () => { location.href = await shareFromText(e.boards.map((b) => b.text)); };
    d.querySelector('.forget').onclick = () => {
      remember(LOG, JSON.stringify(readJson(LOG, []).filter((x) => x.id !== e.id)));
      d.remove();
    };
    return d;
  }));
  $('logEmpty').hidden = log.length > 0;
  closeSheets();
  $('logSheet').classList.add('open');
}
function openHome() {
  const tables = myTables();
  $('tableList').replaceChildren(...tables.map((t) => {
    const a = document.createElement('a');
    a.className = 'item tableRow';
    a.href = tableUrl(t.room);
    a.onclick = (e) => { e.preventDefault(); goTo(t.room); };
    a.innerHTML = `<span class="grow"><b>#${esc(t.room)}</b><br><small>as ${esc(t.name ?? '?')} · ${when(t.at)}</small></span><span aria-hidden="true">›</span>`;
    return a;
  }));
  $('tablesHead').hidden = !tables.length;
  const n = readJson(LOG, []).length;
  $('homeLog').hidden = !n;
  $('homeLog').textContent = `Past games (${n})`;
  $('homeSheet').classList.add('open');
}
$('homeNew').onclick = () => goTo(newRoom());
$('homeLog').onclick = openLog;
$('logClose').onclick = () => { closeSheets(); if (home) openHome(); };
$('logMenuBtn').onclick = openLog;
$('homeMenuBtn').onclick = () => goTo('');

// ---- network ----------------------------------------------------------------

const peers = new Set();
const joined = (id) => { peers.add(id); X.on_peer_join(); drainOutbox(); renderUi(true); };
const left = (id) => { peers.delete(id); X.on_link_lost(linkId(id)); renderUi(true); };

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
  // Players find each other through public Nostr relays. Trystero's default
  // picks 5 hobby relays by hashing the app id, and when they're down or
  // slow nobody meets until a reload happens to get through; these are
  // large, widely run open relays, all used at once.
  const config = { appId: 'shamrock-p2p', relayConfig: { urls: signal ? [signal] : RELAYS } };
  let room, action;
  const enter = () => {
    room = lib.joinRoom(config, roomId);
    action = room.makeAction('m');
    action.onMessage = (data, { peerId }) => receive(new Uint8Array(data), linkId(peerId));
    room.onPeerJoin = joined;
    room.onPeerLeave = left;
    stats.joins++;
  };
  enter();
  // Alone? Announce again from scratch (what a reload did), soon at first,
  // then backing off; and at once when the page comes back to the
  // foreground, since phones drop sockets in background tabs.
  let since = performance.now(), wait = REJOIN_FIRST_MS;
  const rejoin = () => {
    try { room.leave(); } catch {}
    enter();
    since = performance.now();
  };
  setInterval(() => {
    if (peers.size) { since = performance.now(); wait = REJOIN_FIRST_MS; return; }
    if (performance.now() - since > wait) { wait = Math.min(wait * 1.6, 30000); rejoin(); }
  }, 1000);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible' && !peers.size) rejoin(); });
  addEventListener('pagehide', () => room.leave());
  return { send: (u8) => peers.size && action.send(u8), rejoin };
}
// ?relays=host1,host2 overrides the list (for testing from a phone).
const RELAYS = params.get('relays')?.split(',').map((h) => (h.includes('://') ? h : `wss://${h}`))
  ?? ['wss://relay.damus.io', 'wss://nos.lol', 'wss://relay.primal.net', 'wss://nostr.mom', 'wss://relay.snort.social', 'wss://offchain.pub'];
const REJOIN_FIRST_MS = +(params.get('rejoin') ?? 12000);

// Solo play (a share link) never connects to anyone.
if (!solo && !home) connect().then((n) => { net = n; }, (e) => toast(`Can't connect: ${e.message}`, 10000));

// Returning to this room re-takes your seat; a first visit asks. Wait a
// moment first so we've heard the table's state (and existing names).
if (home) {
  document.body.classList.add('home');
  openHome();
} else if (solo) {
  // Just play: an anonymous local seat, straight into the shared clovers.
  document.body.classList.add('solo');
  X.join(writeText('You'));
  if (customNames.length) X.action(1);
  drainOutbox();
} else {
  const saved = params.has('fresh') ? null : recall(nameKey);
  if (saved) noteTable(saved);
  if (saved) setTimeout(() => joinAs(saved), 1500);
  else if (saved === null) setTimeout(() => { if (!ui().seated) openJoin(); }, 900);
}

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
  setTheme: (t) => { themePref = t; applyTheme(); },
  peerCount: () => peers.size,
  camera: () => ({ ...cam }),
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
  shareFromText,
  unseal,
  wordFit: () => wordsMeasured && scratchJson(X.word_fit()),
  wordScales: (card) => scratchJson(X.card_word_scales(writeText(card.join('|')))),
  wordSize: () => X.word_size(),
  measure: async (words, size = X.word_size()) => {
    await document.fonts.load(`700 ${size}px Figtree`);
    const c = document.createElement('canvas').getContext('2d');
    c.font = FONTS[1](size);
    return words.map((w) => [w, c.measureText(w).width]);
  },
};
