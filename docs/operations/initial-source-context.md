# Prepare source before the first model request

The source candidate adds `clew context packet` for Kotlin analysis. A client
can attach its output before starting a model conversation, avoiding the model
request that merely chooses the first source-retrieval command. Selection and
rendering make no model calls. This capability is a local candidate after
0.7.1; it is not part of the published 0.7.1 archives.

Supply one to three exact identifiers from the task. No implementation paths,
answer text, or acceptance oracle are used to select source. Normal analysis
admission still applies, including the chosen compilation and compiler profile.

```sh
clew context packet \
  --repo /absolute/repository --target-ref refs/heads/main \
  --language kotlin --profile kotlin-jvm-gradle-analysis \
  --compilation :/main --identifier ImportController
```

The packet binds an immutable repository snapshot, generation authority and
source CAS references. It retains the session for later use. A class name must
be unique; constructors do not become additional class-name matches. An
ambiguous or missing identifier produces `ABSTAIN`, with no guessed source.
`READY_WITH_LIMITS` means that the requested root sources are available, not
that the task is complete or that all relevant behavior was captured.

The first policy includes complete declarations, or complete files up to 8 KiB,
and traverses retained exact K2 call targets in both directions for at most two
hops within each compilation. Complete selected classes seed their direct
members' calls. Unresolved, ambiguous and external targets are not invented.
Properties, type dependencies and cross-compilation calls are not followed.
Calls do not establish runtime dispatch, callback execution or invocation order.

Test companions are selected by the implementation filename plus `Test.kt` or
`Tests.kt` under a test directory. Their relationship is explicitly unverified,
and no tests are executed. Differently named tests may require native discovery.
Source delivery is limited to 64 KiB, individual declarations to 32 KiB, test
companions to four complete files and 16 KiB in total, and source declarations
to 24 windows. Graph exploration considers at most 128 declarations. A large
declaration is omitted whole and reported, rather than silently cut into a
misleading fragment. The JSON response has a separate 128 KiB bound.

## Client preparation

The source repository includes `scripts/prepare_agent_context.py`. It invokes
the supported launcher, verifies the packet, retains raw diagnostics and writes
a private `prompt.md` for the client's initial request. During source development,
pass `--clew /absolute/codeclew-checkout/clew`; do not invoke capsule binaries.

```sh
python3 -I -S scripts/prepare_agent_context.py \
  --repo /absolute/repository --target-ref refs/heads/main \
  --profile kotlin-jvm-gradle-analysis --compilation :/main \
  --identifier ImportController \
  --question-file /absolute/question.txt \
  --output-dir /absolute/new-private-preparation
```

The output directory must be new. Attach the resulting prompt as the first
message to the agent, before any model-driven discovery. The script itself does
not launch a model. It records zero preparation model calls/tokens, elapsed
preparation time, the command outcome and the initial-message digest. All prompt
tokens must still count in the model's actual input usage.

Preparation stops on failed admission or unusable root selection. A caller may
explicitly choose `--allow-native-on-failure`; the resulting initial message
retains the failure and authorizes native continuation without claiming successful
managed evidence. Failed preparation time and all subsequent agent tokens belong
in the comparison. Diagnostics and retained sessions are not deleted to obtain a
successful result.

This candidate does not establish a threefold token reduction. The previous
[manual source calibration](../plans/token-value-three-criteria.md) excluded
automatic discovery and cannot substitute for an equal-quality comparison of
this implementation with a disciplined native agent.
