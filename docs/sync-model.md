# Shamrock sync model

Status: **proposed**. It replaces the special cases added along the way (set-aside detection by deck shuffle, separate end-of-round logging) with one model. The parts marked *(exists)* are already built and tested; *(new)* is what this proposal adds or changes, each with its tests. Nothing is deferred.

## What it has to handle

Everything strange that happens at a table falls into a few general cases:

| Case | Examples |
|---|---|
| **One table, in order** | Normal play: deal, write, guess, next round. |
| **One table, diverged** | A partition heals; two people deal or press Proceed at once; a phone that was offline for three rounds comes back; a reload restores a stale save. |
| **Two tables, one name** | Two groups used the same room name without ever meeting, or one group split up and played on separately, and then they meet. |
| **Comings and goings** | Joining late, reloading, closing a tab mid-gesture, everyone leaving and someone coming back, being alone. |

The model should treat each case by its *relationship*, never by the story that caused it.

## Principles

1. **The room name is the address, and a room holds one live table.** Anyone who opens the link is at that table.
2. **The shared table is small and current: the seats plus the current round.** Nothing else syncs. A newcomer gets exactly what is happening now, never the past.
3. **Merging is a join.** Every rule is commutative, associative and idempotent, so devices agree whatever order messages arrive in, and nothing ever needs undoing.
4. **History is personal.** Each device keeps its own copy of every round it took part in (Past games). Work is never lost from the device that did it, even when the table moves on without it.
5. **Every change of round is explained, the same way whatever caused it**, from the round's own header.
6. **Presence is not state.** Who is here, pointers and "someone is dragging this card" live in short-lived messages that end when a device leaves. They never enter the table, so they can't go stale.
7. **The table heals itself.** It does this without anyone needing to notice.

## The table (replicated)

```
seats:   seat id → { name, active, stamp }          last writer wins per seat
moves:   table id → { room, stamp }                where a group moved to (new)
round:   header + body
  header { table, epoch, n, parent, dealer,         which round this is, and where it came from
           players, seed, deck, stamp, custom }
  body   { clue versions per writer, start,         merged per register, as today
           cards, checks, next clover }
```

- **Seats** *(exists)* are keyed by name: picking a name takes that seat. A seat is never deleted, only marked inactive (Remove, or New game for players not present). A device that is here keeps its own seat active.
- **The round header** identifies the round and its lineage:
  - `table` *(new)* is fixed at the very first deal at this table, and is inherited by every round after it, across New games. It answers "is this the same table as mine?"
  - `epoch` *(exists)* counts New games.
  - `n` *(exists)* is the round number within the game.
  - `parent` *(new)* is the stamp of the round this one followed.
  - `dealer` *(new)* is the seat that dealt it, so changes can be attributed.
  - The deck shuffle is derived from `(table, epoch)`, so the `deck_seed` field goes *(new, a simplification)*.
- **The round body** is the same as today *(exists)*:
  - Every version of every writer's clues is kept, so "what counted when guessing started" is well defined even when an edit races Proceed.
  - Guessing starts at the latest Start.
  - Cards are last writer wins; checks only grow; the clover being guessed is last writer wins.
  - An op that belongs to a different round than the current one is ignored by the table, though see "History is personal" below.

## Which round is current

When two rounds meet, one total order decides, the same on every device:

1. **Higher epoch.** A New game beats everything before it.
2. **Higher round number.** The game further along carries on.
3. **More players dealt in.** A lone device's deal can't beat the whole table's.
4. **Older.** An established deal beats a later one.

The winner becomes current. On a device whose round lost, the round body is cleared and the seats are untouched.

This order is the *mechanism*. It is meaningful for one table (cases 1–2). For two tables it is merely deterministic, which is why case 3 also gets a human choice (see "Moving your group").

## Explaining a change *(new)*

Whenever a device's current round is replaced by one it didn't deal, it compares the incoming header with the round it had. One classifier covers every cause:

| Incoming round vs. mine | What it means | Notice (not shown to the one who caused it) |
|---|---|---|
| I had no table yet | Arriving (first visit, empty save) | none |
| `parent` is my round | The table moved on | "Cy dealt round 3." / "Cy dealt round 3; you're in next round." |
| Same `table` and `epoch`, `parent` isn't mine | I missed rounds, or two deals crossed | "Ben dealt round 3 while you were away." or, if mine was also a fresh deal, "Ben dealt at the same moment; continuing Ben's." |
| Same `table`, higher `epoch` | Someone started a New game | "Ana started a new game." |
| Different `table` | Another group has been playing in this room | "Cy and Dee have also been playing at this table." plus **Move us to a new table** |

Whenever the round I'm leaving had anything in it, the notice adds "Your round is in Past games."

## History is personal *(new: one rule replaces two paths)*

**When the round a device is in stops being current, for any reason, the device writes its own copy to Past games.** Reasons include finishing, a teammate dealing again, losing a crossed deal, a New game, or another table winning.

- The copy is whatever this device had: clues (drafts included), placements, guesses.
- A finished round is recorded as finished. Anything else is marked *unfinished*, with each clover marked done, guessed so far, or not guessed.
- *Play these again* offers only clovers that have all four clues. Half-written ones can be read but not replayed.
- Past games is capped at the last 60 rounds per device *(exists)*.
- **Late edits** *(new)*: each device keeps the last round it was pushed out of alive in the background, until it next deals or joins a new round of its own (at most 10 minutes). A change that arrives for that round merges into it, updating the Past games copy and what a Move would carry. A teammate's last edit that crossed the switch in flight still lands.
- Older rounds than that are only on the devices that played them. The table never shares the past.

## Moving your group *(new)*

When two tables meet in one room, playing together is the default and needs nothing. The notice on the outranked side offers **Move us to a new table**:

1. **One tap, by anyone in the group.** Their device opens a fresh room under a suggested name, seeded with its copy of the group's table (the round they were in, and their seats). They arrive seated, mid-round, where they were.
2. **The room remembers the move.** A small shared record goes into the old room: `moves[group's table id] = new room`, last writer wins, a few bytes. Every device of that group sees it whenever it next connects: now, after a reload, or later from its save.
3. **The rest of the group gets one tap to follow:** "Ana moved your group to #quiet-heron-dell. Go there?" Following seeds the new room with *their own* copy too. Copies of the same round merge, so everyone's drafts arrive.
4. **Nobody moves without choosing.** The other group isn't affected and keeps the room. Anyone who ignores the prompt stays at the merged table.

The same action is in the ⋯ menu at any time: **Start a fresh table with this group**.

## Presence and pointers *(exists)*

- **Hello every 2 s:** device, seat and a hash of the table. Who is "here" is who has a live connection.
- **Pointer messages** (at most 30 per second, expiring after 4 s) carry where a guesser's finger is and which card they're holding. A hold ends when they let go, go quiet or disconnect.

## Self-healing *(exists)*

| Mechanism | Covers |
|---|---|
| Full sync each time a connection opens (seats + current round) | Anyone joining or reconnecting |
| Table hash in every Hello: if a peer's differs twice running with neither side changing, resync (at most every 5 s) | Devices that drifted apart while staying connected (a change that missed someone whose sender then left) |
| Each device saves the table to localStorage (every 2 s and on leaving) and restores it on load if under 6 h old | Every holder reloading or closing; a lone device coming back |
| A device's saved name rejoins 1.5 s after load | Reloads |

A restored save is not trusted. It merges like any other sync and is classified like any other round. So a stale one is either caught up quietly or recognised as another table.

## What players can do

| Situation | Affordance |
|---|---|
| A round changes under you | The notice above (one line, about 8 s), naming who and what. |
| Your round is replaced | It's in Past games, marked unfinished, readable, with complete clovers replayable. |
| Two tables meet in one room *(new)* | Default: play together. Nothing blocks, the next deal includes everyone. The notice offers **Move us to a new table**: you arrive at a fresh room seated and mid-round, and your group is offered one tap to follow. |
| You want your own table *(new)* | **Start a fresh table with this group** in the ⋯ menu: the same Move, any time. |
| Everyone dealt in has left, nobody submitted | **Deal again** in the strip *(exists)*. |
| Some players stuck writing | **Proceed with N of M** *(exists)*. |
| Alone or disconnected | Status says *Waiting for others* / *Reconnecting…*; the page retries on its own *(exists)*. |
| A round ends | The recap: every clover's answer, one after another *(exists)*. |

## Bounds

- **Shared table:** the seats plus one round. The only growth inside a round is clue versions (about 60–100 bytes per edit), so a chatty six-player round is about 15 KB. It never accumulates across rounds, so syncs, hashes and saves stay that size however long a table plays. No compaction needed.
- **Seats:** one per name ever used, about 30 bytes each; a hundred names is 3 KB. Rather than prune them (which a merge would undo), the simulator checks the table stays under a fixed budget (50 KB) through its longest games.
- **Moves:** one per group that ever moved out of a room, a few bytes each.
- **The last replaced round**, kept for late edits: at most one round per device, dropped after 10 minutes.
- **Past games:** 60 rounds per device.

## Guarantees, and how they're checked

The network simulator (`core/tests/network.rs`, 300 chaotic games per run, 5,000 in a soak) checks, after every run heals:

- one state on every device;
- nobody here without their seat;
- no device's table ever goes back to less progress;
- no hold outlives its hand;
- the round can be played to its end;
- *(new)* every round a device was in is in its Past games once replaced, with that device's last view of it;
- *(new)* every round change a device didn't cause produces exactly one notice, classified correctly (the simulator knows the true cause);
- *(new)* a late edit to a just-replaced round reaches every device that was in it, in Past games;
- *(new)* a moved group, once everyone follows, has one table at the new room with every member's work, and the old room is untouched;
- *(new)* the shared table stays under its size budget.

Fault scenarios in real browsers (`harness/faults.mjs`) cover the page's own handling: join storm, cut-off phone, lag, reload, everyone leaving, relay outage, death mid-drag, being alone, two groups meeting. *(new)* Each also checks the notice and the Past games entry. A new scenario moves a group: one taps Move, another follows from the prompt, a third reopens the old room later and is offered the move from its save; both rooms are checked afterwards.

## Changes from today

| Remove | Add |
|---|---|
| The set-aside event and its deck-shuffle test | `table`, `parent`, `dealer` in the round header |
| `deck_seed` (derived from table and epoch instead) | One classifier and one notice for every round change |
| End-of-round logging as a separate path | One rule: a round leaving current goes to Past games |
| | Past games: unfinished marks; replay only complete clovers |
| | The last replaced round kept briefly, so late edits land in it |
| | **Move us to a new table**: a move record in the room, a prompt to follow, and the same action in the menu |
| | A size budget for the table, checked by the simulator |
