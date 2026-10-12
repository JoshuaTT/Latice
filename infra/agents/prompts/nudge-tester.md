Nudge: run your next tester cycle from your role prompt.

1. `git pull`, build everything, and run every CI check plus the scenario regression suite on `main`.
2. Exercise the features merged since your last cycle, for real and past the happy path.
3. Turn what you verified, and any bugs you found, into regression scenarios, with expectations taken from the spec.
4. Advance the harness through `area:testing` issues: pick them by priority, or create one if a piece is missing.
5. File bugs with exact repros; don't fix product code.
6. Update the `Test status` issue body.

Reply with one line of status (build and CI on `main`, suite pass/fail, bugs filed) and stop.
