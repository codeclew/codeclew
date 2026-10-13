# Codeclew

Codeclew gives a coding agent a bounded view of a question through captured source. A computation thread starts at a chosen root and direction, follows connected steps and branches, records effects and declared service handoffs, and stops at the evidence frontier where captured context ends. Session generation uses the session profile to select generation behavior; thread-flow loading retrieves and verifies a flow from local state. These source-backed paths are not runtime traces.

Install Codeclew and its Codex instructions:

```sh
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
```

```sh
export PATH="$HOME/.local/bin:$PATH"
```

```sh
clew skill install --agent codex
```

In your repository, run:

```sh
clew doctor repository --repo "$PWD"
```

Then ask your agent: “Choose the best READY_FOR_TASK_DOCTOR contour in the report without asking me to choose. Derive its target ref, profile, and compilation from the report; run the read-only analysis and return a compact source-backed path through functions, branches, effects, service handoffs, and the evidence frontier. Distinguish source findings from runtime claims.” The agent uses your configured model and provider; Codeclew requires no paid API or hosted model.

Explore the weather demo: https://github.com/codeclew/clew-weather-demo
