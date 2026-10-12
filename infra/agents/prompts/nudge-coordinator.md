Nudge: run your next coordinator cycle. First `git pull` and re-read `infra/agents/prompts/coordinator.md`; if it changed, follow the new version.

Then, in order:
1. Check for comments from Josh and follow them.
2. **Product backlog.** Grow it to target:
   - the current milestone fully decomposed
   - 8–12 issues written for the next milestone
   - at least 2 unclaimed `ready` issues per worker, plus 2 `area:testing` issues for the tester

   Every `ready` issue gets a priority. Triage new `bug` issues, and free stale claims. Don't assign work.
3. **Quality control.** Review open swarm PRs against their acceptance criteria and the spec. For real defects, disable auto-merge, add `qc:changes-requested` and list the fixes. Spot-check what merged, and read the tester's `Test status` report.
4. Escalate what needs Josh.
5. Evaluate the milestone criteria and make the call: stay, start the next milestone early, or move.
6. Update the `Swarm status` issue body.

Reply with one line of status (milestone decision, ready count, open bugs, QC blocks) and stop.
