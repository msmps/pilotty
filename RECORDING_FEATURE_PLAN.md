# Pilotty Recording Feature Plan

## Purpose

Add daemon-managed terminal recording for spawned sessions without introducing runtime behavior in this change. The feature records rendered terminal content to MP4, not desktop video, and must preserve current Linux support while using a macOS-only render backend.

## Locked Requirements and Decisions

### CLI surface

- `pilotty spawn --record` enables recording for the spawned session.
- `pilotty spawn --record-path <PATH>` overrides the output path.
- If `--record-path` is omitted, the daemon writes to the Desktop using `pilotty-recording-YYYYMMDD-HHMMSS-SSS.mp4`.
- Filename generation must be collision-safe via suffixing when needed.

### Recording behavior

- Output is **terminal-content MP4**, not desktop capture.
- Recording is **daemon-managed**, not tied to the CLI process lifetime.
- Continuous PTY consumption is required while a recorded session is alive.
- The recorder sink is **lossy and non-blocking**; session correctness wins over recording completeness.
- Use an **append-only spool** for captured terminal frames/events plus **persisted render jobs**.
- Final success, failure, cancellation, and shutdown paths must use a **unified finalization flow**.

### Platform and packaging

- The render backend is **macOS-only**.
- Packaging should avoid heavy surprise dependencies.
- Recording must not break existing Linux support; unsupported recording requests must fail clearly while the rest of pilotty continues to work.

### API / protocol expectations

- Recording metadata must use structured states, not free-form strings.
- Structured responses should expose a nested optional `recording` object.
- Current teardown is fragmented; a small lifecycle refactor is warranted before or alongside recording.

## Architecture Overview

### High-level flow

1. CLI parses `spawn --record` and optional `--record-path`.
2. Spawn request carries recording intent to the daemon.
3. Daemon creates the PTY session and, if requested, allocates recording state.
4. PTY output is consumed continuously by a session reader path that updates both:
   - the terminal emulator used by snapshots and automation
   - a lossy, non-blocking recording sink that appends capture data to spool storage
5. When the session ends, is killed, or the daemon shuts down, a unified finalization path:
   - closes the spool
   - persists a render job
   - transitions recording status
   - optionally triggers or resumes rendering on macOS
6. Render completion updates persisted recording metadata so later structured responses can report the final state.

### Core design choices

#### 1. Continuous reader ownership

The current snapshot-driven PTY draining model is insufficient for recording. Recording needs a dedicated continuous consumption path so terminal output is captured even when no client is polling snapshots.

#### 2. Spool first, render second

Capture and rendering are separated:

- **Capture path:** lightweight, append-only, daemon-safe, optimized for not blocking PTY handling.
- **Render path:** asynchronous, persisted, resumable, macOS-only.

This keeps session interactivity decoupled from expensive video generation.

#### 3. Unified lifecycle finalization

Session exit, manual kill, idle shutdown, daemon stop, and capture/setup failures should converge on one finalization routine so recording state transitions and cleanup stay consistent.

## Lifecycle and State Machine

### Session + recording lifecycle

```text
spawn requested
  -> recording requested?
       no  -> session created
       yes -> pre-spawn recording validation
             -> validation failed -> spawn error
             -> validation succeeded -> session created + active
                                         -> finalizing
                                         -> render_pending
                                         -> rendering
                                         -> completed | failed | canceled
```

### Recording status states

Suggested durable state set:

- `active`
- `finalizing`
- `render_pending`
- `rendering`
- `completed`
- `failed`
- `canceled`

### State transition notes

- Recording requests that fail on unsupported platform, invalid path, spool creation failure, metadata persistence failure, or recorder startup initialization failure are **pre-spawn errors**, not in-session state transitions.
- `active -> finalizing` on normal child exit, manual kill, shutdown, or internal recorder termination.
- `finalizing -> render_pending` once spool closure and render job persistence succeed.
- `render_pending -> rendering -> completed|failed` under the macOS renderer.
- `active|finalizing|render_pending|rendering -> canceled` when teardown explicitly abandons the artifact.

## Failure Semantics and Recovery

### Non-blocking guarantees

- PTY reading must continue even if the recorder sink falls behind.
- If the sink cannot accept every frame/event, it may drop data rather than block terminal processing.
- Snapshot accuracy and input handling take priority over recording fidelity.

### Failure behavior

- Unsupported platform: if recording is requested on non-macOS, **spawn fails before PTY creation** with a clear unsupported-platform error.
- Capture setup failure: if output path validation, spool creation, initial metadata persistence, or backend initialization fails, **spawn fails before PTY creation** and no session is left running without recording.
- Runtime capture degradation: once recording has started successfully, later sink overflow or spool/runtime recorder failures degrade only the recording path and do not affect PTY/session lifetime.
- Render failure: retain spool/job metadata and surface `failed` status plus error detail for inspection or retry.
- Daemon crash/restart: persisted render jobs allow recovery; daemon startup should scan and resume runnable jobs on macOS.
- Partial capture: final artifact may be incomplete only after successful startup followed by later recording degradation; metadata must reflect whether capture or rendering degraded.

### Recovery model

- Append-only spool files are never rewritten in place.
- Persisted recording metadata is the source of truth for current status.
- Persisted render jobs make rendering resumable after daemon restart.
- Finalization must be idempotent so repeated teardown paths do not corrupt state.

## Structured Response and Status Model

### Nested recording object

Responses that already expose session data should optionally include:

```json
{
  "recording": {
    "enabled": true,
    "status": "render_pending",
    "output_path": "/Users/alice/Desktop/pilotty-recording-20260324-153012-123.mp4",
    "error": null
  }
}
```

### Minimum recording fields

- `enabled: bool`
- `status: enum`
- `output_path: Option<String>`
- `error: Option<{ code, message }>`

### Optional extended fields

- `recording_id`
- `started_at`
- `finalized_at`
- `render_job_id`
- `dropped_events`
- `spool_path` (daemon/debug oriented; may be omitted from user-facing output if too internal)

### Initial protocol touch points

- `Command::Spawn` request payload
- `ResponseData::SessionCreated`
- `ResponseData::Sessions`
- Any future session detail/status responses

## Files and Components Likely to Change

### CLI / protocol

- `crates/pilotty-cli/src/args.rs` — add `--record` and `--record-path`.
- `crates/pilotty-core/src/protocol.rs` — extend spawn request and structured response payloads with optional recording metadata.
- `crates/pilotty-cli/src/main.rs` — wire new flags into request creation and response printing.

### Daemon lifecycle

- `crates/pilotty-cli/src/daemon/server.rs` — spawn handling, shutdown path, unified finalization hooks.
- `crates/pilotty-cli/src/daemon/session.rs` — continuous PTY consumption refactor, session-owned recording state, teardown consolidation.
- `crates/pilotty-cli/src/daemon/pty.rs` — reader integration points, if current handle shape prevents continuous consumption.
- `crates/pilotty-cli/src/daemon/mod.rs` — module wiring.
- `crates/pilotty-cli/src/daemon/paths.rs` — Desktop default path, recording storage paths, collision-safe naming.

### New likely modules

- `crates/pilotty-cli/src/daemon/recording/mod.rs`
- `crates/pilotty-cli/src/daemon/recording/state.rs`
- `crates/pilotty-cli/src/daemon/recording/spool.rs`
- `crates/pilotty-cli/src/daemon/recording/jobs.rs`
- `crates/pilotty-cli/src/daemon/recording/render_macos.rs`

### Packaging / build

- `crates/pilotty-cli/Cargo.toml` — feature-gated or target-gated macOS render dependencies.
- `Cargo.toml` — only if shared dependency declarations are needed.
- `npm/README.md` and `README.md` — follow-up docs only after implementation lands.

## Phased Rollout / Implementation Plan

### Phase 1: Lifecycle groundwork

- Refactor PTY/session ownership to support continuous consumption.
- Introduce unified session finalization used by kill, exit cleanup, idle shutdown, and daemon stop.
- Preserve existing snapshot/input behavior during the refactor.

### Phase 2: Protocol and metadata

- Add spawn flags and protocol fields.
- Add recording metadata/status enums to shared protocol types.
- Return nested optional `recording` objects from session-related responses.

### Phase 3: Capture pipeline

- Create recording state, append-only spool writer, and persisted metadata.
- Feed recorder events from the continuous PTY reader through a lossy non-blocking sink.
- Implement output path resolution and collision-safe default naming.

### Phase 4: Render jobs and macOS backend

- Persist render jobs during finalization.
- Implement macOS-only renderer.
- Resume pending jobs on daemon startup.
- Return `unsupported` on non-macOS recording requests without regressing normal session usage.

### Phase 5: Hardening

- Verify crash recovery and duplicate finalization safety.
- Confirm dependency footprint is acceptable for source and packaged installs.
- Update user-facing docs after runtime behavior is complete.

## Verification Strategy and Exit Criteria

### Verification strategy

- Integration-test spawn, session exit, kill, and daemon shutdown with and without recording enabled.
- Verify PTY output still reaches snapshots while recording is active.
- Verify recorder sink backpressure cannot stall PTY consumption.
- Verify append-only spool creation and persisted render jobs survive daemon restart.
- Verify default Desktop naming, custom `--record-path`, and collision-safe suffixing.
- Verify structured responses expose nested `recording` objects with stable statuses.
- Verify non-macOS builds still compile and that recording requests fail gracefully there.

### Exit criteria

- `pilotty spawn --record` and `--record-path` are implemented end-to-end.
- Recording is daemon-managed and finalized correctly across all teardown paths.
- Continuous PTY consumption replaces snapshot-only draining where needed.
- Capture path is lossy/non-blocking and does not regress session responsiveness.
- Render jobs are persisted and recoverable after daemon restart.
- macOS produces terminal-content MP4 output.
- Linux remains supported with clear unsupported-recording behavior.
- Dependency additions are target-scoped or otherwise unsurprising for users.
