#!/usr/bin/env bash
# LATTICE agent swarm launcher for DigitalOcean Managed Agents (Harness Runtime).
#
# Usage: ./swarm.sh <command> [agent names...]
#
#   check            Verify doctl, credentials, and the doctl flags this script uses
#   models           List model IDs available to your model access key
#   render <name>    Print the rendered spec for one agent (for review or manual launch)
#   up [names]       Create sessions for all agents in swarm.conf (or just those named)
#   status           List swarm sessions
#   nudge [names]    Send every agent (or those named) the "continue" prompt
#   watch [minutes]  Nudge all agents every N minutes (default 20) until Ctrl-C
#   attach <name>    Open an interactive session with one agent (Ctrl-D detaches)
#   logs <name>      Replay one agent's event history
#   pause [names]    Pause sessions (stops CPU/memory billing; storage still bills)
#   resume [names]   Resume paused sessions
#   down [names]     Remove sessions (asks first)
#
# Agents coordinate through GitHub issues and PRs, not through this script:
# the coordinator files and assigns issues (label swarm:<name>), workers pick
# them up, and CI plus CODEOWNERS decide what merges. See README.md.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF="$HERE/swarm.conf"
PREFIX="lattice-"
RENDER_DIR="${TMPDIR:-/tmp}/lattice-swarm"

die()  { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }

# Load credentials from .env if present (gitignored).
if [[ -f "$HERE/.env" ]]; then
  set -a; source "$HERE/.env"; set +a
fi

need_creds() {
  [[ -n "${HARNESS_INFERENCE_API_KEY:-}" ]] || die "HARNESS_INFERENCE_API_KEY is not set (see .env.example)"
  [[ -n "${GITHUB_TOKEN:-}" ]] || die "GITHUB_TOKEN is not set (see .env.example)"
}

# Print roster lines: name role adapter model size
roster() { grep -vE '^[[:space:]]*(#|$)' "$CONF"; }

agent_line() {
  local line
  line="$(roster | awk -v n="$1" '$1 == n')"
  [[ -n "$line" ]] || die "no agent named '$1' in swarm.conf"
  echo "$line"
}

selected_names() {
  if [[ $# -gt 0 ]]; then printf '%s\n' "$@"; else roster | awk '{print $1}'; fi
}

roster_markdown() {
  roster | awk '{printf "- %s: %s, %s via %s\n", $1, $2, $4, $3}'
}

# Escape a value for use as a sed replacement.
sed_escape() { printf '%s' "$1" | sed -e 's/[\/&|]/\\&/g'; }

render_spec() {
  local name="$1" role adapter model size
  read -r _ role adapter model size <<<"$(agent_line "$name")"
  local spec="$HERE/specs/$adapter.yaml"
  [[ -f "$spec" ]] || die "no spec for adapter '$adapter' (expected $spec)"
  sed -e "s|\${AGENT_NAME}|$(sed_escape "$name")|g" \
      -e "s|\${AGENT_ROLE}|$(sed_escape "$role")|g" \
      -e "s|\${AGENT_MODEL}|$(sed_escape "$model")|g" \
      -e "s|\${AGENT_SIZE}|$(sed_escape "$size")|g" \
      "$spec"
}

render_prompt() {
  local name="$1" role adapter model size
  read -r _ role adapter model size <<<"$(agent_line "$name")"
  local file="$HERE/prompts/$role.md"
  [[ -f "$file" ]] || die "no prompt for role '$role' (expected $file)"
  local roster_text
  roster_text="$(roster_markdown)"
  # Substitute simple placeholders, then the multi-line roster.
  sed -e "s|{{NAME}}|$(sed_escape "$name")|g" \
      -e "s|{{MODEL}}|$(sed_escape "$model")|g" \
      -e "s|{{ADAPTER}}|$(sed_escape "$adapter")|g" \
      "$file" | while IFS= read -r l; do
        if [[ "$l" == "{{ROSTER}}" ]]; then printf '%s\n' "$roster_text"; else printf '%s\n' "$l"; fi
      done
}

cmd_check() {
  command -v doctl >/dev/null || die "doctl not found (brew install doctl)"
  info "doctl $(doctl version | head -1)"
  doctl account get >/dev/null && info "doctl is authenticated"
  need_creds && info "credentials present"
  info "prepayment balance:"; doctl harness-runtime balance || true
  info "Flags used by this script (confirm they exist):"
  echo "  create --spec <file> --prompt <text>"
  doctl harness-runtime create --help | grep -E -- '--spec|--prompt' || echo "  !! create flags not found; check 'doctl harness-runtime create --help'"
  echo "  prompt <session> <text>"
  doctl harness-runtime prompt --help | sed -n '1,15p'
}

cmd_models() {
  need_creds
  curl -fsS https://inference.do-ai.run/v1/models \
    -H "Authorization: Bearer $HARNESS_INFERENCE_API_KEY" \
    | python3 -c 'import json,sys; [print(m["id"]) for m in json.load(sys.stdin)["data"]]'
}

cmd_render() {
  [[ $# -eq 1 ]] || die "usage: swarm.sh render <name>"
  render_spec "$1"
}

cmd_up() {
  need_creds
  mkdir -p "$RENDER_DIR"
  local name
  # Start the coordinator first so issues exist before workers look for them.
  local names; names="$(selected_names "$@")"
  local ordered
  ordered="$( (grep -x coordinator <<<"$names" || true); (grep -vx coordinator <<<"$names" || true) )"
  for name in $ordered; do
    local spec="$RENDER_DIR/$name.yaml"
    render_spec "$name" > "$spec"
    info "creating ${PREFIX}$name"
    doctl harness-runtime create --spec "$spec" --prompt "$(render_prompt "$name")"
    if [[ "$name" == "coordinator" ]]; then
      info "giving the coordinator a head start (90s) to file and assign issues"
      sleep 90
    fi
  done
  info "done. Watch progress in the 'Swarm status' issue, or run: ./swarm.sh status"
}

cmd_status() { doctl harness-runtime list | { head -1; grep -- "$PREFIX" || true; }; }

cmd_nudge() {
  local text name pids=()
  text="$(cat "$HERE/prompts/nudge.md")"
  for name in $(selected_names "$@"); do
    (
      out="$(doctl harness-runtime prompt "${PREFIX}$name" "$text" 2>&1 | tail -3)"
      echo "[$name] $out"
    ) &
    pids+=($!)
  done
  for p in "${pids[@]}"; do wait "$p" || true; done
}

cmd_watch() {
  local minutes="${1:-20}"
  info "nudging all agents every $minutes minutes (Ctrl-C to stop)"
  while true; do
    date
    cmd_nudge
    sleep $((minutes * 60))
  done
}

cmd_each() {
  local verb="$1"; shift
  local name
  for name in $(selected_names "$@"); do
    info "$verb ${PREFIX}$name"
    doctl harness-runtime "$verb" "${PREFIX}$name" || true
  done
}

cmd_down() {
  local names; names="$(selected_names "$@" | tr '\n' ' ')"
  read -r -p "Remove sessions: $names? Workspaces are deleted. [y/N] " ok
  [[ "$ok" == [yY] ]] || { echo "aborted"; return; }
  cmd_each remove "$@"
}

main() {
  local cmd="${1:-}"; shift || true
  case "$cmd" in
    check)   cmd_check ;;
    models)  cmd_models ;;
    render)  cmd_render "$@" ;;
    up)      cmd_up "$@" ;;
    status)  cmd_status ;;
    nudge)   cmd_nudge "$@" ;;
    watch)   cmd_watch "$@" ;;
    attach)  [[ $# -eq 1 ]] || die "usage: swarm.sh attach <name>"; doctl harness-runtime launch "${PREFIX}$1" ;;
    logs)    [[ $# -eq 1 ]] || die "usage: swarm.sh logs <name>"; doctl harness-runtime logs "${PREFIX}$1" ;;
    pause)   cmd_each pause "$@" ;;
    resume)  cmd_each resume "$@" ;;
    down)    cmd_down "$@" ;;
    ""|-h|--help|help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try: swarm.sh help)" ;;
  esac
}

main "$@"
