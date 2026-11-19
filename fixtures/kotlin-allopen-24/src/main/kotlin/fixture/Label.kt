package fixture

@Target(AnnotationTarget.CLASS)
annotation class OpenForExtension

@OpenForExtension
class Label {
    fun text(): String = "base"
}

class Derived : Label() {
    override fun text(): String = super.text() + "!"
}

fun display(label: Label): String = label.text()
