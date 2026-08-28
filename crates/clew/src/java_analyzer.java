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
        if (!"21".equals(release) || sources.isEmpty()) {
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
