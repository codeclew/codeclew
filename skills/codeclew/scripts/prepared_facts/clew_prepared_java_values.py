"""Collect exact constant and predicate bindings from bounded javac attribution."""

from base64 import b64decode, b64encode
from collections import defaultdict
from pathlib import Path, PurePosixPath
import re
import runpy
import shutil
import subprocess


LOCATE = runpy.run_path(str(Path(__file__).with_name("clew_reader_java_sources.py")))["locate"]
MAX_UNITS = 6
NAME = r"[A-Za-z_$][A-Za-z0-9_$]*"
QUALIFIED = rf"{NAME}(?:[ \t]*\.[ \t]*{NAME})*"
PACKAGE = re.compile(rf"(?:^|\n)[ \t]*package[ \t]+({QUALIFIED})[ \t]*;")
IMPORT = re.compile(rf"(?:^|\n)[ \t]*import[ \t]+(static[ \t]+)?({QUALIFIED}(?:[ \t]*\.[ \t]*\*)?)[ \t]*;")


def names(node):
    if node[0] == "id":
        return [node[3]]
    if node[0] == "member":
        owner = names(node[3])
        if owner:
            return [*owner, node[4]]
    return []


def source_candidates(path, raw, selectors):
    # These package and import spellings bound Git lookup. Only javac's selected
    # field element can establish a reference to a returned candidate file.
    text = raw.decode()
    package = PACKAGE.search(text)
    package = re.sub(r"\s+", "", package[1]).split(".") if package else []
    folder = PurePosixPath(path).parent
    if package and tuple(package) != folder.parts[-len(package):]:
        return
    root = folder.parents[len(package) - 1] if package else folder
    imports = {}
    candidates = []
    for declaration in IMPORT.finditer(text):
        parts = re.sub(r"\s+", "", declaration[2]).split(".")
        if declaration[1]:
            candidates.append(parts[:-1])
        elif parts[-1] != "*":
            imports[parts[-1]] = parts
    for selector in selectors:
        if selector[0] in imports:
            candidates.append(imports[selector[0]])
        candidates.extend((package + selector[:1], selector))
    for candidate in candidates:
        if candidate:
            yield str(root.joinpath(*candidate[:-1], candidate[-1] + ".java"))


def attributed_bindings(snapshot, bodies, qualifiers):
    java = shutil.which("java")
    if not snapshot or not java:
        return {}, {}
    files, selected = {}, defaultdict(list)
    for identifier, source in bodies.items():
        if source.revision != snapshot.revision:
            continue
        raw = snapshot.read(source.file)
        if raw != source.text and (start := LOCATE(raw, source.text, source.line, source.offset)) is not None:
            selected[source.file].append((identifier, start, start + len(source.text)))
            files[source.file] = raw
    if not selected:
        return {}, {}
    candidates = (candidate for file, raw in files.items()
                  for candidate in source_candidates(file, raw, [name for identifier, _, _ in selected[file]
                                                                  for name in qualifiers.get(identifier, ())]))
    nearby = snapshot.existing_paths(candidates)
    for file in nearby:
        if len(files) >= MAX_UNITS:
            break
        if file not in files and (raw := snapshot.read(file)):
            files[file] = raw
    request = b"\n".join(b"\t".join((b64encode(file.encode()), b64encode(raw),
                                     ",".join(f"{start}:{end}" for _, start, end in selected[file]).encode()))
                         for file, raw in files.items()) + b"\n"
    script = Path(__file__).with_name("clew_prepared_java_field_uses.java")
    try:
        process = subprocess.run([java, "--source", "21", str(script)], input=request,
                                 capture_output=True, timeout=snapshot.remaining(8) if hasattr(snapshot, "remaining") else 8, check=True)
        return attributed_uses(snapshot, files, selected, process.stdout)
    except (OSError, subprocess.SubprocessError, IndexError, ValueError, UnicodeError):
        return {}, {}


def attributed_uses(snapshot, files, selected, output):
    results, predicates = defaultdict(dict), defaultdict(dict)
    paths = list(files)
    for row in output.decode().splitlines():
        parts = row.split("\t")
        if parts[0] == "P":
            parse_predicate(parts[1:], paths, files, selected, predicates)
            continue
        source, start, end, owner, first, last, begin, finish, constant = parts
        source, start, end, owner, first, last, begin, finish = map(
            int, (source, start, end, owner, first, last, begin, finish))
        file, declaration = paths[source], paths[owner]
        if not 0 <= first <= begin < finish <= last <= len(files[declaration]):
            continue
        if constant not in {"TRUE", "FALSE", "EMPTY_STRING", "NONEMPTY_STRING", "PRIMITIVE"}:
            continue
        for identifier, lower, upper in selected[file]:
            if lower <= start < end <= upper and (ref := snapshot.cite(declaration, first, last)):
                initializer = files[declaration][begin:finish].decode()
                results[identifier][start - lower, end - lower] = initializer, constant, ref
    return results, predicates


def parse_predicate(row, paths, files, selected, predicates):
    source, start, end, first, last = map(int, row[:5])
    file = paths[source]
    raw = files[file]
    if not 0 <= first < last <= len(raw):
        return
    formals = [b64decode(name).decode() for name in row[5].split(",") if name]
    nominal = b64decode(row[7]).decode() if row[7] else None
    witnesses = [tuple(map(int, item.split(":"))) for item in row[8].split(",") if item]
    if any(not 0 <= low < high <= len(raw) for low, high in witnesses):
        return
    steps = []
    for step in row[6].split(";"):
        kind, *positions = step.split(",")
        bounds = list(map(int, positions))
        if kind not in {"L", "T", "F", "R", "A", "O"} or len(bounds) != (4 if kind in {"A", "O"} else 2):
            return
        fragments = []
        for lower, upper in zip(bounds[::2], bounds[1::2]):
            if not first <= lower < upper <= last:
                return
            fragments.append(raw[lower:upper].decode())
        steps.append((kind, fragments))
    for identifier, lower, upper in selected[file]:
        if lower <= start < end <= upper:
            predicates[identifier][start - lower, end - lower] = (file, first, last, formals, steps, nominal, witnesses)
