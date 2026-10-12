# LATTICE agent swarm

Runs a team of coding agents on [DigitalOcean Managed Agents](https://docs.digitalocean.com/products/managed-agents/) to build LATTICE. Each agent is a Harness Runtime session (its own microVM) running Claude Code, OpenCode, or Codex CLI against a model from DigitalOcean Inference, all on one DigitalOcean key.

## How the swarm works

There's no special orchestration service. **GitHub is the coordination layer:**

```
            ┌──────────────────── GitHub: JoshuaTT/Latice ─────────────────────┐
            │  prioritized backlog (ready, priority:*)   PRs   CI   human-gate  │
            └──▲──────────────▲───────────────▲───────────────▲───────────▲────┘
  writes,      │  self-assign │  self-assign   │  runs games,  │           │ approves
  prioritizes, │  (swarm:<n>),│  area:testing, │  files bugs   │           │ needs-human
  reviews PRs  │  open PRs    │  harness PRs   │               │           │ PRs
         ┌─────┴─────┐  ┌─────┴──┐       ┌─────┴──┐                   ┌────┴───┐
         │coordinator│  │workers │       │ tester │                   │  Josh  │
         └───────────┘  └────────┘       └────────┘                   └────────┘
                 ▲ nudge every N min (swarm.sh watch, from your machine)
```

- **Coordinator: product owner and quality controller** (one session).
  - **Backlog:** keeps it deep and prioritized: the current milestone fully broken into precise issues, the next milestone planned ahead, at least 2 `ready` issues per worker, and `area:testing` work for the tester.
  - **Quality control:** reviews PRs and merged work against the spec. It blocks defective PRs by turning off their auto-merge and adding `qc:changes-requested`. It also triages the tester's bugs.
  - **Milestones:** decides when to move to the next one, recorded as a `Milestone decision` comment with evidence.
  - **Dashboard:** keeps the **`Swarm status`** issue up to date.
  - **It doesn't assign work.**
- **Workers** pick their own tasks from the `ready` backlog in priority order, claim them with their `swarm:<name>` label, and loop: branch, implement, test, PR, auto-merge, next. They follow `AGENTS.md`.
- **Tester:**
  - builds and runs the software against real scenarios, and plays full games with scripted players and no models
  - owns the scenario harness (`harness/`) and regression suite (`tests/scenarios/`)
  - files bugs with exact repros, and reports in a **`Test status`** issue
- **CI, rulesets and `human-gate`** decide what merges. Code with green CI merges by itself; PRs labeled `needs-human`, or touching code-owned paths, wait for you.
- **Nudges** keep the loop going. Agents stop when they run out of work or finish a task. `swarm.sh watch` sends each one its role's nudge on a timer (`prompts/nudge-<role>.md`, else `prompts/nudge.md`). Idle sessions pause automatically after 30 minutes and wake on the next nudge, so a quiet swarm costs almost nothing in compute.

The roster (`swarm.conf`) lists each agent's role, harness and model. Edit it freely; mixing models doubles as data on which models build well together.

## One-time setup

1. **Merge the scaffold PR** so `AGENTS.md` and the specs are on `main`. Agents read them from there.
2. **Repo settings** (Settings → General → Pull Requests): turn on **Allow auto-merge** and **Automatically delete head branches**.
3. **DigitalOcean**
   - Create a **model access key** (Inference → Model Access Keys; starts with `doo-v1-`).
   - Install and authenticate doctl: `brew install doctl && doctl auth init`.
   - Managed Agents may need a prepaid balance: `doctl harness-runtime balance`.
4. **GitHub token**: a fine-grained personal access token for `JoshuaTT/Latice` only, with **Contents, Pull requests, Issues: read and write**. Don't grant **Administration** (so agents can't bypass rulesets) or **Workflows** (so they can't change CI).
5. **Credentials**: `cp .env.example .env` in this folder and fill in both values. `.env` is gitignored.
6. **Check everything**:
   ```bash
   ./swarm.sh check     # doctl, auth, credentials, and the doctl flags this script uses
   ./swarm.sh models    # model IDs your key can use; fix swarm.conf if any differ
   ```

## Running it

```bash
./swarm.sh check       # must end with "all checks passed"
./swarm.sh up          # starts the coordinator, waits until it has assigned issues, then starts the workers
./swarm.sh watch 20    # nudges idle agents every 20 minutes; leave this running
```

How prompts run: `doctl harness-runtime prompt` blocks until the agent finishes its run, which can take hours. So `swarm.sh` sends every prompt in the background (it survives closing the terminal), tracks each agent's current run, and never nudges an agent that's still mid-run. Each agent's replies go to `infra/agents/.swarm/<name>.log` (gitignored). `./swarm.sh status` shows which agents are mid-run.

Then watch the `Swarm status` issue and the PR list. Your job:
- approve PRs labeled `needs-human` (spec, `.github`, sandbox, golden hashes)
- answer issues labeled `blocked:human`
- do the milestone checkpoint reviews in `spec/system-design.md` §11.3

Other commands:

| Command | What it does |
|---|---|
| `./swarm.sh status` | List swarm sessions |
| `./swarm.sh nudge [names]` | Nudge now (all, or named agents). Uses `prompts/nudge-<role>.md` if it exists (the coordinator has its own), else `prompts/nudge.md` |
| `./swarm.sh tell <name> <message>` | Send one agent a one-off message, e.g. `./swarm.sh tell coordinator "Prioritize the LatticePy spec"`, or `@file` |
| `./swarm.sh attach w2` | Talk to one agent interactively; `Ctrl-D` detaches |
| `./swarm.sh logs w2` | Replay an agent's event history |
| `./swarm.sh pause` / `resume` | Stop / restart compute billing (storage still bills) |
| `./swarm.sh up w5` | Add an agent after adding it to `swarm.conf` |
| `./swarm.sh down` | Remove sessions (asks first) |
| `./swarm.sh render w3` | Print one agent's rendered spec |

## Running an agent on your laptop

Set an agent's adapter to `local` in `swarm.conf` to run it as Claude Code on your machine, billed to your own Anthropic account instead of DigitalOcean:

```
# name         role         adapter      model    size
coordinator    coordinator  local        opus     -
```

The model column takes a Claude Code model alias (`opus`, `sonnet`, `haiku`) or a full model name. Everything else works the same: `up`, `nudge`, `watch`, `status`, `logs`, `attach`, `pause`, and `down` handle local and DigitalOcean agents together.

**Setup:** install Claude Code and sign in (`claude auth login`). To bill an Anthropic Console API key instead of your login, set `LOCAL_ANTHROPIC_API_KEY` in `.env`; `LOCAL_MAX_BUDGET_USD` caps each run's spend. `./swarm.sh check` makes one tiny tool-free call per local agent to confirm sign-in, model access, and the sandbox.

**How it runs:**
- Each local agent gets its own workspace in `~/lattice-swarm/<name>` (change with `LATTICE_LOCAL_HOME`). The script clones the repo into `~/lattice-swarm/<name>/Latice` itself, outside the sandbox, because the sandbox never lets a command write `.git/config` or `.git/hooks`, so an agent can't clone on its own. Claude Code runs with the clone as its working directory.
- Git pushes authenticate through a credential helper that reads the agent's token from its environment at run time, so the token is never written to disk; your global helpers (such as the macOS keychain) are switched off for these clones. `gh` uses the same token and its own config folder (`~/lattice-swarm/<name>/.gh-config`), never your `~/.config/gh`.
- Local agents get extra notes in their role prompt: skip the clone steps, avoid commands that write git config (`git push -u`, `git config`), and push with `git push origin HEAD`.
- Runs use `claude -p`; nudges use `--continue`, so the agent keeps its conversation across nudges. `attach` opens that same conversation interactively.
- Its environment is scrubbed: the DigitalOcean key is removed, and `gh`/`git` act as the agent's GitHub token, not your own `gh` login.
- `caffeinate` keeps the Mac awake while a run is in progress. If the laptop is closed or shut down, the run stops and the next nudge continues it.

**Containment.** A DigitalOcean agent runs in its own microVM; a local one runs on your machine, so the script turns on Claude Code's OS-level sandbox for its shell commands (`./swarm.sh render <name>` shows the exact settings):
- writes only to its workspace and tool caches (`~/.cargo`, `~/.rustup`, `~/.cache`, `~/.npm`, `~/.local`, `~/Library/Caches`)
- network only to GitHub and the Rust, Python, and npm package registries
- no reading `~/.ssh`, cloud credentials, your `gh`/`doctl` config, or this folder's `.env`
- the same deny rules as the DigitalOcean specs (no force-push, no pushing to `main`, no admin merges)
- permission prompts are refused rather than waiting for you, and the sandbox can't fall back to running unsandboxed

`gh` runs outside the sandbox (Go tools can fail TLS checks inside it on macOS), so it uses the agent's token with your normal access to the network. Claude Code still loads your personal `~/.claude` settings and `CLAUDE.md`; MCP servers are not loaded.

## Tuning the swarm

- **Fewer agents early.** M0 is mostly spec PRs that need your approval, so start with the coordinator and two workers (`./swarm.sh up coordinator w1 w2`) and add more at M1, when the engine work can run in parallel.
- **Models.** Put Claude models on `claude-code`, and other families on `opencode` or `codex`. Every PR records `Agent: <name> (<model> via <harness>)`, so you can compare later.
- **Nudge interval.** 15–30 minutes is plenty. More frequent nudges mostly burn tokens on "nothing to do".
- **Cost.** Inference tokens dominate. Set spending alerts in DigitalOcean before leaving a swarm running overnight, and `./swarm.sh pause` when you're not around to review.

## Later: run without your laptop

`swarm.sh watch` runs on your machine. To run unattended, Harness Runtime **triggers** can start a fresh session per event (`doctl harness-runtime triggers create`), such as a cron trigger every 30 minutes that runs one worker cycle, or a GitHub webhook when an issue gets a `swarm:*` label. Two caveats: unattended runs auto-approve every `ask` rule (these specs already use deny rules for that reason), and each run starts from a fresh session, so workers re-clone each time.

## Troubleshooting

- **A doctl command fails with an unknown flag.** Managed Agents is in public preview and its CLI is changing. Run `./swarm.sh check`, compare with `doctl harness-runtime <command> --help`, and adjust `swarm.sh`.
- **doctl asks for an Anthropic API key.** Don't paste one. The `claude-code` adapter requires `ANTHROPIC_API_KEY`, and our Claude Code agents send it to DigitalOcean Inference, so `swarm.sh` sets it to your DigitalOcean model access key automatically. If you see the prompt, you're running `doctl` directly instead of through `swarm.sh`, or `.env` isn't loaded.
- **"403 this model is not available for your subscription tier".** DigitalOcean Inference Tier 1 and 2 accounts can't use Anthropic or OpenAI models (except gpt-oss). Use open-weight models in `swarm.conf` until your account reaches Tier 3; `./swarm.sh check` makes a 1-token test call per model so this shows up before `up`.
- **What did an agent say?** `cat infra/agents/.swarm/<name>.log` for its replies to each prompt; `./swarm.sh logs <name>` for the full event history on DigitalOcean.
- **An agent asked for approval.** Prompts run with `--on-hitl reject`, so approval requests are refused rather than left hanging. If an agent keeps hitting one, add an explicit rule for it in `specs/<adapter>.yaml`.
- **An agent times out installing something.** Its host isn't in the spec's `egress` list. Add it to `specs/<adapter>.yaml` and recreate the session.
- **Agents sit idle.** Make sure `watch` is running and the coordinator has created issues with `swarm:<name>` labels.
- **Two agents worked on the same issue.** Check that the issue had exactly one `swarm:*` label; the coordinator is responsible for that.
