# Three criteria for changing agent token efficiency

Status: diagnostic calibration, 2026-09-09. No product advantage is claimed.

The same `gpt-6-astra` / high model analyzed the existing K-A1 descriptor and
K-A2 project-JDK cache tasks. Source packets were manually selected before these
runs, contained only original source with line numbers, and used the same frozen
revision. These packets are an optimistic integration calibration, not an
autonomous Codeclew implementation. Native tools remained available for missing
facts. Parent source review checked the existing acceptance criteria and known
limits; there was no independent reviewer.

## Sufficient source in one retrieval

| Case | Earlier batched Default | Packet-first total tokens | Commands | Acceptance |
|---|---:|---:|---:|---|
| K-A1 | 61,863 | 41,147 | 1 | PASS |
| K-A2 | 220,879 | 109,347 | 4 | FAIL |

The packets contained 7,201 and 48,041 bytes. K-A2 still reread the fingerprint
helper and searched for the producer field, then omitted the established fact
that `inspect` does not copy `jdkHome` into its canonical output. Its lower token
count therefore does not establish equivalent-quality savings. Both packet
runs had two TLS reconnect events, so these are not isolated causal estimates.

Product criterion: the packet must support the producer, consumer predicates,
relevant helpers, test evidence and material limits, with no critical omissions.
Returning an exact root declaration is insufficient.

## Marginal cost of retaining source in a conversation

An identical follow-up asked about 255/256 parameter slots and array dimensions,
including the distinction from UTF-16 input length.

| Mode | Total tokens | Commands | Acceptance |
|---|---:|---:|---|
| Resume the original source conversation | 23,044 | 0 | PASS |
| Fresh conversation reading the same packet | 40,780 | 1 | PASS |

The marginal reduction was 43.5% (1.77x). Including the first packet answer, the
two-question totals were 64,191 versus 81,927, a 21.6% reduction. Neither follow-up
had transport errors. This measures retained model evidence, not compiler-cache
speed. Index reuse alone does not remove repeated model interactions.

## Provide the source packet before the first model request

The identical source bytes were then included in the initial prompt. Their
tokens count in input; no answer or acceptance rubric was supplied. Both agents
finished without tools and preserved the requested source facts and limits,
including K-A2's producer omission and memory-cache bypass. Neither had transport
errors.

| Case | Earlier batched Default | Preloaded total tokens | Observed ratio | Acceptance |
|---|---:|---:|---:|---|
| K-A1 | 61,863 | 21,759 | 2.84x | PASS |
| K-A2 | 220,879 | 31,000 | 7.13x | PASS |

This establishes a promising feasibility result for the longer chain, not a
Codeclew benchmark result. Manual source selection removed discovery work and
the packet-specific instructions differ from Default. The short case still
falls slightly below 3x. Do not generalize two consumed self-hosting diagnostics
or attribute the entire ratio to the timing of source delivery.

The earlier independent mechanical calibration remains relevant: the same
2,415 output bytes cost 37,352 tokens in one retrieval versus 75,075 in three
sequential retrievals. Preloading can also remove the initial model request
that chooses the first retrieval command.

## Next product test

Assemble a bounded source packet before starting the agent: use explicit task
identifiers, existing compiler identities, bounded source/call relationships,
test evidence and complete relevant declarations. Preserve source bindings and
unknown relationships. Continue with new evidence only while checking freshness.

First demonstrate automatic packet selection without manual paths or an oracle.
Then compare fresh tasks against equally disciplined Default, counting packet
preparation, admission failures and native continuation. Require no critical
omissions and measure cumulative input/output plus marginal continuation cost.
The current Codeclew threefold-efficiency goal remains unfulfilled.

All numbers are actual cumulative input plus output, including cached input;
they are not monetary costs or output-byte token estimates. Default was not
rerun. The two initial packet attempts used a nonexistent macOS launcher path;
their 158,775 tokens are retained and excluded from the one-retrieval comparison.
The corrected launcher was checked before replacement attempts. Successful
attempts were not repeated to obtain a favorable number.
