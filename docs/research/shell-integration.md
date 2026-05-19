# Shell Integration

## Current Scope

- Shell integration is opt-in with `TESSERA_ENABLE_SHELL_INTEGRATION=1`.
- The proxy backend is required with `TESSERA_TERMINAL_BACKEND=proxy`.
- The direct backend is unaffected and remains the default terminal path.
- Fish is the primary supported shell for this milestone.
- Bash and Zsh integration are secondary/prototype only.
- Block UI is still not implemented.

## Fish Integration

When shell integration is enabled and `$SHELL` resolves to `fish`, the proxy
starts Fish with an init command that sources:

```text
shell-integration/tessera.fish
```

The script path is resolved from `TESSERA_SHELL_INTEGRATION_DIR` first, then
from the development layout around `target/debug/tessera-pty-proxy`.

Tessera does not modify persistent shell config. In particular, it does not
write to `~/.config/fish/config.fish`, `~/.bashrc`, `~/.zshrc`, or other shell
startup files.

## Prompt Ownership

The Fish integration is additive:

- Starship prompt is expected to keep working.
- Tessera does not override `fish_prompt`.
- Tessera does not call `starship init`.
- Tessera only defines helper functions and Fish event hooks for OSC 133
  markers.

Fish versions with native OSC 133 prompt marking, such as Fish 4.x with the
`mark-prompt` feature enabled, already emit command lifecycle markers. In that
case Tessera's Fish script exits after its load guard and lets Fish own the
markers, avoiding duplicate command-finished events while preserving the user's
prompt.

Fish prompt-start markers use the `fish_prompt` event hook. If that event is
missing or unreliable in a given Fish version, command start and command finish
events can still be useful for this milestone.
