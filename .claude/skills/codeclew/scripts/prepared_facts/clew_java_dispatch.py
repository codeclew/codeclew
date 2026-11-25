"""Select supported typed Java handler tables from exact retained dispatch wiring."""

from collections import defaultdict
from pathlib import Path
import re
import runpy
import subprocess
import time


CHAIN = runpy.run_path(str(Path(__file__).with_name("clew_java_chain.py")))
TRAM = "io.eventuate.tram.events.subscriber."
BUILDER = TRAM + "DomainEventHandlersBuilder"
TABLE = TRAM + "DomainEventHandlers"
FACTORY = TRAM + "DomainEventDispatcherFactory"
DISPATCHER = TRAM + "DomainEventDispatcher"
ENVELOPE = TRAM + "DomainEventEnvelope"
CONSUMER = "io.eventuate.tram.messaging.consumer.MessageConsumer"
NAME_MAPPING = "io.eventuate.tram.events.common.DomainEventNameMapping"
BEAN = "org.springframework.context.annotation.Bean"
COMPONENT = "org.springframework.stereotype.Component"
ADAPTER = "EVENTUATE_TRAM_DOMAIN_EVENT"
KIND = "JAVA_TYPED_DISPATCH"
CHANNEL = re.compile(r"[A-Za-z0-9._$-]{1,249}\Z")
QUALIFIED = re.compile(r"[A-Za-z_$][A-Za-z_$0-9]*(?:\.[A-Za-z_$][A-Za-z_$0-9]*)+\Z")
MAX_FACTORIES = 12
MAX_CALLBACKS = 24
MAX_CHAIN_STEPS = 24


class Capture:
    """Expose the runner's already verified complete files to its Java unit parser."""

    def __init__(self, files, deadline):
        self.files, self.deadline = files, deadline

    def paths(self):
        return self.files

    def read(self, file):
        return self.files[file]["text"].encode() if file in self.files else None

    def remaining(self, seconds):
        return max(0, min(seconds, self.deadline - time.monotonic()))


def kind(node, expected):
    return node is not None and node[0] == expected


def name(node):
    if kind(node, "id"):
        return node[3]
    if kind(node, "member") and (qualifier := name(node[3])):
        return qualifier + "." + node[4]
    return ""


def invocation(node):
    if not kind(node, "call"):
        return None
    select, args = node[3:5]
    if kind(select, "member"):
        return select[3], select[4], args
    if kind(select, "id"):
        return None, select[3], args
    return None


def literal(node):
    return node[4] if kind(node, "literal") and len(node) == 5 else None


def returned(declaration):
    if not kind(declaration[8], "block"):
        return None
    statements = declaration[8][3]
    if not statements or not kind(statements[-1], "return"):
        return None
    aliases = {}
    for statement in statements[:-1]:
        if not kind(statement, "let") or statement[3] in aliases or statement[4] is None:
            return None
        aliases[statement[3]] = statement
    return statements[-1][3], aliases


def resolve_alias(node, aliases):
    proof, seen = [], set()
    while kind(node, "id") and (key := node[3]) in aliases and aliases[key][1] < node[1]:
        if key in seen:
            return None, ()
        seen.add(key)
        binding = aliases[key]
        proof.append(binding)
        node = binding[4]
    return node, tuple(proof)


def site(owner, node):
    return {"file": owner.path, "start": node[1], "end": node[2]}


def imports(owner, written, qualified):
    return [{"file": item.path, "start": item.start, "end": item.end}
            for item in owner.import_proof(written, qualified)]


def annotation(units, owner, nodes, qualified):
    matches = [node for node in nodes if units.qualify(owner, node[3]) == qualified]
    return matches[0] if len(matches) == 1 else None


class Dispatch:
    def __init__(self, units):
        self.units = units

    def type_binding(self, owner, method, receiver):
        label = name(receiver)
        if label == "this":
            return owner.qualified, None
        if label and "." not in label:
            parameters = [parameter for parameter in method[6] if parameter[3] == label]
            if parameters:
                parameter = parameters[0]
                return self.units.qualify(owner, parameter[4]), parameter
        field = label.removeprefix("this.")
        fields = [member for member in owner.fields(field) if "FINAL" in member[5]] if field else []
        if len(fields) == 1:
            return self.units.qualify(owner, fields[0][4]), fields[0]
        return "", None

    def constructed(self, owner, value, qualified):
        if not kind(value, "new") or not kind(value[4], "type") or value[3] is not None:
            return False
        if self.units.qualify(owner, value[4][3]) != qualified:
            return False
        selected = self.units.lookup(owner, qualified)
        if not selected:
            return False
        constructors = selected.methods("<init>")
        arity = len(value[5])
        return sum(len(method[6]) == arity for method in constructors) == 1 if constructors else arity == 0

    def receiver(self, owner, method, value):
        if name(value) == "this" and "STATIC" in method[5]:
            return None, ()
        if value is None or name(value) == "this":
            return owner, ()
        if kind(value, "new") and kind(value[4], "type"):
            target = self.units.qualify(owner, value[4][3])
            if self.constructed(owner, value, target):
                return self.units.lookup(owner, target), (site(owner, value),)
            return None, ()
        target, declaration = self.type_binding(owner, method, value)
        if not target or not declaration or not (selected := self.units.lookup(owner, target)):
            return None, ()
        if kind(declaration, "field") and self.constructed(owner, declaration[6], target):
            return selected, (site(owner, declaration),)
        beans = [candidate for candidate in owner.methods() if self.units.qualify(owner, candidate[4]) == target
                 and annotation(self.units, owner, candidate[7], BEAN)]
        if beans:
            if len(beans) == 1 and (result := returned(beans[0])):
                value, _ = resolve_alias(*result)
                if self.constructed(owner, value, target):
                    return selected, (site(owner, declaration), site(owner, beans[0]))
            return None, ()
        if configured := annotation(self.units, selected, selected.node[6], COMPONENT):
            return selected, (site(owner, declaration), site(selected, configured))
        return None, ()

    def class_literal(self, owner, node):
        if not kind(node, "member") or node[4] != "class" or not (written := name(node[3])):
            return None
        qualified = self.units.qualify(owner, written)
        if not QUALIFIED.fullmatch(qualified):
            return None
        if "." not in written and not owner.import_proof(written, qualified):
            if not self.units.lookup(owner, qualified):
                return None
        return qualified

    def channel(self, owner, argument):
        value = literal(argument)
        if not isinstance(value, str) and (call := invocation(argument)):
            receiver, member, args = call
            if member == "getName" and not args:
                value = self.class_literal(owner, receiver)
        return value if isinstance(value, str) and CHANNEL.fullmatch(value) and value not in {".", ".."} else None

    def callback(self, owner, method, argument, message):
        if kind(argument, "methodref") and argument[5] == "INVOKE":
            receiver, member = argument[3:5]
        elif kind(argument, "deferred") and len(argument[3]) == 1:
            body = argument[4]
            if kind(body, "block") and len(body[3]) == 1 and kind(body[3][0], "expr"):
                body = body[3][0][3]
            call = invocation(body)
            if not call or len(call[2]) != 1 or name(call[2][0]) != argument[3][0]:
                return None
            receiver, member, _ = call
            if receiver is not None and name(receiver) == argument[3][0]:
                return None
        else:
            return None
        target, proof = self.receiver(owner, method, receiver)
        if not target:
            return None
        candidates = [candidate for candidate in target.methods(member) if len(candidate[6]) == 1]
        if len(candidates) != 1 or not candidates[0][8] or candidates[0][4] != "void":
            return None
        callback = candidates[0]
        if receiver is None and "STATIC" in method[5] and "STATIC" not in callback[5]:
            return None
        parameter = callback[6][0]
        if (self.units.qualify(target, parameter[4]) != ENVELOPE or len(parameter[6]) != 1
                or self.units.qualify(target, parameter[6][0]) != message):
            return None
        return target, callback, (*proof, *imports(target, parameter[4], ENVELOPE),
                                  *imports(target, parameter[6][0], message))

    def table(self, owner, method):
        if self.units.qualify(owner, method[4]) != TABLE or not (result := returned(method)):
            return []
        current, aliases = result
        stages, origin = [], []
        builder = None
        variables = {*aliases, *(item[3] for item in method[6]), *(field[3] for field in owner.fields())}
        for _ in range(MAX_CHAIN_STEPS):
            current, bound = resolve_alias(current, aliases)
            origin.extend(bound)
            call = invocation(current)
            if not call:
                return []
            receiver, member, arguments = call
            stages.append((current, member, arguments))
            written = name(receiver)
            if (member == "forAggregateType" and self.units.qualify(owner, written) == BUILDER
                    and written.split(".", 1)[0] not in variables):
                builder = written
                break
            current = receiver
        else:
            return []
        if stages[0][1:] != ("build", []) or len(stages[-1][2]) != 1:
            return []
        channel_site = stages[-1][2][0]
        if not (channel := self.channel(owner, channel_site)):
            return []
        callbacks = defaultdict(list)
        for step, member, args in reversed(stages[1:-1]):
            if member == "andForAggregateType" and len(args) == 1:
                channel_site = args[0]
                channel = self.channel(owner, channel_site)
                if not channel:
                    return []
                continue
            if member != "onEvent" or len(args) != 2:
                return []
            message = self.class_literal(owner, args[0])
            if not message:
                return []
            callbacks[channel, message].append((step, channel_site, args))
        output = []
        for (channel, message), group in callbacks.items():
            if len(group) != 1:
                continue
            selected, channel_site, args = group[0]
            if not (callback := self.callback(owner, method, args[1], message)):
                continue
            target, declaration, binding = callback
            supports = [*(site(owner, node) for node in (*origin, stages[-1][0], channel_site, selected, method)),
                        *imports(owner, builder, BUILDER), *imports(owner, name(args[0][3]), message), *binding]
            output.append({"kind": KIND, "name": f"{target.qualified}.{declaration[3]}",
                           "callback": site(target, declaration), "adapter": ADAPTER, "channel": channel, "eventType": message,
                           "assemblySources": supports})
        return output

    def factory(self, owner, factory):
        bean = annotation(self.units, owner, factory[7], BEAN)
        if not bean or self.units.qualify(owner, factory[4]) != DISPATCHER or not (result := returned(factory)):
            return []
        value, aliases = result
        value, alias_proof = resolve_alias(value, aliases)
        call = invocation(value)
        provider_proof = []
        receiver_proof, table_proof = (), ()
        if call and call[1] == "make" and len(call[2]) == 2:
            receiver, receiver_proof = resolve_alias(call[0], aliases)
            selected, factory_binding = self.type_binding(owner, factory, receiver)
            if selected != FACTORY:
                return []
            provider_proof.extend([site(owner, factory_binding), *imports(owner, factory_binding[4], FACTORY)])
            operand = call[2][1]
        elif (kind(value, "new") and kind(value[4], "type") and self.units.qualify(owner, value[4][3]) == DISPATCHER
              and len(value[5]) == 4 and any(kind(option, "set") and name(option[3]) == "initMethod" and
                                            literal(option[4]) == "initialize" for option in bean[4])):
            for argument, expected in zip(value[5][2:], (CONSUMER, NAME_MAPPING)):
                receiver, bound = resolve_alias(argument, aliases)
                selected, declaration = self.type_binding(owner, factory, receiver)
                if selected != expected:
                    return []
                provider_proof.extend([*(site(owner, node) for node in (*bound, declaration)),
                                       *imports(owner, declaration[4], expected)])
            operand = value[5][1]
        else:
            return []
        operand, operand_proof = resolve_alias(operand, aliases)
        table = invocation(operand)
        if not table or table[2]:
            return []
        receiver, table_proof = resolve_alias(table[0], aliases)
        target, activation = self.receiver(owner, factory, receiver)
        if not target:
            return []
        methods = [method for method in target.methods(table[1]) if not method[6]]
        if len(methods) != 1 or receiver is None and "STATIC" in factory[5] and "STATIC" not in methods[0][5]:
            return []
        proof = [*(site(owner, item) for item in (bean, operand, *alias_proof, *operand_proof, *receiver_proof, *table_proof)),
                 *provider_proof, *imports(owner, bean[3], BEAN)]
        return [{**callback, "registration": site(owner, value),
                 "assemblySources": [*proof, *activation, *callback["assemblySources"]]}
                for callback in self.table(target, methods[0])]


def discover(files, *, deadline):
    roots = [file for file, row in files.items() if "DomainEventDispatcher" in row["text"] and "@" in row["text"]]
    if not roots:
        return []
    found = []
    try:
        with CHAIN["open_units"](Capture(files, deadline), dispatch=True) as units:
            if units is None:
                return []
            provider = {BUILDER, TABLE, FACTORY, DISPATCHER, ENVELOPE}
            shadow_files = [file for file in files if any(Path(file).stem == type_.rsplit(".", 1)[1] for type_ in provider)]
            if any(owner.qualified in provider for file in shadow_files[:MAX_FACTORIES] for owner in units.read(file)):
                return []
            scanner = Dispatch(units)
            for file in roots[:MAX_FACTORIES]:
                if time.monotonic() >= deadline or len(found) >= MAX_CALLBACKS:
                    break
                for owner in units.read(file):
                    for factory in owner.methods():
                        found.extend(scanner.factory(owner, factory))
    except (OSError, subprocess.SubprocessError, ValueError):
        return found[:MAX_CALLBACKS]
    return found[:MAX_CALLBACKS]
