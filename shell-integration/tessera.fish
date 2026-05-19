# Tessera OSC 133 shell integration prototype for Fish.
#
# Tessera sources this only when TESSERA_ENABLE_SHELL_INTEGRATION=1 is set for
# the proxy backend. It is intentionally additive: it does not initialize
# Starship, override fish_prompt, or modify persistent shell configuration.

if set -q __TESSERA_FISH_INTEGRATION
    return 0
end
set -g __TESSERA_FISH_INTEGRATION 1

function __tessera_fish_has_native_osc133
    status test-feature mark-prompt >/dev/null 2>/dev/null
end

if __tessera_fish_has_native_osc133
    return 0
end

function __tessera_osc133
    printf '\033]133;%s\007' $argv[1]
end

function __tessera_prompt_start --on-event fish_prompt
    __tessera_osc133 A
end

function __tessera_preexec --on-event fish_preexec
    __tessera_osc133 B
    __tessera_osc133 C
end

function __tessera_postexec --on-event fish_postexec
    set -l command_status $status
    __tessera_osc133 "D;$command_status"
end
