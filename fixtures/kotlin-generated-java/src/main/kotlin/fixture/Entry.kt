package fixture
import fixture.generated.OwnedValue
fun generatedLabel(): String = OwnedValue.label()
fun jdk25Authority() = java.lang.classfile.ClassFile.of()
