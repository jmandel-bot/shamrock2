//! What the page says: the status strip (its words, shorter versions for
//! narrow screens, and which buttons show), the menu's switches, the
//! recap's heading, and the sentences for notices. The shell only shows
//! these; every decision is made here, where native tests and the network
//! simulator can check it.

use super::*;

/// `App::action` codes. The strip names them (`"deal"`, `"ready"`, …) for
/// the shell, which also has two of its own: `"join"` and `"ownTable"`.
pub const ACT_DEAL: u32 = 1;
pub const ACT_READY: u32 = 2;
pub const ACT_CHECK: u32 = 3;
pub const ACT_NEXT: u32 = 4;
pub const ACT_PROCEED: u32 = 5;
pub const ACT_REDEAL: u32 = 6;
pub const ACT_NEW_GAME: u32 = 7;

/// Marks around text the shell shows in bold (a name).
pub const BOLD_ON: char = '\u{1}';
pub const BOLD_OFF: char = '\u{2}';

/// A strip button: its label, whether it can be pressed, and what it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    pub enabled: bool,
    pub act: &'static str,
}

/// The status strip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Strip {
    /// The status in plain words, longest first: the shell shows the first
    /// that fits beside the buttons.
    pub hint: Vec<String>,
    /// Not connected: a pulsing dot before the words.
    pub pulse: bool,
    pub go: Option<Button>,
    pub alt: Option<Button>,
    /// The decoy stepper shows on the board.
    pub decoys: bool,
    /// Tapping the status opens `"clovers"` (pick which one) or `"recap"`.
    pub tap: Option<&'static str>,
    pub invite: bool,
    /// The how-to tip for now (`"write"` or `"guess"`); shown once per device.
    pub tip: Option<&'static str>,
    /// The submission progress bar shows.
    pub segs: bool,
}

const ORDINAL: [&str; 10] = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];

fn bold(s: &str) -> String {
    format!("{BOLD_ON}{s}{BOLD_OFF}")
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// "a", "a and b", "a, b and c".
pub fn and_list(xs: &[String]) -> String {
    match xs {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn button(label: &str, enabled: bool, act: &'static str) -> Option<Button> {
    Some(Button { label: label.into(), enabled, act })
}

impl App {
    /// How the strip calls a player: their first name, or the whole name
    /// when another player shares that first name; at most 12 characters,
    /// so a long name can't push the buttons off.
    pub fn short_name(&self, seat: u32) -> String {
        let first = |n: &str| n.split_whitespace().next().unwrap_or("").to_string();
        let name = self.name_of(seat);
        let f = first(&name);
        let clash = self.shared.players().iter().any(|&p| p != seat && first(&self.name_of(p)) == f);
        let t = if clash { name } else { f };
        if t.chars().count() > 12 {
            format!("{}…", t.chars().take(11).collect::<String>())
        } else {
            t
        }
    }

    pub fn strip(&self) -> Strip {
        let s = &self.shared;
        let phase = s.phase();
        let n = s.puzzles().len();
        let seated = self.seat.is_some_and(|me| s.players().contains(&me));
        let (k, writer) = match phase {
            Phase::Guessing(k) => (k, s.puzzles().get(k).copied()),
            _ => (0, None),
        };
        let done = writer.is_some_and(|w| s.check(w).done());
        let is_writer = writer.is_some() && writer == self.seat;
        let whose = writer.map(|w| format!("{} clover", bold(&format!("{}'s", self.short_name(w))))).unwrap_or_default();
        let clover = vec![
            format!("{whose}, {} of {n}", ORDINAL.get(k).map_or(format!("number {}", k + 1), |o| o.to_string())),
            format!("{whose}, {} of {n}", k + 1),
            whose.clone(),
        ];
        let solved = writer.map(|w| format!("Solved {} clover!", bold(&format!("{}'s", self.short_name(w))))).unwrap_or_default();
        let dealable = !self.dealable().is_empty();
        let mut st = Strip { segs: phase == Phase::Writing, ..Strip::default() };

        if self.solo {
            match phase {
                Phase::Guessing(_) if done => {
                    st.hint = vec![solved];
                    st.go = button(if k + 1 < n { "Next clover" } else { "Done" }, true, "next");
                }
                Phase::Guessing(_) => {
                    st.hint = clover;
                    st.go = button("Guess", self.can_check(), "check");
                }
                _ => {
                    st.hint = vec![if Custom::all().is_empty() { "No clovers in this link" } else { "All done" }.into()];
                    st.go = button("Start your own table", true, "ownTable");
                }
            }
        } else if !seated {
            st.hint = vec![match phase {
                _ if s.players().is_empty() => "No one's playing yet".into(),
                Phase::Guessing(_) => format!("Watching {whose}"),
                _ => "Watching".into(),
            }];
            st.go = button("Join", true, "join");
        } else {
            match phase {
                Phase::Lobby => {
                    let customs = Custom::all();
                    if self.custom_room && s.round.n == 0 && !customs.is_empty() {
                        let names: Vec<String> = customs.iter().map(|c| bold(&c.name)).collect();
                        st.hint = vec![format!("Opens with {}", and_list(&names))];
                        st.go = button("Start guessing", dealable, "deal");
                    } else {
                        let here = self.dealable().len();
                        st.hint = vec![if here < 2 { "Just you here".into() } else { format!("{here} players here") }];
                        st.go = button("Start", dealable, "deal");
                    }
                }
                Phase::Writing => {
                    let in_round = self.seat.is_some_and(|me| s.round.players().contains(&me));
                    let sub = self.my_sub();
                    let filled = sub.clues.iter().filter(|c| !c.is_empty()).count();
                    let round = s.round_players();
                    let left: Vec<String> = round.iter().filter(|p| !s.is_ready(**p)).map(|&p| bold(&self.short_name(p))).collect();
                    let ready = self.ready_count();
                    let who = if left.is_empty() { "the others".into() } else { and_list(&left) };
                    let mut waiting = vec![format!("Waiting for {who}")];
                    if left.len() > 1 {
                        waiting.push(format!("Waiting for {}", plural(left.len(), "player")));
                        waiting.push(format!("Waiting for {}", left.len()));
                    }
                    if !in_round {
                        st.hint = vec!["You're in the next round".into()];
                    } else if !sub.ready {
                        st.tip = Some("write");
                        st.hint = vec![if filled < 4 { format!("{filled} of 4 clues written") } else { format!("Clues done · {}", plural(sub.decoys as usize, "decoy")) }];
                        st.go = button("Submit", self.can_ready(), "ready");
                    } else {
                        st.hint = [vec![format!("Submitted · waiting for {who}")], waiting.clone()].concat();
                    }
                    st.decoys = in_round && filled >= 4;
                    if (sub.ready || !in_round) && ready > 0 && ready < round.len() {
                        // The status beside it names who is missing, so the button needn't.
                        st.alt = button("Start anyway", true, "proceed");
                        st.hint = if sub.ready { waiting } else { [vec![format!("You're in next round · waiting for {who}")], waiting].concat() };
                    }
                    // Everyone dealt in has gone and nobody submitted: nothing
                    // to start with, so offer a fresh deal for whoever is here.
                    let present = self.present();
                    if !in_round && ready == 0 && round.iter().all(|p| !present.contains(p)) && dealable {
                        st.hint = vec!["The writers have left".into()];
                        st.go = button("Deal again", true, "redeal");
                    }
                }
                Phase::Guessing(_) if done => {
                    st.hint = vec![solved];
                    st.go = button(if k + 1 < n { "Next clover" } else { "Finish round" }, true, "next");
                }
                Phase::Guessing(_) if is_writer => st.hint = vec!["They're guessing your clover".into()],
                Phase::Guessing(_) => {
                    st.hint = clover;
                    st.go = button("Guess", self.can_check(), "check");
                }
                Phase::End => {
                    st.hint = vec![format!("Round {} is over", s.round.n)];
                    st.go = button("Next round", dealable, "deal");
                }
            }
        }
        if matches!(phase, Phase::Guessing(_)) && !(is_writer && !done) && (self.solo || seated) {
            st.tip = Some("guess");
        }
        // Not connected to anyone: say so rather than look normal while broken.
        if !self.solo && self.links == 0 {
            st.pulse = true;
            let others = s.players().iter().any(|&p| Some(p) != self.seat);
            st.hint = vec![if others { "Reconnecting to the table…" } else { "Waiting for others" }.into()];
        }
        st.tap = match phase {
            Phase::Guessing(_) if n > 1 => Some("clovers"),
            Phase::End if n > 0 => Some("recap"),
            _ => None,
        };
        st.invite = !self.solo && (phase == Phase::Lobby || self.links == 0);
        st.tip = st.tip.filter(|_| seated || self.solo);
        st
    }

    pub(super) fn strip_json(&self) -> String {
        let st = self.strip();
        let btn = |b: &Option<Button>| {
            b.as_ref().map_or("null".into(), |b| format!("{{\"label\":{},\"enabled\":{},\"act\":\"{}\"}}", json_str(&b.label), b.enabled, b.act))
        };
        let opt = |o: Option<&str>| o.map_or("null".into(), |x| format!("\"{x}\""));
        let hint: Vec<String> = st.hint.iter().map(|h| json_str(h)).collect();
        format!(
            "{{\"hint\":[{}],\"pulse\":{},\"go\":{},\"alt\":{},\"decoys\":{},\"tap\":{},\"invite\":{},\"tip\":{},\"segs\":{}}}",
            hint.join(","),
            st.pulse,
            btn(&st.go),
            btn(&st.alt),
            st.decoys,
            opt(st.tap),
            st.invite,
            opt(st.tip),
            st.segs
        )
    }

    /// Menu items that only make sense some of the time, and the recap's
    /// heading and buttons.
    pub(super) fn menu_json(&self) -> String {
        let s = &self.shared;
        let seated = self.seat.is_some_and(|me| s.players().contains(&me));
        let unsubmit = s.phase() == Phase::Writing && seated && self.my_sub().ready;
        let head = if self.solo { "All done ☘".to_string() } else { format!("Round {} done ☘", s.round.n) };
        format!(
            "{{\"unsubmit\":{unsubmit},\"share\":{},\"recapHead\":{},\"recapShare\":{},\"recapNext\":{}}}",
            !s.puzzles().is_empty(),
            json_str(&head),
            !self.solo,
            !self.solo && seated
        )
    }

    /// The sentence for a change of round someone else made (see
    /// `finish_change`), and the button offered with it, if any.
    pub(super) fn notice_text(&self, kind: &str, others: &[String], kept_unfinished: bool) -> (String, Option<(&'static str, &'static str)>) {
        let new = self.shared.round;
        let who = if new.dealer != 0 { self.name_of(new.dealer) } else { "Someone".into() };
        let dealt_in = self.seat.is_some_and(|me| new.players().contains(&me));
        let later = if dealt_in { "" } else { " You're in next round." };
        let n = new.n;
        let mut text = match kind {
            "next" => format!("{who} dealt round {n}.{later}"),
            "missed" => format!("{who} dealt round {n} while you were away.{later}"),
            "crossed" => format!("{who} dealt at the same moment as you; continuing {who}'s.{later}"),
            "newGame" => format!("{who} started a new game."),
            _ => format!(
                "{} {} also been playing at this table.{later}",
                if others.is_empty() { "Others".into() } else { and_list(others) },
                if others.len() == 1 { "has" } else { "have" }
            ),
        };
        if kept_unfinished {
            text.push_str(" Your round is in Past games.");
        }
        let action = (kind == "otherTable").then_some(("move", "Move us to a new table"));
        (text, action)
    }

    /// A table saved on this device, from `version` of the protocol and
    /// `age_ms` old: merged in like any other sync (so a stale save can't
    /// undo newer moves) if it's from this version and recent enough.
    pub fn restore(&mut self, bytes: &[u8], version: u32, age_ms: f64) -> bool {
        if version != PROTOCOL as u32 || !(0.0..SAVE_TTL_MS).contains(&age_ms) {
            return false;
        }
        self.on_packet(bytes, 0);
        true
    }
}

/// How long a save on this device stays good for restoring a table.
pub const SAVE_TTL_MS: f64 = 6.0 * 3600e3;
