import com.sun.source.tree.AnnotationTree;
import com.sun.source.tree.ClassTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.MemberReferenceTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.Deque;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import javax.lang.model.element.AnnotationMirror;
import javax.lang.model.element.AnnotationValue;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.Modifier;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.ArrayType;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.ExecutableType;
import javax.lang.model.type.NoType;
import javax.lang.model.type.PrimitiveType;
import javax.lang.model.type.TypeKind;
import javax.lang.model.type.TypeMirror;
import javax.lang.model.util.Elements;
import javax.lang.model.util.Types;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;

final class CodeclewJavaAnalyzer {
    private static final String SCHEMA = "codeclew-java-compiler-fact/1.0";

    public static void main(String[] args) throws Exception {
        if (args.length != 4) {
            System.exit(2);
        }
        Path root = Path.of(args[0]).toRealPath();
        List<Path> sources = readSources(root, Path.of(args[1]));
        List<String> classpath = readLines(Path.of(args[2]));
        String release = args[3];
        if (!release.matches("[0-9]+") || Integer.parseInt(release) < 17 || sources.isEmpty()) {
            System.exit(2);
        }

        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) {
            System.exit(3);
        }
        DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
        List<Map<String, Object>> facts = new ArrayList<>();
        try (StandardJavaFileManager files = compiler.getStandardFileManager(
                diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
            List<String> options = new ArrayList<>(List.of(
                    "--release", release, "-proc:none", "-implicit:none", "-Xlint:none"));
            if (!classpath.isEmpty()) {
                options.add("-classpath");
                options.add(String.join(System.getProperty("path.separator"), classpath));
            }
            Iterable<? extends JavaFileObject> units = files.getJavaFileObjectsFromPaths(sources);
            JavacTask task = (JavacTask) compiler.getTask(
                    null, files, diagnostics, options, null, units);
            List<CompilationUnitTree> parsed = new ArrayList<>();
            task.parse().forEach(parsed::add);
            task.analyze();
            boolean failed = diagnostics.getDiagnostics().stream()
                    .anyMatch(row -> row.getKind() == Diagnostic.Kind.ERROR);
            if (failed) {
                facts.clear();
                diagnostics.getDiagnostics().stream()
                        .filter(row -> row.getKind() == Diagnostic.Kind.ERROR)
                        .sorted(Comparator.comparing((Diagnostic<? extends JavaFileObject> row) -> row.getCode())
                                .thenComparingLong(Diagnostic::getLineNumber))
                        .forEach(row -> facts.add(diagnosticBoundary(root, row)));
            } else {
                Trees trees = Trees.instance(task);
                Analyzer analyzer = new Analyzer(
                        root, trees, task.getElements(), task.getTypes(), facts);
                parsed.forEach(unit -> analyzer.scan(unit, null));
            }
        }
        TreeSet<String> canonical = new TreeSet<>();
        for (Map<String, Object> fact : facts) {
            canonical.add(json(fact));
        }
        canonical.forEach(System.out::println);
    }

    private static List<Path> readSources(Path root, Path list) throws IOException {
        List<Path> result = new ArrayList<>();
        for (String relative : readLines(list)) {
            Path candidate = root.resolve(relative).normalize();
            if (candidate.isAbsolute()
                    && candidate.startsWith(root)
                    && candidate.toString().endsWith(".java")
                    && Files.isRegularFile(candidate)
                    && !Files.isSymbolicLink(candidate)) {
                result.add(candidate);
            } else {
                throw new IOException("invalid source authority");
            }
        }
        result.sort(Comparator.naturalOrder());
        if (result.size() != new TreeSet<>(result).size()) {
            throw new IOException("duplicate source authority");
        }
        return result;
    }

    private static List<String> readLines(Path path) throws IOException {
        List<String> values = Files.readAllLines(path, StandardCharsets.UTF_8);
        if (values.stream().anyMatch(value -> value.isBlank() || value.indexOf('\0') >= 0)) {
            throw new IOException("invalid manifest authority");
        }
        return values;
    }

    private static Map<String, Object> diagnosticBoundary(
            Path root, Diagnostic<? extends JavaFileObject> diagnostic) {
        Map<String, Object> row = base("BOUNDARY");
        row.put("code", "JAVA_COMPILER_DIAGNOSTIC");
        row.put("diagnosticCode", safeToken(diagnostic.getCode()));
        row.put("line", Math.max(0, diagnostic.getLineNumber()));
        if (diagnostic.getSource() != null) {
            try {
                Path source = Path.of(diagnostic.getSource().toUri()).toRealPath();
                if (source.startsWith(root)) {
                    row.put("file", relative(root, source));
                }
            } catch (Exception ignored) {
                // A non-file diagnostic remains a bounded compilation boundary.
            }
        }
        row.put("requiredChecks", List.of("FIX_JAVA_CLASSPATH_OR_DIAGNOSTIC"));
        row.put("resolution", "UNKNOWN");
        return row;
    }

    private static final class Analyzer extends TreePathScanner<Void, Void> {
        private final Path root;
        private final Trees trees;
        private final Elements elements;
        private final Types types;
        private final SourcePositions positions;
        private final List<Map<String, Object>> facts;
        private final Deque<String> owners = new ArrayDeque<>();
        private final Deque<String> executableOwners = new ArrayDeque<>();

        private Analyzer(
                Path root,
                Trees trees,
                Elements elements,
                Types types,
                List<Map<String, Object>> facts) {
            this.root = root;
            this.trees = trees;
            this.elements = elements;
            this.types = types;
            this.positions = trees.getSourcePositions();
            this.facts = facts;
        }

        @Override
        public Void visitClass(ClassTree tree, Void unused) {
            Element element = trees.getElement(getCurrentPath());
            if (!(element instanceof TypeElement type)) {
                boundary("JAVA_CLASS_SYMBOL_UNRESOLVED", tree);
                return null;
            }
            String identity = classIdentity(type);
            Map<String, Object> row = declaration(
                    declarationKind(type.getKind()), identity, ownerOf(type), null, type, tree);
            row.put("interfaces", type.getInterfaces().stream()
                    .map(this::typeIdentity).sorted().toList());
            TypeMirror superclass = type.getSuperclass();
            if (superclass != null && superclass.getKind() != TypeKind.NONE) {
                row.put("superclass", typeIdentity(superclass));
            }
            facts.add(row);
            row.put("spring", new SpringReader().readInherited(type));
            owners.push(identity);
            super.visitClass(tree, unused);
            owners.pop();
            return null;
        }

        @Override
        public Void visitMethod(MethodTree tree, Void unused) {
            Element element = trees.getElement(getCurrentPath());
            if (!(element instanceof ExecutableElement executable)) {
                boundary("JAVA_METHOD_SYMBOL_UNRESOLVED", tree);
                return null;
            }
            String descriptor = executableDescriptor(executable);
            if (descriptor == null) {
                boundary("JAVA_METHOD_DESCRIPTOR_UNRESOLVED", tree);
                return null;
            }
            String owner = ownerOf(executable);
            String name = executable.getKind() == ElementKind.CONSTRUCTOR
                    ? "<init>" : executable.getSimpleName().toString();
            String identity = "method:" + owner + "#" + name + descriptor;
            facts.add(declaration(
                    executable.getKind() == ElementKind.CONSTRUCTOR ? "CONSTRUCTOR" : "METHOD",
                    identity, owner, descriptor, executable, tree));
            executableOwners.push(identity);
            super.visitMethod(tree, unused);
            executableOwners.pop();
            return null;
        }

        @Override
        public Void visitVariable(VariableTree tree, Void unused) {
            Element element = trees.getElement(getCurrentPath());
            if (element instanceof VariableElement variable
                    && Set.of(ElementKind.FIELD, ElementKind.ENUM_CONSTANT, ElementKind.RECORD_COMPONENT)
                            .contains(variable.getKind())) {
                String descriptor = descriptor(variable.asType());
                if (descriptor == null) {
                    boundary("JAVA_FIELD_DESCRIPTOR_UNRESOLVED", tree);
                } else {
                    String owner = ownerOf(variable);
                    facts.add(declaration(
                            "FIELD", "field:" + owner + "#" + variable.getSimpleName() + ":" + descriptor,
                            owner, descriptor, variable, tree));
                }
            }
            return super.visitVariable(tree, unused);
        }

        @Override
        public Void visitMethodInvocation(MethodInvocationTree tree, Void unused) {
            relation("CALLS", trees.getElement(getCurrentPath()), tree);
            return super.visitMethodInvocation(tree, unused);
        }

        @Override
        public Void visitNewClass(NewClassTree tree, Void unused) {
            relation("CONSTRUCTS", trees.getElement(getCurrentPath()), tree);
            return super.visitNewClass(tree, unused);
        }

        @Override
        public Void visitMemberReference(MemberReferenceTree tree, Void unused) {
            relation("REFERENCES", trees.getElement(getCurrentPath()), tree);
            return super.visitMemberReference(tree, unused);
        }

        @Override
        public Void visitIdentifier(IdentifierTree tree, Void unused) {
            typeUse(trees.getElement(getCurrentPath()), tree);
            return super.visitIdentifier(tree, unused);
        }

        @Override
        public Void visitMemberSelect(MemberSelectTree tree, Void unused) {
            typeUse(trees.getElement(getCurrentPath()), tree);
            return super.visitMemberSelect(tree, unused);
        }

        private Map<String, Object> declaration(
                String kind,
                String identity,
                String owner,
                String descriptor,
                Element element,
                Tree tree) {
            Map<String, Object> row = base("DECLARATION");
            row.put("declarationKind", kind);
            row.put("symbolIdentity", identity);
            row.put("ownerIdentity", owner);
            if (descriptor != null) {
                row.put("jvmDescriptor", descriptor);
            }
            row.put("modifiers", element.getModifiers().stream()
                    .map(Modifier::name).sorted().toList());
            row.put("annotations", annotations(element));
            if (element instanceof ExecutableElement method && element.getKind() == ElementKind.METHOD) {
                row.put("spring", new SpringReader().read(method));
            }
            anchor(row, tree);
            row.put("resolution", "COMPILER_EXACT");
            return row;
        }

        private void relation(String kind, Element target, Tree tree) {
            if (executableOwners.isEmpty()) {
                return;
            }
            if (!(target instanceof ExecutableElement executable)) {
                boundary("JAVA_CALL_TARGET_UNRESOLVED", tree);
                return;
            }
            String descriptor = executableDescriptor(executable);
            if (descriptor == null) {
                boundary("JAVA_CALL_DESCRIPTOR_UNRESOLVED", tree);
                return;
            }
            String name = executable.getKind() == ElementKind.CONSTRUCTOR
                    ? "<init>" : executable.getSimpleName().toString();
            Map<String, Object> row = base("RELATION");
            row.put("relationKind", kind);
            row.put("sourceIdentity", executableOwners.peek());
            row.put("targetIdentity", "method:" + ownerOf(executable) + "#" + name + descriptor);
            anchor(row, tree);
            row.put("resolution", "COMPILER_EXACT");
            facts.add(row);
        }

        private void typeUse(Element target, Tree tree) {
            if (owners.isEmpty() || !(target instanceof TypeElement type)) {
                return;
            }
            String source = executableOwners.isEmpty() ? owners.peek() : executableOwners.peek();
            String targetIdentity = classIdentity(type);
            if (source.equals(targetIdentity)) {
                return;
            }
            Map<String, Object> row = base("RELATION");
            row.put("relationKind", "TYPE_USES");
            row.put("sourceIdentity", source);
            row.put("targetIdentity", targetIdentity);
            anchor(row, tree);
            row.put("resolution", "COMPILER_EXACT");
            facts.add(row);
        }

        private void boundary(String code, Tree tree) {
            Map<String, Object> row = base("BOUNDARY");
            row.put("code", code);
            row.put("requiredChecks", List.of("VERIFY_JAVA_COMPILER_RESOLUTION"));
            row.put("resolution", "UNKNOWN");
            anchor(row, tree);
            facts.add(row);
        }

        private void anchor(Map<String, Object> row, Tree tree) {
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            try {
                Path source = Path.of(unit.getSourceFile().toUri()).toRealPath();
                if (!source.startsWith(root)) {
                    throw new IllegalStateException("source escaped root");
                }
                row.put("file", relative(root, source));
            } catch (Exception failure) {
                throw new IllegalStateException("source authority unavailable", failure);
            }
            long start = positions.getStartPosition(unit, tree);
            long end = positions.getEndPosition(unit, tree);
            if (start >= 0) {
                row.put("start", start);
            }
            if (end >= start && end >= 0) {
                row.put("end", end);
            }
        }

        private List<String> annotations(Element element) {
            return element.getAnnotationMirrors().stream()
                    .map(AnnotationMirror::getAnnotationType)
                    .map(this::typeIdentity)
                    .sorted()
                    .toList();
        }

        private record SpringBinding(String annotation, List<String> chain, Map<String, Object> attributes) {}

        /** Interpret resolved annotation identities without claiming live Spring registration. */
        private final class SpringReader {
            private static final String WEB = "org.springframework.web.bind.annotation.";
            private static final String KAFKA = "org.springframework.kafka.annotation.";
            private static final String SCHEDULE = "org.springframework.scheduling.annotation.";
            private static final String CONTROLLER = "org.springframework.stereotype.Controller";
            private static final String FEIGN_CLIENT = "org.springframework.cloud.openfeign.FeignClient";
            private static final String ALIAS = "org.springframework.core.annotation.AliasFor";
            private final Set<String> boundaries = new TreeSet<>();
            private int visits;

            Map<String, Object> read(ExecutableElement method) {
                return metadata(readEntries(method, (TypeElement) method.getEnclosingElement()));
            }

            private List<Map<String, Object>> readEntries(ExecutableElement method, TypeElement owner) {
                List<TypeElement> hierarchy = hierarchy(owner);
                List<List<SpringBinding>> classBindings = hierarchy.stream().map(this::expandAll).toList();
                List<SpringBinding> bindings = new ArrayList<>(expandAll(method));
                Set<String> directFamilies = new TreeSet<>();
                bindings.forEach(binding -> directFamilies.add(binding.annotation()));
                Set<String> inheritedFamilies = new TreeSet<>();
                for (TypeElement type : hierarchy) {
                    if (type.equals(owner)) continue;
                    for (Element element : type.getEnclosedElements()) {
                        if (element instanceof ExecutableElement base && base.getKind() == ElementKind.METHOD
                                && elements.overrides(method, base, owner)) {
                            List<SpringBinding> inherited = expandAll(base);
                            for (SpringBinding binding : inherited) {
                                if (!directFamilies.contains(binding.annotation()) && !inheritedFamilies.contains(binding.annotation())) {
                                    bindings.add(binding);
                                }
                            }
                            inherited.forEach(binding -> inheritedFamilies.add(binding.annotation()));
                        }
                    }
                }
                List<SpringBinding> mappings = family(bindings, WEB + "RequestMapping");
                List<SpringBinding> typeMappings = firstFamily(classBindings, WEB + "RequestMapping");
                boolean controller = classBindings.stream().flatMap(List::stream)
                        .anyMatch(binding -> binding.annotation().equals(CONTROLLER));
                boolean outboundFeign = classBindings.get(0).stream().anyMatch(binding -> binding.annotation().equals(FEIGN_CLIENT)) && !controller;
                List<Map<String, Object>> entries = new ArrayList<>();
                if (mappings.size() > 1 || typeMappings.size() > 1) boundaries.add("MULTIPLE_REQUEST_MAPPINGS_ON_ELEMENT");
                for (SpringBinding binding : mappings) {
                    if (outboundFeign) {
                        boundaries.add("OUTBOUND_FEIGN_CLIENT_NOT_SERVER_ENTRYPOINT");
                        continue;
                    }
                    Map<String, Object> entry = entry(binding, "HTTP_ENDPOINT");
                    entry.put("controller", controller);
                    entry.put("classAttributes", typeMappings.stream().map(SpringBinding::attributes).toList());
                    if (!controller) boundaries.add("CONTROLLER_REGISTRATION_UNPROVEN");
                    entries.add(entry);
                }
                family(bindings, KAFKA + "KafkaListener").forEach(binding -> entries.add(entry(binding, "KAFKA_LISTENER")));
                List<SpringBinding> handlers = family(bindings, KAFKA + "KafkaHandler");
                if (!handlers.isEmpty()) {
                    List<SpringBinding> listeners = firstFamily(classBindings, KAFKA + "KafkaListener");
                    if (listeners.isEmpty()) boundaries.add("KAFKA_HANDLER_WITHOUT_CLASS_LISTENER");
                    for (SpringBinding binding : listeners) {
                        Map<String, Object> entry = entry(binding, "KAFKA_LISTENER");
                        entry.put("handlerAttributes", handlers.get(0).attributes());
                        entries.add(entry);
                    }
                }
                family(bindings, SCHEDULE + "Scheduled").forEach(binding -> entries.add(entry(binding, "SCHEDULED_JOB")));
                if (!entries.isEmpty() && method.getModifiers().contains(Modifier.ABSTRACT)) {
                    boundaries.add("ABSTRACT_HANDLER_REQUIRES_IMPLEMENTATION");
                }
                return entries;
            }

            private Map<String, Object> metadata(List<Map<String, Object>> entries) {
                return Map.of("schema", "spring-entrypoints/0.1", "authority", "JAVAC_RESOLVED_ANNOTATIONS",
                        "entries", entries, "boundaries", new ArrayList<>(boundaries));
            }

            Map<String, Object> readInherited(TypeElement owner) {
                List<Map<String, Object>> entries = new ArrayList<>();
                if (owner.getModifiers().contains(Modifier.ABSTRACT) || owner.getKind().isInterface()) return metadata(entries);
                int members = 0;
                Set<String> targets = new TreeSet<>();
                for (Element member : elements.getAllMembers(owner)) {
                    if (++members > 4096) { boundaries.add("INHERITED_MEMBER_LIMIT"); break; }
                    if (!(member instanceof ExecutableElement method) || member.getKind() != ElementKind.METHOD
                            || member.getEnclosingElement().equals(owner) || method.getModifiers().contains(Modifier.ABSTRACT)
                            || method.getEnclosingElement().toString().equals("java.lang.Object")) continue;
                    List<Map<String, Object>> inherited = readEntries(method, owner);
                    if (inherited.isEmpty()) continue;
                    String descriptor = executableDescriptor(method);
                    if (descriptor == null) { boundaries.add("INHERITED_HANDLER_DESCRIPTOR_UNRESOLVED"); continue; }
                    String target = "method:" + ownerOf(method) + "#" + method.getSimpleName() + descriptor;
                    if (!targets.add(target)) continue;
                    if (trees.getPath(method) == null) boundaries.add("INHERITED_HANDLER_SOURCE_UNAVAILABLE");
                    for (Map<String, Object> entry : inherited) {
                        entry.put("targetSymbol", target);
                        entry.put("beanClass", classIdentity(owner));
                        entries.add(entry);
                    }
                }
                return metadata(entries);
            }

            private List<TypeElement> hierarchy(TypeElement owner) {
                List<TypeElement> result = new ArrayList<>();
                Deque<TypeElement> queue = new ArrayDeque<>();
                Set<String> seen = new TreeSet<>();
                queue.add(owner);
                while (!queue.isEmpty()) {
                    TypeElement next = queue.removeFirst();
                    if (!seen.add(next.getQualifiedName().toString())) continue;
                    if (seen.size() > 128) { boundaries.add("TYPE_HIERARCHY_LIMIT"); break; }
                    result.add(next);
                    for (TypeMirror supertype : types.directSupertypes(next.asType())) {
                        if (types.asElement(supertype) instanceof TypeElement type) queue.add(type);
                    }
                }
                return result;
            }

            private List<SpringBinding> family(List<SpringBinding> bindings, String name) {
                return bindings.stream().filter(binding -> binding.annotation().equals(name)).toList();
            }

            private List<SpringBinding> firstFamily(List<List<SpringBinding>> classes, String name) {
                return classes.stream().map(bindings -> family(bindings, name))
                        .filter(bindings -> !bindings.isEmpty()).findFirst().orElse(List.of());
            }

            private Map<String, Object> entry(SpringBinding binding, String kind) {
                Map<String, Object> result = new LinkedHashMap<>();
                result.put("kind", kind);
                result.put("annotation", binding.annotation());
                result.put("annotationChain", binding.chain());
                result.put("attributes", binding.attributes());
                result.put("registration", "RUNTIME_CONDITIONAL");
                return result;
            }

            private List<SpringBinding> expandAll(Element element) {
                List<SpringBinding> result = new ArrayList<>();
                for (AnnotationMirror annotation : element.getAnnotationMirrors()) result.addAll(expand(annotation, List.of()));
                return result;
            }

            private String annotationName(AnnotationMirror annotation) {
                return ((TypeElement) annotation.getAnnotationType().asElement()).getQualifiedName().toString();
            }

            private List<SpringBinding> expand(AnnotationMirror annotation, List<String> path) {
                String name = annotationName(annotation);
                if (path.contains(name) || name.startsWith("java.lang.annotation.")) return List.of();
                if (++visits > 2048 || path.size() >= 32) { boundaries.add("ANNOTATION_GRAPH_LIMIT"); return List.of(); }
                List<String> chain = new ArrayList<>(path);
                chain.add(name);
                if (name.equals(KAFKA + "KafkaListeners") || name.equals(SCHEDULE + "Schedules")) {
                    String expected = name.equals(KAFKA + "KafkaListeners") ? KAFKA + "KafkaListener" : SCHEDULE + "Scheduled";
                    List<SpringBinding> nested = new ArrayList<>();
                    for (Map.Entry<? extends ExecutableElement, ? extends AnnotationValue> member : annotation.getElementValues().entrySet()) {
                        if (!member.getKey().getSimpleName().contentEquals("value")) continue;
                        Object value = member.getValue().getValue();
                        if (value instanceof List<?> values) for (Object child : values) {
                            if (child instanceof AnnotationValue av && av.getValue() instanceof AnnotationMirror mirror
                                    && annotationName(mirror).equals(expected)) nested.addAll(expand(mirror, chain));
                            else boundaries.add("INVALID_REPEATABLE_CONTAINER");
                        }
                    }
                    if (nested.isEmpty()) boundaries.add("UNRESOLVED_REPEATABLE_CONTAINER");
                    return nested;
                }
                Map<String, Object> attributes = arguments(annotation, false);
                String method = switch (name) {
                    case WEB + "GetMapping" -> "GET";
                    case WEB + "PostMapping" -> "POST";
                    case WEB + "PutMapping" -> "PUT";
                    case WEB + "DeleteMapping" -> "DELETE";
                    case WEB + "PatchMapping" -> "PATCH";
                    default -> null;
                };
                if (method != null) {
                    attributes = aliases(attributes);
                    attributes.put("method", List.of(method));
                    return List.of(new SpringBinding(WEB + "RequestMapping", chain, attributes));
                }
                if (Set.of(WEB + "RequestMapping", KAFKA + "KafkaListener", KAFKA + "KafkaHandler", SCHEDULE + "Scheduled", CONTROLLER, FEIGN_CLIENT).contains(name)) {
                    return List.of(new SpringBinding(name, chain, name.equals(WEB + "RequestMapping") ? aliases(attributes) : attributes));
                }
                TypeElement declaration = (TypeElement) annotation.getAnnotationType().asElement();
                List<SpringBinding> metas = new ArrayList<>();
                for (AnnotationMirror meta : declaration.getAnnotationMirrors()) metas.addAll(expand(meta, chain));
                if (metas.isEmpty()) return List.of();
                Map<String, Object> effective = arguments(annotation, true);
                List<SpringBinding> result = new ArrayList<>();
                for (SpringBinding meta : metas) {
                    Map<String, Object> merged = new LinkedHashMap<>(meta.attributes());
                    for (Element member : declaration.getEnclosedElements()) {
                        if (!(member instanceof ExecutableElement)) continue;
                        String memberName = member.getSimpleName().toString();
                        for (AnnotationMirror alias : member.getAnnotationMirrors()) {
                            if (!annotationName(alias).equals(ALIAS)) continue;
                            Map<String, Object> aliasArgs = arguments(alias, false);
                            String target = String.valueOf(aliasArgs.getOrDefault("annotation", "java.lang.annotation.Annotation"));
                            String targetName = String.valueOf(aliasArgs.getOrDefault("attribute", ""));
                            if (targetName.isEmpty()) targetName = String.valueOf(aliasArgs.getOrDefault("value", ""));
                            if (targetName.isEmpty()) targetName = memberName;
                            Object selected = effective.get(memberName);
                            if (target.equals("java.lang.annotation.Annotation") || target.equals(name)) {
                                if (attributes.containsKey(memberName)) selected = attributes.get(memberName);
                                else if (attributes.containsKey(targetName)) selected = attributes.get(targetName);
                                if (attributes.containsKey(memberName) && attributes.containsKey(targetName)
                                        && !attributes.get(memberName).equals(attributes.get(targetName))) boundaries.add("CONFLICTING_ANNOTATION_ALIASES");
                                if (selected != null && merged.containsKey(targetName)) merged.put(targetName, selected);
                            } else if (target.equals(meta.annotation()) || meta.chain().contains(target)) {
                                String rootName = aliasRootAttribute(target, targetName, meta.annotation(), new TreeSet<>());
                                if (selected != null && rootName != null) merged.put(rootName, selected);
                            }
                        }
                    }
                    effective.forEach((key, value) -> { if (!key.equals("value") && merged.containsKey(key)) merged.put(key, value); });
                    result.add(new SpringBinding(meta.annotation(), meta.chain(), meta.annotation().equals(WEB + "RequestMapping") ? aliases(merged) : merged));
                }
                return result;
            }

            private String aliasRootAttribute(String annotation, String attribute, String root, Set<String> seen) {
                if (annotation.equals(root)) return root.equals(WEB + "RequestMapping") && attribute.equals("value") ? "path" : attribute;
                if (root.equals(WEB + "RequestMapping") && Set.of(WEB + "GetMapping", WEB + "PostMapping",
                        WEB + "PutMapping", WEB + "DeleteMapping", WEB + "PatchMapping").contains(annotation)) return attribute.equals("value") ? "path" : attribute;
                if (!seen.add(annotation + "#" + attribute) || seen.size() > 32) {
                    boundaries.add("UNRESOLVED_TRANSITIVE_ANNOTATION_ALIAS"); return null;
                }
                TypeElement declaration = elements.getTypeElement(annotation);
                if (declaration != null) for (Element member : declaration.getEnclosedElements()) {
                    if (!member.getSimpleName().contentEquals(attribute)) continue;
                    for (AnnotationMirror alias : member.getAnnotationMirrors()) {
                        if (!annotationName(alias).equals(ALIAS)) continue;
                        Map<String, Object> args = arguments(alias, false);
                        String target = String.valueOf(args.getOrDefault("annotation", annotation));
                        if (target.equals("java.lang.annotation.Annotation")) target = annotation;
                        String name = String.valueOf(args.getOrDefault("attribute", ""));
                        if (name.isEmpty()) name = String.valueOf(args.getOrDefault("value", ""));
                        if (name.isEmpty()) name = attribute;
                        return aliasRootAttribute(target, name, root, seen);
                    }
                    // Spring's same-name convention also applies through intermediate compositions.
                    TypeElement rootType = elements.getTypeElement(root);
                    if (rootType != null && rootType.getEnclosedElements().stream()
                            .anyMatch(candidate -> candidate.getSimpleName().contentEquals(attribute))) return attribute;
                }
                boundaries.add("UNRESOLVED_TRANSITIVE_ANNOTATION_ALIAS");
                return null;
            }

            private Map<String, Object> aliases(Map<String, Object> attributes) {
                Map<String, Object> result = new LinkedHashMap<>(attributes);
                Object value = attributes.get("value"), path = attributes.get("path");
                if (value instanceof List<?> list && list.isEmpty()) value = null;
                if (path instanceof List<?> list && list.isEmpty()) path = null;
                if (value != null && path != null && !value.equals(path)) boundaries.add("CONFLICTING_PATH_ALIASES");
                if (path != null || value != null) result.put("path", path != null ? path : value);
                result.remove("value");
                return result;
            }

            private Map<String, Object> arguments(AnnotationMirror annotation, boolean defaults) {
                Map<String, Object> result = new LinkedHashMap<>();
                var arguments = defaults ? elements.getElementValuesWithDefaults(annotation) : annotation.getElementValues();
                arguments.forEach((key, value) -> result.put(key.getSimpleName().toString(), value(value, 0)));
                return result;
            }

            private Object value(AnnotationValue annotation, int depth) {
                if (depth > 32) { boundaries.add("ANNOTATION_VALUE_LIMIT"); return null; }
                Object value = annotation.getValue();
                if (value instanceof String string) {
                    if (string.contains("${") || string.contains("#{")) boundaries.add("RUNTIME_EXPRESSION");
                    return string;
                }
                if (value instanceof Number || value instanceof Boolean) return value;
                if (value instanceof Character character) return character.toString();
                if (value instanceof VariableElement constant) return constant.getSimpleName().toString();
                if (value instanceof TypeMirror type) return type.toString();
                if (value instanceof AnnotationMirror nested) return Map.of("annotation", annotationName(nested), "attributes", arguments(nested, false));
                if (value instanceof List<?> list) {
                    List<Object> result = new ArrayList<>();
                    for (Object child : list) if (child instanceof AnnotationValue av) result.add(value(av, depth + 1));
                    return result;
                }
                boundaries.add("UNRESOLVED_ANNOTATION_VALUE");
                return null;
            }
        }

        private String executableDescriptor(ExecutableElement executable) {
            ExecutableType type = (ExecutableType) executable.asType();
            StringBuilder value = new StringBuilder("(");
            for (TypeMirror parameter : type.getParameterTypes()) {
                String descriptor = descriptor(parameter);
                if (descriptor == null) {
                    return null;
                }
                value.append(descriptor);
            }
            String result = descriptor(type.getReturnType());
            return result == null ? null : value.append(')').append(result).toString();
        }

        private String descriptor(TypeMirror type) {
            try {
                return switch (type.getKind()) {
                    case BOOLEAN -> "Z";
                    case BYTE -> "B";
                    case SHORT -> "S";
                    case INT -> "I";
                    case LONG -> "J";
                    case CHAR -> "C";
                    case FLOAT -> "F";
                    case DOUBLE -> "D";
                    case VOID -> "V";
                    case ARRAY -> "[" + descriptor(((ArrayType) type).getComponentType());
                    case DECLARED -> "L" + binaryName((TypeElement) ((DeclaredType) type)
                            .asElement()).replace('.', '/') + ";";
                    case TYPEVAR, WILDCARD, INTERSECTION -> descriptor(types.erasure(type));
                    default -> null;
                };
            } catch (RuntimeException failure) {
                return null;
            }
        }

        private String typeIdentity(TypeMirror mirror) {
            TypeMirror erased = types.erasure(mirror);
            if (erased instanceof DeclaredType declared && declared.asElement() instanceof TypeElement type) {
                return classIdentity(type);
            }
            return erased.toString();
        }

        private String classIdentity(TypeElement type) {
            return "class:" + binaryName(type);
        }

        private String binaryName(TypeElement type) {
            return elements.getBinaryName(type).toString();
        }

        private String ownerOf(Element element) {
            Element current = element.getEnclosingElement();
            while (current != null && !(current instanceof TypeElement)) {
                current = current.getEnclosingElement();
            }
            return current instanceof TypeElement type ? classIdentity(type) : "module:unnamed";
        }

        private String declarationKind(ElementKind kind) {
            return switch (kind) {
                case INTERFACE -> "INTERFACE";
                case ENUM -> "ENUM";
                case RECORD -> "RECORD";
                case ANNOTATION_TYPE -> "ANNOTATION";
                default -> "CLASS";
            };
        }
    }

    private static Map<String, Object> base(String kind) {
        Map<String, Object> row = new LinkedHashMap<>();
        row.put("schema", SCHEMA);
        row.put("kind", kind);
        return row;
    }

    private static String relative(Path root, Path source) {
        return root.relativize(source).toString().replace('\\', '/');
    }

    private static String safeToken(String value) {
        if (value == null) {
            return "UNKNOWN";
        }
        String safe = value.replaceAll("[^A-Za-z0-9_.-]", "_");
        return safe.length() > 128 ? safe.substring(0, 128) : safe;
    }

    private static String json(Object value) {
        if (value == null) {
            return "null";
        }
        if (value instanceof String string) {
            return quote(string);
        }
        if (value instanceof Number || value instanceof Boolean) {
            return value.toString();
        }
        if (value instanceof Map<?, ?> map) {
            return map.entrySet().stream()
                    .sorted(Comparator.comparing(entry -> entry.getKey().toString()))
                    .map(entry -> quote(entry.getKey().toString()) + ":" + json(entry.getValue()))
                    .reduce("{", (left, right) -> left.equals("{") ? left + right : left + "," + right)
                    + "}";
        }
        if (value instanceof Collection<?> collection) {
            return collection.stream().map(CodeclewJavaAnalyzer::json)
                    .reduce("[", (left, right) -> left.equals("[") ? left + right : left + "," + right)
                    + "]";
        }
        throw new IllegalArgumentException("unsupported JSON value");
    }

    private static String quote(String value) {
        StringBuilder result = new StringBuilder("\"");
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            switch (character) {
                case '\"' -> result.append("\\\"");
                case '\\' -> result.append("\\\\");
                case '\b' -> result.append("\\b");
                case '\f' -> result.append("\\f");
                case '\n' -> result.append("\\n");
                case '\r' -> result.append("\\r");
                case '\t' -> result.append("\\t");
                default -> {
                    if (character < 0x20) {
                        result.append(String.format("\\u%04x", (int) character));
                    } else {
                        result.append(character);
                    }
                }
            }
        }
        return result.append('\"').toString();
    }
}
