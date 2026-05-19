# Tessera OSC 133 shell integration prototype for Zsh.
#
# Source this manually for testing. Tessera does not install or inject it into
# user shell configuration yet.

if [[ ${__TESSERA_ZSH_INTEGRATION:-0} == 1 ]]; then
  return 0
fi
__TESSERA_ZSH_INTEGRATION=1

__tessera_osc133() {
  printf '\033]133;%s\007' "$1"
}

__tessera_precmd() {
  local status=$?

  if [[ ${__tessera_seen_prompt:-0} == 1 ]]; then
    __tessera_osc133 "D;$status"
  fi

  __tessera_seen_prompt=1
  __tessera_osc133 A
}

__tessera_preexec() {
  __tessera_osc133 B
  __tessera_osc133 C
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd __tessera_precmd
add-zsh-hook preexec __tessera_preexec
