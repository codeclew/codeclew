# Task promotion

> **DRAFT / UNREVIEWED / NOT PUBLISHED** Structure and evidence\-label binding were validated\. Semantic correctness was not reviewed\.

<nav>**Contents:** [ Glossary ](#glossary) · [ Internal process explanation ](#ordered-behavior) · [ Process outline ](#process-outline) · [ Authored pseudocode ](#authored-pseudocode) · [ Source\-local process projection ](#source-process-projection) · [ Prepared values and checks ](#preparations) · [ Full summary ](#summary)</nav>

## Uncertainties

- Task and Task\.State declarations are not retained; field invariants and additional state semantics are unknown\.
- The isBefore body is not retained\. The explanation preserves its boolean result and call prerequisite without asserting its internal comparison rules or handling of a null now\.
- TaskRepository is retained only as an interface\. Concrete receiver identity, repository mutations, persistence, transaction commitment, and external or asynchronous completion are not established\.
- promote contains no exception handler\. Exceptions during field access, condition evaluation, or repository calls prevent later statements from being reached; concrete repository exception behavior is unknown\.
- Retained provider relations are callsite evidence, not a runtime trace\. Statement order described here comes from the retained promote body, and no executed invocation or constructor\-to\-promote ordering is established\.

<a id="process-outline"></a>
## Process outline

1. [**action:** Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\.](#block-receive_inputs)
2. [**decision:** Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\.](#block-choose_ready_branch)

<a id="glossary"></a>
## Glossary

<a id="glossary-invocation"></a>
### Promotion invocation _(Request)_

The public boolean promote\(Task task, Instant now\) method receives task and now and produces a boolean on normal completion\.<details class="citations"><summary>Evidence (2)</summary><span class="citation-list">[d76](#evidence-8), [s11](#evidence-13)</span></details>  **Glossary:** [Promotion invocation](#glossary-invocation)

<details>
<summary>Technical names</summary>

**Technical names:** ``promote`` · ``Task`` · ``Instant``

**Declaration references:** ``d76`` — d76 · method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z · class:example\.dispatch\.TaskStateTransitions · :/main

</details>

<a id="glossary-task"></a>
### Task input _(Technical carrier)_

The Task parameter supplies the state, id, and deadline fields read by promote\. Its declaration and field definitions are not retained\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task)

**Uncertainty:** The packet does not establish additional Task invariants, field types, or business semantics\.

<details>
<summary>Technical names</summary>

**Technical names:** ``Task`` · ``task`` · ``task.state`` · ``task.id`` · ``task.deadline``

**Declaration references:** ``d76`` — d76 · method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z · class:example\.dispatch\.TaskStateTransitions · :/main

</details>

<a id="glossary-states"></a>
### State comparison and requested target _(Term)_

Task\.State\.READY is the first comparison value\. Task\.State\.WAITING\_RESPONSE is both the requested state on that branch and the prerequisite comparison value for the deadline check\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [State comparison and requested target](#glossary-states)

<details>
<summary>Technical names</summary>

**Technical names:** ``Task.State.READY`` · ``Task.State.WAITING_RESPONSE``

**Declaration references:** ``d76`` — d76 · method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z · class:example\.dispatch\.TaskStateTransitions · :/main

</details>

<a id="glossary-deadline"></a>
### Deadline check _(Term)_

The expression task\.deadline\.isBefore\(now\) supplies the second operand of the expiry condition\. now is the supplied Instant argument; promote does not obtain a clock value itself\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Deadline check](#glossary-deadline)

**Uncertainty:** The isBefore implementation is not retained, so its internal comparison and handling of a null now are not established by the supplied source\.

<details>
<summary>Technical names</summary>

**Technical names:** ``task.deadline.isBefore(now)`` · ``Instant`` · ``now``

**Declaration references:** ``d76`` — d76 · method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z · class:example\.dispatch\.TaskStateTransitions · :/main

</details>

<a id="glossary-repository"></a>
### Repository interface and field _(Technical carrier)_

TaskStateTransitions holds a private final TaskRepository repository\. The retained interface declares void changeState\(String id, Task\.State state\) and void recordError\(String id, String reason\), which are the operations used by promote\.<details class="citations"><summary>Evidence (3)</summary><span class="citation-list">[d95](#evidence-11), [s19](#evidence-14), [s30](#evidence-15)</span></details>  **Glossary:** [Repository interface and field](#glossary-repository)

**Uncertainty:** No concrete repository implementation or runtime receiver identity is established\.

<details>
<summary>Technical names</summary>

**Technical names:** ``TaskRepository`` · ``repository`` · ``changeState`` · ``recordError``

**Declaration references:** ``d79`` — d79 · class:example\.dispatch\.TaskRepository · module:unnamed · :/main; ``d95`` — d95 · field:class:example\.dispatch\.TaskStateTransitions\#repository:Lexample/dispatch/TaskRepository; · class:example\.dispatch\.TaskStateTransitions

</details>

<a id="authored-pseudocode"></a>
## Authored pseudocode

Structure and branch order follow the supplied answer steps.

- [**action:** Use the task and now parameters supplied to promote\. The method reads task fields directly, doe…](#block-receive_inputs)
- [**decision:** The task state equals Task\.State\.READY](#block-choose_ready_branch)
- _Scope 1 — When the condition holds_
- [**action:** Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id…](#block-request_waiting_state)
- [**return:** After changeState returns normally, return the literal true\. This reports the selected local br…](#block-return_true)
- _End Scope 1_
- _Scope 2 — When the condition does not hold_
- [**decision:** The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true](#block-choose_expiry_branch)
- _Scope 3 — When the condition holds_
- [**action:** Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id…](#block-request_expiry_error) _Scope 3_
- _End Scope 3_
- _Scope 4 — When the condition does not hold_
- [**action:** When the conjunction is false, skip recordError and continue to the final return\. If its first…](#block-skip_error_recording) _Scope 4_
- _End Scope 4_
- [**return:** Return the literal false after the second condition and any selected recordError call complete…](#block-return_false)
- _End Scope 2_

<a id="source-process-projection"></a>
## Source\-local process projection

Built from retained source syntax; this is not execution evidence\.

### Parsed source outline

```text
Entry: promote(task, now)
[D] if (task.state == Task.State.READY) then
  repository.changeState(task.id, Task.State.WAITING_RESPONSE)
  return true
[D] if (task.state == Task.State.WAITING_RESPONSE && task.deadline.isBefore(now)) then
  repository.recordError(task.id, "Response deadline expired")
return false

```

[Editable PlantUML](process-flow.puml)

**source:** [``s30``](#source-4)

[Rendered SVG](process-flow.svg)

<a id="ordered-behavior"></a>
## Internal process explanation

<a id="block-receive_inputs"></a>1. **action:** Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\.  **Shared preparation:** [Constructor assignment of the repository field](#preparation-repository_assignment)
- **Glossary:** [Deadline check](#glossary-deadline) · [Promotion invocation](#glossary-invocation) · [Repository interface and field](#glossary-repository) · [Task input](#glossary-task)
<a id="block-choose_ready_branch"></a>2. **decision:** [The task state equals Task\.State\.READY](#predicate-state_is_ready)
- **Glossary:** [Promotion invocation](#glossary-invocation) · [State comparison and requested target](#glossary-states) · [Task input](#glossary-task)
  **When the condition holds:**
<a id="block-request_waiting_state"></a>   - **action:** Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id and the concrete Task\.State\.WAITING\_RESPONSE value as state\. The method does not assign task\.state locally\. A null repository or an exception from the call prevents the following return\.  **Shared preparation:** [Constructor assignment of the repository field](#preparation-repository_assignment)
   - **Glossary:** [Repository interface and field](#glossary-repository) · [State comparison and requested target](#glossary-states) · [Task input](#glossary-task)
<a id="block-return_true"></a>   - **return:** After changeState returns normally, return the literal true\. This reports the selected local branch; the retained interface supplies no evidence of persistence or transaction commitment\.
   - **Glossary:** [Promotion invocation](#glossary-invocation) · [Repository interface and field](#glossary-repository)
  **When the condition does not hold:**
<a id="block-choose_expiry_branch"></a>   - **decision:** [The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true](#predicate-waiting_and_deadline_check_true)
   - **Glossary:** [Deadline check](#glossary-deadline) · [Repository interface and field](#glossary-repository) · [State comparison and requested target](#glossary-states) · [Task input](#glossary-task)
     **When the condition holds:**
<a id="block-request_expiry_error"></a>      - **action:** Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id and the exact literal "Response deadline expired" as reason\. This branch contains no changeState call or local task\-field assignment\. If the repository call fails, the later return false is not reached\.  **Shared preparation:** [Constructor assignment of the repository field](#preparation-repository_assignment)
      - **Glossary:** [Deadline check](#glossary-deadline) · [Repository interface and field](#glossary-repository) · [Task input](#glossary-task)
     **When the condition does not hold:**
<a id="block-skip_error_recording"></a>      - **action:** When the conjunction is false, skip recordError and continue to the final return\. If its first operand was false, isBefore was not called; if its first operand was true, isBefore returned false\.
      - **Glossary:** [Deadline check](#glossary-deadline) · [Repository interface and field](#glossary-repository) · [State comparison and requested target](#glossary-states)
<a id="block-return_false"></a>   - **return:** Return the literal false after the second condition and any selected recordError call complete normally\. The value is false both when error recording was requested and when no repository operation was requested\.
   - **Glossary:** [Promotion invocation](#glossary-invocation) · [Repository interface and field](#glossary-repository)
<a id="semantic-predicates"></a>
### Condition meaning and source checks

<details>
<summary id="predicate-state_is_ready">The task state equals Task\.State\.READY</summary>

**Condition meaning:** The first branch is selected exactly when task\.state == Task\.State\.READY evaluates to true\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states)

**Exact source check:** task\.state == Task\.State\.READY<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states)

**Evaluation behavior:** Read task\.state and compare it with Task\.State\.READY\. A null task fails during the field read before either branch can run; a null state compares unequal\. True selects the changeState call followed by return true on normal completion, and the later deadline condition is not reached\. False proceeds to the second condition\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states) · [Repository interface and field](#glossary-repository) · [Deadline check](#glossary-deadline)

</details>

<details>
<summary id="predicate-waiting_and_deadline_check_true">The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true</summary>

**Condition meaning:** The error\-recording branch is selected exactly when both operands of task\.state == Task\.State\.WAITING\_RESPONSE &amp;&amp; task\.deadline\.isBefore\(now\) evaluate to true\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states) · [Deadline check](#glossary-deadline)

**Exact source check:** task\.state == Task\.State\.WAITING\_RESPONSE &amp;&amp; task\.deadline\.isBefore\(now\)<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states) · [Deadline check](#glossary-deadline)

**Evaluation behavior:** This condition is reached only after the READY comparison was false\. Evaluate task\.state == Task\.State\.WAITING\_RESPONSE first\. If false, including when the state read yields null, short\-circuit without reading task\.deadline or calling isBefore, then reach return false\. If true, read task\.deadline and call isBefore with now unchanged\. A null deadline fails at that call; now has no local null check, and its treatment inside isBefore is unknown\. A true helper result selects recordError; a false result skips it\. Both paths reach return false on normal completion\. An exception during evaluation or recordError prevents that return\.<details class="citations"><summary>Evidence (1)</summary><span class="citation-list">[s11](#evidence-13)</span></details>  **Glossary:** [Task input](#glossary-task) · [State comparison and requested target](#glossary-states) · [Deadline check](#glossary-deadline) · [Repository interface and field](#glossary-repository)

**Uncertainty:** The retained packet does not contain the isBefore body or the concrete recordError implementation\.

</details>

<a id="preparations"></a>
## Prepared values and checks

<a id="preparation-repository_assignment"></a>
### Constructor assignment of the repository field

The retained TaskStateTransitions constructor assigns its TaskRepository argument directly to the repository field\. This explains the field's declared initialization without establishing a constructor call or runtime receiver for the invocation under discussion\.<details class="citations"><summary>Evidence (2)</summary><span class="citation-list">[s30](#evidence-15), [d95](#evidence-11)</span></details>  **Glossary:** [Repository interface and field](#glossary-repository)

<a id="block-constructor_assign_repository"></a>1. **action:** this\.repository = repository copies the supplied reference into the field without transformation or a null check\. A null argument is therefore also assignable by this constructor\.
- **Glossary:** [Repository interface and field](#glossary-repository)

<details id="summary" class="full-summary">
<summary>Full summary</summary>

```text
promote takes a Task and an Instant. When task.state equals Task.State.READY, it requests Task.State.WAITING_RESPONSE for task.id and returns true after that call returns normally. Otherwise, it requests error recording only when task.state equals Task.State.WAITING_RESPONSE and task.deadline.isBefore(now) returns true; that path and all other normally completed paths return false. The retained source establishes local checks and repository call arguments, but does not establish persisted changes or external completion.
```

<details class="citations"><summary>Evidence (2)</summary><span class="citation-list">[s11](#evidence-13), [s19](#evidence-14)</span></details>

  **Glossary:** [Promotion invocation](#glossary-invocation) · [Task input](#glossary-task) · [State comparison and requested target](#glossary-states) · [Deadline check](#glossary-deadline) · [Repository interface and field](#glossary-repository)

</details>

<details class="technical-details"><summary>Technical reference</summary>

<a id="technical-reference"></a>
**Packet digest:** `sha256:1db6fa2df4d8d9c2e9498341a2984fe224b499d89d86412c5c3569a40d4956a2`

<details class="preparation-technical-reference"><summary>Preparation source and technical metadata</summary>

### Constructor assignment of the repository field

- **Source identity:** ``d83`` — d83 · class:example\.dispatch\.TaskStateTransitions · module:unnamed · :/main
- **`constructor_assign_repository`:**  (**From:** TaskStateTransitions\(TaskRepository repository\): repository · **To:** this\.repository)


</details>

### Evidence by block

| Meaning block | Block ID | Evidence |
|---|---|---|
| Summary | [``summary``](#summary) | [s11](#evidence-13), [s19](#evidence-14) |
| Promotion invocation | [``glossary-invocation``](#glossary-invocation) | [d76](#evidence-8), [s11](#evidence-13) |
| Task input | [``glossary-task``](#glossary-task) | [s11](#evidence-13) |
| State comparison and requested target | [``glossary-states``](#glossary-states) | [s11](#evidence-13) |
| Deadline check | [``glossary-deadline``](#glossary-deadline) | [s11](#evidence-13) |
| Repository interface and field | [``glossary-repository``](#glossary-repository) | [d95](#evidence-11), [s19](#evidence-14), [s30](#evidence-15) |
| The task state equals Task\.State\.READY · meaning | [``predicate-state_is_ready-meaning``](#predicate-state_is_ready) | [s11](#evidence-13) |
| The task state equals Task\.State\.READY · exact source check | [``predicate-state_is_ready-source-check``](#predicate-state_is_ready) | [s11](#evidence-13) |
| The task state equals Task\.State\.READY · evaluation | [``predicate-state_is_ready-evaluation``](#predicate-state_is_ready) | [s11](#evidence-13) |
| The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true · meaning | [``predicate-waiting_and_deadline_check_true-meaning``](#predicate-waiting_and_deadline_check_true) | [s11](#evidence-13) |
| The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true · exact source check | [``predicate-waiting_and_deadline_check_true-source-check``](#predicate-waiting_and_deadline_check_true) | [s11](#evidence-13) |
| The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true · evaluation | [``predicate-waiting_and_deadline_check_true-evaluation``](#predicate-waiting_and_deadline_check_true) | [s11](#evidence-13) |
| Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\. | [``step-receive_inputs``](#block-receive_inputs) | [s11](#evidence-13), [s30](#evidence-15) |
| Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. | [``step-choose_ready_branch``](#block-choose_ready_branch) | [s11](#evidence-13) |
| Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id and the concrete Task\.State\.WAITING\_RESPONSE value as state\. The method does not assign task\.state locally\. A null repository or an exception from the call prevents the following return\. | [``step-request_waiting_state``](#block-request_waiting_state) | [s11](#evidence-13), [s19](#evidence-14) |
| After changeState returns normally, return the literal true\. This reports the selected local branch; the retained interface supplies no evidence of persistence or transaction commitment\. | [``step-return_true``](#block-return_true) | [s11](#evidence-13), [s19](#evidence-14) |
| After the READY comparison is false, evaluate the WAITING\_RESPONSE comparison and, only when that comparison is true, task\.deadline\.isBefore\(now\)\. Use the resulting conjunction to choose whether to request error recording\. | [``step-choose_expiry_branch``](#block-choose_expiry_branch) | [s11](#evidence-13) |
| Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id and the exact literal "Response deadline expired" as reason\. This branch contains no changeState call or local task\-field assignment\. If the repository call fails, the later return false is not reached\. | [``step-request_expiry_error``](#block-request_expiry_error) | [s11](#evidence-13), [s19](#evidence-14) |
| When the conjunction is false, skip recordError and continue to the final return\. If its first operand was false, isBefore was not called; if its first operand was true, isBefore returned false\. | [``step-skip_error_recording``](#block-skip_error_recording) | [s11](#evidence-13) |
| Return the literal false after the second condition and any selected recordError call complete normally\. The value is false both when error recording was requested and when no repository operation was requested\. | [``step-return_false``](#block-return_false) | [s11](#evidence-13) |
| Constructor assignment of the repository field | [``preparation-repository_assignment``](#preparation-repository_assignment) | [s30](#evidence-15), [d95](#evidence-11) |
| this\.repository = repository copies the supplied reference into the field without transformation or a null check\. A null argument is therefore also assignable by this constructor\. | [``step-constructor_assign_repository``](#block-constructor_assign_repository) | [s30](#evidence-15) |

<a id="data-movement"></a>
## Data movement stated in steps

This view contains only explicit from/to values in authored steps; matching names do not create a link\.

| Context | From | To | Step | Evidence | Uncertainty |
|---|---|---|---|---|---|
| Operation | promote\(Task task, Instant now\): supplied task and now | task field reads and the now argument to task\.deadline\.isBefore\(now\) | [action](#block-receive_inputs) — Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\. | [s11](#evidence-13), [s30](#evidence-15) | — |
| Operation → When the condition holds: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. | task\.id and Task\.State\.WAITING\_RESPONSE | repository\.changeState\(String id, Task\.State state\) arguments | [action](#block-request_waiting_state) — Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id and the concrete Task\.State\.WAITING\_RESPONSE value as state\. The method does not assign task\.state locally\. A null repository or an exception from the call prevents the following return\. | [s11](#evidence-13), [s19](#evidence-14) | — |
| Operation → When the condition holds: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. | literal true | promote boolean return value | [return](#block-return_true) — After changeState returns normally, return the literal true\. This reports the selected local branch; the retained interface supplies no evidence of persistence or transaction commitment\. | [s11](#evidence-13), [s19](#evidence-14) | — |
| Operation → When the condition does not hold: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. | task\.state; conditionally task\.deadline and now | boolean result of task\.state == Task\.State\.WAITING\_RESPONSE &amp;&amp; task\.deadline\.isBefore\(now\) | [decision](#block-choose_expiry_branch) — After the READY comparison is false, evaluate the WAITING\_RESPONSE comparison and, only when that comparison is true, task\.deadline\.isBefore\(now\)\. Use the resulting conjunction to choose whether to request error recording\. | [s11](#evidence-13) | — |
| Operation → When the condition does not hold: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. → When the condition holds: After the READY comparison is false, evaluate the WAITING\_RESPONSE comparison and, only when that comparison is true, task\.deadline\.isBefore\(now\)\. Use the resulting conjunction to choose whether to request error recording\. | task\.id and literal "Response deadline expired" | repository\.recordError\(String id, String reason\) arguments | [action](#block-request_expiry_error) — Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id and the exact literal "Response deadline expired" as reason\. This branch contains no changeState call or local task\-field assignment\. If the repository call fails, the later return false is not reached\. | [s11](#evidence-13), [s19](#evidence-14) | — |
| Operation → When the condition does not hold: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\. | literal false | promote boolean return value | [return](#block-return_false) — Return the literal false after the second condition and any selected recordError call complete normally\. The value is false both when error recording was requested and when no repository operation was requested\. | [s11](#evidence-13) | — |
| Prepared values and checks: Constructor assignment of the repository field | TaskStateTransitions\(TaskRepository repository\): repository | this\.repository | [action](#block-constructor_assign_repository) — this\.repository = repository copies the supplied reference into the field without transformation or a null check\. A null argument is therefore also assignable by this constructor\. | [s30](#evidence-15) | — |

<a id="packet-facts"></a>
## Process types and fields

### Retained types

| Type | Kind | Owner | Superclass | Interfaces |
|---|---|---|---|---|
| TaskRepository ``class:example.dispatch.TaskRepository`` | INTERFACE | module:unnamed | — |  |
| TaskStateTransitions ``class:example.dispatch.TaskStateTransitions`` | CLASS | module:unnamed | class:java\.lang\.Object |  |

### Retained fields

| Owner | Field | Declared type | Modifiers | Annotations | Declaration tokens | Evidence |
|---|---|---|---|---|---|---|
| TaskStateTransitions \(class:example\.dispatch\.TaskStateTransitions\) | repository | Lexample/dispatch/TaskRepository; | FINAL, PRIVATE |  | `private final TaskRepository repository ;` | [d95](#evidence-11) |

<a id="cited-evidence"></a>
## Cited evidence

<a id="evidence-8"></a>- **d76** — exact internal process root  _(source: [`src/main/java/example/dispatch/TaskStateTransitions.java`:9–18](#source-1), [`src/main/java/example/dispatch/TaskStateTransitions.java`:9–18](#source-4))_
<a id="evidence-11"></a>- **d95** — retained field declaration  _(No retained source location is available for this evidence in the packet\.)_
<a id="evidence-13"></a>- **s11** — retained callable source  _(source: [`src/main/java/example/dispatch/TaskStateTransitions.java`:9–18](#source-1))_
<a id="evidence-14"></a>- **s19** — retained containing type source  _(source: [`src/main/java/example/dispatch/Model.java`:16–21](#source-2))_
<a id="evidence-15"></a>- **s30** — retained containing type source  _(source: [`src/main/java/example/dispatch/TaskStateTransitions.java`:5–19](#source-3), [`src/main/java/example/dispatch/TaskStateTransitions.java`:9–18](#source-4))_

<a id="source-locations"></a>
## Retained source locations

<a id="source-1"></a>- **`src/main/java/example/dispatch/TaskStateTransitions.java`:** 9–18 _(s11)_

<details>
<summary>View retained excerpt</summary>

```text
    public boolean promote(Task task, Instant now) {
        if (task.state == Task.State.READY) {
            repository.changeState(task.id, Task.State.WAITING_RESPONSE);
            return true;
        }
        if (task.state == Task.State.WAITING_RESPONSE && task.deadline.isBefore(now)) {
            repository.recordError(task.id, "Response deadline expired");
        }
        return false;
    }
```
</details>


**Process blocks citing this excerpt:** [decision: After the READY comparison is false, evaluate the WAITING\_RESPONSE comparison and, only when that comparison is true, task\.deadline\.isBefore\(now\)\. Use the resulting conjunction to choose whether to request error recording\.](#block-choose_expiry_branch) · [decision: Evaluate the READY comparison first\. Its true branch requests a state change and returns; only its false branch reaches the deadline decision\.](#block-choose_ready_branch) · [action: Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\.](#block-receive_inputs) · [action: Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id and the exact literal "Response deadline expired" as reason\. This branch contains no changeState call or local task\-field assignment\. If the repository call fails, the later return false is not reached\.](#block-request_expiry_error) · [action: Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id and the concrete Task\.State\.WAITING\_RESPONSE value as state\. The method does not assign task\.state locally\. A null repository or an exception from the call prevents the following return\.](#block-request_waiting_state) · [return: Return the literal false after the second condition and any selected recordError call complete normally\. The value is false both when error recording was requested and when no repository operation was requested\.](#block-return_false) · [return: After changeState returns normally, return the literal true\. This reports the selected local branch; the retained interface supplies no evidence of persistence or transaction commitment\.](#block-return_true) · [action: When the conjunction is false, skip recordError and continue to the final return\. If its first operand was false, isBefore was not called; if its first operand was true, isBefore returned false\.](#block-skip_error_recording) · [The task state equals Task\.State\.READY](#predicate-state_is_ready) · [The task state equals Task\.State\.WAITING\_RESPONSE and task\.deadline\.isBefore\(now\) returns true](#predicate-waiting_and_deadline_check_true) · [Summary](#summary)
<a id="source-2"></a>- **`src/main/java/example/dispatch/Model.java`:** 16–21 _(s19)_

<details>
<summary>View retained excerpt</summary>

```text
interface TaskRepository {
    List<Task> findPending();
    void changeState(String id, Task.State state);
    void deferUntil(String id, Instant until);
    void recordError(String id, String reason);
}
```
</details>


**Process blocks citing this excerpt:** [action: Call repository\.recordError\(task\.id, "Response deadline expired"\)\. Pass task\.id unchanged as id and the exact literal "Response deadline expired" as reason\. This branch contains no changeState call or local task\-field assignment\. If the repository call fails, the later return false is not reached\.](#block-request_expiry_error) · [action: Call repository\.changeState\(task\.id, Task\.State\.WAITING\_RESPONSE\)\. Pass task\.id unchanged as id and the concrete Task\.State\.WAITING\_RESPONSE value as state\. The method does not assign task\.state locally\. A null repository or an exception from the call prevents the following return\.](#block-request_waiting_state) · [return: After changeState returns normally, return the literal true\. This reports the selected local branch; the retained interface supplies no evidence of persistence or transaction commitment\.](#block-return_true) · [Summary](#summary)
<a id="source-3"></a>- **`src/main/java/example/dispatch/TaskStateTransitions.java`:** 5–19 _(s30)_

<details>
<summary>View retained excerpt</summary>

```text
public final class TaskStateTransitions {
    private final TaskRepository repository;
    public TaskStateTransitions(TaskRepository repository) { this.repository = repository; }

    public boolean promote(Task task, Instant now) {
        if (task.state == Task.State.READY) {
            repository.changeState(task.id, Task.State.WAITING_RESPONSE);
            return true;
        }
        if (task.state == Task.State.WAITING_RESPONSE && task.deadline.isBefore(now)) {
            repository.recordError(task.id, "Response deadline expired");
        }
        return false;
    }
}
```
</details>


**Process blocks citing this excerpt:** [action: this\.repository = repository copies the supplied reference into the field without transformation or a null check\. A null argument is therefore also assignable by this constructor\.](#block-constructor_assign_repository) · [action: Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\.](#block-receive_inputs) · [Constructor assignment of the repository field](#preparation-repository_assignment)
<a id="source-4"></a>- **`src/main/java/example/dispatch/TaskStateTransitions.java`:** 9–18 _(s30)_

<details>
<summary>View retained excerpt</summary>

```text
{
        if (task.state == Task.State.READY) {
            repository.changeState(task.id, Task.State.WAITING_RESPONSE);
            return true;
        }
        if (task.state == Task.State.WAITING_RESPONSE && task.deadline.isBefore(now)) {
            repository.recordError(task.id, "Response deadline expired");
        }
        return false;
    
```
</details>


**Process blocks citing this excerpt:** [action: this\.repository = repository copies the supplied reference into the field without transformation or a null check\. A null argument is therefore also assignable by this constructor\.](#block-constructor_assign_repository) · [action: Use the task and now parameters supplied to promote\. The method reads task fields directly, does not fetch a task or obtain the current time, and performs no explicit input validation\. Its repository calls use the repository field whose retained constructor assignment is explained separately\.](#block-receive_inputs) · [Constructor assignment of the repository field](#preparation-repository_assignment)

## Packet gaps and limits

### Captured gaps

- `\{"code":"CALL\_SITE\_SOURCE\_NOT\_CONTAINED\_IN\_METHOD\_BODY","count":3,"examples":\[\{"factId":"dispatch:call\-relation:b43b718a16670bd6a71e2038","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z"\},\{"factId":"dispatch:call\-relation:bfafbdfb074487a2f33e3693","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z"\},\{"factId":"dispatch:call\-relation:f48d71f7f45e315eec560431","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z"\}\]\}`
- `\{"code":"CALL\_TARGET\_BODY\_NOT\_CAPTURED","count":1,"examples":\[\{"from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main","target":"method:class:java\.time\.Instant\#isBefore\(Ljava/time/Instant;\)Z"\}\]\}`
- `\{"code":"DIRECT\_DECLARED\_TYPES\_ONLY","count":1,"examples":\[null\]\}`
- `\{"code":"METHOD\_BODY\_PARSE\_UNAVAILABLE","count":2,"examples":\[\{"symbol":"method:class:example\.dispatch\.TaskRepository\#changeState\(Ljava/lang/String;Lexample/dispatch/Task$State;\)V"\},\{"symbol":"method:class:example\.dispatch\.TaskRepository\#recordError\(Ljava/lang/String;Ljava/lang/String;\)V"\}\]\}`
- `\{"code":"PROCESS\_QUALIFIED\_FIELD\_NOT\_CAPTURED","count":1,"examples":\[\{"candidateCount":0,"field":"State","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main","typeReference":"d110"\}\]\}`
- `\{"code":"PROCESS\_QUALIFIED\_SOURCE\_REFERENCE\_UNSUPPORTED","count":3,"examples":\[\{"from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main","spelling":"State\.READY"\},\{"from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main","spelling":"State\.WAITING\_RESPONSE"\},\{"from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main","spelling":"deadline\.isBefore"\}\]\}`
- `\{"code":"PROCESS\_TYPE\_DECLARATION\_NOT\_CAPTURED","count":1,"examples":\[\{"candidateCount":0,"identity":"class:java\.lang\.Object","scope":":/main"\}\]\}`
- `\{"code":"SOURCE\_FIELD\_RECEIVER\_BODY\_UNAVAILABLE","count":2,"examples":\[\{"fieldReference":"d95","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main"\},\{"fieldReference":"d95","from":"method:class:example\.dispatch\.TaskStateTransitions\#promote\(Lexample/dispatch/Task;Ljava/time/Instant;\)Z","scope":":/main"\}\]\}`
- `\{"code":"SOURCE\_REFERENCE\_CANDIDATE\_AUTHORITY","count":1,"examples":\[null\]\}`
- `\{"code":"STRUCTURAL\_DATAFLOW\_NOT\_AVAILABLE","count":1,"examples":\[null\]\}`

### Interpretation limits

- `Retained FLOW and compiler relations preserve provider evidence, not runtime execution or statement timing\.`
- `SOURCE\_REFERENCE\_CANDIDATE edges identify same\-scope source\-context candidates only; they do not establish executed calls, receiver identity, inheritance dispatch, or order\.`
- `Only the supplied root, selected declarations, retained method source, fields, types, and listed frontiers support claims; missing context remains unknown\.`
- `A field type or superclass declaration does not establish the runtime object or selected override\.`

### Coverage boundaries

- `EXCEPTION\_FLOW\_REQUIRES\_SOURCE\_REVIEW`
- `SHORT\_CIRCUIT\_FLOW\_REQUIRES\_SOURCE\_REVIEW`

<details>
<summary>Full evidence inventory</summary>

<a id="evidence-1"></a>- **c1** — saved capture coverage
<a id="evidence-2"></a>- **d126** — frozen saved process intention, not source evidence
<a id="evidence-3"></a>- **d13** — retained callsite evidence
<a id="evidence-4"></a>- **d14** — retained callsite evidence
<a id="evidence-5"></a>- **d18** — retained callsite evidence
<a id="evidence-6"></a>- **d34** — retained callsite evidence
<a id="evidence-7"></a>- **d37** — retained callsite evidence
<a id="evidence-9"></a>- **d79** — retained containing type declaration
<a id="evidence-10"></a>- **d83** — retained containing type declaration
<a id="evidence-12"></a>- **p1** — selected internal process context

</details>
</details>
