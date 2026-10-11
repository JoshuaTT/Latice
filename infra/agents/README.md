# LATTICE agent swarm

Runs a team of coding agents on [DigitalOcean Managed Agents](https://docs.digitalocean.com/products/managed-agents/) to build LATTICE. Each agent is a Harness Runtime session (its own microVM) running Claude Code, OpenCode, or Codex CLI against a model from DigitalOcean Inference, all on one DigitalOcean key.

## How the swarm works

There's no special orchestration service. **GitHub is the coordination layer:**

```
            ┌──────────────── GitHub: JoshuaTT/Latice ────────────────┐
            │  issues (labeled swarm:<name>)   PRs   CI   CODEOWNERS  │
            └──▲──────────────▲────────────▲───────────▲──────────▲───┘
   files &     │  pick up     │            │           │          │ approves
   assigns     │  issues,     │            │           │          │ spec + owned
   issues      │  open PRs    │            │           │          │ paths
         ┌─────┴─────┐  ┌─────┴──┐  ┌──────┴─┐  ┌──────┴─┐  ┌─────┴──┐
         │coordinator│  │   w1   │  │   w2   │  │   w3   │  │  Josh  │
         └───────────┘  └────────┘  └────────┘  └────────┘  └────────┘
                 ▲ nudge every N min (swarm.sh watch, from your machine)
```

- **Coordinator** (one session): turns the current milestone in `AGENTS.md` §12 into small issues, assigns each to a worker with a `swarm:<name>` label, watches PRs, escalates anything that needs you, and keeps a **`Swarm status`** issue up to date. That issue is your dashboard.
- **Workers** (the rest): loop on their assigned issues: branch → implement → test → PR → auto-merge → next. They follow `AGENTS.md`.
- **CI, rulesets, and CODEOWNERS** decide what merges. Code with green CI merges by itself; spec and other code-owned changes wait for you.
- **Nudges** keep the loop going. Agents stop when they run out of work or finish a task. `swarm.sh watch` sends each one a short "continue" prompt on a timer. Idle sessions pause automatically (30 min) and wake on the next nudge, so a quiet swarm costs almost nothing in compute.

The default roster (`swarm.conf`) is a coordinator plus four workers on four different model/harness combinations. Edit it freely.

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
| `./swarm.sh nudge [names]` | Nudge now (all, or named agents) |
| `./swarm.sh attach w2` | Talk to one agent interactively; `Ctrl-D` detaches |
| `./swarm.sh logs w2` | Replay an agent's event history |
| `./swarm.sh pause` / `resume` | Stop / restart compute billing (storage still bills) |
| `./swarm.sh up w5` | Add an agent after adding it to `swarm.conf` |
| `./swarm.sh down` | Remove sessions (asks first) |
| `./swarm.sh render w3` | Print one agent's rendered spec |

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
- **What did an agent say?** `cat infra/agents/.swarm/<name>.log` for its replies to each prompt; `./swarm.sh logs <name>` for the full event history on DigitalOcean.
- **An agent asked for approval.** Prompts run with `--on-hitl reject`, so approval requests are refused rather than left hanging. If an agent keeps hitting one, add an explicit rule for it in `specs/<adapter>.yaml`.
- **An agent times out installing something.** Its host isn't in the spec's `egress` list. Add it to `specs/<adapter>.yaml` and recreate the session.
- **Agents sit idle.** Make sure `watch` is running and the coordinator has created issues with `swarm:<name>` labels.
- **Two agents worked on the same issue.** Check that the issue had exactly one `swarm:*` label; the coordinator is responsible for that.
