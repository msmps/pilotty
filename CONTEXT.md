# pilotty

Terminal automation for AI agents: a daemon manages PTY-backed sessions so that agents
can run, observe, and operate interactive TUI applications through a CLI.

## Language

### Sessions and lifecycle

**Session**:
A PTY-backed process the daemon is currently running and emulating, addressed by id or name.
_Avoid_: pane, tab, terminal (for the managed thing itself)

**Tombstone**:
A bounded, short-lived record of an ended session: exit metadata plus final evidence.
Not a session; never counts toward session limits.
_Avoid_: dead session, session history

**Exited**:
The state of a session whose process has ended but whose tombstone is still available.
_Avoid_: killed (reserve for explicit `kill`), dead

**Expired**:
An ended session whose tombstone is gone (TTL, eviction, or daemon restart);
indistinguishable from never-existed.

**Finalization**:
The bounded transition from a live session to a tombstone: capture exit status, drain
output to EOF or the deadline, capture final evidence, then stop the session runtime.
_Avoid_: cleanup (too broad), reap (the cleaner's mechanism rather than the transition)

**Output complete**:
Whether the pump observed PTY EOF before finalization's drain deadline. False means the
final evidence is truthful but may omit later output from a descendant that kept the PTY
open.
_Avoid_: truncated (reserved for retention-ring capacity loss)

### Observation

**Snapshot**:
A point-in-time capture of a session's current screen text, cursor, and dimensions.
_Avoid_: frame, capture (as a noun)

**Revision**:
A monotonic per-session counter that advances whenever screen state may have changed;
the identity used for change detection and future diffs.
_Avoid_: version (reserved for protocol/releases), generation

**Settled**:
Screen content unchanged for the requested quiet window. A property of the *screen*.
_Avoid_: stable, quiet

**Idle**:
No output from the session's process for some duration. A property of the *process* —
a screen can be settled while the process is busy, and output can arrive without
visibly changing the screen.

**Capture outcome**:
Why a waited snapshot returned: `settled`, `changed`, `deadline`, or `exited`.
_Avoid_: capture reason

### Evidence

**Retention ring**:
The bounded in-memory buffer of a session's most recent raw output bytes. `output` renders
its readable terminal-history tail; `output --ansi` serves the exact retained bytes.
Always on, always bounded, truncation always reported.
_Avoid_: scrollback (the emulator concept), log file

**Recording**:
An opt-in, append-only file of everything a session emitted over time, replayable later.
Distinct from the retention ring, which is bounded and in-memory.
_Avoid_: cast (that is the export format), tape
