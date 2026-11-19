package fixture.processor;
import java.io.IOException;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.*;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
import javax.tools.Diagnostic;
@SupportedAnnotationTypes("fixture.processor.GenerateValue")
@SupportedOptions("generationMode")
public final class ValueProcessor extends AbstractProcessor {
    private boolean processed;
    @Override public SourceVersion getSupportedSourceVersion() { return SourceVersion.RELEASE_21; }
    @Override public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment round) {
        if (processed || round.processingOver() || annotations.isEmpty()) return false;
        processed = true;
        String mode = processingEnv.getOptions().getOrDefault("generationMode", "string");
        if (mode.equals("failed")) {
            processingEnv.getMessager().printMessage(Diagnostic.Kind.ERROR, "OWNED_KAPT_PROCESSOR_FAILURE");
            return true;
        }
        if (mode.equals("removed")) return true;
        String method = mode.equals("changed") ? "public static int value() { return 42; }" :
            "public static String value() { return \"generated-by-real-kapt\"; }";
        try (Writer output = processingEnv.getFiler().createSourceFile("fixture.generated.GeneratedValue").openWriter()) {
            output.write("package fixture.generated; public final class GeneratedValue { " + method + " }");
        } catch (IOException error) { throw new IllegalStateException(error); }
        return true;
    }
}
