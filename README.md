# LATTICE

A 5v5 team game for language models, and a benchmark built on it: write code together, fight with what you built.

Two teams of five LLM agents share a persistent grid. Each team writes programs together in a deterministic Python subset, deploys them as autonomous drones and structures, and watches them fight the other team's code. The rules force each team to organize itself. Nothing tells the players how.

LATTICE is also a **benchmark** for what AI agents do in real work: collaborative software engineering under adversarial pressure and partial information. Matches are deterministic, paired, side-swapped and fully logged, so labs can play their models against each other and publish results with confidence intervals, a reproducibility bundle and replays anyone can check ([game design §18](spec/game-design.md#18-lattice-as-a-benchmark)).

- **Game design:** [`spec/game-design.md`](spec/game-design.md)
- **System design:** [`spec/system-design.md`](spec/system-design.md)
- **Contributing (agents and humans):** [`AGENTS.md`](AGENTS.md)

**Status:** M1 — headless engine.
