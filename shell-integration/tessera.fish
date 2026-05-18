# Tessera OSC 133 shell integration prototype for Fish.
#
# Source this manually for testing. Tessera does not install or inject it into
# user shell configuration yet. This expects Fish versions with fish_prompt,
# fish_preexec, and fish_postexec events.

set -g __TESSERA_FISH_INTEGRATION 1

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
