package fixture
import fixture.processor.GenerateValue
import fixture.generated.GeneratedValue
@GenerateValue
class GenerationInput
fun generatedValue(): String = GeneratedValue.value()
