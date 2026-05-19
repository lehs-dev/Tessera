# PTY Proxy Research Plan

## Target Architecture

```text
VTE Terminal
    ↕ stdin/stdout
tessera-pty-proxy
    ↕ PTY
user shell
```

## Responsibilities

- VTE remains the terminal renderer.
- The proxy relays input from VTE to the user shell.
- The proxy relays output from the user shell back to VTE.
- The proxy parses OSC 133 markers from the shell output stream.
- The proxy forwards the original terminal output to VTE unchanged.
- The proxy sends semantic events back to Tessera through a dedicated event fd.

The direct VTE shell backend remains the default `TerminalSession` path. The
proxy backend is experimental and opt-in:

```bash
TESSERA_TERMINAL_BACKEND=proxy cargo run --bin tessera
```

When the proxy backend is selected, `TerminalSession` spawns
`tessera-pty-proxy` inside VTE instead of spawning the user's shell directly.
The proxy then starts `$SHELL` inside its own PTY.

## Event Channel Spike

`tessera-pty-proxy` now supports a dedicated semantic event output channel:

```text
TESSERA_EVENT_FD=<fd>
```

When `TESSERA_EVENT_FD` is set, OSC 133 semantic events are written as JSONL to
that file descriptor. Forwarded terminal output still goes only to stdout and is
not mixed with the event protocol.

When `TESSERA_EVENT_FD` is not set, the proxy logs semantic events to stderr for
standalone debugging only. Stderr is not the GUI semantic event channel.

In the current GUI integration, semantic JSONL events are read by Tessera and
logged. They are not rendered as block UI yet.

Manual smoke test:

```bash
SHELL=/bin/sh cargo run --bin tessera-pty-proxy
```

## Risks

- Ctrl+C / Ctrl+D / Ctrl+Z behavior must remain correct.
- Terminal resize and SIGWINCH propagation must be reliable.
- Bracketed paste must pass through without corruption.
- Mouse reporting must pass through without corruption.
- TUI apps such as vim, nvim, less, htop, fzf, tmux, and ssh must remain usable.
- Child process lifecycle tracking must be accurate.
- The relay must avoid added latency, deadlocks, and unbounded buffering.

## Staged Plan

```text
Sprint 3:
  Build standalone tessera-pty-proxy spike.

Sprint 4:
  Add IPC semantic event channel.

Sprint 5:
  Integrate proxy behind TerminalSession feature flag.

Sprint 6:
  Build CommandBlock metadata model.

Sprint 7:
  Add block actions and minimal UI.
```
