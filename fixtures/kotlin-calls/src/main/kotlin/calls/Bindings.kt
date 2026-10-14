package calls

interface BindingApi {
    fun pick(count: Int, text: String, enabled: Boolean = true): String
    fun pick(text: String): String
}

class BindingApiImpl : BindingApi {
    override fun pick(count: Int, text: String, enabled: Boolean): String =
        if (enabled) "$count:$text" else text

    override fun pick(text: String): String = text
}

fun lookupBindingText(id: String): String = id

fun namedDefault(api: BindingApi): String =
    api.pick(text = lookupBindingText("é😀"), count = 7)

fun overloadCalls(): Int = overloaded(7) + overloaded("seven")

fun extensionDefault(input: String): String = input.decorate()
