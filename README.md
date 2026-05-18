# Tessera

A Linux-native terminal workspace with structured command blocks.

Tessera is an experimental GTK/libadwaita terminal workspace for Linux desktops.
It uses VTE as the terminal backend and focuses on developer-oriented command
workflows: structured command blocks, readable output, command history, and
IDE-like command editing.

## Goals

- Native GTK/libadwaita Linux desktop integration
- VTE-based terminal correctness
- Structured command blocks
- Copy, search, collapse, and rerun command output
- IDE-like command input for shell workflows
- No AI, no cloud dependency, no custom UI toolkit

## Non-goals

- Reimplementing a terminal emulator from scratch
- Replacing tmux
- Becoming a full IDE
- Shipping AI features
- Building a custom GPU rendering stack

## Roadmap

### Phase 0: Terminal foundation

- [ ] GTK/libadwaita application shell
- [ ] Embedded VTE terminal
- [ ] Spawn default user shell
- [ ] Basic shortcuts

### Phase 1: Session model

- [ ] Multiple tabs
- [ ] Session lifecycle
- [ ] Basic preferences
- [ ] Copy/paste behavior

### Phase 2: Command blocks

- [ ] OSC 133 shell integration
- [ ] Command boundary detection
- [ ] Exit code and duration tracking
- [ ] Copy command/output
- [ ] Jump between blocks

### Phase 3: IDE-like workflow

- [ ] Dedicated command editor
- [ ] Multi-line command editing
- [ ] Command snippets
- [ ] History search
- [ ] Export block as Markdown