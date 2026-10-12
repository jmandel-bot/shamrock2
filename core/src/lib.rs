//! Raw C-ABI exports for the browser shim. No wasm-bindgen: JS reads the
//! command buffer / outbox straight out of linear memory and writes inbound
//! bytes and text into `inbox`.

pub mod app;
pub mod model;
pub mod words;

use app::App;

struct Globals {
    app: App,
    inbox: Vec<u8>,
    scratch: Vec<u8>,
    probe: [f32; 4],
}

static mut G: Option<Globals> = None;

fn g() -> &'static mut Globals {
    // Single-threaded wasm; the shim never re-enters.
    unsafe { (*core::ptr::addr_of_mut!(G)).as_mut().expect("init not called") }
}

fn inbox_str(len: u32) -> String {
    let ib = &g().inbox;
    String::from_utf8_lossy(&ib[..(len as usize).min(ib.len())]).into_owned()
}

fn to_scratch(s: String) -> u32 {
    g().scratch = s.into_bytes();
    g().scratch.len() as u32
}

/// Returns this device's id.
#[no_mangle]
pub extern "C" fn init(seed_lo: u32, seed_hi: u32) -> u32 {
    let app = App::new((seed_hi as u64) << 32 | seed_lo as u64);
    let id = app.device;
    unsafe {
        *core::ptr::addr_of_mut!(G) = Some(Globals { app, inbox: Vec::new(), scratch: Vec::new(), probe: [0.0; 4] });
    }
    id
}

#[no_mangle]
pub extern "C" fn resize(w: f32, h: f32, dpr: f32) {
    let a = &mut g().app;
    (a.w, a.h, a.dpr) = (w.max(1.0), h.max(1.0), dpr.max(0.5));
}

/// Advances animation, flushes network, renders. Returns command count.
#[no_mangle]
pub extern "C" fn frame(now: f64) -> u32 {
    g().app.frame(now);
    g().app.cmds.len() as u32
}

#[no_mangle]
pub extern "C" fn cmd_ptr() -> *const f32 {
    g().app.cmds.as_ptr()
}

#[no_mangle]
pub extern "C" fn str_ptr() -> *const u8 {
    g().app.strs.as_ptr()
}

#[no_mangle]
pub extern "C" fn outbox_ptr() -> *const u8 {
    g().app.outbox.as_ptr()
}

#[no_mangle]
pub extern "C" fn outbox_len() -> u32 {
    g().app.outbox.len() as u32
}

#[no_mangle]
pub extern "C" fn outbox_clear() {
    g().app.outbox.clear();
}

/// Returns a buffer of at least `len` bytes for JS to fill before a call
/// that consumes the inbox.
#[no_mangle]
pub extern "C" fn inbox_reserve(len: u32) -> *mut u8 {
    let ib = &mut g().inbox;
    ib.resize(len as usize, 0);
    ib.as_mut_ptr()
}

/// Inbound packet (in the inbox) from the peer on connection `link`.
#[no_mangle]
pub extern "C" fn on_message(len: u32, link: u32) {
    let bytes = core::mem::take(&mut g().inbox);
    g().app.on_packet(&bytes[..(len as usize).min(bytes.len())], link);
    g().inbox = bytes;
}

#[no_mangle]
pub extern "C" fn on_link_lost(link: u32) {
    g().app.on_link_lost(link);
}

#[no_mangle]
pub extern "C" fn on_peer_join() {
    g().app.on_peer_join();
}

#[no_mangle]
pub extern "C" fn pointer_down(x: f32, y: f32) {
    g().app.pointer_down(x, y);
}

#[no_mangle]
pub extern "C" fn pointer_move(x: f32, y: f32) {
    g().app.pointer_move(x, y);
}

#[no_mangle]
pub extern "C" fn pointer_up(x: f32, y: f32) {
    g().app.pointer_up(x, y);
}

#[no_mangle]
pub extern "C" fn rotate_view(dir: i32) {
    g().app.rotate_view(dir);
}

#[no_mangle]
pub extern "C" fn action(code: u32) {
    g().app.action(code);
}

/// Name is in the inbox. Joining with an existing name takes that seat over.
#[no_mangle]
pub extern "C" fn join(len: u32) {
    let s = inbox_str(len);
    g().app.join(&s);
}

#[no_mangle]
pub extern "C" fn watch() {
    g().app.watch();
}

#[no_mangle]
pub extern "C" fn remove_seat(id: u32) {
    g().app.remove_seat(id);
}

/// Local card size (display only). Returns the clamped value.
#[no_mangle]
pub extern "C" fn set_zoom(z: f32) -> f32 {
    g().app.set_zoom(z);
    g().app.zoom
}

#[no_mangle]
pub extern "C" fn set_decoys(n: u32) {
    g().app.set_decoys(n.min(255) as u8);
}

/// Clue text is in the inbox. `advance` != 0 opens the next empty leaf.
#[no_mangle]
pub extern "C" fn set_clue(idx: u32, len: u32, advance: u32) {
    let s = inbox_str(len);
    g().app.set_clue(idx as u8, &s, advance != 0);
}

/// Re-send the clue editor geometry for leaf `d` (after a resize).
#[no_mangle]
pub extern "C" fn edit_clue(d: u32) {
    g().app.edit_clue((d % 4) as u8);
}

/// UI state (JSON) → scratch; returns its length.
#[no_mangle]
pub extern "C" fn ui() -> u32 {
    to_scratch(g().app.ui_json())
}

/// Canonical shared-state digest (JSON) → scratch; returns its length.
#[no_mangle]
pub extern "C" fn digest() -> u32 {
    to_scratch(g().app.shared.digest())
}

#[no_mangle]
pub extern "C" fn scratch_ptr() -> *const u8 {
    g().scratch.as_ptr()
}

// ---- harness probes --------------------------------------------------------

#[no_mangle]
pub extern "C" fn settle() {
    g().app.settle();
}

#[no_mangle]
pub extern "C" fn view_quarter() -> i32 {
    g().app.q
}

#[no_mangle]
pub extern "C" fn probe_ptr() -> *const f32 {
    g().probe.as_ptr()
}

fn probe2((x, y): (f32, f32)) {
    g().probe = [x, y, 0.0, 0.0];
}

#[no_mangle]
pub extern "C" fn probe_card(i: u32) {
    let (x, y, a) = g().app.card_screen(i as usize);
    g().probe = [x, y, a, 0.0];
}

#[no_mangle]
pub extern "C" fn probe_slot(s: u32) {
    probe2(g().app.slot_screen((s as usize).min(model::NSLOTS - 1)));
}

#[no_mangle]
pub extern "C" fn probe_board(x: f32, y: f32) {
    probe2(g().app.board_screen(x, y));
}

#[no_mangle]
pub extern "C" fn probe_petal(d: u32) {
    probe2(g().app.petal_screen((d % 4) as u8));
}

#[no_mangle]
pub extern "C" fn probe_hub() {
    probe2(g().app.hub_screen());
}

#[no_mangle]
pub extern "C" fn probe_shown_angle(i: u32) -> f32 {
    g().app.shown_angle(i as usize)
}

/// Rendered size of card words, in CSS px.
#[no_mangle]
pub extern "C" fn probe_word_px() -> f32 {
    app::WORD_SIZE * g().app.scale() * g().app.zoom
}

/// Rendered card edge length, in CSS px.
#[no_mangle]
pub extern "C" fn probe_card_px() -> f32 {
    model::CARD * g().app.scale() * g().app.zoom
}

/// Harness only: correct rotation for spot `s` of the clover being guessed
/// (255 if none). Submissions are shared state, so this is no secret from
/// devtools anyway; the game is played on trust.
#[no_mangle]
pub extern "C" fn answer_rot(s: u32) -> u32 {
    let a = &g().app;
    match a.scene().writer {
        Some(w) if matches!(a.shared.phase(), model::Phase::Guessing(_)) => {
            model::deal(a.shared.round.seed, w).sol_rot[(s as usize) % model::NSLOTS] as u32
        }
        _ => 255,
    }
}
