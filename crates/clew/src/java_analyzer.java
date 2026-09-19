import com.sun.source.tree.AnnotationTree;
import com.sun.source.tree.BinaryTree;
import com.sun.source.tree.ConditionalExpressionTree;
import com.sun.source.tree.ThrowTree;
import com.sun.source.tree.SynchronizedTree;
import com.sun.source.tree.IfTree;
import com.sun.source.tree.LiteralTree;
import com.sun.source.tree.ReturnTree;
import com.sun.source.tree.WhileLoopTree;
import com.sun.source.tree.ForLoopTree;
import com.sun.source.tree.EnhancedForLoopTree;
import com.sun.source.tree.DoWhileLoopTree;
import com.sun.source.tree.LambdaExpressionTree;
import com.sun.source.tree.ExpressionStatementTree;
import com.sun.source.tree.SwitchTree;
import com.sun.source.tree.TryTree;
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
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.TreeMap;
import java.util.jar.JarFile;
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
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

final class CodeclewJavaAnalyzer {
    private static final String SCHEMA = "codeclew-java-compiler-fact/1.0";
    // Annotation definitions are a finite, service-wide set. Sharing them across
    // every declaration avoids re-expanding (and re-serializing) the same
    // definition once per method/class, which otherwise balloons the fact set.
    private static final Map<String, Object> GLOBAL_DEFINITIONS = new TreeMap<>();
    private static final Set<String> GLOBAL_COLLECTING = new TreeSet<>();
    // Bound the inherited-callable catalog on a class declaration so its fact
    // stays under the per-fact byte budget even for very wide hierarchies.
    private static final int INHERITED_CALLABLES_BYTES = 50_000;
    // Bound a method's documentation flow so its declaration fact stays under
    // the per-fact byte budget; a long body is truncated with a boundary.
    private static final int DOCUMENTATION_FLOW_BYTES = 45_000;
    // Bound a single annotation definition so it never makes a registry shard
    // exceed the per-fact byte budget. Oversized members are truncated and the
    // definition is explicitly marked bounded instead of silently dropped.
    private static final int DEFINITION_MEMBERS_BYTES = 60_000;

    public static void main(String[] args) throws Exception {
        // Seven fixed arguments plus optional processor options and the closed
        // no-AP execution marker and its owned empty source path.
        if (args.length != 7 && args.length != 8 && args.length != 9 && args.length != 10) {
            System.exit(2);
        }
        Path root = Path.of(args[0]).toRealPath();
        List<Path> sources = readSources(root, Path.of(args[1]));
        List<String> classpath = readLines(Path.of(args[2]));
        String release = args[3];
        // Generated-source output root. Empty means no annotation processing.
        String genDir = args[4];
        // Explicitly admitted processor class names (comma-separated). Empty
        // means processors are not authorized by name.
        String processorList = args[5];
        // Explicitly admitted processor path (resolved <annotationProcessorPaths>
        // artifacts). Empty means no processor path is admitted. When either
        // processorList or processorPath is non-empty, annotation processing is
        // enabled and its emitted sources/classes are isolated to genDir.
        String processorPath = args[6];
        // Explicitly admitted processor options (newline-separated `-A...`).
        // Empty means no processor options are surfaced to the analyzer. They
        // only reach a processor that was explicitly admitted by name/path.
        String processorOptions = args.length > 7 ? args[7] : "";
        boolean closedNoAp = args.length == 10 && "CLOSED_NO_AP".equals(args[8]);
        String closedEmptySourcePath = args.length == 10 ? args[9] : "";
        if (args.length >= 9 && !closedNoAp) {
            System.exit(2);
        }
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
                    "--release", release, "-implicit:none", "-Xlint:none"));
            if (closedNoAp) {
                if (!processorList.isEmpty() || !processorPath.isEmpty() || !processorOptions.isEmpty()) {
                    System.exit(2);
                }
                rejectJarManifestClassPath(classpath);
                // Empty SOURCE_PATH and the exact admitted CLASS_PATH are
                // installed through the file manager so javac cannot fall
                // back to the process working directory or ambient lookup.
                files.setLocationFromPaths(StandardLocation.SOURCE_PATH, List.of());
                files.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath.stream()
                        .map(Path::of)
                        .toList());
                options.add("-sourcepath");
                options.add(closedEmptySourcePath);
            }
            boolean processing = !processorList.isEmpty() || !processorPath.isEmpty();
            if (!processing) {
                // No processors are admitted: arbitrary service-discovered
                // processors are not authorized to run or mutate the input tree.
                options.add("-proc:none");
            } else {
                // Only explicitly admitted processors run, and their emitted
                // sources/classes are isolated to the disposable genDir, never
                // the repository input tree. Only parse()/analyze() run, so no
                // bytecode is emitted into the repository.
                if (!processorList.isEmpty()) {
                    options.add("-processor");
                    options.add(processorList);
                }
                if (!processorPath.isEmpty()) {
                    options.add("-processorpath");
                    options.add(processorPath);
                }
                options.add("-s");
                options.add(genDir);
                options.add("-d");
                options.add(genDir + "/classes");
                // Surface only explicitly admitted processor options; they are
                // part of model identity and reach the admitted processor only.
                for (String option : processorOptions.split("\n")) {
                    if (!option.isEmpty()) {
                        options.add(option);
                    }
                }
            }
            if (closedNoAp || !classpath.isEmpty()) {
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
        // The annotation-definition registry is a bounded, service-wide set. It is
        // emitted once here (in shards below the per-fact byte budget); every
        // declaration reattaches it before framework interpretation.
        for (Map<String, Object> shard : registryShards()) {
            canonical.add(json(shard));
        }
        canonical.forEach(System.out::println);
    }

    private static List<Map<String, Object>> registryShards() {
        // Keep each shard under the adapter's per-fact byte budget (64 KiB) so
        // the shared catalog is not rejected as an oversized single fact.
        final int budget = 20_000;
        List<Map<String, Object>> shards = new ArrayList<>();
        Map<String, Object> shard = new TreeMap<>();
        for (Map.Entry<String, Object> entry : GLOBAL_DEFINITIONS.entrySet()) {
            Map<String, Object> candidate = new TreeMap<>(shard);
            candidate.put(entry.getKey(), entry.getValue());
            if (!shard.isEmpty() && utf8(registry(candidate)) >= budget) {
                // Copy the completed shard: the registry must retain an
                // independent map so a later shard.clear() cannot empty it.
                shards.add(registry(new TreeMap<>(shard)));
                shard.clear();
            }
            shard.put(entry.getKey(), entry.getValue());
        }
        if (!shard.isEmpty()) {
            shards.add(registry(new TreeMap<>(shard)));
        }
        return shards;
    }

    private static int utf8(Map<String, Object> value) {
        return json(value).getBytes(StandardCharsets.UTF_8).length;
    }

    private static Map<String, Object> registry(Map<String, Object> definitions) {
        Map<String, Object> registry = new LinkedHashMap<>();
        registry.put("kind", "ANNOTATION_REGISTRY");
        registry.put("schema", SCHEMA);
        registry.put("authority", "JAVAC_RESOLVED_ANNOTATIONS");
        registry.put("definitions", definitions);
        return registry;
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
        if (result.size() != new TreeSet<>(result).size()) {
            throw new IOException("duplicate source authority");
        }
        return result;
    }

    private static void rejectJarManifestClassPath(List<String> classpath) throws IOException {
        for (String entry : classpath) {
            Path path = Path.of(entry);
            if (!Files.isRegularFile(path)) {
                continue;
            }
            byte[] magic = new byte[4];
            int read;
            try (var input = Files.newInputStream(path)) {
                read = input.read(magic);
            }
            boolean zipMagic = read >= 4
                    && magic[0] == 'P'
                    && magic[1] == 'K'
                    && ((magic[2] == 3 && magic[3] == 4)
                        || (magic[2] == 5 && magic[3] == 6)
                        || (magic[2] == 7 && magic[3] == 8));
            boolean jarNamed = path.getFileName().toString().toLowerCase(Locale.ROOT).endsWith(".jar");
            if (!zipMagic && !jarNamed) {
                continue;
            }
            try (JarFile jar = new JarFile(path.toFile(), false)) {
                if (jar.getManifest() != null
                        && jar.getManifest().getMainAttributes().getValue("Class-Path") != null) {
                    throw new IOException("JAR manifest Class-Path expansion is outside closed authority");
                }
            } catch (java.util.zip.ZipException error) {
                throw new IOException("closed Java classpath archive is malformed", error);
            }
        }
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
        private final Map<CompilationUnitTree, int[]> utf8Offsets = new IdentityHashMap<>();

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
            row.put("jvmAnnotations", new JvmAnnotationReader().readInherited(type));
            owners.push(identity);
            super.visitClass(tree, unused);
            owners.pop();
            return null;
        }

        @Override
        public Void visitMethod(MethodTree tree, Void unused) {
            // javac inserts default constructors and other synthetic methods.
            // They have no exact source range and must not become source cards.
            if (!hasSourceRange(tree)) {
                return null;
            }
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
            Map<String, Object> declaration = declaration(
                    executable.getKind() == ElementKind.CONSTRUCTOR ? "CONSTRUCTOR" : "METHOD",
                    identity, owner, descriptor, executable, tree);
            if (tree.getBody() != null) {
                declaration.put("documentation", new DocumentationFlow().read(tree, executable));
            }
            facts.add(declaration);
            executableOwners.push(identity);
            super.visitMethod(tree, unused);
            executableOwners.pop();
            return null;
        }

        @Override
        public Void visitVariable(VariableTree tree, Void unused) {
            if (!hasSourceRange(tree)) {
                return null;
            }
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

        /** Bounded source structure, not a runtime trace or a general control-flow proof. */
        private final class DocumentationFlow extends TreePathScanner<Void, Void> {
            private final List<Map<String, Object>> events = new ArrayList<>();
            private final Set<String> boundaries = new TreeSet<>();
            private int groups = 0;

            private Map<String, Object> read(MethodTree tree, ExecutableElement method) {
                scan(new TreePath(getCurrentPathOfAnalyzer(), tree.getBody()), null);
                Map<String, Object> result = new LinkedHashMap<>();
                result.put("schema", "codeclew-java-documentation-flow/1.0");
                result.put("authority", "JAVAC_SOURCE_STRUCTURE");
                result.put("parameterTypes", method.getParameters().stream()
                        .map(p -> types.erasure(p.asType()).toString()).toList());
                result.put("events", events);
                result.put("boundaries", new ArrayList<>(boundaries));
                if (utf8(result) > DOCUMENTATION_FLOW_BYTES) {
                    // Bound the flow so the enclosing declaration fact stays under
                    // the per-fact byte budget; retain the source-order prefix.
                    List<Map<String, Object>> kept = new ArrayList<>();
                    int bytes = 0;
                    for (Map<String, Object> row : events) {
                        int cost = utf8(row) + 2;
                        if (!kept.isEmpty() && bytes + cost > DOCUMENTATION_FLOW_BYTES) {
                            break;
                        }
                        kept.add(row);
                        bytes += cost;
                    }
                    boundaries.add("DOCUMENTATION_FLOW_BYTE_BUDGET");
                    result.put("events", kept);
                    result.put("boundaries", new ArrayList<>(boundaries));
                }
                return result;
            }

            private Map<String, Object> event(String kind, Tree tree) {
                Map<String, Object> row = new LinkedHashMap<>();
                row.put("kind", kind);
                if (hasSourceRange(tree)) anchor(row, tree);
                if (events.size() < 512) events.add(row);
                else boundaries.add("DOCUMENTATION_FLOW_EVENT_BUDGET");
                return row;
            }

            @Override public Void visitClass(ClassTree tree, Void unused) {
                boundaries.add("LOCAL_CLASS_BODY_NOT_EXPANDED"); return null;
            }
            @Override public Void visitLambdaExpression(LambdaExpressionTree tree, Void unused) {
                boundaries.add("LAMBDA_EXECUTION_NOT_EXPANDED"); event("BOUNDARY", tree); return null;
            }
            @Override public Void visitIf(IfTree tree, Void unused) {
                scan(tree.getCondition(), null);
                Map<String, Object> branch = event("IF", tree.getCondition());
                branch.put("condition", tree.getCondition().toString());
                branch.put("group", ++groups);
                scan(tree.getThenStatement(), null);
                if (tree.getElseStatement() != null) {
                    event("ELSE", tree.getElseStatement()); scan(tree.getElseStatement(), null);
                }
                event("END", tree); return null;
            }
            @Override public Void visitWhileLoop(WhileLoopTree tree, Void unused) {
                event("LOOP", tree.getCondition()).put("condition", tree.getCondition().toString());
                scan(tree.getCondition(), null); scan(tree.getStatement(), null); event("END", tree); return null;
            }
            @Override public Void visitForLoop(ForLoopTree tree, Void unused) {
                scan(tree.getInitializer(), null); event("LOOP", tree).put("condition", "for-loop");
                scan(tree.getCondition(), null); scan(tree.getStatement(), null); scan(tree.getUpdate(), null);
                event("END", tree); return null;
            }
            @Override public Void visitEnhancedForLoop(EnhancedForLoopTree tree, Void unused) {
                scan(tree.getExpression(), null); event("LOOP", tree).put("condition", "for-each");
                scan(tree.getStatement(), null); event("END", tree); return null;
            }
            @Override public Void visitDoWhileLoop(DoWhileLoopTree tree, Void unused) {
                event("LOOP", tree).put("condition", "do-while"); scan(tree.getStatement(), null);
                scan(tree.getCondition(), null); event("END", tree); return null;
            }
            @Override public Void visitSwitch(SwitchTree tree, Void unused) {
                boundaries.add("SWITCH_FLOW_REQUIRES_SOURCE_REVIEW"); event("BOUNDARY", tree); return null;
            }
            @Override public Void visitTry(TryTree tree, Void unused) {
                boundaries.add("EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW"); event("BOUNDARY", tree); return null;
            }
            @Override public Void visitReturn(ReturnTree tree, Void unused) {
                scan(tree.getExpression(), null); event("RETURN", tree); return null;
            }
            @Override public Void visitVariable(VariableTree tree, Void unused) {
                scan(tree.getInitializer(), null); event("LOCAL", tree); return null;
            }
            @Override public Void visitExpressionStatement(ExpressionStatementTree tree, Void unused) {
                scan(tree.getExpression(), null);
                if (!(tree.getExpression() instanceof MethodInvocationTree)) event("STATEMENT", tree);
                return null;
            }
            @Override public Void visitBinary(BinaryTree tree, Void unused) {
                if (tree.getKind() == Tree.Kind.CONDITIONAL_AND || tree.getKind() == Tree.Kind.CONDITIONAL_OR) {
                    boundaries.add("SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"); event("BOUNDARY", tree); return null;
                }
                return super.visitBinary(tree, unused);
            }
            @Override public Void visitConditionalExpression(ConditionalExpressionTree tree, Void unused) {
                boundaries.add("TERNARY_FLOW_REQUIRES_SOURCE_REVIEW"); event("BOUNDARY", tree); return null;
            }
            @Override public Void visitSynchronized(SynchronizedTree tree, Void unused) {
                boundaries.add("SYNCHRONIZED_FLOW_REQUIRES_SOURCE_REVIEW"); event("BOUNDARY", tree); return null;
            }
            @Override public Void visitThrow(ThrowTree tree, Void unused) {
                scan(tree.getExpression(), null); event("THROW", tree); return null;
            }
            @Override public Void visitNewClass(NewClassTree tree, Void unused) {
                super.visitNewClass(tree, unused);
                Map<String, Object> row = event("CONSTRUCT", tree);
                Element target = trees.getElement(getCurrentPath());
                if (target instanceof ExecutableElement constructor) {
                    String descriptor = executableDescriptor(constructor);
                    if (descriptor != null) {
                        row.put("target", "method:" + ownerOf(constructor) + "#<init>" + descriptor);
                        row.put("resolution", "COMPILER_EXACT");
                    } else boundaries.add("DOCUMENTATION_CONSTRUCTOR_DESCRIPTOR_UNRESOLVED");
                } else boundaries.add("DOCUMENTATION_CONSTRUCTOR_UNRESOLVED");
                return null;
            }
            @Override public Void visitMethodInvocation(MethodInvocationTree tree, Void unused) {
                // Argument/receiver calls are evaluated before this invocation.
                super.visitMethodInvocation(tree, unused);
                Map<String, Object> row = event("CALL", tree);
                Element target = trees.getElement(getCurrentPath());
                if (target instanceof ExecutableElement method) {
                    String descriptor = executableDescriptor(method);
                    if (descriptor != null) {
                        row.put("target", "method:" + ownerOf(method) + "#" + method.getSimpleName() + descriptor);
                        row.put("resolution", "COMPILER_EXACT");
                        row.put("http", http(tree, method));
                    } else boundaries.add("DOCUMENTATION_CALL_DESCRIPTOR_UNRESOLVED");
                } else boundaries.add("DOCUMENTATION_CALL_TARGET_UNRESOLVED");
                return null;
            }

            private Map<String, Object> http(MethodInvocationTree call, ExecutableElement method) {
                Map<String, Object> result = new LinkedHashMap<>();
                String owner = ownerOf(method);
                String name = method.getSimpleName().toString();
                if (owner.equals("class:org.springframework.web.client.RestTemplate")) {
                    String verb = switch (name) {
                        case "postForObject", "postForEntity" -> "POST";
                        case "getForObject", "getForEntity" -> "GET";
                        case "put" -> "PUT";
                        case "delete" -> "DELETE";
                        default -> null;
                    };
                    if (verb == null || call.getArguments().isEmpty()) return result;
                    result.put("adapter", "SPRING_REST_TEMPLATE_LITERAL_SUFFIX/1.0");
                    result.put("method", verb);
                    Tree uri = call.getArguments().get(0);
                    if (uri instanceof BinaryTree binary && binary.getKind() == Tree.Kind.PLUS
                            && binary.getRightOperand() instanceof LiteralTree suffix
                            && suffix.getValue() instanceof String path && path.startsWith("/")) {
                        result.put("path", path);
                        Element base = trees.getElement(new TreePath(getCurrentPath(), binary.getLeftOperand()));
                        if (base instanceof VariableElement variable) {
                            for (AnnotationMirror annotation : variable.getAnnotationMirrors()) {
                                if (!annotation.getAnnotationType().toString().equals("org.springframework.beans.factory.annotation.Value")) continue;
                                for (AnnotationValue value : annotation.getElementValues().values()) {
                                    if (value.getValue() instanceof String expression && expression.startsWith("${")
                                            && expression.endsWith("}")) {
                                        String key = expression.substring(2, expression.length() - 1).split(":", 2)[0];
                                        result.put("destinationConfigKey", key);
                                    }
                                }
                            }
                        }
                    }
                    if (!result.containsKey("path")) result.put("boundary", "DYNAMIC_OR_UNSUPPORTED_CLIENT_URL");
                    return result;
                }
                if (owner.equals("class:org.springframework.web.client.RestClient")) {
                    String verb = switch (name) {
                        case "get" -> "GET";
                        case "post" -> "POST";
                        case "put" -> "PUT";
                        case "delete" -> "DELETE";
                        case "patch" -> "PATCH";
                        case "head" -> "HEAD";
                        case "options" -> "OPTIONS";
                        case "method" -> httpMethodConstant(call);
                        default -> null;
                    };
                    if (verb == null) return result;
                    Tree uri = restClientUri(call);
                    if (uri == null) return result;
                    result.put("adapter", "SPRING_REST_CLIENT_URI/1.0");
                    result.put("method", verb);
                    // Separate a normalized request path from authority/host: a
                    // literal relative path (or URI template) is kept as the path;
                    // a literal absolute URL is split so its authority never
                    // masquerades as a route path. URI templates remain templates,
                    // never exact concrete routes. A configured client or a request
                    // specification without a resolvable uri is not an executed
                    // request and produces no egress (restClientUri returns null).
                    if (uri instanceof LiteralTree literal && literal.getValue() instanceof String value) {
                        if (value.startsWith("/")) {
                            result.put("path", value);
                        } else if (value.startsWith("http://") || value.startsWith("https://")) {
                            int scheme = value.indexOf("://") + 3;
                            int slash = value.indexOf('/', scheme);
                            if (slash < 0) {
                                result.put("authority", value.substring(scheme));
                                result.put("path", "/");
                            } else {
                                result.put("authority", value.substring(scheme, slash));
                                result.put("path", value.substring(slash));
                            }
                        } else {
                            result.put("boundary", "DYNAMIC_OR_UNSUPPORTED_CLIENT_URL");
                        }
                    } else {
                        result.put("boundary", "DYNAMIC_OR_UNSUPPORTED_CLIENT_URL");
                    }
                    return result;
                }
                return result;
            }

            /** Resolve the HTTP verb from a statically-typed `method(HttpMethod.CONSTANT)`
             *  argument (the `HttpMethod` enum constant name). Non-constant or
             *  unrecognized arguments return null so no egress is claimed. */
            private String httpMethodConstant(MethodInvocationTree call) {
                if (call.getArguments().isEmpty()) return null;
                Tree arg = call.getArguments().get(0);
                if (arg instanceof MemberSelectTree select) {
                    String id = select.getIdentifier().toString();
                    switch (id) {
                        case "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS" -> {
                            return id;
                        }
                        default -> {
                            return null;
                        }
                    }
                }
                return null;
            }

            /** For `restClient.verb().uri(url)` (and the longer fluent chains
             *  `client.post().uri(url).retrieve().body(...)`), return the `url`
             *  argument tree. The verb call (`client.post()`) sits beneath a
             *  `MemberSelect("uri")` whose parent is the `uri(...)` invocation,
             *  so we walk through an intervening MemberSelect before matching the
             *  receiver/selector identity. Older forms where the uri invocation is
             *  the direct parent are also accepted. */
            private Tree restClientUri(MethodInvocationTree verbCall) {
                TreePath path = getCurrentPath().getParentPath();
                if (path == null) return null;
                Tree leaf = path.getLeaf();
                if (leaf instanceof MemberSelectTree select) {
                    if (!select.getIdentifier().contentEquals("uri")) return null;
                    path = path.getParentPath();
                    if (path == null) return null;
                    leaf = path.getLeaf();
                }
                if (leaf instanceof MethodInvocationTree chain
                        && chain.getMethodSelect() instanceof MemberSelectTree select
                        && select.getIdentifier().contentEquals("uri")
                        && !chain.getArguments().isEmpty()) {
                    return chain.getArguments().get(0);
                }
                return null;
            }
        }

        private TreePath getCurrentPathOfAnalyzer() { return getCurrentPath(); }

        private Map<String, Object> declaration(
                String kind,
                String identity,
                String owner,
                String descriptor,
                Element element,
                Tree tree) {
            Map<String, Object> row = base("DECLARATION");
            row.put("declarationKind", kind);
            row.put("name", element.getSimpleName().toString());
            if (element instanceof TypeElement type) {
                row.put("qualifiedName", type.getQualifiedName().toString());
            }
            row.put("symbolIdentity", identity);
            row.put("ownerIdentity", owner);
            if (descriptor != null) {
                row.put("jvmDescriptor", descriptor);
            }
            row.put("modifiers", element.getModifiers().stream()
                    .map(Modifier::name).sorted().toList());
            row.put("annotations", annotations(element));
            if (element instanceof ExecutableElement method && element.getKind() == ElementKind.METHOD) {
                row.put("jvmAnnotations", new JvmAnnotationReader().read(method));
            }
            anchor(row, tree);
            row.put("resolution", "COMPILER_EXACT");
            return row;
        }

        private void relation(String kind, Element target, Tree tree) {
            if (executableOwners.isEmpty() || !hasSourceRange(tree)) {
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

        private boolean hasSourceRange(Tree tree) {
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            long start = positions.getStartPosition(unit, tree);
            long end = positions.getEndPosition(unit, tree);
            return start >= 0 && end >= start;
        }

        private int[] byteOffsets(CompilationUnitTree unit) {
            return utf8Offsets.computeIfAbsent(unit, key -> {
                try {
                    String text = key.getSourceFile().getCharContent(true).toString();
                    int[] offsets = new int[text.length() + 1];
                    int bytes = 0;
                    for (int i = 0; i < text.length();) {
                        int codePoint = text.codePointAt(i);
                        offsets[i] = bytes;
                        if (Character.charCount(codePoint) == 2) {
                            offsets[i + 1] = -1;
                        }
                        bytes += codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
                        i += Character.charCount(codePoint);
                    }
                    offsets[text.length()] = bytes;
                    return offsets;
                } catch (IOException failure) {
                    throw new IllegalStateException("source coordinates unavailable", failure);
                }
            });
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
            if (start >= 0 && end >= start) {
                row.put("startLine", unit.getLineMap().getLineNumber(start));
                row.put("endLine", unit.getLineMap().getLineNumber(Math.max(start, end - 1)));
                int[] offsets = byteOffsets(unit);
                row.put("byteStart", offsets[Math.toIntExact(start)]);
                row.put("byteEnd", offsets[Math.toIntExact(end)]);
            }
        }

        private List<String> annotations(Element element) {
            return element.getAnnotationMirrors().stream()
                    .map(AnnotationMirror::getAnnotationType)
                    .map(this::typeIdentity)
                    .sorted()
                    .toList();
        }

        /** Exports only compiler observations; framework rules run over the sealed result. */
        private final class JvmAnnotationReader {
            private final Map<String, Object> definitions = GLOBAL_DEFINITIONS;
            private final Set<String> collecting = GLOBAL_COLLECTING;
            private final Set<String> boundaries = new TreeSet<>();
            private int depth;
            private int visits;

            Map<String, Object> read(ExecutableElement method) {
                TypeElement owner = (TypeElement) method.getEnclosingElement();
                return finish(methodIdentity(method), List.of(callable(method, owner, false)), owner);
            }

            Map<String, Object> readInherited(TypeElement owner) {
                List<Map<String, Object>> callables = new ArrayList<>();
                int callablesBytes = 0;
                if (!owner.getModifiers().contains(Modifier.ABSTRACT) && !owner.getKind().isInterface()) {
                    Set<String> seen = new TreeSet<>();
                    int members = 0;
                    for (Element member : elements.getAllMembers(owner)) {
                        if (++members > 4096) { boundaries.add("INHERITED_MEMBER_LIMIT"); break; }
                        if (!(member instanceof ExecutableElement method) || member.getKind() != ElementKind.METHOD
                                || member.getEnclosingElement().equals(owner) || method.getModifiers().contains(Modifier.ABSTRACT)
                                || member.getEnclosingElement().toString().equals("java.lang.Object")) continue;
                        if (executableDescriptor(method) == null) { boundaries.add("INHERITED_CALLABLE_IDENTITY_UNRESOLVED"); continue; }
                        if (!seen.add(methodIdentity(method))) continue;
                        Map<String, Object> candidate = callable(method, owner, true);
                        int cost = utf8(candidate);
                        if (!callables.isEmpty() && callablesBytes + cost > INHERITED_CALLABLES_BYTES) {
                            boundaries.add("INHERITED_CALLABLES_TRUNCATED");
                            break;
                        }
                        callables.add(candidate);
                        callablesBytes += cost;
                    }
                }
                return finish(classIdentity(owner), callables, owner);
            }

            private Map<String, Object> finish(String identity, List<Map<String, Object>> callables, TypeElement owner) {
                // Definitions and the class hierarchy are emitted once as a shared
                // registry; consumers reattach them before framework interpretation.
                return Map.of("schema", "jvm-annotation-facts/1.0", "authority", "JAVAC_RESOLVED_ANNOTATIONS",
                        "declaration", identity, "definitions", Map.of(), "types", List.of(), "callables", callables,
                        "boundaries", new ArrayList<>(boundaries),
                        "coverage", Map.of("status", boundaries.isEmpty() ? "COMPLETE" : "PARTIAL", "scope", "REACHABLE_ANNOTATIONS_AND_HIERARCHY"));
            }

            private String methodIdentity(ExecutableElement method) {
                String descriptor = executableDescriptor(method);
                return "method:" + ownerOf(method) + "#" + method.getSimpleName() + (descriptor == null ? "" : descriptor);
            }

            private Map<String, Object> callable(ExecutableElement method, TypeElement owner, boolean inherited) {
                List<Map<String, Object>> bases = new ArrayList<>();
                for (TypeElement type : hierarchy(owner)) {
                    if (type.equals(owner)) continue;
                    for (Element element : type.getEnclosedElements()) {
                        if (element instanceof ExecutableElement base && base.getKind() == ElementKind.METHOD
                                && elements.overrides(method, base, owner)) bases.add(method(base, List.of()));
                    }
                }
                // The callable carries the full annotated type hierarchy so Spring consumers
                // resolve interface/superclass route prefixes and inherited context.
                return Map.of("method", method(method, bases), "classes", types(owner), "beanClass", classIdentity(owner),
                        "abstractMethod", method.getModifiers().contains(Modifier.ABSTRACT), "inherited", inherited,
                        "implementationSource", trees.getPath(method) != null);
            }

            private Map<String, Object> method(ExecutableElement method, List<Map<String, Object>> bases) {
                return Map.of("identity", methodIdentity(method), "annotations", annotationUses(method), "overrides", bases);
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

            private List<Map<String, Object>> types(TypeElement owner) {
                return hierarchy(owner).stream().map(this::typeRow).toList();
            }

            private Map<String, Object> typeRow(TypeElement type) {
                Map<String, Object> row = new LinkedHashMap<>();
                row.put("identity", classIdentity(type));
                row.put("annotations", annotationUses(type));
                row.put("directSupertypes", types.directSupertypes(type.asType()).stream().map(Object::toString).toList());
                return row;
            }

            private Map<String, Object> origin(Element element, AnnotationMirror annotation) {
                Map<String, Object> value = new LinkedHashMap<>();
                value.put("kind", "BINARY");
                value.put("identity", element.toString());
                TreePath path = trees.getPath(element);
                Tree tree = annotation == null ? trees.getTree(element) : trees.getTree(element, annotation);
                if (path != null && tree != null) {
                    long start = positions.getStartPosition(path.getCompilationUnit(), tree);
                    long end = positions.getEndPosition(path.getCompilationUnit(), tree);
                    if (start >= 0 && end >= start) {
                        value.put("kind", "SOURCE"); value.put("start", start); value.put("end", end);
                    }
                }
                return value;
            }

            private List<Map<String, Object>> annotationUses(Element element) {
                List<Map<String, Object>> result = new ArrayList<>();
                for (AnnotationMirror annotation : element.getAnnotationMirrors()) {
                    Map<String, Object> use = annotation(annotation, element);
                    if (use != null) result.add(use);
                }
                return result;
            }

            private Map<String, Object> annotation(AnnotationMirror annotation, Element owner) {
                if (++visits > 32768 || depth >= 32) { boundaries.add("ANNOTATION_GRAPH_LIMIT"); return null; }
                TypeElement declaration = (TypeElement) annotation.getAnnotationType().asElement();
                depth++;
                try {
                    definition(declaration);
                    Map<String, Object> arguments = new TreeMap<>();
                    annotation.getElementValues().forEach((key, argument) -> arguments.put(key.getSimpleName().toString(), value(argument, owner, 0)));
                    return Map.of("typeName", declaration.getQualifiedName().toString(), "arguments", arguments, "origin", origin(owner, annotation));
                } finally { depth--; }
            }

            private void definition(TypeElement declaration) {
                String id = declaration.getQualifiedName().toString();
                if (definitions.containsKey(id) || !collecting.add(id)) return;
                if (definitions.size() + collecting.size() > 2048) { boundaries.add("ANNOTATION_DEFINITION_LIMIT"); collecting.remove(id); return; }
                try {
                    Map<String, Object> members = new TreeMap<>();
                    for (Element element : declaration.getEnclosedElements()) {
                        if (!(element instanceof ExecutableElement member) || element.getKind() != ElementKind.METHOD) continue;
                        Map<String, Object> row = new LinkedHashMap<>();
                        row.put("annotations", annotationUses(member));
                        row.put("returnType", member.getReturnType().toString());
                        if (member.getDefaultValue() != null) row.put("defaultValue", value(member.getDefaultValue(), member, 0));
                        members.put(member.getSimpleName().toString(), row);
                    }
                    Map<String, Object> complete = Map.of("origin", origin(declaration, null), "annotations", annotationUses(declaration), "members", members);
                    if (utf8(complete) > DEFINITION_MEMBERS_BYTES) {
                        // Bound oversized definitions explicitly; never silently drop
                        // a record or let it inflate a registry shard past the fact
                        // budget. The framework treats a bounded definition as PARTIAL.
                        Map<String, Object> boundedMembers = new TreeMap<>();
                        int bytes = 0;
                        for (Map.Entry<String, Object> member : members.entrySet()) {
                            int cost = utf8((Map<String, Object>) member.getValue()) + 2;
                            if (!boundedMembers.isEmpty() && bytes + cost > DEFINITION_MEMBERS_BYTES) break;
                            boundedMembers.put(member.getKey(), member.getValue());
                            bytes += cost;
                        }
                        boundaries.add("ANNOTATION_DEFINITION_BOUNDED");
                        definitions.put(id, Map.of("origin", origin(declaration, null), "annotations", annotationUses(declaration), "members", boundedMembers, "bounded", List.of("DEFINITION_MEMBER_BUDGET")));
                    } else {
                        definitions.put(id, complete);
                    }
                } finally { collecting.remove(id); }
            }

            private Map<String, Object> unknown(String reason) {
                boundaries.add(reason); return Map.of("kind", "UNRESOLVED", "reason", reason);
            }
            private Map<String, Object> value(AnnotationValue annotation, Element owner, int level) {
                if (level > 32) return unknown("ANNOTATION_VALUE_LIMIT");
                Object value = annotation.getValue();
                if (value instanceof String || value instanceof Number || value instanceof Boolean) return Map.of("kind", "CONSTANT", "value", value);
                if (value instanceof Character character) return Map.of("kind", "CONSTANT", "value", character.toString());
                if (value instanceof VariableElement constant) return Map.of("kind", "ENUM", "type", constant.asType().toString(), "value", constant.getSimpleName().toString());
                if (value instanceof TypeMirror type) {
                    if (types.asElement(type) instanceof TypeElement declaration && declaration.getKind() == ElementKind.ANNOTATION_TYPE) definition(declaration);
                    return Map.of("kind", "CLASS", "value", type.toString());
                }
                if (value instanceof AnnotationMirror nested) {
                    Map<String, Object> use = annotation(nested, owner);
                    return use == null ? unknown("UNRESOLVED_ANNOTATION_CLASS") : Map.of("kind", "ANNOTATION", "value", use);
                }
                if (value instanceof List<?> list) {
                    List<Map<String, Object>> values = new ArrayList<>();
                    for (Object child : list) if (child instanceof AnnotationValue argument) values.add(value(argument, owner, level + 1));
                    return Map.of("kind", "ARRAY", "values", values);
                }
                return unknown("UNRESOLVED_ANNOTATION_VALUE");
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
