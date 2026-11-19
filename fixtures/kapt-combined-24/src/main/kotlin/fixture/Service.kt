package fixture

import fixture.generated.GeneratedValue
import org.springframework.stereotype.Component

@Component
class LabelService {
    fun label(): String = GeneratedValue.value()
}

class SpecialLabel : LabelService() {
    override fun label(): String = super.label()
}

fun labelFor(service: LabelService): String = service.label()

fun independentNumber(): Int = fixture.generated.IndependentJava.number()
