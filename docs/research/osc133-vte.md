# OSC 133 and VTE Research

## Goal

Evaluate whether Tessera can use shell-emitted OSC 133 markers with VTE to build
future command block metadata while keeping VTE as the terminal renderer.

## Findings

- OSC 133 can be emitted by shell integration scripts for Bash, Zsh, and Fish.
- The useful markers for Tessera are prompt start, prompt end, command start,
  and command finished with an optional exit status.
- VTE can internally parse or react to semantic prompt markers.
- The current public VTE API does not provide Tessera with a clean semantic
  `command_started` / `command_finished` event stream from those markers.
- Tessera should not depend on private VTE internals or terminal rendering
  behavior to recover command lifecycle events.

## Current Decision

Tessera needs its own semantic event strategy.

For now, Tessera keeps VTE as the renderer and keeps the existing VTE
`spawn_async()` path that starts the user's shell directly. The OSC 133 parser
added in this milestone is independent infrastructure only; it is not wired into
terminal IO yet.

## Open Questions

- Which shell integration behavior is reliable enough across Bash, Zsh, and Fish?
- How should Tessera handle shells that do not load integration scripts?
- What IPC shape should carry command lifecycle events back to the GTK process?
- How should semantic events be correlated with visible terminal output offsets?
- What fallback behavior should exist for nested shells, SSH, tmux, and TUI apps?

## Next Step

The next viable direction is a PTY proxy/interceptor spike. The spike should
relay terminal IO unchanged, parse OSC 133 from shell output, and report semantic
events over a future IPC channel without replacing VTE as the renderer.
