#!/usr/bin/env bash
# LATTICE agent swarm launcher for DigitalOcean Managed Agents (Harness Runtime).
#
# Usage: ./swarm.sh <command> [agent names...]
#
#   check            Preflight: doctl, credentials, repo settings, model IDs, dry-run spec
#   models           List model IDs available to your model access key
#   render <name>    Print the rendered spec for one agent
#   up [names]       Create sessions and send each agent its role prompt
#   status           List swarm sessions and whether each agent is mid-run
#   nudge [names]    Send the "continue" prompt to agents that aren't mid-run
#   watch [minutes]  Nudge every N minutes (default 20) until Ctrl-C
#   attach <name>    Open an interactive session with one agent (Ctrl-D detaches)
#   logs <name>      Replay one agent's event history
#   pause [names]    Pause sessions (stops CPU/memory billing; storage still bills)
#   resume [names]   Resume paused sessions
#   down [names]     Remove sessions (asks first)
#
# Prompts run in the background: `doctl harness-runtime prompt` blocks until the
# agent finishes its run, which can take hours. Each agent's current run is
# tracked by a PID file, so nudges never overlap with a run in progress and one
# busy agent never holds up the others. Run output goes to a log per agent.
#
# Agents coordinate through GitHub issues and PRs, not through this script.
# See README.md.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF="$HERE/swarm.conf"
REPO="JoshuaTT/Latice"
PREFIX="lattice-"
STATE_DIR="$HERE/.swarm"          # gitignored: rendered specs, PID files, run logs
COORD_WAIT_MIN="${COORD_WAIT_MIN:-30}"

die()  { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }
warn() { echo "!!  $*" >&2; }
ok()   { echo " ok $*"; }

# Load credentials from .env if present (gitignored).
if [[ -f "$HERE/.env" ]]; then
  set -a; source "$HERE/.env"; set +a
fi

need_creds() {
  [[ -n "${HARNESS_INFERENCE_API_KEY:-}" ]] || die "HARNESS_INFERENCE_API_KEY is not set (see .env.example)"
  [[ -n "${GITHUB_TOKEN:-}" ]] || die "GITHUB_TOKEN is not set (see .env.example)"
  # The claude-code adapter requires ANTHROPIC_API_KEY. Our Claude Code agents
  # send it to DigitalOcean Inference (ANTHROPIC_BASE_URL in the spec), so it
  # must be the DigitalOcean model access key, never a real Anthropic key.
  # This export only affects doctl processes started by this script.
  export ANTHROPIC_API_KEY="$HARNESS_INFERENCE_API_KEY"
}

need_doctl() { command -v doctl >/dev/null || die "doctl not found (brew install doctl)"; }

# ---------- roster ----------

# Roster lines: name role adapter model size
roster() { grep -vE '^[[:space:]]*(#|$)' "$CONF"; }

agent_line() {
  local line
  line="$(roster | awk -v n="$1" '$1 == n')"
  [[ -n "$line" ]] || die "no agent named '$1' in swarm.conf"
  echo "$line"
}

selected_names() {
  local n
  if [[ $# -gt 0 ]]; then
    for n in "$@"; do agent_line "$n" >/dev/null; echo "$n"; done
  else
    roster | awk '{print $1}'
  fi
}

worker_names() { roster | awk '$2 == "worker" {print $1}'; }

roster_markdown() { roster | awk '{printf "- %s: %s, %s via %s\n", $1, $2, $4, $3}'; }

session() { echo "${PREFIX}$1"; }

# ---------- rendering ----------

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
  local roster_text l
  roster_text="$(roster_markdown)"
  sed -e "s|{{NAME}}|$(sed_escape "$name")|g" \
      -e "s|{{MODEL}}|$(sed_escape "$model")|g" \
      -e "s|{{ADAPTER}}|$(sed_escape "$adapter")|g" \
      "$file" | while IFS= read -r l || [[ -n "$l" ]]; do
        if [[ "$l" == "{{ROSTER}}" ]]; then printf '%s\n' "$roster_text"; else printf '%s\n' "$l"; fi
      done
}

# ---------- background prompt runs ----------

pid_file() { echo "$STATE_DIR/$1.pid"; }
log_file() { echo "$STATE_DIR/$1.log"; }

is_busy() {
  local f pid stat comm
  f="$(pid_file "$1")"
  [[ -f "$f" ]] || return 1
  pid="$(cat "$f")"
  [[ -n "$pid" ]] || return 1
  # Alive, not a zombie, and still a doctl process (guards against PID reuse).
  stat="$(ps -o stat= -p "$pid" 2>/dev/null | tr -d ' ')" || return 1
  [[ -n "$stat" && "$stat" != Z* ]] || return 1
  comm="$(ps -o comm= -p "$pid" 2>/dev/null)"
  [[ "$comm" == *doctl* ]]
}

session_exists() { doctl harness-runtime show "$(session "$1")" >/dev/null 2>&1; }

# send_prompt <name> <prompt-file>: start a background run unless one is in progress.
send_prompt() {
  local name="$1" file="$2"
  if is_busy "$name"; then
    echo "[$name] still running (pid $(cat "$(pid_file "$name")")); not nudged"
    return 0
  fi
  local log; log="$(log_file "$name")"
  {
    echo
    echo "===== $(date '+%Y-%m-%d %H:%M:%S') prompt: $(basename "$file")"
  } >>"$log"
  # nohup: the run must survive this script (and its terminal) exiting.
  # --on-hitl reject: nobody is there to approve, so refuse instead of hanging.
  nohup doctl harness-runtime prompt "$(session "$name")" - \
      --on-hitl reject --quiet <"$file" >>"$log" 2>&1 &
  echo $! >"$(pid_file "$name")"
  echo "[$name] prompt sent (pid $!, log: ${log#"$HERE"/})"
}

# ---------- GitHub helpers (curl only; no gh or python needed) ----------

gh_get() {
  curl -fsS -m 20 "https://api.github.com/repos/$REPO$1" \
    -H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json"
}

# True when at least one open issue carries a swarm:<worker> label.
assignments_exist() {
  local pattern
  pattern="$(worker_names | paste -sd '|' -)"
  [[ -n "$pattern" ]] || return 1
  gh_get "/issues?state=open&per_page=100" 2>/dev/null \
    | grep -qE "\"name\": *\"swarm:($pattern)\""
}

# ---------- commands ----------

cmd_check() {
  local fail=0
  need_doctl
  ok "doctl $(doctl version | head -1)"
  doctl account get >/dev/null 2>&1 && ok "doctl is authenticated" || { warn "doctl is not authenticated (doctl auth init)"; fail=1; }
  need_creds; ok "credentials loaded"

  info "doctl flags this script relies on"
  local h
  h="$(doctl harness-runtime create --help 2>&1 || true)"
  for f in --spec --dry-run; do grep -q -- "$f" <<<"$h" && ok "create $f" || { warn "create $f missing"; fail=1; }; done
  h="$(doctl harness-runtime prompt --help 2>&1 || true)"
  for f in --on-hitl --quiet; do grep -q -- "$f" <<<"$h" && ok "prompt $f" || { warn "prompt $f missing"; fail=1; }; done

  info "prepayment balance"
  doctl harness-runtime balance || warn "could not read balance"

  info "GitHub repo and token"
  local repo
  if repo="$(gh_get "")"; then
    ok "token can read $REPO"
    grep -qE '"allow_auto_merge": *true' <<<"$repo" && ok "auto-merge enabled" \
      || { warn "auto-merge is OFF: Settings > General > Pull Requests > Allow auto-merge"; fail=1; }
    grep -qE '"delete_branch_on_merge": *true' <<<"$repo" && ok "delete head branches enabled" \
      || warn "automatic head-branch deletion is off (recommended: on)"
  else
    warn "token cannot read $REPO"; fail=1
  fi
  gh_get "/contents/AGENTS.md" >/dev/null 2>&1 && ok "AGENTS.md is on main" || { warn "AGENTS.md not found on main"; fail=1; }
  if gh_get "/branches/main/protection" >/dev/null 2>&1; then
    warn "token has Administration access; remove it so agents can't bypass rulesets"; fail=1
  else
    ok "token has no Administration access"
  fi

  info "model IDs in swarm.conf"
  local models m
  if models="$(cmd_models 2>/dev/null)" && [[ -n "$models" ]]; then
    for m in $(roster | awk '{print $4}' | sort -u); do
      grep -qx -- "$m" <<<"$models" && ok "$m" || { warn "$m is not available (run ./swarm.sh models)"; fail=1; }
    done
  else
    warn "could not list models (check HARNESS_INFERENCE_API_KEY)"; fail=1
  fi

  info "dry run: render every spec and let doctl resolve it (nothing is created)"
  mkdir -p "$STATE_DIR"
  local name out
  for name in $(selected_names); do
    render_spec "$name" >"$STATE_DIR/$name.yaml"
    if out="$(doctl harness-runtime create --spec "$STATE_DIR/$name.yaml" --dry-run 2>&1)"; then
      ok "$(session "$name") spec accepted"
    else
      warn "$(session "$name") spec rejected:"; echo "$out" | sed 's/^/      /' >&2; fail=1
    fi
  done

  echo
  if [[ $fail -eq 0 ]]; then info "all checks passed"; else warn "fix the items above before ./swarm.sh up"; return 1; fi
}

cmd_models() {
  need_creds
  curl -fsS -m 20 https://inference.do-ai.run/v1/models \
    -H "Authorization: Bearer $HARNESS_INFERENCE_API_KEY" \
    | grep -oE '"id"[[:space:]]*:[[:space:]]*"[^"]+"' \
    | sed -E 's/.*"([^"]+)"$/\1/' | sort -u
}

cmd_render() {
  [[ $# -eq 1 ]] || die "usage: swarm.sh render <name>"
  render_spec "$1"
}

create_and_start() {
  local name="$1"
  render_spec "$name" >"$STATE_DIR/$name.yaml"
  render_prompt "$name" >"$STATE_DIR/$name.prompt.md"
  if session_exists "$name"; then
    info "$(session "$name") already exists; not recreating (use 'nudge' to wake it)"
    return 0
  fi
  info "creating $(session "$name") (waits for the sandbox to be ready)"
  doctl harness-runtime create --spec "$STATE_DIR/$name.yaml"
  send_prompt "$name" "$STATE_DIR/$name.prompt.md"
}

cmd_up() {
  need_doctl; need_creds
  mkdir -p "$STATE_DIR"
  local names name has_coord=false workers=()
  names="$(selected_names "$@")"
  for name in $names; do
    if [[ "$name" == "coordinator" ]]; then has_coord=true; else workers+=("$name"); fi
  done

  if $has_coord; then
    create_and_start coordinator
    if [[ ${#workers[@]} -gt 0 ]] && ! assignments_exist; then
      info "waiting up to $COORD_WAIT_MIN min for the coordinator to file and assign issues (Ctrl-C to stop; rerun 'up' for workers later)"
      local waited=0
      until assignments_exist; do
        if [[ $waited -ge $((COORD_WAIT_MIN * 60)) ]]; then
          warn "no assignments yet; starting workers anyway (they'll go idle until nudged)"
          break
        fi
        sleep 30; waited=$((waited + 30)); printf '.'
      done
      echo
    fi
  fi

  if [[ ${#workers[@]} -gt 0 ]]; then
    for name in "${workers[@]}"; do create_and_start "$name"; done
  fi
  info "done. Next: ./swarm.sh watch 20, and keep an eye on the 'Swarm status' issue"
}

cmd_status() {
  need_doctl
  local names name state
  names="$(selected_names)"
  info "all sessions on your DigitalOcean team"
  doctl harness-runtime list --format SessionID,Name,Status,PauseReason,CreatedAt
  echo
  info "swarm agents"
  for name in $names; do
    if is_busy "$name"; then
      state="mid-run (pid $(cat "$(pid_file "$name")"))"
    elif session_exists "$name"; then
      state="idle"
    else
      state="no session"
    fi
    printf '  %-12s %-21s %s\n' "$name" "$(session "$name")" "$state"
  done
}

cmd_nudge() {
  need_doctl
  mkdir -p "$STATE_DIR"
  local names name
  names="$(selected_names "$@")"
  for name in $names; do
    if ! is_busy "$name" && ! session_exists "$name"; then
      echo "[$name] no session (start it with: ./swarm.sh up $name)"
      continue
    fi
    send_prompt "$name" "$HERE/prompts/nudge.md"
  done
}

cmd_watch() {
  local minutes="${1:-20}"
  [[ "$minutes" =~ ^[0-9]+$ && "$minutes" -gt 0 ]] || die "usage: swarm.sh watch [minutes]"
  info "nudging idle agents every $minutes minutes (Ctrl-C stops nudging; running agents keep going)"
  while true; do
    echo "--- $(date '+%H:%M:%S')"
    cmd_nudge
    sleep $((minutes * 60))
  done
}

cmd_each() {
  need_doctl
  local verb="$1"; shift
  local names name
  names="$(selected_names "$@")"
  for name in $names; do
    info "$verb $(session "$name")"
    doctl harness-runtime "$verb" "$(session "$name")" || warn "$verb failed for $(session "$name")"
  done
}

cmd_down() {
  need_doctl
  local all name existing=()
  all="$(selected_names "$@")"
  for name in $all; do
    if session_exists "$name"; then existing+=("$name"); else echo "[$name] no session; nothing to remove"; fi
  done
  [[ ${#existing[@]} -gt 0 ]] || return 0
  read -r -p "Remove sessions: ${existing[*]}? Workspaces are deleted. [y/N] " reply
  [[ "$reply" == [yY] ]] || { echo "aborted"; return; }
  local pid
  for name in "${existing[@]}"; do
    if is_busy "$name"; then
      pid="$(cat "$(pid_file "$name")")"; kill "$pid" 2>/dev/null || true
    fi
    rm -f "$(pid_file "$name")"
  done
  cmd_each remove "${existing[@]}"
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
    attach)  [[ $# -eq 1 ]] || die "usage: swarm.sh attach <name>"; need_doctl; doctl harness-runtime launch "$(session "$1")" ;;
    logs)    [[ $# -eq 1 ]] || die "usage: swarm.sh logs <name>"; need_doctl; doctl harness-runtime logs "$(session "$1")" ;;
    pause)   cmd_each pause "$@" ;;
    resume)  cmd_each resume "$@" ;;
    down)    cmd_down "$@" ;;
    ""|-h|--help|help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try: swarm.sh help)" ;;
  esac
}

main "$@"
