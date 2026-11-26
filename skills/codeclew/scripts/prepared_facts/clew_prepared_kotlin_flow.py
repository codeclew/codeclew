"""Follow sparse source continuations for each retained Kotlin decision."""

from dataclasses import dataclass, replace
from itertools import islice
import re


PROTECTED = {"TRY_BODY", "CATCH_BODY", "FINALLY_BODY"}


@dataclass(frozen=True)
class Action:
    note: object
    region: str
    choices: tuple = ()
    continues: bool = True
    guard_site: bool = False


@dataclass(frozen=True)
class Shape:
    actions: tuple = ()
    resumes: bool = True


def unique(actions):
    by_site = {}
    for action in actions:
        by_site.setdefault(action.note.site, action)
    return tuple(by_site.values())


def join(items):
    return ", ".join(items[:-1]) + " and " + items[-1] if len(items) > 1 else items[0]


def condition_code(text):
    match = re.fullmatch(r'`([^`]+)`(?: is (true|false|null|non-null)| yields a non-null value)?', text)
    if not match:
        return None
    expression, predicate = match.groups()
    if predicate == 'true' or text.endswith('`'):
        return expression
    if predicate == 'false':
        return f'!({expression})'
    return f"({expression}) {'==' if predicate == 'null' else '!='} null"


def action_code(note):
    spans = re.findall(r'`([^`\n]+)`', note.text)
    return spans[0] if spans else ''


def path_case(body, site, condition, actions, meaning, **fields):
    return dict(group=f'{body.ref}:{site.start}:{site.end}', scope=body.function.name,
                condition=condition, actions=actions, meaning=meaning, **fields)


def chunks(items, limit=6):
    source = iter(items)
    while part := tuple(islice(source, limit)):
        yield part


class Controls:
    """Join each explicit arm and abrupt site to its own lexical successors."""

    def __init__(self, body, scope, proof, throw_future=(), *, check_paths=False):
        self.body = body
        self.scope = scope
        self.proof = proof
        self.shapes = {}
        self.flow_budget = 480
        self.throw_future = throw_future
        self.check_paths = check_paths

    def action(self, statement, region):
        if note := self.body.action(statement):
            return (Action(note, region),)
        return ()

    def alternatives(self, statement):
        return self.body.decision(statement)

    def try_shape(self, statement):
        protected = self.shape(statement.body, "TRY_BODY")
        cleanup = self.shape(statement.cleanup, "FINALLY_BODY")
        # A matching catch is a possible way to the end of the protected region.
        resumes = protected.resumes or any(self.shape(suite, "CATCH_BODY").resumes
                                           for _, _, suite in statement.handlers)
        return Shape((*protected.actions, *cleanup.actions), resumes and cleanup.resumes)

    def statement_shape(self, statement, region):
        key = (statement.site, region)
        if key in self.shapes:
            return self.shapes[key]
        if self.flow_budget <= 0:
            return Shape()
        self.flow_budget -= 1
        if nested := self.body.expression_control(statement):
            shape = self.statement_shape(nested, region)
            resumes = shape.resumes and statement.kind != "return"
            result = Shape((*shape.actions, *self.action(statement, region)), resumes)
        elif statement.kind == "try":
            result = self.try_shape(statement)
        elif branches := self.alternatives(statement):
            guard, arms = branches
            shapes = [(arm, self.shape(suite, region)) for arm, suite in arms]
            actions = tuple(replace(action, choices=((guard.site, arm), *action.choices),
                                    continues=action.continues and shape.resumes)
                            for arm, shape in shapes for action in shape.actions)
            result = Shape(actions, any(shape.resumes for _, shape in shapes))
        elif statement.kind in {"for", "while"}:
            # These source operations require entry to an iteration.
            result = Shape(self.shape(statement.body, "ITERATION").actions)
        else:
            guard = self.body.hard_guard(statement)
            terminal = bool(guard and guard.verified and not guard.true)
            resumes = statement.kind not in {"return", "throw", "break", "continue"} and not terminal
            actions = self.action(statement, region)
            if guard:
                actions = tuple(replace(action, guard_site=True) for action in actions)
            result = Shape(actions, resumes)
        self.shapes[key] = result
        return result

    def shape(self, suite, region):
        actions = []
        for statement in suite:
            shape = self.statement_shape(statement, region)
            actions.extend(shape.actions)
            if not shape.resumes:
                return Shape(tuple(actions), False)
        return Shape(tuple(actions))

    def scope_for(self, region):
        return {"TRY_BODY": "the try body in ", "CATCH_BODY": "a matching catch in ",
                "FINALLY_BODY": "finally in "}.get(region, "") + self.scope

    def skip_groups(self, actions, region):
        pending = unique(actions)
        buckets = {}
        for action in pending:
            if region not in PROTECTED and action.region == "TRY_BODY":
                key = "TRY_BODY"
            elif region != "ITERATION" and action.region == "ITERATION":
                key = "ITERATION"
            else:
                key = region
            buckets.setdefault(key, []).append(action.note)
        for area, notes in buckets.items():
            suffix = ({"TRY_BODY": " in the later try body", "ITERATION": " in the later loop body"}.get(area, "")
                      if area != region else "")
            for part in chunks(notes):
                yield part, suffix

    def emit_arm(self, guard, arm, arms, region, surrounding):
        # Nested sites belong to their own decisions, with both guards cited.
        selected = [action for action in self.shape(arms[arm], region).actions if not action.choices]
        bypass = self.shape(arms.get(not arm, ()), region).actions
        if not selected and not bypass:
            return
        guards = (*surrounding, (guard, arm))
        predicate = join(self.body.guard_labels(guards))
        condition = [note for note, _ in guards]
        prefix = f"If execution reaches this decision in {self.scope_for(region)} and {predicate}, "
        notes = [item.note for item in unique(selected)]
        for part in chunks(notes):
            self.body.projector.add("GUARDED_EFFECTS", prefix + "the selected arm contains the source sites for "
                                    + join([note.text for note in part]) + ".",
                                    [*condition, *part], self.proof,
                                    scope="SOURCE_DECISION", region=region, condition=predicate)
        for part, suffix in self.skip_groups(bypass, region):
            self.body.projector.add("GUARDED_EFFECTS", prefix + "the opposite arm is skipped, including "
                                    + join([note.text for note in part]) + suffix + ".",
                                    [*condition, *part], self.proof,
                                    scope="SOURCE_DECISION", region=region, condition=predicate)

    def exit(self, statement, guards, subsequent, alternatives, region, callback, catches):
        terminal = self.body.note(statement.site)
        if terminal is None:
            return
        words = self.body.words(statement)
        is_labeled = statement.kind == "return" and words[:1] == ("@",)
        if is_labeled and (not callback or words[1:2] != (callback,)):
            return
        if callback and statement.kind == "return" and not is_labeled:
            return  # Inline and non-local destinations need separate evidence.
        if not guards and statement.kind not in {"throw", "break", "continue"}:
            return
        if statement.kind == "throw" and catches:
            alternatives = ()
        conditions = join(self.body.guard_labels(guards)) if guards else ""
        branch = f" and {conditions}" if conditions else ""
        control = {"return": "the callback returns" if is_labeled else "the callable returns",
                   "throw": "the source throws", "break": "the loop exits",
                   "continue": "the current iteration ends"}[statement.kind]
        operand = " once its operand finishes evaluating" if statement.kind in {"return", "throw"} and words else ""
        description = f"If execution reaches {terminal.text} in {self.scope_for(region)}{branch}, {control}{operand}."
        if statement.kind == "throw" and catches:
            description += " Continuation depends on matching a local catch."
        if not guards and not subsequent:
            return
        proofs = [*(condition for condition, _ in guards), terminal]
        skip_set = (*subsequent, *alternatives) if guards else subsequent
        groups = list(self.skip_groups(skip_set, region))
        if not groups:
            groups = [((), "")]
        for part, suffix in groups:
            tail = " This path bypasses " + join([note.text for note in part]) + suffix + "." if part else ""
            self.body.projector.add("EARLY_EXIT", description + tail, [*proofs, *part], self.proof,
                                    scope="LEXICAL_EXIT", region=region)

    @staticmethod
    def compact_actions(actions, limit=5):
        entries = unique(actions)
        if len(entries) > limit:
            head = min(2, limit - 1)
            entries = (*entries[:head], *entries[-(limit - head):])
        return tuple(action.note for action in entries)

    def compatible_continuation(self, actions, guards):
        selected = {guard.site: (guard, holds) for guard, holds in guards}
        result = []
        for action in actions:
            if not action.continues or any(site in selected and selected[site][1] != arm for site, arm in action.choices):
                continue
            choices = {site: (self.body.note(site), arm) for site, arm in action.choices}
            candidate = selected | {site: choice for site, choice in choices.items() if choice[0]}
            if self.body.feasible_guards(tuple(candidate.values())):
                selected = candidate
                result.append(action)
        return result

    def default_choice(self, statement):
        tokens = statement.expression
        if statement.kind != 'expression' or len(tokens) < 6 or tokens[0].text != 'val':
            return
        unit = self.body.unit
        start = unit.token_indices[tokens[0].start]
        top = unit.container[start]
        operators = [(i, token.text) for i, token in enumerate(tokens)
                     if unit.container[start + i] == top and token.text in {'=', '?:'}]
        if len(operators) != 2 or [word for _, word in operators] != ['=', '?:']:
            return
        left, pivot = (i for i, _ in operators)
        if pivot <= left + 1 or pivot + 1 == len(tokens) or tokens[pivot + 1].text in {'throw', 'return'}:
            return
        source = self.body.note(unit.site(start + left + 1, start + pivot))
        default = self.body.note(unit.site(start + pivot + 1, start + len(tokens)))
        if not source or not default:
            return
        lhs, rhs = action_code(source), action_code(default)
        if not lhs or not rhs or lhs == rhs:
            return
        self.body.projector.add('GUARDED_EFFECTS', f'`{tokens[1].text}` keeps {source.text} when it is non-null '
                                f'and evaluates {default.text} when the left expression yields null.',
                                [source, default], self.proof, scope='SOURCE_DECISION',
                                _conditionCases=[
                                    path_case(self.body, statement.site, f'({lhs}) != null', [lhs], 'use the left value', stepKind='note'),
                                    path_case(self.body, statement.site, f'({lhs}) == null', [rhs], 'evaluate the fallback', stepKind='note')])

    def emit_hard_exit(self, hard, guards, downstream, region, catches, returns):
        scope = self.scope_for(region)
        conditions = self.body.guard_labels(guards)
        if hard.bindings:
            types = join([f"`kotlin.{name}`" for name in hard.bindings])
            conditions.append(f"the cited selectors resolve to {types}")
        if hard.false:
            conditions.append(hard.false)
        cause = " and " + join(conditions) if conditions else ""
        base = (f"If execution reaches this guard in {scope}{cause}, it evaluates {hard.failure.text} and throws "
                f"`{hard.exception}` if its arguments finish evaluating.")
        if catches:
            base += " A matching local catch may handle the failure."
        sources = [*(note for note, _ in guards), *hard.proof]
        authority = "QUALIFIED_OR_SELECTED_STDLIB" if hard.verified else "STDLIB_BINDING_OPEN"
        route = self.compatible_continuation(downstream, guards) if hard.true and not returns else ()
        effects = [action for action in route if not action.guard_site]
        passing = self.compact_actions(effects or route)
        compatible = {action.note.site for action in route}
        bypassed = self.compact_actions([action for action in downstream if action.note.site not in compatible],
                                       limit=2 if passing else 5)
        if passing:
            sequence = " → ".join(note.text for note in passing)
            base += (f" If the guard is reached and {hard.true}, these downstream sites can follow in "
                     f"source order, where reached: {sequence}. The throw bypasses this continuation.")
        elif hard.true and not returns:
            base += f" If the guard is reached and {hard.true}, it permits normal continuation."
        if bypassed:
            sites = join([note.text for note in bypassed])
            base += f" The throw bypasses later sites including {sites}."
        outside = self.compact_actions(self.throw_future, limit=3) if not catches else ()
        if outside:
            sites = join([note.text for note in outside])
            base += f" If the throw escapes the loop, it bypasses after-loop sites including {sites}."
        cases = []
        if self.check_paths and hard.verified and not (guards or catches or returns) and passing:
            positive, negative = condition_code(hard.true), condition_code(hard.false)
            if positive and negative and (effect := action_code(passing[-1])):
                cases = [path_case(self.body, hard.failure.site, positive, [effect], 'continue to this action'),
                         path_case(self.body, hard.failure.site, negative, [hard.exception, effect],
                                   'throw the exception and skip the action', stepKind='note', skip=[effect])]
        self.body.projector.add("HARD_EXIT", base, [*sources, *passing, *bypassed, *outside], self.proof,
                                identity=(self.body.ref, hard.failure.site),
                                scope="STDLIB_HARD_EXIT", region=region, authority=authority, _conditionCases=cases)

    def visit(self, suite, region, guards=(), future=(), alternatives=(), callback=None, catches=False):
        if not suite or self.flow_budget <= 0:
            return
        shapes = [self.statement_shape(statement, region) for statement in suite]
        suffixes = [future]
        for shape in reversed(shapes):
            suffixes.append((*shape.actions, *suffixes[-1]) if shape.resumes else shape.actions)
        for index, statement in enumerate(suite):
            subsequent = suffixes[len(suite) - index - 1]
            if nested := self.body.expression_control(statement):
                self.visit((nested,), region, guards, () if statement.kind == "return" else subsequent,
                           alternatives, callback, catches)
                if statement.kind == "return" and self.statement_shape(nested, region).resumes:
                    self.exit(statement, guards, subsequent, alternatives, region, callback, catches)
            elif hard := self.body.hard_guard(statement):
                self.emit_hard_exit(hard, guards, subsequent, region, catches, statement.kind == "return")
            elif branches := self.alternatives(statement):
                condition, arms = branches
                by_arm = dict(arms)
                for arm, selected in arms:
                    branch_guards = (*guards, (condition, arm))
                    if not self.body.feasible_guards(branch_guards):
                        continue
                    self.emit_arm(condition, arm, by_arm, region, guards)
                    other = self.shape(by_arm.get(not arm, ()), region)
                    reachable_after = subsequent if other.resumes or self.shape(selected, region).resumes else ()
                    self.visit(selected, region, branch_guards, reachable_after,
                               (*other.actions, *alternatives), callback, catches)
            elif statement.kind == "try":
                outer = () if statement.handlers else subsequent
                self.visit(statement.body, "TRY_BODY", guards, outer, alternatives, callback,
                           catches or bool(statement.handlers))
                for _, _, handler in statement.handlers:
                    self.visit(handler, "CATCH_BODY", guards, subsequent, alternatives, callback, catches)
                self.visit(statement.cleanup, "FINALLY_BODY", guards, subsequent, alternatives, callback, catches)
            elif statement.kind in {"for", "while"}:
                self.body.iteration(statement, callback, catches, self.scope_for(region), subsequent)
            elif statement.kind in {"return", "throw", "break", "continue"}:
                self.exit(statement, guards, subsequent, alternatives, region, callback, catches)
            elif self.check_paths and region == "CALLABLE" and not guards:
                self.default_choice(statement)
            if not shapes[index].resumes:
                break
