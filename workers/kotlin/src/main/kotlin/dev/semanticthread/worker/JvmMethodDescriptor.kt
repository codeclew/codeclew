package dev.semanticthread.worker

/**
 * Canonicalize the class names emitted by FIR's JVM signature helper and
 * admit only Codeclew's strict, complete method-descriptor subset.
 *
 * FIR renders a nested class in an object type as `Outer.Inner`, while a JVM
 * descriptor uses the binary name `Outer$Inner`. Package separators are
 * already `/`, so dots are rewritten only while parsing an `L...;` token.
 */
internal fun canonicalJvmMethodDescriptor(raw: String): String? =
    JvmMethodDescriptorCanonicalizer(raw).canonicalize()

private class JvmMethodDescriptorCanonicalizer(
    private val raw: String,
) {
    private val canonical = StringBuilder(raw.length)
    private var offset = 0

    fun canonicalize(): String? {
        if (!consume('(')) return null
        canonical.append('(')
        var parameterSlots = 0
        while (peek() != ')') {
            val slots = fieldType() ?: return null
            parameterSlots += slots
            if (parameterSlots > 255) return null
        }
        offset += 1
        canonical.append(')')
        if (peek() == 'V') {
            offset += 1
            canonical.append('V')
        } else if (fieldType() == null) {
            return null
        }
        return canonical.toString().takeIf { offset == raw.length }
    }

    private fun fieldType(): Int? {
        var dimensions = 0
        while (peek() == '[') {
            dimensions += 1
            if (dimensions > 255) return null
            offset += 1
            canonical.append('[')
        }
        return when (val token = peek()) {
            'B', 'C', 'D', 'F', 'I', 'J', 'S', 'Z' -> {
                offset += 1
                canonical.append(token)
                if (dimensions == 0 && (token == 'D' || token == 'J')) 2 else 1
            }
            'L' -> if (objectType()) 1 else null
            else -> null
        }
    }

    private fun objectType(): Boolean {
        offset += 1
        canonical.append('L')
        var segmentLength = 0
        while (true) {
            val token = peek() ?: return false
            when {
                token == ';' -> {
                    if (segmentLength == 0) return false
                    offset += 1
                    canonical.append(';')
                    return true
                }
                token == '/' -> {
                    if (segmentLength == 0) return false
                    offset += 1
                    canonical.append('/')
                    segmentLength = 0
                }
                token == '.' -> {
                    if (segmentLength == 0) return false
                    offset += 1
                    canonical.append('$')
                    segmentLength = 0
                }
                token == '[' || token == '(' || token == ')' || token.code <= 31 || token.code == 127 ->
                    return false
                else -> {
                    offset += 1
                    canonical.append(token)
                    segmentLength += 1
                }
            }
        }
    }

    private fun consume(expected: Char): Boolean {
        if (peek() != expected) return false
        offset += 1
        return true
    }

    private fun peek(): Char? = raw.getOrNull(offset)
}
