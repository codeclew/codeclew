import com.sun.source.tree.*;
import com.sun.source.util.*;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.*;
import javax.lang.model.element.Modifier;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.TypeKind;
import javax.lang.model.util.Types;
import javax.tools.*;

/** Attribute static constants and local predicate calls in bounded, pinned source files. */
class ClewPreparedJavaFieldUses {
    record Bounds(int start, int end) {
        boolean contains(Bounds inner) {
            return start <= inner.start && inner.end <= end;
        }
    }

    static final class Source extends SimpleJavaFileObject {
        final String text;
        final int[] bytes;
        final int index;
        final List<Bounds> selections = new ArrayList<>();

        Source(String path, String text, int index) throws Exception {
            super(new URI("memory", null, "/" + path, null), Kind.SOURCE);
            this.text = text;
            this.index = index;
            this.bytes = new int[text.length() + 1];
            for (int at = 0; at < text.length();) {
                int point = text.codePointAt(at);
                int next = at + Character.charCount(point);
                bytes[next] = bytes[at] + (point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4);
                at = next;
            }
        }

        @Override
        public CharSequence getCharContent(boolean ignoreEncodingErrors) {
            return text;
        }

        Bounds bounds(SourcePositions positions, CompilationUnitTree unit, Tree tree) {
            long start = positions.getStartPosition(unit, tree);
            long end = positions.getEndPosition(unit, tree);
            if (start < 0 || start >= end || end >= bytes.length) return null;
            return new Bounds(bytes[(int) start], bytes[(int) end]);
        }
    }

    static String text(String encoded) {
        return new String(Base64.getDecoder().decode(encoded), StandardCharsets.UTF_8);
    }

    static List<Source> request() throws Exception {
        var result = new ArrayList<Source>();
        try (var input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            for (String row; (row = input.readLine()) != null;) {
                String[] columns = row.split("\t", -1);
                var source = new Source(text(columns[0]), text(columns[1]), result.size());
                if (!columns[2].isEmpty()) {
                    for (String selection : columns[2].split(",")) {
                        String[] parts = selection.split(":");
                        source.selections.add(new Bounds(Integer.parseInt(parts[0]), Integer.parseInt(parts[1])));
                    }
                }
                result.add(source);
            }
        }
        return result;
    }

    record NominalType(String name, Tree importSite) {}
    record NominalBinding(MethodTree method, String name, List<Tree> witnesses) {}

    static final class Scanner extends TreePathScanner<Void, Void> {
        final Trees trees;
        final SourcePositions positions;
        final Map<URI, Source> sources;
        final Types types;
        final List<? extends Diagnostic<? extends JavaFileObject>> diagnostics;
        int remaining = 160;
        int predicates = 24;

        Scanner(Trees trees, Types types, Map<URI, Source> sources,
                List<? extends Diagnostic<? extends JavaFileObject>> diagnostics) {
            this.trees = trees;
            this.positions = trees.getSourcePositions();
            this.sources = sources;
            this.types = types;
            this.diagnostics = diagnostics;
        }

        @Override
        public Void visitIdentifier(IdentifierTree name, Void ignored) {
            inspect(name);
            return super.visitIdentifier(name, ignored);
        }

        @Override
        public Void visitMemberSelect(MemberSelectTree member, Void ignored) {
            inspect(member);
            return super.visitMemberSelect(member, ignored);
        }

        @Override
        public Void visitMethodInvocation(MethodInvocationTree invocation, Void ignored) {
            if (predicates > 0) predicate(invocation);
            return super.visitMethodInvocation(invocation, ignored);
        }

        void predicate(MethodInvocationTree invocation) {
            var call = getCurrentPath();
            var unit = call.getCompilationUnit();
            var source = sources.get(unit.getSourceFile().toUri());
            var use = source.bounds(positions, unit, invocation);
            if (use == null || source.selections.stream().noneMatch(selection -> selection.contains(use))) return;
            var lexicalOwner = call;
            while (lexicalOwner != null && !(lexicalOwner.getLeaf() instanceof ClassTree)) {
                lexicalOwner = lexicalOwner.getParentPath();
            }
            if (lexicalOwner == null) return;
            var declaration = compilerPredicate(invocation, call, lexicalOwner);
            if (declaration != null) {
                var bounds = source.bounds(positions, unit, declaration);
                if (bounds != null && !hasError(source, bounds) && !hasError(source, use)
                        && emitPredicate(source, unit, use, bounds, declaration, null)) return;
            }
            var nominal = nominalPredicate(invocation, call, lexicalOwner);
            if (nominal != null) {
                var bounds = source.bounds(positions, unit, nominal.method);
                emitPredicate(source, unit, use, bounds, nominal.method, nominal);
            }
        }

        MethodTree compilerPredicate(MethodInvocationTree invocation, TreePath call, TreePath lexicalOwner) {
            if (!(trees.getElement(call) instanceof ExecutableElement method)
                    || method.getReturnType().getKind() != TypeKind.BOOLEAN
                    || method.isVarArgs() || !method.getTypeParameters().isEmpty()
                    || method.getParameters().size() != invocation.getArguments().size()
                    || !(trees.getElement(lexicalOwner) instanceof TypeElement owner)
                    || !owner.equals(method.getEnclosingElement())
                    || (Collections.disjoint(method.getModifiers(), Set.of(Modifier.STATIC, Modifier.PRIVATE, Modifier.FINAL))
                        && !owner.getModifiers().contains(Modifier.FINAL))) return null;
            for (int index = 0; index < method.getParameters().size(); index++) {
                var type = trees.getTypeMirror(new TreePath(call, invocation.getArguments().get(index)));
                if (type == null || type.getKind() == TypeKind.ERROR
                        || !types.isSameType(type, method.getParameters().get(index).asType())) return null;
            }
            var definition = trees.getPath(method);
            return definition != null && definition.getCompilationUnit() == call.getCompilationUnit()
                    && definition.getLeaf() instanceof MethodTree declaration ? declaration : null;
        }

        NominalBinding nominalPredicate(MethodInvocationTree invocation, TreePath call, TreePath lexicalOwner) {
            var unit = call.getCompilationUnit();
            var target = invocation.getMethodSelect();
            String name;
            if (target instanceof IdentifierTree identifier) {
                name = identifier.getName().toString();
            } else if (target instanceof MemberSelectTree select
                    && select.getExpression() instanceof IdentifierTree receiver && receiver.getName().contentEquals("this")) {
                name = select.getIdentifier().toString();
            } else {
                return null;
            }
            if (!invocation.getTypeArguments().isEmpty() || invocation.getArguments().size() != 1
                    || !(invocation.getArguments().getFirst() instanceof IdentifierTree actual)) return null;
            var owner = (ClassTree) lexicalOwner.getLeaf();
            var candidates = owner.getMembers().stream().filter(MethodTree.class::isInstance).map(MethodTree.class::cast)
                    .filter(method -> method.getName().contentEquals(name)).toList();
            if (candidates.size() != 1) return null;
            var method = candidates.getFirst();
            if (method.getParameters().size() != 1 || !method.getModifiers().getFlags().contains(Modifier.PRIVATE)
                    || !method.getTypeParameters().isEmpty()
                    || !(method.getReturnType() instanceof PrimitiveTypeTree bare) || bare.getPrimitiveTypeKind() != TypeKind.BOOLEAN)
                return null;
            var formal = method.getParameters().getFirst();
            var formalType = trees.getTypeMirror(new TreePath(new TreePath(lexicalOwner, method), formal));
            var actualElement = trees.getElement(new TreePath(call, actual));
            if (formalType == null || formalType.getKind() != TypeKind.ERROR
                    || !(actualElement instanceof VariableElement variable) || variable.asType().getKind() != TypeKind.ERROR)
                return null;
            for (var path = call.getParentPath(); path != null && path.getLeaf() != owner; path = path.getParentPath()) {
                if (!(path.getLeaf() instanceof CatchTree handler)) continue;
                var caught = handler.getParameter();
                if (!actualElement.equals(trees.getElement(new TreePath(path, caught)))) continue;
                var argumentType = nominalType(unit, caught.getType());
                var parameterType = nominalType(unit, formal.getType());
                if (argumentType == null || parameterType == null || !argumentType.name.equals(parameterType.name)) return null;
                var witnesses = new LinkedHashSet<Tree>();
                witnesses.add(caught);
                if (argumentType.importSite != null) witnesses.add(argumentType.importSite);
                if (parameterType.importSite != null) witnesses.add(parameterType.importSite);
                return new NominalBinding(method, argumentType.name, List.copyOf(witnesses));
            }
            return null;
        }

        NominalType nominalType(CompilationUnitTree unit, Tree type) {
            if (type instanceof MemberSelectTree) return new NominalType(type.toString(), null);
            if (!(type instanceof IdentifierTree identifier)) return null;
            String suffix = "." + identifier.getName();
            var matches = unit.getImports().stream().filter(item -> !item.isStatic()
                    && item.getQualifiedIdentifier().toString().endsWith(suffix)).toList();
            if (matches.size() != 1) return null;
            var selected = matches.getFirst();
            return new NominalType(selected.getQualifiedIdentifier().toString(), selected);
        }

        boolean emitPredicate(Source source, CompilationUnitTree unit, Bounds use, Bounds bounds,
                              MethodTree declaration, NominalBinding nominal) {
            if (bounds == null || bounds.end - bounds.start > 1800 || declaration.getBody() == null) return false;
            var steps = predicateSteps(source, unit, declaration.getBody());
            if (steps == null) return false;
            var formals = new StringJoiner(",");
            for (var parameter : declaration.getParameters()) formals.add(encoded(parameter.getName().toString()));
            var witnesses = new StringJoiner(",");
            if (nominal != null) {
                for (var node : nominal.witnesses) {
                    var proof = source.bounds(positions, unit, node);
                    if (proof == null) return false;
                    witnesses.add(proof.start + ":" + proof.end);
                }
            }
            System.out.printf("P\t%d\t%d\t%d\t%d\t%d\t%s\t%s\t%s\t%s%n",
                    source.index, use.start, use.end, bounds.start, bounds.end, formals, steps,
                    nominal == null ? "" : encoded(nominal.name), witnesses);
            predicates--;
            return true;
        }

        String encoded(String value) {
            return Base64.getEncoder().encodeToString(value.getBytes(StandardCharsets.UTF_8));
        }

        boolean hasError(Source source, Bounds span) {
            return diagnostics.stream().anyMatch(item -> item.getKind() == Diagnostic.Kind.ERROR
                    && item.getSource() != null && item.getSource().toUri().equals(source.toUri())
                    && item.getPosition() >= 0 && item.getPosition() < source.bytes.length
                    && span.start <= source.bytes[(int) item.getPosition()]
                    && source.bytes[(int) item.getPosition()] < span.end);
        }

        String predicateSteps(Source source, CompilationUnitTree unit, BlockTree body) {
            var statements = body.getStatements();
            var steps = new StringJoiner(";");
            if (statements.isEmpty() || statements.size() > 4
                    || !(statements.getLast() instanceof ReturnTree result) || result.getExpression() == null) return null;
            for (var statement : statements.subList(0, statements.size() - 1)) {
                if (statement instanceof VariableTree local && local.getInitializer() != null) {
                    var bounds = source.bounds(positions, unit, local);
                    if (bounds == null) return null;
                    steps.add("L," + bounds.start + "," + bounds.end);
                } else if (statement instanceof IfTree branch && branch.getElseStatement() == null) {
                    var arm = branch.getThenStatement();
                    if (arm instanceof BlockTree block && block.getStatements().size() == 1) arm = block.getStatements().getFirst();
                    if (!(arm instanceof ReturnTree exit) || !(exit.getExpression() instanceof LiteralTree literal)
                            || !(literal.getValue() instanceof Boolean truth)) return null;
                    var condition = source.bounds(positions, unit, branch.getCondition());
                    if (condition == null) return null;
                    steps.add((truth ? "T" : "F") + "," + condition.start + "," + condition.end);
                } else {
                    return null;
                }
            }
            var expression = result.getExpression();
            var bounds = source.bounds(positions, unit, expression);
            if (bounds == null) return null;
            steps.add("R," + bounds.start + "," + bounds.end);
            while (expression instanceof ParenthesizedTree group) expression = group.getExpression();
            if (expression instanceof BinaryTree binary && (binary.getKind() == Tree.Kind.CONDITIONAL_AND
                    || binary.getKind() == Tree.Kind.CONDITIONAL_OR)) {
                var left = source.bounds(positions, unit, binary.getLeftOperand());
                var right = source.bounds(positions, unit, binary.getRightOperand());
                if (left == null || right == null) return null;
                String gate = binary.getKind() == Tree.Kind.CONDITIONAL_AND ? "A" : "O";
                steps.add(gate + "," + left.start + "," + left.end + "," + right.start + "," + right.end);
            }
            return steps.toString();
        }

        void inspect(ExpressionTree expression) {
            if (remaining == 0) return;
            var path = getCurrentPath();
            var unit = path.getCompilationUnit();
            var source = sources.get(unit.getSourceFile().toUri());
            var use = source.bounds(positions, unit, expression);
            if (use == null || source.selections.stream().noneMatch(selection -> selection.contains(use))) return;
            if (!(trees.getElement(path) instanceof VariableElement field)
                    || field.getKind() != ElementKind.FIELD
                    || !field.getModifiers().containsAll(Set.of(Modifier.STATIC, Modifier.FINAL))
                    || field.asType().getKind() == TypeKind.ERROR || field.getConstantValue() == null) return;
            var type = trees.getTypeMirror(path);
            if (type == null || type.getKind() == TypeKind.ERROR) return;
            var declarationPath = trees.getPath(field);
            if (declarationPath == null || !(declarationPath.getLeaf() instanceof VariableTree declaration)
                    || declaration.getInitializer() == null) return;
            var ownerUnit = declarationPath.getCompilationUnit();
            var owner = sources.get(ownerUnit.getSourceFile().toUri());
            if (owner == null) return;
            var definition = owner.bounds(positions, ownerUnit, declaration);
            var initializer = owner.bounds(positions, ownerUnit, declaration.getInitializer());
            if (definition == null || initializer == null || !definition.contains(initializer)) return;
            var constant = switch (field.getConstantValue()) {
                case Boolean value -> value ? "TRUE" : "FALSE";
                case String value -> value.isEmpty() ? "EMPTY_STRING" : "NONEMPTY_STRING";
                default -> "PRIMITIVE";
            };
            System.out.printf("%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%s%n",
                    source.index, use.start, use.end, owner.index, definition.start, definition.end,
                    initializer.start, initializer.end, constant);
            remaining--;
        }
    }

    public static void main(String[] args) throws Exception {
        var requested = request();
        if (requested.isEmpty()) return;
        var compiler = ToolProvider.getSystemJavaCompiler();
        var diagnostics = new DiagnosticCollector<JavaFileObject>();
        try (var files = compiler.getStandardFileManager(diagnostics, null, StandardCharsets.UTF_8)) {
            files.setLocationFromPaths(StandardLocation.CLASS_PATH, List.of());
            files.setLocationFromPaths(StandardLocation.SOURCE_PATH, List.of());
            var task = (JavacTask) compiler.getTask(null, files, diagnostics,
                    List.of("-proc:none", "-implicit:none"), null, requested);
            var units = new ArrayList<CompilationUnitTree>();
            task.parse().forEach(units::add);
            // Unrelated missing project dependencies do not prevent javac resolving an exact selected field.
            task.analyze();
            if (diagnostics.getDiagnostics().stream().anyMatch(item -> item.getCode().equals("compiler.err.duplicate.class"))) {
                return;
            }
            var sources = new HashMap<URI, Source>();
            requested.forEach(source -> sources.put(source.toUri(), source));
            var scanner = new Scanner(Trees.instance(task), task.getTypes(), sources, diagnostics.getDiagnostics());
            for (var unit : units) {
                if (!sources.get(unit.getSourceFile().toUri()).selections.isEmpty()) {
                    scanner.scan(unit, null);
                }
            }
        }
    }
}
