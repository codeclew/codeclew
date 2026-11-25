import com.sun.source.tree.*;
import com.sun.source.util.DocTrees;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.Trees;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Base64;
import java.util.List;
import java.util.function.Function;
import javax.lang.model.element.Modifier;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.ToolProvider;

/** Parse sealed Java method excerpts without attribution or annotation processing. */
class ClewPreparedJavaSyntax {
    private static final String PREFIX = "class ClewRetainedMethod {\n";
    private static final int MAX_NODES = 360;
    private static final int MAX_DEPTH = 32;

    static Object parse(String text) throws Exception {
        return parse(text, false);
    }

    static Object parse(String text, boolean fullUnit) throws Exception {
        return parse(text, fullUnit, false);
    }

    static Object parse(String text, boolean fullUnit, boolean dispatchSyntax) throws Exception {
        String wrapped = fullUnit ? text : PREFIX + text + "\n}";
        var file = new SimpleJavaFileObject(URI.create("string:///ClewRetainedMethod.java"), JavaFileObject.Kind.SOURCE) {
            @Override public CharSequence getCharContent(boolean ignoreEncodingErrors) { return wrapped; }
        };
        var diagnostics = new DiagnosticCollector<JavaFileObject>();
        var compiler = ToolProvider.getSystemJavaCompiler();
        try (var files = compiler.getStandardFileManager(diagnostics, null, StandardCharsets.UTF_8)) {
            var task = (JavacTask) compiler.getTask(null, files, diagnostics, List.of("-proc:none"), null, List.of(file));
            var unit = task.parse().iterator().next();
            if (diagnostics.getDiagnostics().stream().anyMatch(d -> d.getKind() == Diagnostic.Kind.ERROR)) {
                return null;
            }
            var syntax = new ClewPreparedJavaSyntax(unit, Trees.instance(task).getSourcePositions(),
                    fullUnit ? DocTrees.instance(task) : null, text, fullUnit, dispatchSyntax);
            if (fullUnit) {
                return syntax.compilationUnit();
            }
            if (unit.getTypeDecls().size() != 1
                    || !(unit.getTypeDecls().getFirst() instanceof ClassTree container)
                    || container.getMembers().size() != 1
                    || !(container.getMembers().getFirst() instanceof MethodTree method)
                    || method.getBody() == null) {
                return null;
            }
            var parameters = method.getParameters().stream()
                    .map(parameter -> syntax.node(parameter, "parameter", parameter.getName().toString(),
                            syntax.typeName(parameter.getType()))).toList();
            var signature = method.getParameters().isEmpty() || !variableArity(method.getParameters().getLast())
                    ? "FIXED" : "VARARGS";
            return List.of(method.getName().toString(), parameters, syntax.statement(method.getBody()), signature);
        }
    }

    static boolean variableArity(VariableTree parameter) {
        // Javac's tree representation prints the normalized ellipsis immediately before the name.
        String declaration = parameter.toString();
        String name = parameter.getName().toString();
        return declaration.endsWith(name) && declaration.substring(0, declaration.length() - name.length())
                .stripTrailing().endsWith("...");
    }

    final CompilationUnitTree unit;
    final SourcePositions positions;
    final DocTrees docs;
    final int[] byteOffsets;
    final boolean fullUnit;
    final boolean dispatchSyntax;
    int nodes;
    int depth;

    ClewPreparedJavaSyntax(CompilationUnitTree unit, SourcePositions positions, DocTrees docs, String text,
                          boolean fullUnit, boolean dispatchSyntax) {
        this.unit = unit;
        this.positions = positions;
        this.docs = docs;
        this.fullUnit = fullUnit;
        this.dispatchSyntax = dispatchSyntax;
        this.byteOffsets = new int[text.length() + 1];
        for (int index = 0; index < text.length();) {
            int point = text.codePointAt(index);
            int next = index + Character.charCount(point);
            byteOffsets[next] = byteOffsets[index] + (point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4);
            index = next;
        }
    }

    List<Object> node(Tree tree, String kind, Object... payload) {
        int prefix = fullUnit ? 0 : PREFIX.length();
        int start = (int) positions.getStartPosition(unit, tree) - prefix;
        int end = (int) positions.getEndPosition(unit, tree) - prefix;
        return range(start, end, kind, payload);
    }

    List<Object> range(int start, int end, String kind, Object... payload) {
        if (start < 0 || start >= end || end >= byteOffsets.length || ++nodes > MAX_NODES) {
            throw new IllegalArgumentException("Unusable retained source positions or node budget");
        }
        var result = new ArrayList<Object>(List.of(kind, byteOffsets[start], byteOffsets[end]));
        result.addAll(Arrays.asList(payload));
        return result;
    }

    Object compilationUnit() {
        var namespace = unit.getPackageName();
        var imports = unit.getImports().stream().filter(entry -> dispatchSyntax || !entry.isStatic())
                .map(entry -> {
                    nodes = 0;
                    return node(entry, entry.isStatic() ? "static-import" : "import", entry.getQualifiedIdentifier().toString());
                }).toList();
        var declarations = unit.getTypeDecls().stream().filter(ClassTree.class::isInstance)
                .map(ClassTree.class::cast).map(this::typeDeclaration).toList();
        return List.of(namespace == null ? "" : namespace.toString(), imports, declarations);
    }

    Object typeDeclaration(ClassTree entry) {
        var members = new ArrayList<Object>();
        var ownerPath = new TreePath(new TreePath(unit), entry);
        for (var member : entry.getMembers()) {
            nodes = 0;
            if (member instanceof MethodTree method) {
                members.add(methodDeclaration(entry, method, new TreePath(ownerPath, method)));
            } else if (member instanceof VariableTree field) {
                members.add(node(field, "field", field.getName().toString(), typeName(field.getType()),
                        modifiers(field.getModifiers()), expression(field.getInitializer())));
            } else if (member instanceof BlockTree block) {
                members.add(node(block, block.isStatic() ? "static-init" : "instance-init"));
            }
        }
        nodes = 0;
        var supertypes = new ArrayList<Object>();
        if (entry.getExtendsClause() != null) {
            var parent = entry.getExtendsClause();
            supertypes.add(node(parent, "supertype", typeName(parent)));
        }
        for (var parent : entry.getImplementsClause()) {
            supertypes.add(node(parent, "supertype", typeName(parent)));
        }
        return node(entry, "class", entry.getSimpleName().toString(), entry.getKind().name(),
                modifiers(entry.getModifiers()), annotations(entry.getModifiers()), members,
                entry.getExtendsClause() == null && entry.getImplementsClause().isEmpty() ? "DIRECT" : "INHERITS",
                entry.getTypeParameters().stream().map(parameter -> parameter.getName().toString()).toList(),
                entry.getMembers().stream().filter(ClassTree.class::isInstance).map(ClassTree.class::cast)
                        .map(type -> type.getSimpleName().toString()).toList(), supertypes);
    }

    Object methodDeclaration(ClassTree owner, MethodTree method, TreePath path) {
        Object body = null;
        try {
            body = statement(method.getBody());
        } catch (IllegalArgumentException unsupported) {
            // Still retain the signature so it participates in overload checks.
        }
        nodes = 0;
        var params = new ArrayList<List<Object>>();
        boolean compact = false;
        for (var parameter : method.getParameters()) {
            VariableTree address = parameter;
            if (positions.getEndPosition(unit, parameter) < 0 && owner.getKind() == Tree.Kind.RECORD
                    && method.getName().contentEquals("<init>")) {
                address = owner.getMembers().stream().filter(VariableTree.class::isInstance)
                        .map(VariableTree.class::cast).filter(field -> field.getName().equals(parameter.getName()))
                        .findFirst().orElse(parameter);
                compact = true;
            }
            var symbol = node(address, "parameter", parameter.getName().toString(), typeName(parameter.getType()),
                    parameterized(parameter.getType()) ? "GENERIC" : "DIRECT");
            if (dispatchSyntax) symbol.add(typeArguments(parameter.getType()));
            params.add(symbol);
        }
        return node(method, "method", method.getName().toString(), typeName(method.getReturnType()),
                modifiers(method.getModifiers()), params, annotations(method.getModifiers()), body,
                method.getTypeParameters().isEmpty() ? "DIRECT" : "GENERIC", compact ? "COMPACT" : "EXPLICIT",
                firstSentence(path),
                method.getTypeParameters().stream().map(parameter -> parameter.getName().toString()).toList());
    }

    Object firstSentence(TreePath path) {
        var comment = docs.getDocCommentTree(path);
        if (comment == null || comment.getFirstSentence().isEmpty()) {
            return null;
        }
        var coordinates = docs.getSourcePositions();
        int start = (int) coordinates.getStartPosition(unit, comment, comment.getFirstSentence().getFirst());
        int end = (int) coordinates.getEndPosition(unit, comment, comment.getFirstSentence().getLast());
        return start < 0 || start >= end ? null : range(start, end, "javadoc");
    }

    List<String> modifiers(ModifiersTree modifiers) {
        return modifiers.getFlags().stream().map(Modifier::name).sorted().toList();
    }

    List<Object> annotations(ModifiersTree modifiers) {
        return modifiers.getAnnotations().stream().map(annotation -> {
            var symbol = node(annotation, "annotation", annotation.getAnnotationType().toString());
            if (dispatchSyntax) symbol.add(annotation.getArguments().stream().map(this::expression).toList());
            return (Object) symbol;
        }).toList();
    }

    String typeName(Tree type) {
        return switch (type) {
            case null -> "";
            case ParameterizedTypeTree generic -> typeName(generic.getType());
            case AnnotatedTypeTree annotated -> typeName(annotated.getUnderlyingType());
            case ArrayTypeTree array -> typeName(array.getType()) + "[]";
            case PrimitiveTypeTree primitive -> primitive.toString();
            case IdentifierTree name -> name.getName().toString();
            case MemberSelectTree member -> typeName(member.getExpression()) + "." + member.getIdentifier();
            default -> "";
        };
    }

    boolean parameterized(Tree type) {
        return type instanceof ParameterizedTypeTree
                || type instanceof AnnotatedTypeTree annotated && parameterized(annotated.getUnderlyingType());
    }

    List<String> typeArguments(Tree type) {
        return switch (type) {
            case ParameterizedTypeTree generic -> generic.getTypeArguments().stream().map(argument -> parameterized(argument) ? "" : typeName(argument)).toList();
            case AnnotatedTypeTree annotated -> typeArguments(annotated.getUnderlyingType());
            default -> List.of();
        };
    }

    Object statement(StatementTree tree) {
        return walk(tree, this::statementNode);
    }

    Object statementNode(StatementTree tree) {
        return switch (tree) {
            case BlockTree block -> node(tree, "block", block.getStatements().stream().map(this::statement).toList());
            case IfTree check -> node(tree, "if", expression(check.getCondition()),
                    statement(check.getThenStatement()), statement(check.getElseStatement()));
            case EnhancedForLoopTree loop -> node(tree, "foreach",
                    node(loop.getVariable(), "parameter", loop.getVariable().getName().toString()),
                    expression(loop.getExpression()), statement(loop.getStatement()));
            case BreakTree stop -> node(tree, "break", stop.getLabel() == null ? null : stop.getLabel().toString());
            case ContinueTree next -> node(tree, "continue", next.getLabel() == null ? null : next.getLabel().toString());
            case AssertTree check -> node(tree, "assert", expression(check.getCondition()), expression(check.getDetail()));
            case ReturnTree exit -> node(tree, "return", expression(exit.getExpression()));
            case ThrowTree exit -> node(tree, "throw", expression(exit.getExpression()));
            case VariableTree local when fullUnit -> node(tree, "let", local.getName().toString(),
                    expression(local.getInitializer()), typeName(local.getType()));
            case VariableTree local -> node(tree, "let", local.getName().toString(), expression(local.getInitializer()));
            case ExpressionStatementTree entry -> node(tree, "expr", expression(entry.getExpression()));
            case EmptyStatementTree empty -> node(tree, "empty");
            case TryTree attempt -> node(tree, "try", statement(attempt.getBlock()),
                    attempt.getCatches().stream().map(this::handler).toList(), statement(attempt.getFinallyBlock()),
                    !attempt.getResources().isEmpty() ? "resources" : null);
            default -> node(tree, "boundary", switch (tree.getKind()) {
                case FOR_LOOP, WHILE_LOOP, DO_WHILE_LOOP -> "loop";
                case SWITCH -> "switch statement";
                default -> "statement";
            });
        };
    }

    Object handler(CatchTree handler) {
        var parameter = handler.getParameter();
        return node(handler, "catch", node(parameter, "parameter", parameter.getName().toString()),
                statement(handler.getBlock()));
    }

    Object expression(ExpressionTree tree) {
        return walk(tree, this::expressionNode);
    }

    <T extends Tree> Object walk(T tree, Function<T, Object> project) {
        if (tree == null) {
            return null;
        }
        if (++depth > MAX_DEPTH) {
            throw new IllegalArgumentException("Retained source depth budget");
        }
        try {
            return project.apply(tree);
        } finally {
            depth--;
        }
    }

    Object expressionNode(ExpressionTree tree) {
        return switch (tree) {
            case ParenthesizedTree group -> expression(group.getExpression());
            case IdentifierTree name -> node(tree, "id", name.getName().toString());
            case LiteralTree literal when dispatchSyntax && literal.getValue() instanceof String value ->
                    node(tree, "literal", value.isEmpty() ? "EMPTY_STRING" : "NONEMPTY_STRING", value);
            case LiteralTree literal -> node(tree, "literal", switch (literal.getValue()) {
                case null -> "NULL";
                case Boolean truth -> truth ? "TRUE" : "FALSE";
                case String text -> text.isEmpty() ? "EMPTY_STRING" : "NONEMPTY_STRING";
                default -> null;
            });
            case MemberReferenceTree reference when dispatchSyntax -> node(tree, "methodref",
                    expression(reference.getQualifierExpression()), reference.getName().toString(), reference.getMode().name());
            case LambdaExpressionTree lambda when dispatchSyntax -> node(tree, "deferred",
                    lambda.getParameters().stream().map(parameter -> parameter.getName().toString()).toList(),
                    lambda.getBody() instanceof ExpressionTree body ? expression(body) : statement((StatementTree) lambda.getBody()));
            case LambdaExpressionTree lambda -> node(tree, "deferred");
            case MemberSelectTree member -> node(tree, "member", expression(member.getExpression()), member.getIdentifier().toString());
            case MethodInvocationTree call -> node(tree, "call", expression(call.getMethodSelect()),
                    call.getArguments().stream().map(this::expression).toList());
            case NewClassTree value when fullUnit && value.getClassBody() != null -> node(tree, "open");
            case NewClassTree value -> node(tree, "new", expression(value.getEnclosingExpression()),
                    fullUnit ? node(value.getIdentifier(), "type", typeName(value.getIdentifier())) : node(value.getIdentifier(), "type"),
                    value.getArguments().stream().map(this::expression).toList());
            case NewArrayTree array when dispatchSyntax && array.getType() == null && array.getInitializers() != null ->
                    node(tree, "annotation-array", array.getInitializers().stream().map(this::expression).toList());
            case AssignmentTree binding -> node(tree, "set", expression(binding.getVariable()), expression(binding.getExpression()));
            case ConditionalExpressionTree select -> node(tree, "choose", expression(select.getCondition()),
                    expression(select.getTrueExpression()), expression(select.getFalseExpression()));
            case BinaryTree op -> switch (op.getKind()) {
                case PLUS -> node(tree, dispatchSyntax ? "concat" : "ordered",
                        expression(op.getLeftOperand()), expression(op.getRightOperand()));
                case CONDITIONAL_AND -> node(tree, "and", expression(op.getLeftOperand()), expression(op.getRightOperand()));
                case CONDITIONAL_OR -> node(tree, "or", expression(op.getLeftOperand()), expression(op.getRightOperand()));
                case EQUAL_TO, NOT_EQUAL_TO -> node(tree, op.getKind() == Tree.Kind.EQUAL_TO ? "eq" : "neq",
                        expression(op.getLeftOperand()), expression(op.getRightOperand()));
                default -> node(tree, "ordered", expression(op.getLeftOperand()), expression(op.getRightOperand()));
            };
            case UnaryTree op -> switch (op.getKind()) {
                case LOGICAL_COMPLEMENT -> node(tree, "not", expression(op.getExpression()));
                case POSTFIX_INCREMENT, POSTFIX_DECREMENT, PREFIX_INCREMENT, PREFIX_DECREMENT ->
                        node(tree, "mutate", expression(op.getExpression()));
                default -> node(tree, "ordered", expression(op.getExpression()));
            };
            case CompoundAssignmentTree binding -> node(tree, "mutate", expression(binding.getVariable()),
                    expression(binding.getExpression()));
            case InstanceOfTree check -> node(tree, "ordered", expression(check.getExpression()));
            case TypeCastTree cast -> node(tree, "ordered", expression(cast.getExpression()));
            case ArrayAccessTree index -> node(tree, "array", expression(index.getExpression()), expression(index.getIndex()));
            default -> node(tree, "open");
        };
    }

    static String json(Object value) {
        if (value == null || value instanceof Integer) {
            return String.valueOf(value);
        }
        if (value instanceof List<?> items) {
            var output = new StringBuilder("[");
            for (Object item : items) {
                if (output.length() > 1) output.append(',');
                output.append(json(item));
            }
            return output.append(']').toString();
        }
        var quoted = new StringBuilder("\"");
        for (char c : ((String) value).toCharArray()) {
            switch (c) {
                case '"' -> quoted.append("\\\"");
                case '\\' -> quoted.append("\\\\");
                default -> {
                    if (c < 0x20) quoted.append(String.format("\\u%04x", (int) c));
                    else quoted.append(c);
                }
            }
        }
        return quoted.append('"').toString();
    }

    public static void main(String[] args) throws Exception {
        boolean dispatch = Arrays.asList(args).contains("--dispatch-unit");
        boolean fullUnit = dispatch || Arrays.asList(args).contains("--unit");
        var input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        for (String row; (row = input.readLine()) != null;) {
            Object syntax = null;
            try {
                syntax = parse(new String(Base64.getDecoder().decode(row), StandardCharsets.UTF_8), fullUnit, dispatch);
            } catch (RuntimeException | StackOverflowError unsupported) {
                // A malformed or oversized excerpt cannot produce syntax facts.
            }
            System.out.println(json(syntax));
        }
    }
}
