"""Project bounded Java submissions through exact typed Eventuate protocol operands."""

from dataclasses import dataclass
from itertools import islice
import re

ADAPTER = "EVENTUATE_TRAM_DOMAIN_EVENT"
NAMESPACE = "io.eventuate.tram.events.publisher."
DIRECT = NAMESPACE + "DomainEventPublisher"
AGGREGATE = NAMESPACE + "AbstractDomainEventPublisherForAggregateImpl"
PORT = NAMESPACE + "DomainEventPublisherForAggregate"
BUNDLE = NAMESPACE + "ResultWithTypedEvents"
EVENT = "io.eventuate.tram.events.common.DomainEvent"
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
MAX_FACTS = 4


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


class Publications:
    def __init__(self, units):
        self.units = units
        self.rows = {}
        self.aggregate_cache = {}
        self.beans = None

    def qualified(self, owner, spelling, parameters=()):
        """Return explicit Java type evidence, or a unique retained same-package declaration."""
        if not spelling or spelling in parameters or spelling in owner.node[9] or spelling in owner.node[10]:
            return None
        if CLASS.fullmatch(spelling):
            return spelling, ()
        imported = [item for item in owner.import_nodes if item[3].endswith("." + spelling)]
        if len(imported) == 1:
            return imported[0][3], (owner.site(imported[0]),)
        if imported:
            return None
        name = owner.package + "." + spelling
        if owner.package and CLASS.fullmatch(name) and (target := self.units.lookup(owner, name)):
            if any(item.endswith(".*") for item in owner.imports) and target.path != owner.path:
                return None
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

    def construct(self, frame, node, written, operands, target):
        site = frame.site(node)
        match = CONSTRUCTOR.match(target or "")
        if target and not match:
            return None
        if match and match[1] == BUNDLE:
            mode = BUNDLE_CALLS.get(target)
            if mode and len(operands) == 2:
                items = self.event_items(operands[1], mode)
                if items:
                    return Operand("BUNDLE", BUNDLE, (site,), items)
            return None
        if match:
            name = match[1]
            return Operand("EVENT", name, (site,), compiler=True) if CLASS.fullmatch(name) else None
        if not (resolved := self.qualified(frame.owner, written, frame.method[12])):
            return None
        name, type_proof = resolved
        declaration = self.units.lookup(frame.owner, name)
        bases = [(base, self.qualified(declaration, base[3])) for base in declaration.node[11]] if declaration else []
        for base, qualified in bases:
            if qualified and qualified[0] == EVENT:
                return Operand("EVENT", name, (site, *type_proof, declaration.site(base), *qualified[1]), compatible=True)
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
        if receiver and receiver.kind == "BUNDLE" and target == BUNDLE_GET and not operands:
            return Operand("LIST", None, (frame.site(node), *receiver.sources), receiver.items,
                           documented_collection=True)
        if len(operands) <= 8 and (proof := self.list_factory(frame, state, node, target)) is not None:
            items = tuple(row for row in operands if row and row.kind == "EVENT")
            if items and len(items) == len(operands):
                return Operand("LIST", None, (frame.site(node), *proof), items)
        return None

    def bean_receiver(self, port):
        if self.beans is None:
            by_port = {}
            # The installed source cache bounds reads and does not load arbitrary classpaths.
            for file in islice(dict.fromkeys(self.units.snapshot.paths()), 12):
                for candidate in self.units.read(file):
                    for method in candidate.methods():
                        if not method[8] or len(method[8][3]) != 1 or method[8][3][0][0] != "return":
                            continue
                        annotation = next(((item, verified) for item in method[7]
                                           if (verified := self.qualified(candidate, item[3])) and verified[0] == BEAN), None)
                        returned = method[8][3][0][3]
                        if not annotation or not returned or returned[0] != "new" or returned[3]:
                            continue
                        declared = self.qualified(candidate, method[4])
                        constructed = self.qualified(candidate, returned[4][3])
                        if declared and constructed:
                            item, proof = annotation
                            sources = (candidate.site(item), candidate.site(method), *proof[1], *declared[1], *constructed[1])
                            by_port.setdefault(declared[0], []).append((constructed[0], sources))
            self.beans = by_port
        choices = self.beans.get(port, ())
        if len(choices) == 1:
            return choices[0]
        return None

    def aggregate(self, owner, receiver_type, name):
        key = owner.path, receiver_type, name
        if key not in self.aggregate_cache:
            self.aggregate_cache[key] = self.aggregate_channel(owner, receiver_type, name)
        return self.aggregate_cache[key]

    def aggregate_channel(self, owner, receiver_type, name):
        wrapper = self.units.lookup(owner, receiver_type)
        if wrapper is None or wrapper.methods(name) or receiver_type in {AGGREGATE, PORT}:
            return None
        extra = ()
        if wrapper.node[4] == "INTERFACE":
            bases = {value[0] for item in wrapper.node[11] if (value := self.qualified(wrapper, item[3]))}
            if PORT not in bases:
                return None
            factory = self.bean_receiver(receiver_type)
            if not factory or not (implementation := self.units.lookup(owner, factory[0])) or implementation.methods(name):
                return None
            extra = (*factory[1], self.anchor(wrapper))
            wrapper = implementation
            implemented = {value[0] for item in wrapper.node[11] if (value := self.qualified(wrapper, item[3]))}
            if receiver_type not in implemented:
                return None

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

    def submit(self, frame, state, node, receiver, name, operands, target):
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
            if receiver_type not in {AGGREGATE, PORT} and not self.aggregate(frame.owner, receiver_type, name):
                return
            mode, position = ("LIST" if name == "publish" and operands[1] and operands[1].kind == "LIST" else "SINGLE"), 1
        else:
            return

        site = frame.site(node)
        if position >= len(operands):
            return
        if mode in {"STRING", "CLASS"}:
            channel = operands[0] if operands[0] and operands[0].kind == ("CHANNEL" if mode == "STRING" else "CLASS") else None
            values = self.event_items(operands[position], "LIST")
        else:
            channel = self.aggregate(frame.owner, receiver_type, name) if receiver_type else None
            values = self.event_items(operands[position], mode)
        if not compiler:
            values = tuple(row for row in values if row.compatible)
        if not compiler and not values:
            return
        call_sources = (site, *frame.parents, *[proof for item in state.conditions for proof in item.cites])
        if receiver.site and (not compiler or mode in {"SINGLE", "LIST"}):
            call_sources += (receiver.site, *receiver.provenance, *receiver.type_sources)
        for item in values or (None,):
            topic = channel.name if channel and CHANNEL.fullmatch(channel.name) else None
            event = item.name if item else None
            key = site, topic, event, state.conditions
            witnesses = (*call_sources, *(channel.sources if topic else ()), *(item.sources if item else ()))
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
            self.rows.setdefault(key, (authority, site, topic, event, uncertainty, witnesses, state.conditions))

    def facts(self, snapshot, root):
        output = []
        for authority, site, channel, event, uncertainty, witnesses, guards in self.rows.values():
            if len(output) == MAX_FACTS:
                output.append({"kind": "java-typed-publication-boundary", "authority": "PINNED_JAVAC_SYNTAX",
                               "status": "PREPARATION_LIMIT", "sources": [root],
                               "text": "Additional typed publication paths remain beyond the prepared source limit."})
                break
            condition = " and ".join(dict.fromkeys(item.text for item in guards))
            if len(condition) > 750:
                continue
            coords = [(item.path, item.start, item.end) for item in dict.fromkeys(witnesses)]
            if not (sources := snapshot.cite_many(coords)):
                continue
            text = ("The retained Java call attempts an Eventuate domain event publication"
                    + (f" of `{event}`" if event else "") + (f" on `{channel}`" if channel else "") + ".")
            if not channel or not event:
                text += " The channel or qualified event identity remains open."
            uncertainty += " Adapter mapping, deployment destination, activation and delivery remain open."
            output.append({"kind": "java-typed-publication", "authority": authority, "text": text,
                           "uncertainty": uncertainty.strip(), "sources": list(dict.fromkeys([root, *sources])),
                           **({"condition": condition} if condition else {}),
                           "_edge": {"kind": "JAVA_TYPED_PUBLISH", "adapter": ADAPTER, "site": sources[0],
                                     "channel": channel, "eventType": event,
                                     "occurrence": f"{site.path}:{site.start}:{site.end}"}})
        return output
