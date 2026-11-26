"""Project bounded Java submissions through exact typed Eventuate protocol operands."""

from dataclasses import dataclass, replace
from hashlib import sha256
from itertools import islice, zip_longest
import re

ADAPTER = "EVENTUATE_TRAM_DOMAIN_EVENT"
NAMESPACE = "io.eventuate.tram.events.publisher."
DIRECT = NAMESPACE + "DomainEventPublisher"
AGGREGATE = NAMESPACE + "AbstractDomainEventPublisherForAggregateImpl"
PORT = NAMESPACE + "DomainEventPublisherForAggregate"
BUNDLE = NAMESPACE + "ResultWithTypedEvents"
EVENT = "io.eventuate.tram.events.common.DomainEvent"
ENVELOPE = "io.eventuate.tram.events.subscriber.DomainEventEnvelope"
BEAN = "org.springframework.context.annotation.Bean"
CLASS = re.compile(r"[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+", re.ASCII)
CHANNEL = re.compile(r"[A-Za-z0-9._$-]{1,249}", re.ASCII)
CONSTRUCTOR = re.compile(r"method:class:([^#]+)#<init>\(")
DOMAIN_EVENT = "L" + EVENT.replace(".", "/") + ";"
DIRECT_CALLS = {
    "publish(Ljava/lang/String;Ljava/lang/Object;Ljava/util/List;)V": ("STRING", 2),
    "publish(Ljava/lang/Class;Ljava/lang/Object;Ljava/util/List;)V": ("CLASS", 2),
    "publish(Ljava/lang/String;Ljava/lang/Object;Ljava/util/Map;Ljava/util/List;)V": ("STRING", 3),
}
AGGREGATE_CALLS = {
    f"publishById(Ljava/lang/Object;{DOMAIN_EVENT})V": ("SINGLE", 1),
    f"publish(Ljava/lang/Object;{DOMAIN_EVENT})V": ("SINGLE", 1),
    "publish(Ljava/lang/Object;Ljava/util/List;)V": ("LIST", 1),
}
BUNDLE_CALLS = {
    f"method:class:{BUNDLE}#<init>(Ljava/lang/Object;{DOMAIN_EVENT})V": "SINGLE",
    f"method:class:{BUNDLE}#<init>(Ljava/lang/Object;Ljava/util/List;)V": "LIST",
}
BUNDLE_GET = f"method:class:{BUNDLE}#getEvents()Ljava/util/List;"
BUNDLE_RESULT = f"method:class:{BUNDLE}#getResult()Ljava/lang/Object;"
MAX_FACTS = 4
ENVELOPE_ACCESSORS = {
    "getEvent": ("()" + DOMAIN_EVENT, "PAYLOAD"),
    "getAggregateId": ("()Ljava/lang/String;", "PAYLOAD_VALUE"),
}


def member_identity(target):
    if isinstance(target, str) and target.startswith("method:class:"):
        owner, _, signature = target.removeprefix("method:class:").partition("#")
        return owner, signature
    return "", ""


def path(node):
    if node and node[0] == "id":
        return node[3]
    if node and node[0] == "member" and (base := path(node[3])):
        return base + "." + node[4]
    return None


@dataclass(frozen=True)
class Operand:
    kind: str
    name: str | None
    sources: tuple
    items: tuple = ()
    compiler: bool = False
    compatible: bool = False
    documented_collection: bool = False
    arguments: tuple = ()
    label: str | None = None
    result: object = None
    construction: object = None


class Publications:
    def __init__(self, units, *, entry_kind=None, receiver_event=None):
        self.units = units
        self.entry_kind = entry_kind
        self.receiver_event = receiver_event
        self.rows = {}
        self.replies = {}
        self.effects = {}
        self.attachments = {}
        self.aggregate_cache = {}
        self.event_types = {}
        self.beans = {}

    def qualified(self, owner, spelling, parameters=()):
        """Return explicit Java type evidence, or a unique retained same-package declaration."""
        if not spelling or spelling in parameters or spelling in owner.node[9] or spelling in owner.node[10]:
            return None
        if CLASS.fullmatch(spelling):
            return spelling, ()
        imported = [item for item in owner.import_nodes if item[0] == "import" and item[3].endswith("." + spelling)]
        if len(imported) == 1:
            return imported[0][3], (owner.site(imported[0]),)
        if imported:
            return None
        name = owner.package + "." + spelling
        if owner.package and CLASS.fullmatch(name) and (target := self.units.lookup(owner, name, protocol=True)):
            return name, (self.anchor(target),)
        return None

    @staticmethod
    def anchor(owner):
        end = min((row[1] for row in owner.members if row[1] > owner.node[1]), default=owner.node[2])
        return type(owner.site(owner.node))(owner.path, owner.node[1], end)

    def class_operand(self, owner, node, parameters=()):
        if not node or node[0] != "member" or node[4] != "class":
            return None
        if resolved := self.qualified(owner, path(node[3]), parameters):
            return Operand("CLASS", resolved[0], (owner.site(node), *resolved[1]))
        return None

    def literal(self, owner, node):
        if node[0] != "literal" or node[3] != "NONEMPTY_STRING":
            return None
        literal = self.units.code(owner.site(node))
        if literal and literal[1:-1].startswith('"') and literal[1:-1].endswith('"'):
            name = literal[2:-2]
            if CHANNEL.fullmatch(name) and name not in {".", ".."}:
                return Operand("CHANNEL", name, (owner.site(node),))
        return None

    def list_factory(self, frame, state, node, target):
        written = path(node[3])
        owner, _, name = written.rpartition(".") if written else ("", "", "")
        expected = {"of": "java.util.List", "singletonList": "java.util.Collections"}.get(name)
        if not expected:
            return None
        if target:
            selected_owner, selected = member_identity(target)
            return () if selected_owner == expected and selected.startswith(name + "(") else None
        if not owner or owner.partition(".")[0] in state.bindings or frame.owner.fields(owner.partition(".")[0]):
            return None
        resolved = self.qualified(frame.owner, owner, frame.method[12])
        if resolved and resolved[0] == expected:
            return resolved[1]
        return None

    def construct(self, frame, node, written, values, target):
        site = frame.site(node)
        match = CONSTRUCTOR.match(target or "")
        if target and not match:
            return None
        resolved = (match[1], ()) if match else self.qualified(frame.owner, written, frame.method[12])
        if not resolved:
            return None
        name, type_proof = resolved
        if name == BUNDLE and len(values) == 2:
            events = values[1].publication_operand
            mode = (BUNDLE_CALLS.get(target) if target else
                    ("LIST" if events and events.kind == "LIST" else "SINGLE"))
            if mode:
                items = self.event_items(events, mode)
                if not target:
                    items = tuple(item for item in items if item.compatible)
                return Operand("BUNDLE", BUNDLE, (site, *type_proof), items, result=values[0], construction=site)
            return None
        key = frame.owner.path, name
        if key not in self.event_types:
            self.event_types[key] = self.event_declaration(frame.owner, name) if CLASS.fullmatch(name) else None
        compatible = self.event_types[key]
        if compatible is not None or match and CLASS.fullmatch(name):
            args = tuple((argument.label, argument.site) for argument in values
                         if argument.label and argument.site and argument.label != "the cited expression")[:3]
            return Operand("EVENT", name, (site, *type_proof, *(compatible or ())),
                           compiler=bool(match), compatible=compatible is not None, arguments=args, construction=site)
        return None

    def event_declaration(self, owner, name, seen=()):
        if name in seen or len(seen) >= 4 or not (declaration := self.units.lookup(owner, name, protocol=True)):
            return None
        for base in declaration.node[11]:
            if resolved := self.qualified(declaration, base[3]):
                parent, sources = resolved
                parent_proof = () if parent == EVENT else self.event_declaration(declaration, parent, (*seen, name))
                if parent_proof is not None:
                    return declaration.site(base), *sources, *parent_proof
        return None

    @staticmethod
    def event_items(value, mode):
        if value and mode == "SINGLE" and value.kind == "EVENT":
            return (value,)
        if value and mode == "LIST" and value.kind == "LIST":
            return value.items
        return ()

    def call_operand(self, frame, state, node, receiver, name, operands, target):
        if name == "getName" and not operands and receiver and receiver.kind == "CLASS":
            if not target or target == "method:class:java.lang.Class#getName()Ljava/lang/String;":
                return Operand("CHANNEL", receiver.name, (frame.site(node), *receiver.sources))
        if name == "getEvents" and receiver and receiver.kind == "BUNDLE" and target in {None, BUNDLE_GET} and not operands:
            return Operand("LIST", None, (frame.site(node), *receiver.sources), receiver.items,
                           documented_collection=True)
        if len(operands) <= 8 and (proof := self.list_factory(frame, state, node, target)) is not None:
            items = tuple(row for row in operands if row and row.kind == "EVENT")
            if items and len(items) == len(operands):
                return Operand("LIST", None, (frame.site(node), *proof), items)
        return None

    def bundle_result(self, frame, node, operand, target):
        if operand and operand.kind == "BUNDLE" and target in {None, BUNDLE_RESULT} and operand.result is not None:
            result = operand.result
            return replace(result, provenance=(*result.provenance, frame.site(node), *operand.sources))
        return None

    def generic_factory(self, owner, name, arity):
        # Operands flow through a fixed source body. An erased generic return type
        # alone cannot establish which event objects a collection contains.
        methods = owner.methods(name)
        if len(methods) == 1:
            method = methods[0]
            if (method[12] and {"STATIC", "PRIVATE"}.intersection(method[5])
                    and method[8] and len(method[6]) == arity
                    and all(not parameter[4].endswith("[]") for parameter in method[6])
                    and (result := self.qualified(owner, method[4], method[12]))
                    and result[0] in {BUNDLE, "java.util.List"}):
                return method
        return None

    def envelope(self, owner, parameter):
        resolved = self.qualified(owner, parameter[4])
        if self.receiver_event and resolved and resolved[0] == ENVELOPE:
            return Operand("ENVELOPE", self.receiver_event, (owner.site(parameter), *resolved[1]))
        return None

    def envelope_call(self, frame, node, receiver, name, values, target, label):
        operand = receiver.publication_operand
        if not operand or values:
            return None
        if operand.kind == "ENVELOPE" and (accessor := ENVELOPE_ACCESSORS.get(name)):
            signature, kind = accessor
            if target in {None, f"method:class:{ENVELOPE}#{name}{signature}"}:
                result_type = operand.name if kind == "PAYLOAD" else "java.lang.String"
                return Operand(kind, operand.name, (frame.site(node), *operand.sources), label=label), result_type
        if operand.kind == "PAYLOAD" and (record := self.units.lookup(frame.owner, operand.name, protocol=True)) and record.record:
            if target and member_identity(target)[0] != record.qualified:
                return None
            if not record.methods(name) and len(components := record.fields(name)) == 1 and "STATIC" not in components[0][5]:
                field = components[0]
                return self.payload_field(operand, record.site(field), label, frame.site(node)), self.units.qualify(record, field[4])
        return None

    @staticmethod
    def payload_field(operand, field, label, site):
        if operand and operand.kind == "PAYLOAD":
            return Operand("PAYLOAD_VALUE", operand.name, (site, field, *operand.sources), label=label)
        return None

    def handler_inputs(self, values, depth=0):
        for value in values:
            if (operand := value.publication_operand) and operand.kind in {"PAYLOAD", "PAYLOAD_VALUE"}:
                if operand.name == self.receiver_event:
                    yield operand
            if depth < 2 and value.constructed:
                yield from self.handler_inputs(value.arguments[:4], depth + 1)

    def handler_effect(self, frame, state, node, values, *, assignment=False):
        if not self.receiver_event:
            return
        site = frame.site(node)
        if not (label := self.units.code(site)):
            return
        inputs = tuple(islice(dict.fromkeys(self.handler_inputs(values[:8])), 3))
        if not inputs:
            return
        dependencies = " and ".join(dict.fromkeys(item.label for item in inputs if item.label))
        if not dependencies:
            return
        description = f"Assigns via {label}" if assignment else f"Attempts {label}"
        cites = (site, *frame.parents, *(proof for item in inputs for proof in item.sources))
        self.effects.setdefault((site, frame.parents, state.conditions, dependencies),
                                (f"{description} with a value from {dependencies} in the received event envelope.",
                                 cites, state.conditions))

    def returned_operand(self, frame, state, node, operand):
        if operand and operand.kind in {"EVENT", "LIST", "BUNDLE", "PAYLOAD", "PAYLOAD_VALUE"}:
            trail = (*frame.parents, frame.site(node), *operand.sources)
            if operand.kind == "BUNDLE":
                self.bundle_attachment(frame, state, node, operand, trail)
            return replace(operand, sources=tuple(dict.fromkeys(trail)))
        return operand

    def bundle_attachment(self, frame, state, node, bundle, trail):
        bundle_expression = self.units.code(bundle.construction) or "the cited result constructor"
        for event in bundle.items:
            event_expression = self.units.code(event.construction) or "the cited event constructor"
            witnesses = (frame.site(node), bundle.construction, event.construction, *trail,
                         *event.sources, *(source for _, source in event.arguments))
            text = (f"`{frame.owner.qualified}.{frame.method[3]}` returns the bundle constructed by {bundle_expression}, "
                    f"containing the `{event.name}` event constructed via {event_expression}.")
            self.attachments.setdefault((bundle.construction, event.construction, state.conditions),
                                        (text, witnesses, state.conditions))

    def bean_receiver(self, port):
        if port not in self.beans:
            choices = []
            simple = port.rsplit(".", 1)[-1].encode()
            files = list(islice(self.units.snapshot.paths_containing(b"Bean", simple), 13))
            if len(files) > 12:
                self.beans[port] = None
                return None
            for file in files:
                declarations = self.units.read(file, protocol=True)
                if not declarations:
                    choices.append(None)
                for candidate in declarations:
                    for method in candidate.methods():
                        declared = self.qualified(candidate, method[4])
                        if not declared or declared[0] != port:
                            continue
                        annotation = next(((item, verified) for item in method[7]
                                           if (verified := self.qualified(candidate, item[3])) and verified[0] == BEAN), None)
                        if not annotation:
                            continue
                        choices.append(self.bean_implementation(candidate, method, declared, annotation))
            self.beans[port] = choices[0] if len(choices) == 1 else None
        return self.beans[port]

    def bean_implementation(self, owner, method, declared, annotation):
        body = method[8]
        if not body or len(body[3]) != 1 or body[3][0][0] != "return":
            return None
        returned = body[3][0][3]
        if (returned and returned[0] == "new" and not returned[3]
                and (constructed := self.qualified(owner, returned[4][3]))
                and (implementation := self.units.lookup(owner, constructed[0], protocol=True))):
            item, proof = annotation
            cites = (owner.site(item), owner.site(method), *proof[1], *declared[1], *constructed[1])
            return implementation, cites
        return None

    def aggregate(self, owner, receiver_type, name):
        key = owner.path, receiver_type, name
        if key not in self.aggregate_cache:
            self.aggregate_cache[key] = self.aggregate_channel(owner, receiver_type, name)
        return self.aggregate_cache[key]

    def aggregate_api(self, owner, receiver_type, name):
        if receiver_type in {AGGREGATE, PORT}:
            return True
        wrapper = self.units.lookup(owner, receiver_type, protocol=True)
        return wrapper is not None and self.ancestry(wrapper, {AGGREGATE, PORT}, name) is not None

    def ancestry(self, owner, targets, member, seen=()):
        if owner.qualified in seen or len(seen) == 4 or owner.methods(member):
            return None
        for base in owner.node[11]:
            if not (resolved := self.qualified(owner, base[3])):
                continue
            name, cites = resolved
            if name in targets:
                return owner.site(base), *cites
            if (parent := self.units.lookup(owner, name, protocol=True)) and (
                    proof := self.ancestry(parent, targets, member, (*seen, owner.qualified))) is not None:
                return owner.site(base), *cites, *proof
        return None

    def aggregate_channel(self, owner, receiver_type, name):
        wrapper = self.units.lookup(owner, receiver_type, protocol=True)
        if wrapper is None or wrapper.methods(name) or receiver_type in {AGGREGATE, PORT}:
            return None
        extra = ()
        if wrapper.node[4] == "INTERFACE":
            proof = self.ancestry(wrapper, {PORT}, name)
            if proof is None or not (factory := self.bean_receiver(receiver_type)):
                return None
            implementation, wiring = factory
            if (implemented := self.ancestry(implementation, {receiver_type}, name)) is None:
                return None
            extra = (*wiring, *proof, *implemented)
            wrapper = implementation

        constructors = wrapper.methods("<init>")
        bases = wrapper.node[11]
        if len(constructors) != 1 or not bases or not (base := self.qualified(wrapper, bases[0][3])) or base[0] != AGGREGATE:
            return None
        body = constructors[0][8]
        if not body or not body[3]:
            return None
        first = body[3][0]
        if first[0] != "expr" or first[3][0] != "call" or path(first[3][3]) != "super" or len(first[3][4]) != 4:
            return None
        if channel := self.class_operand(wrapper, first[3][4][0]):
            return Operand("CHANNEL", channel.name, (*channel.sources, wrapper.site(first), wrapper.site(bases[0]),
                           *base[1], *extra))
        return None

    def submit(self, frame, state, node, receiver, name, values, target):
        operands = tuple(value.publication_operand for value in values)
        api, signature = member_identity(target)
        compiler = bool(target)
        receiver_type = receiver.exact or receiver.type
        if api == DIRECT and (shape := DIRECT_CALLS.get(signature)):
            mode, position = shape
        elif api in {AGGREGATE, PORT} and (shape := AGGREGATE_CALLS.get(signature)):
            mode, position = shape
        elif target:
            return
        elif receiver_type == DIRECT and receiver.site and name == "publish" and len(operands) in {3, 4}:
            position = len(operands) - 1
            mode = "CLASS" if len(operands) == 3 and operands[0] and operands[0].kind == "CLASS" else "STRING"
        elif receiver_type and receiver.site and name in {"publish", "publishById"} and len(operands) == 2:
            if not self.aggregate_api(frame.owner, receiver_type, name):
                return
            mode, position = ("LIST" if name == "publish" and operands[1] and operands[1].kind == "LIST" else "SINGLE"), 1
        else:
            return

        site = frame.site(node)
        if position >= len(operands):
            return
        if mode in {"STRING", "CLASS"}:
            channel = operands[0] if operands[0] and operands[0].kind == ("CHANNEL" if mode == "STRING" else "CLASS") else None
            items = self.event_items(operands[position], "LIST")
        else:
            channel = self.aggregate(frame.owner, receiver_type, name) if receiver_type else None
            items = self.event_items(operands[position], mode)
        if not compiler:
            items = tuple(row for row in items if row.compatible)
        occurrence = "/".join(f"{address.path}:{address.start}:{address.end}" for address in (*frame.parents, site))
        if state.conditions:
            occurrence += ":" + sha256(repr(state.conditions).encode()).hexdigest()[:16]
        aggregate_position = 1 if mode in {"STRING", "CLASS"} else 0
        argument = values[aggregate_position]
        argument_site = frame.site(node[4][aggregate_position])
        aggregate = (argument.label if argument.label and argument.label != "the cited expression"
                     else self.units.code(argument_site))
        call_sources = (site, *frame.parents, *(proof for item in state.conditions for proof in item.cites))
        if aggregate:
            call_sources += (argument_site, argument.site or argument_site)
        if receiver.site and (not compiler or mode in {"SINGLE", "LIST"}):
            call_sources += (receiver.site, *receiver.provenance, *receiver.type_sources)
        aggregate_role = "aggregate" if mode in {"SINGLE", "LIST"} and name == "publish" else "aggregate ID"
        for item in items or (None,):
            topic = channel.name if channel and CHANNEL.fullmatch(channel.name) else None
            event = item.name if item else None
            key = occurrence, topic, event, state.conditions
            witnesses = (*call_sources, *(channel.sources if topic else ()), *(item.sources if item else ()))
            if item:
                witnesses += tuple(source for _, source in item.arguments)
            if operands[position] and operands[position].kind == "LIST":
                witnesses += operands[position].sources
            if topic and mode in {"SINGLE", "LIST"}:
                uncertainty = "The receiver must use the retained constructor and the adapter's class-based channel mapping."
            elif not compiler:
                uncertainty = "The retained receiver and arguments must bind to the library signature at runtime."
            else:
                uncertainty = ""
            authority = "COMPILER_SELECTED_JAVA_PUBLICATION" if compiler else "RETAINED_JAVA_PUBLICATION_CANDIDATE"
            if item and not item.compiler:
                uncertainty = " ".join(filter(None, (uncertainty, "The event class is corroborated by retained source.")))
            if operands[position] and operands[position].documented_collection:
                uncertainty = " ".join(filter(None, (uncertainty,
                    "The event collection must follow the retained Eventuate result accessor contract.")))
            self.rows.setdefault(key, (authority, topic, event, aggregate, aggregate_role, item.arguments if item else (),
                                       uncertainty, witnesses, state.conditions))
        return occurrence, site, len(state.events)

    def reply(self, frame, state, node):
        if self.entry_kind != "HTTP_ENDPOINT" or frame.parents or not state.typed_submissions:
            return
        site = frame.site(node)
        label = self.units.code(site)
        if not label:
            return
        publications = {submitted for _, submitted, _ in state.typed_submissions}
        for occurrence, submitted, offset in state.typed_submissions:
            # Treat other opaque calls as a boundary. They could join, block or
            # consume a result, including during evaluation of the return value.
            subsequent = [event for event in state.events[offset:]
                          if event.kind == "attempt" and event.cites[0] not in publications]
            independent = not subsequent
            cites = (site, submitted, *(event.cites[0] for event in subsequent[:2]))
            self.replies.setdefault((occurrence, site, state.conditions), (label, independent, cites, state.conditions))

    @staticmethod
    def evidence(snapshot, witnesses, guards):
        condition = " and ".join(dict.fromkeys(item.text for item in guards))
        if len(condition) > 750:
            condition = " and ".join(dict.fromkeys(item.brief or item.text for item in guards))
            if len(condition) > 750:
                return None
        cited = (*witnesses, *(proof for item in guards for proof in item.cites))
        coords = [(item.path, item.start, item.end) for item in dict.fromkeys(cited)]
        if sources := (snapshot.cite_many(coords)
                       or snapshot.cite_many(snapshot.compact_witnesses(coords), protocol=True)):
            return sources, condition
        return None

    def ordered_publications(self):
        complete, unresolved = {}, {}
        for key, row in self.rows.items():
            _, channel, event, _, _, _, _, cites, _ = row
            groups, route = (complete, (channel, event)) if channel and event else (unresolved, cites[0])
            groups.setdefault(route, []).append((key, row))

        def precision(candidate):
            key, row = candidate
            head = row[-2][0]
            condition = " and ".join(item.brief or item.text for item in row[-1])
            return len(condition), head.path, head.start, condition, key[0]

        routes = [complete[key] for key in sorted(complete)]
        sites = [unresolved[key] for key in sorted(unresolved, key=lambda site: (site.path, site.start, site.end))]
        alternatives = [sorted(rows, key=precision) for rows in (*routes, *sites)]
        return (item for layer in zip_longest(*alternatives) for item in layer if item is not None)

    def facts(self, snapshot, root):
        output = []
        occurrences = set()
        proof_open = False
        for (occurrence, _, _, _), (authority, channel, event, aggregate, role, arguments,
                                    uncertainty, witnesses, guards) in self.ordered_publications():
            if len(output) == MAX_FACTS:
                output.append({"kind": "java-typed-publication-boundary", "authority": "PINNED_JAVAC_SYNTAX",
                               "status": "PREPARATION_LIMIT", "sources": [root],
                               "text": "Additional typed publication paths remain beyond the prepared source limit."})
                break
            if not (proof := self.evidence(snapshot, witnesses, guards)):
                proof_open = True
                continue
            sources, condition = proof
            text = ("The retained Java call attempts an Eventuate domain event publication"
                    + (f" of `{event}`" if event else "") + (f" on `{channel}`" if channel else "") + ".")
            if not channel or not event:
                text += " The channel or qualified event identity remains open."
            if aggregate:
                text += f" The publication passes {aggregate} as the {role} argument."
            if arguments:
                supplied = ", ".join(dict.fromkeys(label for label, _ in arguments))
                text += f" The event constructor receives {supplied}."
            uncertainty += " Adapter mapping, deployment destination, activation and delivery remain open."
            output.append({"kind": "java-typed-publication", "authority": authority, "text": text,
                           "uncertainty": uncertainty.strip(), "sources": list(dict.fromkeys([root, *sources])),
                           **({"condition": condition} if condition else {}),
                           "_edge": {"kind": "JAVA_TYPED_PUBLISH", "adapter": ADAPTER, "site": sources[0],
                                     "channel": channel, "eventType": event,
                                     "occurrence": occurrence}})
            occurrences.add(occurrence)
        if proof_open:
            output.append({"kind": "java-typed-publication-boundary", "authority": "PINNED_JAVAC_SYNTAX",
                           "status": "PREPARATION_LIMIT", "sources": [root],
                           "text": "Some typed publication paths lack complete admitted source witnesses."})
        replies = (item for item in self.replies.items() if item[0][0] in occurrences)
        for (occurrence, _, _), (label, independent, witnesses, guards) in islice(replies, MAX_FACTS):
            if not (proof := self.evidence(snapshot, witnesses, guards)):
                continue
            sources, condition = proof
            waiting = (" This local source path has no explicit wait for a dispatcher result." if independent
                       else " Other calls on this path leave any wait for a dispatcher result open.")
            output.append({"kind": "java-typed-publication-reply", "authority": "PINNED_JAVAC_SYNTAX",
                           "text": f"After the local publication call returns normally, the HTTP entry can reach {label}." + waiting,
                           "uncertainty": "Wire response, handler completion and their execution order remain open.",
                           "sources": list(dict.fromkeys([root, *sources])), "_edgeCause": occurrence,
                           **({"condition": condition} if condition else {})})
        for text, witnesses, guards in islice(self.effects.values(), MAX_FACTS):
            if not (proof := self.evidence(snapshot, witnesses, guards)):
                continue
            sources, condition = proof
            output.append({"kind": "java-typed-handler-effect", "authority": "PINNED_JAVAC_SYNTAX", "text": text,
                           "uncertainty": "This source evaluation is conditional on entering the handler and reaching the cited path.",
                           "sources": list(dict.fromkeys([root, *sources])),
                           **({"condition": condition} if condition else {})})
        for text, witnesses, guards in islice(self.attachments.values(), MAX_FACTS):
            if not (proof := self.evidence(snapshot, witnesses, guards)):
                continue
            sources, condition = proof
            output.append({"kind": "java-typed-event-attachment", "authority": "PINNED_JAVAC_SYNTAX", "text": text,
                           "uncertainty": "The event collection follows the retained result constructor if evaluation completes normally.",
                           "sources": list(dict.fromkeys([root, *sources])),
                           **({"condition": condition} if condition else {})})
        return output
