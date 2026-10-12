#!/usr/bin/env bash
# LATTICE agent swarm launcher.
#
# Roles (swarm.conf): coordinator (product owner + quality control), worker
# (self-assigns from the backlog), tester (runs real scenarios, owns the
# harness). Each role's prompt is prompts/<role>.md; nudges use
# prompts/nudge-<role>.md when it exists.
#
# Agents run either on DigitalOcean Managed Agents (Harness Runtime) or, with
# adapter "local" in swarm.conf, as Claude Code on this machine using your own
# Anthropic account (sandboxed; see README.md).
#
# Usage: ./swarm.sh <command> [agent names...]
#
#   check            Preflight: tools, credentials, repo settings, models, specs
#   models           List model IDs available to your DigitalOcean model access key
#   render <name>    Print an agent's rendered spec (or local Claude Code settings)
#   up [names]       Start agents and send each its role prompt
#   status           List sessions and whether each agent is mid-run
#   nudge [names]    Send the "continue" prompt to agents that aren't mid-run
#                    (prompts/nudge-<role>.md if it exists, else prompts/nudge.md)
#   tell <name> <message...>   Send one agent a one-off message (or @file)
#   restore [names]  Move DigitalOcean agents back from a fallback model to their configured one
#   watch [minutes]  Nudge every N minutes (default 20) until Ctrl-C
#   attach <name>    Talk to one agent interactively (Ctrl-D detaches)
#   logs <name>      Show an agent's history
#   pause [names]    Pause (DigitalOcean) or stop the current run (local)
#   resume [names]   Resume paused DigitalOcean sessions
#   down [names]     Remove sessions / local workspaces (asks first)
#
# Prompts run in the background, because a run can take hours. Each agent's
# current run is tracked by a PID file, so nudges never overlap with a run in
# progress and one busy agent never holds up the others. Run output goes to
# .swarm/<name>.log.
#
# Model failover (DigitalOcean): when a run fails because its model hit the
# account's daily token limit (429 tokens_per_day), the agent is moved to the
# next available model from the @fallbacks line in swarm.conf: unfinished work
# is saved to a wip/ branch, the session is recreated on the new model, and
# the new session gets a handoff note. Set AUTO_FAILOVER=0 to turn this off.
#
# Agents coordinate through GitHub issues and PRs, not through this script.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF="$HERE/swarm.conf"
REPO="JoshuaTT/Latice"
PREFIX="lattice-"
STATE_DIR="$HERE/.swarm"          # gitignored: rendered specs, PID files, run logs
COORD_WAIT_MIN="${COORD_WAIT_MIN:-30}"
DO_INFERENCE_URL="https://inference.do-ai.run"
LOCAL_HOME="${LATTICE_LOCAL_HOME:-$HOME/lattice-swarm}"   # local agents' workspaces
AUTO_FAILOVER="${AUTO_FAILOVER:-1}"                  # switch models on daily rate limits
RATE_LIMIT_COOLDOWN_HOURS="${RATE_LIMIT_COOLDOWN_HOURS:-6}"  # skip a limited model this long
MAX_FAILOVERS="${MAX_FAILOVERS:-3}"                  # per run

die()  { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }
warn() { echo "!!  $*" >&2; }
ok()   { echo " ok $*"; }

# Load credentials from .env if present (gitignored).
if [[ -f "$HERE/.env" ]]; then
  set -a; source "$HERE/.env"; set +a
fi

# ---------- roster ----------

# Roster lines: name role adapter model size
roster() { grep -vE '^[[:space:]]*(#|$|@)' "$CONF"; }

# Fallback models for DigitalOcean agents, from the "@fallbacks" line(s).
fallback_models() { awk '$1 == "@fallbacks" { for (i = 2; i <= NF; i++) print $i }' "$CONF"; }

agent_line() {
  local line
  line="$(roster | awk -v n="$1" '$1 == n')"
  [[ -n "$line" ]] || die "no agent named '$1' in swarm.conf"
  echo "$line"
}

agent_field() {  # agent_field <name> <column 1-5>
  agent_line "$1" | awk -v c="$2" '{print $c}'
}

selected_names() {
  local n
  if [[ $# -gt 0 ]]; then
    for n in "$@"; do agent_line "$n" >/dev/null; echo "$n"; done
  else
    roster | awk '{print $1}'
  fi
}

is_local() { [[ "$(agent_field "$1" 3)" == "local" ]]; }

any_remote() {  # any_remote [names...]: true if any named (or any) agent runs on DigitalOcean
  local n
  for n in $(selected_names "$@"); do is_local "$n" || return 0; done
  return 1
}

any_local() {
  local n
  for n in $(selected_names "$@"); do is_local "$n" && return 0; done
  return 1
}


adapter_label() { if [[ "$1" == "local" ]]; then echo "claude-code (local)"; else echo "$1"; fi; }

roster_markdown() {
  roster | awk '{a = ($3 == "local") ? "claude-code (local)" : $3; printf "- %s: %s, %s via %s\n", $1, $2, $4, a}'
}

session() { echo "${PREFIX}$1"; }

# ---------- credentials and tools ----------

need_creds() {
  [[ -n "${GITHUB_TOKEN:-}" ]] || die "GITHUB_TOKEN is not set (see .env.example)"
  if any_remote; then
    [[ -n "${HARNESS_INFERENCE_API_KEY:-}" ]] || die "HARNESS_INFERENCE_API_KEY is not set (see .env.example)"
    # The claude-code adapter requires ANTHROPIC_API_KEY. Our DigitalOcean
    # Claude Code agents send it to DigitalOcean Inference, so it must be the
    # DigitalOcean model access key. doctl validates it with
    # GET $ANTHROPIC_BASE_URL/v1/models, reading ANTHROPIC_BASE_URL from this
    # process's environment, so point that at DigitalOcean too.
    #
    # These exports reach doctl only. Local agents get a scrubbed environment
    # (see build_local_cmd) and use your own Anthropic account.
    export ANTHROPIC_API_KEY="$HARNESS_INFERENCE_API_KEY"
    export ANTHROPIC_BASE_URL="$DO_INFERENCE_URL"
  fi
}

need_doctl() { command -v doctl >/dev/null || die "doctl not found (brew install doctl)"; }
need_claude() { command -v claude >/dev/null || die "claude not found (https://claude.com/claude-code)"; }

# need_tools_for [names...]: doctl if any are remote, claude if any are local.
need_tools_for() {
  if any_remote "$@"; then need_doctl; fi
  if any_local "$@"; then need_claude; fi
}

# Same request doctl makes to validate ANTHROPIC_API_KEY for claude-code.
claude_key_check() {
  curl -sS -m 20 -o /dev/null -w '%{http_code}' "$DO_INFERENCE_URL/v1/models" \
    -H "x-api-key: $HARNESS_INFERENCE_API_KEY" \
    -H "anthropic-version: 2023-06-01" -H "Accept: application/json"
}

# model_probe <model>: one-token chat completion on DigitalOcean. Prints the
# body, a newline, then the HTTP status code.
model_probe() {
  curl -sS -m 60 -w $'\n%{http_code}' "$DO_INFERENCE_URL/v1/chat/completions" \
    -H "Authorization: Bearer $HARNESS_INFERENCE_API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"$1\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"max_tokens\":1}" \
    2>/dev/null || printf '\n000'
}

uses_claude_code() { roster | awk '$3 == "claude-code"' | grep -q .; }

# ---------- rendering (DigitalOcean) ----------

sed_escape() { printf '%s' "$1" | sed -e 's/[\/&|]/\\&/g'; }

# The model an agent actually runs: its configured model, unless failover
# moved it to a fallback (recorded in .swarm/<name>.model).
model_override_file() { echo "$STATE_DIR/$1.model"; }
agent_model() {
  local f; f="$(model_override_file "$1")"
  if [[ -s "$f" ]] && ! is_local "$1"; then cat "$f"; else agent_field "$1" 4; fi
}

render_spec() {
  local name="$1" role adapter model size
  read -r _ role adapter model size <<<"$(agent_line "$name")"
  model="$(agent_model "$name")"
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
      -e "s|{{ADAPTER}}|$(sed_escape "$(adapter_label "$adapter")")|g" \
      "$file" | while IFS= read -r l || [[ -n "$l" ]]; do
        if [[ "$l" == "{{ROSTER}}" ]]; then printf '%s\n' "$roster_text"; else printf '%s\n' "$l"; fi
      done
  if [[ "$adapter" == "local" ]]; then
    cat <<'EOF'

## Local workspace notes (you run on Josh's laptop, in a sandbox)

- Your current directory is already a clone of `JoshuaTT/Latice`, made for you outside the sandbox, with your git identity and push credentials configured. **Skip the clone and git-identity setup steps.** Start with `git fetch origin && git checkout main && git pull`.
- The sandbox never lets you write `.git/config` or `.git/hooks`, so avoid commands that change git config: no `git config`, `git remote add`, `git push -u`, or `git branch --set-upstream-to`. Push with `git push origin HEAD` (pushes your current branch to the same name).
- `gh` is already authenticated as the swarm's GitHub identity. Run `gh` commands from the repo root as plain commands (`gh pr create ...`, `gh issue list ...`), not after `cd` or inside `$(...)`, so they run where they work.
- Commands that reference environment variables (`$GITHUB_TOKEN` etc.) are refused automatically; you never need them, because git and gh already have credentials.
EOF
  fi
}

# ---------- local agents (Claude Code on this machine) ----------

ws_dir() { echo "$LOCAL_HOME/$1"; }
# The agent's clone of the repo; Claude Code runs with this as its working
# directory, so gh/git commands don't need a `cd` (a `cd` keeps a call inside
# the sandbox even when the command is excluded from it).
repo_dir() { echo "$(ws_dir "$1")/Latice"; }
gh_config_dir() { echo "$(ws_dir "$1")/.gh-config"; }
local_settings_file() { echo "$STATE_DIR/local-claude-settings-$1.json"; }
local_exists() { [[ -f "$(ws_dir "$1")/.lattice-agent" ]]; }
session_id_file() { echo "$(ws_dir "$1")/.lattice-session-id"; }

new_uuid() {
  if command -v uuidgen >/dev/null; then uuidgen | tr '[:upper:]' '[:lower:]'
  else python3 -c 'import uuid; print(uuid.uuid4())'; fi
}

# Claude Code session ID for a local agent. Each agent's conversation is
# pinned to an ID (--session-id on the first run, --resume afterwards), so
# nudges and `attach` always reach the same conversation. (Interactive
# `claude --continue` skips conversations started with `claude -p`, so it
# can't be used for attach.)
#
# Workspaces started before IDs were tracked: find the newest Claude Code
# transcript recorded for this workspace and adopt its ID.
local_session_id() {
  local name="$1" f ws t id
  f="$(session_id_file "$name")"
  if [[ -s "$f" ]]; then cat "$f"; return 0; fi
  ws="$(ws_dir "$name")"
  local matches=() m
  local dir
  for dir in "$(repo_dir "$name")" "$ws"; do
    while IFS= read -r m; do [[ -n "$m" ]] && matches+=("$m"); done < <(
      grep -lF "\"cwd\":\"$dir\"" "${CLAUDE_CONFIG_DIR:-$HOME/.claude}"/projects/*/*.jsonl 2>/dev/null || true)
    [[ ${#matches[@]} -gt 0 ]] && break
  done
  [[ ${#matches[@]} -gt 0 ]] || return 1
  t="$(ls -t "${matches[@]}" | head -1)"
  [[ "$t" == *.jsonl ]] || return 1
  id="$(basename "$t" .jsonl)"
  echo "$id" >"$f"
  echo "$id"
}

# Claude Code settings for local agents. The agent's shell commands run in
# Claude Code's OS-level sandbox: writes only to its workspace (plus tool
# caches), network only to the hosts below, and no reading your SSH keys,
# cloud credentials, gh/doctl config, or this folder's .env. Permission prompts
# are refused (nobody is there to answer them), and the same never-do-this
# rules as the DigitalOcean specs apply.
write_local_settings() {  # write_local_settings <name>
  local name="$1"
  mkdir -p "$STATE_DIR"
  cat >"$(local_settings_file "$name")" <<EOF
{
  "permissions": {
    "allow": ["Bash(gh *)"],
    "deny": [
      "Bash(git push --force *)",
      "Bash(git push -f *)",
      "Bash(git push origin main)",
      "Bash(git push origin main *)",
      "Bash(gh pr merge --admin*)",
      "Bash(gh pr merge *--admin*)",
      "Bash(gh api *merge*)",
      "Bash(curl *merge*)",
      "Bash(gh api *rulesets*)",
      "Bash(gh api *branches/*/protection*)",
      "Bash(gh repo delete *)",
      "Bash(rm -rf /*)",
      "Bash(rm -rf ~*)"
    ]
  },
  "sandbox": {
    "enabled": true,
    "failIfUnavailable": true,
    "allowUnsandboxedCommands": false,
    "autoAllowBashIfSandboxed": true,
    "excludedCommands": ["gh *"],
    "filesystem": {
      "allowWrite": ["~/.cargo", "~/.rustup", "~/.cache", "~/.npm", "~/.local", "~/Library/Caches", "$(gh_config_dir "$name")"],
      "denyRead": [
        "~/.ssh", "~/.aws", "~/.gnupg", "~/.netrc", "~/.docker",
        "~/.config/gh", "~/.config/doctl", "~/Library/Application Support/doctl",
        "$HERE/.env"
      ]
    },
    "network": {
      "strictAllowlist": true,
      "allowedDomains": [
        "github.com", "api.github.com", "codeload.github.com",
        "objects.githubusercontent.com", "release-assets.githubusercontent.com",
        "static.rust-lang.org", "sh.rustup.rs",
        "crates.io", "index.crates.io", "static.crates.io",
        "pypi.org", "files.pythonhosted.org", "astral.sh",
        "registry.npmjs.org"
      ]
    }
  }
}
EOF
}

# build_local_cmd <name>: set local_cmd to the command that runs Claude Code
# for this agent, with a scrubbed environment: the
# DigitalOcean key and base URL are removed so Claude Code uses your own
# Anthropic login (or LOCAL_ANTHROPIC_API_KEY from .env, if set), and gh/git
# act as the agent's GitHub token rather than your own gh login.
local_cmd=()
build_local_cmd() {
  local name="$1" role model
  role="$(agent_field "$name" 2)"; model="$(agent_field "$name" 4)"
  local_cmd=(env -u ANTHROPIC_API_KEY -u ANTHROPIC_BASE_URL -u ANTHROPIC_AUTH_TOKEN
             -u ANTHROPIC_MODEL -u HARNESS_INFERENCE_API_KEY
             "GITHUB_TOKEN=$GITHUB_TOKEN" "GH_TOKEN=$GITHUB_TOKEN"
             "LATTICE_AGENT_NAME=$name" "LATTICE_AGENT_ROLE=$role"
             "LATTICE_AGENT_MODEL=$model" "LATTICE_AGENT_ADAPTER=local"
             "GH_CONFIG_DIR=$(gh_config_dir "$name")" "GH_PROMPT_DISABLED=1"
             "GIT_TERMINAL_PROMPT=0"
             "GIT_AUTHOR_NAME=lattice-$name" "GIT_AUTHOR_EMAIL=lattice-$name@users.noreply.github.com"
             "GIT_COMMITTER_NAME=lattice-$name" "GIT_COMMITTER_EMAIL=lattice-$name@users.noreply.github.com")
  if [[ -n "${LOCAL_ANTHROPIC_API_KEY:-}" ]]; then
    local_cmd+=("ANTHROPIC_API_KEY=$LOCAL_ANTHROPIC_API_KEY")
  fi
  local_cmd+=(claude --model "$model" --settings "$(local_settings_file "$name")"
              --strict-mcp-config --permission-mode acceptEdits)
}

init_local() {
  local name="$1" ws
  ws="$(ws_dir "$name")"
  mkdir -p "$ws"
  printf 'LATTICE local agent workspace for %s\n' "$name" >"$ws/.lattice-agent"
  new_uuid >"$(session_id_file "$name")"
  write_local_settings "$name"
  ensure_local_clone "$name"
}

# git credential helper for local agents: hands git the agent's token from
# the environment at run time, so the token is never written to disk.
# shellcheck disable=SC2016  # expanded by git's shell, not here
LOCAL_GIT_HELPER='!f() { test "$1" = get || exit 0; echo username=x-access-token; echo "password=$GITHUB_TOKEN"; }; f'

# ensure_local_clone <name>: clone the repo into the agent's workspace and set
# its git config. Runs here, outside the sandbox, because the sandbox never
# lets the agent write .git/config or .git/hooks (so it can't clone itself).
ensure_local_clone() {
  local name="$1" repo
  repo="$(repo_dir "$name")"
  mkdir -p "$(gh_config_dir "$name")"
  if [[ ! -d "$repo/.git" ]]; then
    info "cloning $REPO into $repo (outside the sandbox)"
    GITHUB_TOKEN="$GITHUB_TOKEN" GIT_TERMINAL_PROMPT=0 \
      git -c credential.helper= -c credential.helper="$LOCAL_GIT_HELPER" \
        clone --quiet "${LATTICE_CLONE_URL:-https://github.com/$REPO.git}" "$repo" \
      || die "couldn't clone $REPO into $repo"
  fi
  # Use only our helper: the empty entry resets helpers inherited from your
  # global git config (e.g. the macOS keychain), so agents never use your login.
  git -C "$repo" config --unset-all credential.helper 2>/dev/null || true
  git -C "$repo" config --add credential.helper ""
  git -C "$repo" config --add credential.helper "$LOCAL_GIT_HELPER"
  git -C "$repo" config user.name "lattice-$name"
  git -C "$repo" config user.email "lattice-$name@users.noreply.github.com"
  git -C "$repo" config push.default current
  git -C "$repo" config remote.origin.url "https://github.com/$REPO.git"
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
  # Alive, not a zombie, and still one of our runners (guards against PID reuse).
  stat="$(ps -o stat= -p "$pid" 2>/dev/null | tr -d ' ')" || return 1
  [[ -n "$stat" && "$stat" != Z* ]] || return 1
  comm="$(ps -o args= -p "$pid" 2>/dev/null)"
  case "$comm" in *doctl*|*claude*|*node*|*caffeinate*|*"swarm.sh _run-remote"*) return 0;; *) return 1;; esac
}

session_exists() { doctl harness-runtime show "$(session "$1")" >/dev/null 2>&1; }

agent_exists() { if is_local "$1"; then local_exists "$1"; else session_exists "$1"; fi; }

# send_prompt <name> <prompt-file> [first]: start a background run unless one
# is in progress. "first" starts a new local conversation instead of continuing.
send_prompt() {
  local name="$1" file="$2" first="${3:-}"
  if is_busy "$name"; then
    echo "[$name] still running (pid $(cat "$(pid_file "$name")")); not nudged"
    return 0
  fi
  local log; log="$(log_file "$name")"
  {
    echo
    echo "===== $(date '+%Y-%m-%d %H:%M:%S') prompt: $(basename "$file")"
  } >>"$log"

  if is_local "$name"; then
    write_local_settings "$name"
    ensure_local_clone "$name" >/dev/null
    build_local_cmd "$name"
    local_cmd+=(-p --permission-prompts none --output-format text)
    local sid
    if [[ -n "$first" ]]; then
      local_cmd+=(--session-id "$(cat "$(session_id_file "$name")")")
    elif sid="$(local_session_id "$name")"; then
      local_cmd+=(--resume "$sid")
    else
      local_cmd+=(--continue)   # last resort: most recent conversation in the workspace
    fi
    if [[ -n "${LOCAL_MAX_BUDGET_USD:-}" ]]; then local_cmd+=(--max-budget-usd "$LOCAL_MAX_BUDGET_USD"); fi
    # caffeinate (macOS) keeps the laptop from idle-sleeping mid-run.
    local wrap=(); command -v caffeinate >/dev/null && wrap=(caffeinate -i)
    (
      cd "$(repo_dir "$name")" || exit 1
      nohup ${wrap[@]+"${wrap[@]}"} "${local_cmd[@]}" <"$file" >>"$log" 2>&1 &
      echo $! >"$(pid_file "$name")"
    )
  else
    # Runs `doctl harness-runtime prompt` and handles model failover (see
    # cmd_run_remote). Re-invoking the script keeps it alive after we exit.
    nohup bash "$HERE/swarm.sh" _run-remote "$name" "$file" >>"$log" 2>&1 &
    echo $! >"$(pid_file "$name")"
  fi
  echo "[$name] prompt sent (pid $(cat "$(pid_file "$name")"), log: ${log#"$HERE"/})"
}

# ---------- model failover (DigitalOcean daily token limits) ----------

rate_limit_file() { echo "$STATE_DIR/ratelimits"; }

mark_rate_limited() {  # mark_rate_limited <model>
  mkdir -p "$STATE_DIR"
  printf '%s %s\n' "$1" "$(date +%s)" >>"$(rate_limit_file)"
}

# True while a model is inside its cooldown after hitting a daily limit.
model_cooling() {
  local f now last
  f="$(rate_limit_file)"; [[ -f "$f" ]] || return 1
  last="$(awk -v m="$1" '$1 == m { t = $2 } END { print t + 0 }' "$f")"
  now="$(date +%s)"
  [[ "$last" -gt 0 && $((now - last)) -lt $((RATE_LIMIT_COOLDOWN_HOURS * 3600)) ]]
}

# A failed run whose closing line reports a *daily* limit. Per-minute limits
# are transient, so they're left for the next nudge to retry.
is_daily_limit_failure() {  # is_daily_limit_failure <output-file>
  grep -qiE '^run failed:.*(tokens_per_day|requests_per_day)' "$1"
}

# Models other DigitalOcean agents are running right now.
models_in_use_except() {
  local n
  for n in $(roster | awk -v x="$1" '$3 != "local" && $1 != x {print $1}'); do agent_model "$n"; done | sort -u
}

# pick_fallback <name>: print the best model to move this agent to, or fail.
# Order: its configured model, then the @fallbacks pool, then other agents'
# models. Skips the model it's on and any model cooling down; prefers models
# nobody else is using, so each agent draws on its own daily allowance.
pick_fallback() {
  local name="$1" current configured in_use m candidates=()
  current="$(agent_model "$name")"; configured="$(agent_field "$name" 4)"
  in_use="$(models_in_use_except "$name")"
  candidates+=("$configured")
  while IFS= read -r m; do [[ -n "$m" ]] && candidates+=("$m"); done < <(fallback_models)
  while IFS= read -r m; do [[ -n "$m" ]] && candidates+=("$m"); done <<<"$in_use"
  local pass
  for pass in unused shared; do
    for m in "${candidates[@]}"; do
      [[ "$m" == "$current" ]] && continue
      model_cooling "$m" && continue
      if [[ "$pass" == unused ]] && grep -qx -- "$m" <<<"$in_use"; then continue; fi
      echo "$m"; return 0
    done
  done
  return 1
}

# rescue_wip <name>: before the old session is removed, push any uncommitted
# or unpushed work in its sandbox to wip/<name>/<branch>. Never to the PR
# branch itself, so half-finished work can't trigger auto-merge. Best effort.
rescue_wip() {
  local name="$1" script
  # shellcheck disable=SC2016  # runs in the sandbox's shell
  script='for g in $(find /workspace "$HOME" -maxdepth 4 -name .git -type d 2>/dev/null | sort -u); do
  r=$(dirname "$g"); cd "$r" || continue
  b=$(git rev-parse --abbrev-ref HEAD 2>/dev/null) || continue
  case "$b" in main|HEAD) continue;; esac
  if [ -n "$(git status --porcelain)" ]; then
    git add -A && git -c user.name=lattice-NAME -c user.email=lattice-NAME@users.noreply.github.com commit -qm "wip: auto-saved before model failover"
  fi
  if git push -q origin "HEAD:refs/heads/wip/NAME/$b" 2>/dev/null; then echo "saved $r ($b) to wip/NAME/$b"; fi
done'
  script="${script//NAME/$name}"
  doctl harness-runtime exec "$(session "$name")" --timeout 120 -- bash -lc "$script" 2>&1 \
    | sed 's/^/    [wip] /' || true
}

# notify_status <text>: best-effort comment on the Swarm status issue.
notify_status() {
  local num body
  num="$(gh_get "/issues?state=open&labels=swarm:status&per_page=1" 2>/dev/null \
          | grep -oE '"number": *[0-9]+' | head -1 | grep -oE '[0-9]+')" || return 0
  [[ -n "$num" ]] || return 0
  body="$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
  curl -fsS -m 20 -X POST "https://api.github.com/repos/$REPO/issues/$num/comments" \
    -H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json" \
    -d "{\"body\":\"$body\"}" >/dev/null 2>&1 || true
}

# failover_agent <name> <reason>: move a DigitalOcean agent to another model by
# recreating its session, and write the handoff prompt for the new session.
failover_agent() {
  local name="$1" reason="$2" old new
  old="$(agent_model "$name")"
  if ! new="$(pick_fallback "$name")"; then
    echo "===== $(date '+%Y-%m-%d %H:%M:%S') $name: no fallback model available (all cooling down or in use); will retry on the next nudge"
    return 1
  fi
  echo "===== $(date '+%Y-%m-%d %H:%M:%S') $name: switching $old -> $new ($reason)"
  rescue_wip "$name"
  doctl harness-runtime remove "$(session "$name")" >/dev/null 2>&1 || true
  if [[ "$new" == "$(agent_field "$name" 4)" ]]; then
    rm -f "$(model_override_file "$name")"
  else
    echo "$new" >"$(model_override_file "$name")"
  fi
  render_spec "$name" >"$STATE_DIR/$name.yaml"
  if ! doctl harness-runtime create --spec "$STATE_DIR/$name.yaml"; then
    echo "===== $name: couldn't create the session on $new; will retry on the next nudge"
    return 1
  fi
  {
    render_prompt "$name"
    cat <<EOF

## Handoff: you're taking over mid-stream

You are continuing as **$name** after a model switch: the previous session ran on \`$old\` and stopped ($reason). That session is gone, but everything it pushed is on GitHub. Any unfinished work it hadn't pushed was saved to branches named \`wip/$name/<branch>\`.

Pick up where it left off: check open issues labeled \`swarm:$name\`, your open PRs, and any \`wip/$name/*\` branches. Continue that work, bringing changes over from \`wip/\` onto the real branch if they're worth keeping; never open a PR from a \`wip/\` branch. Then carry on with your normal loop.
EOF
  } >"$STATE_DIR/$name.handoff.md"
  notify_status "Swarm: **$name** switched from \`$old\` to \`$new\` ($reason). Unpushed work, if any, is on \`wip/$name/*\`."
  return 0
}

# _run-remote <name> <prompt-file>: run one DigitalOcean prompt (background),
# and on a daily-limit failure fail over and continue on the new model.
cmd_run_remote() {
  local name="$1" file="$2" attempt=0 out rc model
  need_creds
  out="$STATE_DIR/$name.lastrun"
  while :; do
    # No --quiet: its closing "run failed: ..." line is how failures are detected.
    # --on-hitl reject: nobody is there to approve, so refuse instead of hanging.
    set +e
    doctl harness-runtime prompt "$(session "$name")" - --on-hitl reject <"$file" 2>&1 | tee "$out"
    rc=${PIPESTATUS[0]}
    set -e
    [[ $rc -eq 0 ]] && return 0
    is_daily_limit_failure "$out" || return "$rc"

    model="$(agent_model "$name")"
    mark_rate_limited "$model"
    echo "===== $(date '+%Y-%m-%d %H:%M:%S') $name: $model hit its DigitalOcean daily token limit"
    [[ "$AUTO_FAILOVER" == 1 ]] || { echo "     (AUTO_FAILOVER=0; not switching models)"; return "$rc"; }
    attempt=$((attempt + 1))
    if [[ $attempt -gt $MAX_FAILOVERS ]]; then
      echo "===== $name: gave up after $MAX_FAILOVERS model switches in one run"; return "$rc"
    fi
    failover_agent "$name" "$model hit its daily token limit" || return "$rc"
    file="$STATE_DIR/$name.handoff.md"
  done
}

# restore [names]: move agents on a fallback model back to their configured one.
cmd_restore() {
  local names name
  names="$(selected_names "$@")"
  need_doctl; need_creds
  for name in $names; do
    is_local "$name" && continue
    [[ -s "$(model_override_file "$name")" ]] || { echo "[$name] already on its configured model"; continue; }
    is_busy "$name" && { echo "[$name] mid-run; try again when it's idle"; continue; }
    if model_cooling "$(agent_field "$name" 4)"; then
      echo "[$name] $(agent_field "$name" 4) is still cooling down after a daily limit; try later"; continue
    fi
    failover_agent "$name" "restored to its configured model" && send_prompt "$name" "$STATE_DIR/$name.handoff.md"
  done
}

# ---------- GitHub helpers (curl only; no gh or python needed) ----------

gh_get() {
  curl -fsS -m 20 "https://api.github.com/repos/$REPO$1" \
    -H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json"
}

# True when at least one open issue is labeled `ready`. Agents pick their own
# work from the backlog, so the coordinator only needs to have built one.
ready_work_exists() {
  gh_get "/issues?state=open&labels=ready&per_page=1" 2>/dev/null | grep -q '"number"'
}

# ---------- commands ----------

check_local() {
  local fail=0 name model out
  info "local agents (Claude Code on this machine)"
  if ! command -v claude >/dev/null; then
    warn "claude not found; install Claude Code (https://claude.com/claude-code)"; return 1
  fi
  ok "claude $(claude --version 2>/dev/null | head -1)"
  # Catch a misspelled key name in .env (it would silently fall back to the login).
  if [[ -f "$HERE/.env" ]] && grep -qE '^[A-Z_]*ANTHROPIC[A-Z_]*KEY=.' "$HERE/.env" \
       && ! grep -qE '^LOCAL_ANTHROPIC_API_KEY=.' "$HERE/.env"; then
    warn ".env sets $(grep -oE '^[A-Z_]*ANTHROPIC[A-Z_]*KEY' "$HERE/.env" | head -1), but local agents read LOCAL_ANTHROPIC_API_KEY"
    fail=1
  fi
  if [[ -n "${LOCAL_ANTHROPIC_API_KEY:-}" ]]; then
    ok "using LOCAL_ANTHROPIC_API_KEY from .env"
  elif env -u ANTHROPIC_API_KEY -u ANTHROPIC_BASE_URL claude auth status >/dev/null 2>&1; then
    ok "Claude Code has stored credentials (the test call below confirms they work)"
  else
    warn "Claude Code isn't signed in (run: claude auth login), or set LOCAL_ANTHROPIC_API_KEY in .env"; fail=1
  fi
  if [[ "$(uname -s)" == "Linux" ]] && ! command -v bwrap >/dev/null; then
    warn "the Claude Code sandbox needs bubblewrap on Linux (install bwrap)"; fail=1
  fi
  for name in $(roster | awk '$3 == "local" {print $1}'); do
    model="$(agent_field "$name" 4)"
    write_local_settings "$name"
    build_local_cmd "$name"
    # One tool-free turn: proves auth, model access, the sandbox, and flags.
    # The prompt goes on stdin, like real runs: --tools takes multiple values
    # and would swallow a prompt passed as an argument after it.
    if out="$(cd "${TMPDIR:-/tmp}" && printf 'Reply with the single word OK.\n' \
                | "${local_cmd[@]}" -p --permission-prompts none --max-turns 1 --tools "" 2>&1)"; then
      ok "$name: $model answered ($(printf '%s' "$out" | tr -d '\n' | head -c 40))"
    else
      warn "$name: Claude Code run failed with model '$model':"
      printf '%s\n' "$out" | head -5 | sed 's/^/      /' >&2
      if grep -qiE '401|authenticat|invalid.*(token|key)' <<<"$out"; then
        if [[ -n "${LOCAL_ANTHROPIC_API_KEY:-}" ]]; then
          warn "Anthropic rejected LOCAL_ANTHROPIC_API_KEY; check the key in .env"
        else
          warn "your Claude Code sign-in has expired or is invalid: run 'claude auth login'"
          warn "(or set LOCAL_ANTHROPIC_API_KEY in .env to use an API key instead)"
        fi
      else
        warn "(if a flag is unknown, update Claude Code: claude update)"
      fi
      fail=1
    fi
  done
  return $fail
}

check_remote() {
  local fail=0
  need_doctl
  ok "doctl $(doctl version | head -1)"
  doctl account get >/dev/null 2>&1 && ok "doctl is authenticated" || { warn "doctl is not authenticated (doctl auth init)"; fail=1; }

  info "doctl flags this script relies on"
  local h f
  h="$(doctl harness-runtime create --help 2>&1 || true)"
  for f in --spec --dry-run; do grep -q -- "$f" <<<"$h" && ok "create $f" || { warn "create $f missing"; fail=1; }; done
  h="$(doctl harness-runtime prompt --help 2>&1 || true)"
  for f in --on-hitl --quiet; do grep -q -- "$f" <<<"$h" && ok "prompt $f" || { warn "prompt $f missing"; fail=1; }; done

  info "prepayment balance"
  doctl harness-runtime balance || warn "could not read balance"

  info "DigitalOcean models in swarm.conf, including @fallbacks (a 1-token test call each; costs a fraction of a cent)"
  [[ -n "$(fallback_models)" ]] || warn "no @fallbacks line in swarm.conf: agents can't switch models when one hits its daily limit"
  local models m code body
  if models="$(cmd_models 2>/dev/null)" && [[ -n "$models" ]]; then
    for m in $( { roster | awk '$3 != "local" {print $4}'; fallback_models; } | sort -u); do
      if ! grep -qx -- "$m" <<<"$models"; then
        warn "$m is not a known model ID (run ./swarm.sh models)"; fail=1; continue
      fi
      # The model list shows every model, including ones your account tier
      # can't use, so make a real (tiny) call to find out.
      body="$(model_probe "$m")"; code="${body##*$'\n'}"; body="${body%$'\n'*}"
      if [[ "$code" == 2* ]]; then
        ok "$m"
      elif [[ "$code" == 403 ]]; then
        warn "$m is not available for your DigitalOcean account tier (HTTP 403)"; fail=1
      else
        warn "$m test call failed (HTTP ${code:-none}): $(printf '%s' "$body" | head -c 200)"; fail=1
      fi
    done
  else
    warn "could not list models (check HARNESS_INFERENCE_API_KEY)"; fail=1
  fi

  if uses_claude_code; then
    info "claude-code key check (the request doctl makes before creating a Claude Code session)"
    code="$(claude_key_check || true)"
    if [[ "$code" == 2* ]]; then
      ok "DigitalOcean accepts the model access key via x-api-key (HTTP $code)"
    else
      warn "DigitalOcean returned HTTP ${code:-none} for GET /v1/models with x-api-key."
      warn "doctl will refuse to create claude-code sessions. Use the opencode adapter instead."
      fail=1
    fi
  fi

  info "dry run: render every DigitalOcean spec and let doctl resolve it (nothing is created)"
  mkdir -p "$STATE_DIR"
  local name out
  for name in $(roster | awk '$3 != "local" {print $1}'); do
    render_spec "$name" >"$STATE_DIR/$name.yaml"
    if out="$(doctl harness-runtime create --spec "$STATE_DIR/$name.yaml" --dry-run 2>&1)"; then
      ok "$(session "$name") spec accepted"
    else
      warn "$(session "$name") spec rejected:"; echo "$out" | sed 's/^/      /' >&2; fail=1
    fi
  done
  return $fail
}

cmd_check() {
  local fail=0
  need_creds; ok "credentials loaded"

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

  if any_remote; then check_remote || fail=1; fi
  if any_local; then check_local || fail=1; fi

  echo
  if [[ $fail -eq 0 ]]; then info "all checks passed"; else warn "fix the items above before ./swarm.sh up"; return 1; fi
}

cmd_models() {
  [[ -n "${HARNESS_INFERENCE_API_KEY:-}" ]] || die "HARNESS_INFERENCE_API_KEY is not set (see .env.example)"
  curl -fsS -m 20 "$DO_INFERENCE_URL/v1/models" \
    -H "Authorization: Bearer $HARNESS_INFERENCE_API_KEY" \
    | grep -oE '"id"[[:space:]]*:[[:space:]]*"[^"]+"' \
    | sed -E 's/.*"([^"]+)"$/\1/' | sort -u
}

cmd_render() {
  [[ $# -eq 1 ]] || die "usage: swarm.sh render <name>"
  if is_local "$1"; then
    write_local_settings "$1"
    echo "# $1 runs locally: Claude Code (model $(agent_field "$1" 4)) in $(repo_dir "$1")"
    echo "# with these settings ($(local_settings_file "$1")):"
    cat "$(local_settings_file "$1")"
  else
    render_spec "$1"
  fi
}

create_and_start() {
  local name="$1"
  mkdir -p "$STATE_DIR"
  render_prompt "$name" >"$STATE_DIR/$name.prompt.md"
  if agent_exists "$name"; then
    info "$name already exists; not recreating (use 'nudge' to wake it)"
    return 0
  fi
  if is_local "$name"; then
    info "starting $name locally in $(ws_dir "$name")"
    init_local "$name"
    send_prompt "$name" "$STATE_DIR/$name.prompt.md" first
  else
    render_spec "$name" >"$STATE_DIR/$name.yaml"
    info "creating $(session "$name") (waits for the sandbox to be ready)"
    doctl harness-runtime create --spec "$STATE_DIR/$name.yaml"
    send_prompt "$name" "$STATE_DIR/$name.prompt.md"
  fi
}

cmd_up() {
  local names name has_coord=false workers=()
  names="$(selected_names "$@")"
  # shellcheck disable=SC2086
  need_tools_for $names; need_creds
  mkdir -p "$STATE_DIR"
  for name in $names; do
    if [[ "$name" == "coordinator" ]]; then has_coord=true; else workers+=("$name"); fi
  done

  if $has_coord; then
    create_and_start coordinator
    if [[ ${#workers[@]} -gt 0 ]] && ! ready_work_exists; then
      info "waiting up to $COORD_WAIT_MIN min for the coordinator to build a ready backlog (Ctrl-C to stop; rerun 'up' for the others later)"
      local waited=0
      until ready_work_exists; do
        if [[ $waited -ge $((COORD_WAIT_MIN * 60)) ]]; then
          warn "no ready issues yet; starting the others anyway (they'll go idle until nudged)"
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
  local names name state where
  names="$(selected_names)"
  if any_remote; then
    need_doctl
    info "all sessions on your DigitalOcean team"
    doctl harness-runtime list --format SessionID,Name,Status,PauseReason,CreatedAt
    echo
  fi
  info "swarm agents"
  for name in $names; do
    if is_local "$name"; then where="laptop"; else where="DigitalOcean"; fi
    if is_busy "$name"; then
      state="mid-run (pid $(cat "$(pid_file "$name")"))"
    elif agent_exists "$name"; then
      state="idle"
    else
      state="not started"
    fi
    local shown; shown="$(agent_model "$name")"
    [[ "$shown" != "$(agent_field "$name" 4)" ]] && shown="$shown (fallback)"
    printf '  %-12s %-13s %-28s %s\n' "$name" "$where" "$shown" "$state"
  done
  local m cooling=()
  for m in $( { roster | awk '$3 != "local" {print $4}'; fallback_models; } | sort -u); do
    model_cooling "$m" && cooling+=("$m")
  done
  if [[ ${#cooling[@]} -gt 0 ]]; then
    echo; info "models cooling down after a daily limit (${RATE_LIMIT_COOLDOWN_HOURS}h): ${cooling[*]}"
  fi
  # DigitalOcean sessions named like swarm agents that the roster no longer
  # runs there (e.g. an agent moved to "local") keep running and billing.
  if command -v doctl >/dev/null; then
    local n agent
    for n in $(doctl harness-runtime list --format Name --no-header 2>/dev/null | grep "^$PREFIX" || true); do
      agent="${n#"$PREFIX"}"
      if ! roster | awk -v a="$agent" '$1 == a' | grep -q . || is_local "$agent"; then
        warn "$n is still on DigitalOcean, but swarm.conf doesn't run $agent there."
        warn "    Remove it with: doctl harness-runtime remove $n"
      fi
    done
  fi
}

# Role-specific nudge if there is one (prompts/nudge-coordinator.md), else the
# shared prompts/nudge.md.
nudge_file() {
  local f; f="$HERE/prompts/nudge-$(agent_field "$1" 2).md"
  if [[ -f "$f" ]]; then echo "$f"; else echo "$HERE/prompts/nudge.md"; fi
}

cmd_tell() {
  [[ $# -ge 2 ]] || die "usage: swarm.sh tell <name> <message...> | swarm.sh tell <name> @file"
  local name="$1"; shift
  agent_line "$name" >/dev/null
  need_tools_for "$name"; need_creds
  mkdir -p "$STATE_DIR"
  local msg="$STATE_DIR/$name.tell.md"
  if [[ $# -eq 1 && "$1" == @* ]]; then
    local src="${1#@}"; [[ -f "$src" ]] || die "no such file: $src"
    cp "$src" "$msg"
  else
    printf '%s\n' "$*" >"$msg"
  fi
  if ! is_busy "$name" && ! agent_exists "$name"; then
    die "$name isn't started (./swarm.sh up $name)"
  fi
  if is_busy "$name"; then
    die "$name is mid-run; try again when ./swarm.sh status shows it idle"
  fi
  send_prompt "$name" "$msg"
}

cmd_nudge() {
  local names name
  names="$(selected_names "$@")"
  # shellcheck disable=SC2086
  need_tools_for $names; need_creds
  mkdir -p "$STATE_DIR"
  for name in $names; do
    if ! is_busy "$name" && ! agent_exists "$name"; then
      echo "[$name] not started (start it with: ./swarm.sh up $name)"
      continue
    fi
    send_prompt "$name" "$(nudge_file "$name")"
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

stop_run() {
  local name="$1" pid
  if is_busy "$name"; then
    pid="$(cat "$(pid_file "$name")")"
    pkill -TERM -P "$pid" 2>/dev/null || true
    kill "$pid" 2>/dev/null || true
    echo "[$name] stopped run (pid $pid)"
  fi
  rm -f "$(pid_file "$name")"
}

cmd_each() {
  local verb="$1"; shift
  local names name
  names="$(selected_names "$@")"
  for name in $names; do
    if is_local "$name"; then
      case "$verb" in
        pause)  stop_run "$name"; echo "[$name] local agent paused; 'nudge' continues it" ;;
        resume) echo "[$name] local agents resume on the next nudge" ;;
        *)      die "internal: unsupported verb $verb for local agent" ;;
      esac
    else
      need_doctl
      info "$verb $(session "$name")"
      doctl harness-runtime "$verb" "$(session "$name")" || warn "$verb failed for $(session "$name")"
    fi
  done
}

cmd_attach() {
  [[ $# -eq 1 ]] || die "usage: swarm.sh attach <name>"
  local name="$1"
  if is_local "$name"; then
    need_claude; need_creds
    local_exists "$name" || die "$name hasn't been started (./swarm.sh up $name)"
    is_busy "$name" && die "$name is mid-run; wait for it to finish (or ./swarm.sh pause $name) before attaching"
    write_local_settings "$name"
    build_local_cmd "$name"
    local sid
    sid="$(local_session_id "$name")" \
      || die "no conversation recorded for $name yet (it starts on its first run; check ./swarm.sh logs $name)"
    ensure_local_clone "$name" >/dev/null
    cd "$(repo_dir "$name")"
    exec "${local_cmd[@]}" --resume "$sid"
  else
    need_doctl
    doctl harness-runtime launch "$(session "$name")"
  fi
}

cmd_logs() {
  [[ $# -eq 1 ]] || die "usage: swarm.sh logs <name>"
  if is_local "$1"; then
    local log; log="$(log_file "$1")"
    [[ -f "$log" ]] || die "no log yet for $1"
    tail -n 200 "$log"
  else
    need_doctl
    doctl harness-runtime logs "$(session "$1")"
  fi
}

cmd_down() {
  local all name existing=()
  all="$(selected_names "$@")"
  # shellcheck disable=SC2086
  need_tools_for $all
  for name in $all; do
    if agent_exists "$name"; then existing+=("$name"); else echo "[$name] not started; nothing to remove"; fi
  done
  [[ ${#existing[@]} -gt 0 ]] || return 0
  read -r -p "Remove: ${existing[*]}? Their sessions/workspaces are deleted. [y/N] " reply
  [[ "$reply" == [yY] ]] || { echo "aborted"; return; }
  local ws
  for name in "${existing[@]}"; do
    stop_run "$name"
    if is_local "$name"; then
      ws="$(ws_dir "$name")"
      # Only ever delete a directory this script created.
      if [[ -f "$ws/.lattice-agent" && "$ws" == "$LOCAL_HOME/"* ]]; then
        rm -rf "$ws"; echo "[$name] removed local workspace $ws"
      fi
    else
      info "remove $(session "$name")"
      doctl harness-runtime remove "$(session "$name")" || warn "remove failed for $(session "$name")"
    fi
  done
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
    tell)    cmd_tell "$@" ;;
    restore) cmd_restore "$@" ;;
    _run-remote) cmd_run_remote "$@" ;;
    watch)   cmd_watch "$@" ;;
    attach)  cmd_attach "$@" ;;
    logs)    cmd_logs "$@" ;;
    pause)   cmd_each pause "$@" ;;
    resume)  cmd_each resume "$@" ;;
    down)    cmd_down "$@" ;;
    ""|-h|--help|help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try: swarm.sh help)" ;;
  esac
}

main "$@"
