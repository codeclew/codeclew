package com.acme.relations

interface NumericSource {
    fun read(index: Int): Number
}

class IntegerSource : NumericSource {
    override fun read(index: Int): Int = index + 1
}

class LexicalDecoy {
    fun read(index: Int): Number = index - 1
}

interface LabeledSource {
    val label: String
}

class StaticLabel : LabeledSource {
    override val label: String = "stable"
}

public fun publicDescriptor(value: String): String? = value.takeIf(String::isNotEmpty)

internal fun internalDescriptor(value: String?): String = value.orEmpty()

private fun privateDescriptor(value: Int): Int = value

fun overloadedDescriptor(value: Int): String = value.toString()

fun overloadedDescriptor(value: String?): String? = value

fun <T : CharSequence> genericDescriptor(value: T): T = value

fun callSource(source: NumericSource): Number = source.read(4)

fun combineEqualTypes(first: Int, second: Int): Int = first - second

fun callEqualTypesByName(): Int = combineEqualTypes(second = 2, first = 1)

data class Envelope(val value: String)

data class NullableConstruction(val first: String, val second: String)

fun compilerNullableSource(enabled: Boolean): String? = if (enabled) "value" else null

fun compilerFallback(): String = "fallback"

fun compilerFallbackDecoy(): String = "decoy"

class DirectReturnProjection(
    private val projected: String,
    private val sameTypedDecoy: String,
) {
    fun returnedProperty(): String {
        val ignored = sameTypedDecoy
        check(ignored.isNotEmpty())
        return projected
    }

    fun aliasedProperty(): String {
        val alias = projected
        return alias
    }

    fun branchedProperty(enabled: Boolean): String {
        return if (enabled) projected else sameTypedDecoy
    }

    fun implicitProperty(): String = projected

    fun safeReturnedProperty(): Int? {
        return projected.takeIf(String::isNotEmpty)?.length
    }

    fun elvisReturnedProperty(): String {
        return projected.takeIf(String::isNotEmpty) ?: sameTypedDecoy
    }
}

fun directReturnedCall(value: String?): String {
    return internalDescriptor(value)
}

fun multipleReturnedCalls(value: String?): String {
    return internalDescriptor(value) + internalDescriptor(value)
}

fun unresolvedSourceReturn(supplier: () -> String): String {
    return supplier()
}

fun constructWithNullPolicy(enabled: Boolean, sameTypedDecoy: String): NullableConstruction =
    NullableConstruction(
        second = compilerNullableSource(enabled) ?: compilerFallback(),
        first = sameTypedDecoy,
    )

fun unsupportedSafeCallPolicy(enabled: Boolean): Int? =
    compilerNullableSource(enabled)?.length

fun unsupportedComplexNullPolicy(enabled: Boolean): String =
    compilerNullableSource(enabled)?.trim() ?: compilerFallback()

fun unsupportedLocalConstruction(value: String): String {
    data class LocalConstruction(val value: String)
    return LocalConstruction(value).value
}

class GeneratedConstructorBoundary

class RelationState {
    var field: String = "cold"
    val initial: Envelope = Envelope(field)

    fun configure(source: () -> String): Envelope {
        field = source()
        val observed = field
        return Envelope(observed)
    }

    fun reflectivelyRead(): String =
        javaClass.getDeclaredField("field").get(this) as String
}
