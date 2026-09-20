#!/usr/bin/env bash
# Drives the README demo: types each line, then runs it for real.
set -u
PROMPT=$'\033[38;5;79m❯\033[0m '
CMT=$'\033[38;5;245m'
OFF=$'\033[0m'

type_line() {
  printf '%s' "$PROMPT"
  local s="$1" i
  for ((i = 0; i < ${#s}; i++)); do
    printf '%s' "${s:i:1}"
    sleep 0.028
  done
  printf '\n'
}

note() {   # a typed comment, for narration
  printf '%s' "$PROMPT"
  local s="# $1" i
  printf '%s' "$CMT"
  for ((i = 0; i < ${#s}; i++)); do printf '%s' "${s:i:1}"; sleep 0.022; done
  printf '%s\n' "$OFF"
  sleep 0.5
}

run() {
  type_line "$1"
  sleep 0.35
  eval "$1"
  sleep 1.6
}

clear
sleep 0.8
note "Ask any text a yes/no question. Get a probability, not a paragraph."
run 'jev noul "Is this customer angry?" --state "You charged me twice. Fix it now."'

note "The answer can be the exit code, so shell scripts can branch on it."
run 'jev noul "Is this about billing?" --state-file ticket.txt --fail-under 0.7 --quiet'
run 'echo "exit $? -> route to billing"'

note "Ask many questions about one ticket in a single call."
run 'jev eval -f triage.yaml --state-file ticket.txt'

note "Piped or redirected, the same command speaks JSON. Agents love that."
run "jev eval -f triage.yaml --state-file ticket.txt -o json | jq -c '.answers.department'"
note "One number your code can trust, for a fraction of a cent."
sleep 1.2
