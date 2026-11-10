import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.ReturnTree;
import com.sun.source.tree.Tree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import javax.tools.ToolProvider;

/** Test-only javac position provider; never used by the product launcher. */
public class SourcePositionsProbe {
    public static void main(String[] args) throws Exception {
        Path root = Path.of(args[0]).toRealPath();
        String mode = args[1];
        var compiler = ToolProvider.getSystemJavaCompiler();
        try (var files = compiler.getStandardFileManager(null, null, null)) {
            var inputs = files.getJavaFileObjects(root.resolve("Values.java"));
            var task = (JavacTask) compiler.getTask(null, files, null,
                    List.of("--release", "17", "-proc:none"), null, inputs);
            List<CompilationUnitTree> units = new ArrayList<>();
            task.parse().forEach(units::add);
            task.analyze();
            Trees trees = Trees.instance(task);
            SourcePositions original = trees.getSourcePositions();
            Map<Tree, long[]> replacements = new IdentityHashMap<>();
            new TreePathScanner<Void, Void>() {
                @Override public Void visitMethod(MethodTree method, Void unused) {
                    if (method.getName().contentEquals("canEqual") && mode.equals("expression")) {
                        var parameter = method.getParameters().get(0);
                        Tree expression = ((ReturnTree) method.getBody().getStatements().get(0)).getExpression();
                        replacements.put(expression, new long[] {
                                original.getStartPosition(units.get(0), parameter),
                                original.getEndPosition(units.get(0), parameter)});
                    }
                    if (method.getName().contentEquals("copy") && !mode.equals("ordinary")) {
                        var first = method.getParameters().get(0);
                        long start = original.getStartPosition(units.get(0), first);
                        long end = original.getEndPosition(units.get(0), first);
                        if (mode.equals("parameters")) {
                            replacements.put(method.getParameters().get(1), new long[] {start, end});
                        } else if (mode.equals("empty")) {
                            replacements.put(first, new long[] {start, start});
                        }
                    }
                    return super.visitMethod(method, unused);
                }
            }.scan(units.get(0), null);
            Class<?> analyzerType = Class.forName("CodeclewJavaAnalyzer$Analyzer");
            Constructor<?> constructor = analyzerType.getDeclaredConstructors()[0];
            constructor.setAccessible(true);
            List<Map<String, Object>> facts = new ArrayList<>();
            Object analyzer = constructor.newInstance(root, trees, task.getElements(), task.getTypes(),
                    files, List.of(), facts, units);
            Field positions = analyzerType.getDeclaredField("positions");
            positions.setAccessible(true);
            positions.set(analyzer, new SourcePositions() {
                @Override public long getStartPosition(CompilationUnitTree unit, Tree tree) {
                    return replacements.containsKey(tree) ? replacements.get(tree)[0]
                            : original.getStartPosition(unit, tree);
                }
                @Override public long getEndPosition(CompilationUnitTree unit, Tree tree) {
                    return replacements.containsKey(tree) ? replacements.get(tree)[1]
                            : original.getEndPosition(unit, tree);
                }
            });
            @SuppressWarnings("unchecked")
            var scanner = (TreePathScanner<Void, Void>) analyzer;
            units.forEach(unit -> scanner.scan(unit, null));
            Method json = Class.forName("CodeclewJavaAnalyzer").getDeclaredMethod("json", Object.class);
            json.setAccessible(true);
            for (var fact : facts) System.out.println(json.invoke(null, fact));
        }
    }
}
