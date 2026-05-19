# Tessera OSC 133 shell integration prototype for Bash.
#
# Source this manually for testing. Tessera does not install or inject it into
# user shell configuration yet.

if [[ ${__TESSERA_BASH_INTEGRATION:-0} == 1 ]]; then
  return 0
fi
__TESSERA_BASH_INTEGRATION=1

__tessera_osc133() {
  printf '\033]133;%s\007' "$1"
}

__tessera_prompt_command() {
  local status=$?

  # Bash PROMPT_COMMAND runs before each prompt, including the first prompt.
  # This prototype suppresses D for the first prompt but can still report D
  # after empty input because portable Bash preexec state is limited without
  # using DEBUG traps.
  if [[ ${__tessera_seen_prompt:-0} == 1 ]]; then
    __tessera_osc133 "D;$status"
  fi

  __tessera_seen_prompt=1
  __tessera_osc133 A

  return "$status"
}

__tessera_command_start() {
  __tessera_osc133 B
  __tessera_osc133 C
}

__tessera_prompt_command_decl="$(declare -p PROMPT_COMMAND 2>/dev/null || true)"
if [[ $__tessera_prompt_command_decl == declare\ -a* ]]; then
  PROMPT_COMMAND=(__tessera_prompt_command "${PROMPT_COMMAND[@]}")
elif [[ -n ${PROMPT_COMMAND:-} ]]; then
  PROMPT_COMMAND="__tessera_prompt_command; ${PROMPT_COMMAND}"
else
  PROMPT_COMMAND="__tessera_prompt_command"
fi
unset __tessera_prompt_command_decl

# Bash 4.4 introduced PS0. It expands after a command is accepted and before
# execution, which is the least invasive place to emit B/C for this prototype.
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  PS0='$(__tessera_command_start)'"${PS0-}"
fi
