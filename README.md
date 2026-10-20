# Codeclew

Codeclew is a local CLI that gives engineers and coding agents bounded,
inspectable code evidence for an engineering question. Use it to find exact
source, explain selected behavior, review changes, prepare linked documentation,
and carry out changes admitted by a supported profile.

A **computation thread** is the question's path through selected functions,
branches, effects and service handoffs, ending where the evidence ends. It can
fit inside one method. It is distinct from the multi-session CLI `thread`
container and from an authored documentation process; none is a runtime trace.

Start with the [engineering guide](https://codeclew.github.io/codeclew/engineering.html):

1. [Questions, inputs and outputs](https://codeclew.github.io/codeclew/engineering.html#tasks),
   including one clearly illustrative checkout investigation.
2. [How thread boundaries are chosen](https://codeclew.github.io/codeclew/thread-boundaries.html).
3. [Languages, annotations and workflow limits](https://codeclew.github.io/codeclew/languages-annotations.html).
4. [Cross-service threads](https://codeclew.github.io/codeclew/cross-service.html):
   contracts, explicit relationships, receiver evidence and incomplete outcomes.

Install Codeclew and its Codex instructions:

```sh
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
clew skill install --agent codex
```

In your repository, discover available scopes:

```sh
clew doctor repository --repo "$PWD"
```

Give your agent the question and starting function or behavior, and ask it to
choose a relevant ready scope from that report, use the reported
revision/profile/compilation, and return source with unresolved boundaries.
Use the [editable question/root template](https://codeclew.github.io/codeclew/index.html#ask-agent)
when starting a new investigation. Readiness is discovery, not the answer.
Compiler-backed work needs the project's toolchain and restored dependencies.
For documentation from existing saved evidence, use the saved Check and process
or declaration selection instead of starting a new capture by default.

The agent uses its configured model/provider. Codeclew does not require a paid
API or hosted model service; optional model-assisted authoring is separate.
For execution details, use the [agent workflow](skills/codeclew/SKILL.md) and
[native source documentation guide](docs/guides/native-retained-source.md).
For a full saved publication, the [Docusaurus export](docs/operations/docusaurus-export.md)
provides linked Markdown/MDX pages, source evidence and optional field readers.
Use [remote source rendering](docs/operations/source-link-export.md) to export exact
GitHub/GitLab links instead of automatically attached source text.
The [Weather reader](https://codeclew.github.io/clew-weather-demo/index.html) has
its own retained evidence and limits; it is separate from the fictional example.
