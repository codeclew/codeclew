# Codeclew TypeScript evidence adapter

This adapter runs the repository-local TypeScript compiler on a real strict
project. It emits canonical `codeclew.adapter-output/0.1` JSON containing the
exact snapshot inputs, adapter/toolchain/configuration capability tuples,
compiler-resolved entities and occurrences, conservative reference/call
facts, explicit semantic boundaries, a separate `tsc --noEmit` receipt, and
cost telemetry.

It never claims a closed runtime call graph. TypeScript `any`, unresolved
symbols/modules, computed access, ambient declarations, dynamic imports,
source-map limitations, and open-world dispatch remain visible boundaries.
Compiler acceptance only supports the compiler-check claim. Call-target facts
are explicitly `STATICALLY_APPROXIMATED`/`HEURISTIC` and `PARTIAL`; resolved
references remain compiler-resolved facts with partial enumeration.

```sh
cd adapters/typescript
npm ci
npm test
npm run fixture
```

The CLI accepts the shared `--repo`, `--seed-entity`, `--max-depth`, and
`--max-entities` arguments. It exits `0` for an accepted compiler receipt, `2` after emitting evidence
for a compiler-rejected project, and `64` without evidence for an invalid or
non-strict project configuration.
