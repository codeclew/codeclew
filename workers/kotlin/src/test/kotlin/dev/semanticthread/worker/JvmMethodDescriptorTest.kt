package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class JvmMethodDescriptorTest {
    @Test
    fun acceptsTheSupportedFieldGrammarAndStructuralLimits() {
        for (descriptor in listOf(
            "()V",
            "(BCDFIJSZ)I",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            "([[I[[[Ljava/util/Map\$Entry;)[Ljava/lang/String;",
        )) {
            assertEquals(descriptor, canonicalJvmMethodDescriptor(descriptor))
        }
        val maximumArray = "(" + "[".repeat(255) + "I)V"
        assertEquals(maximumArray, canonicalJvmMethodDescriptor(maximumArray))
        assertNull(canonicalJvmMethodDescriptor("(" + "[".repeat(256) + "I)V"))

        val maximumSingleSlotParameters = "(" + "I".repeat(255) + ")V"
        assertEquals(maximumSingleSlotParameters, canonicalJvmMethodDescriptor(maximumSingleSlotParameters))
        assertNull(canonicalJvmMethodDescriptor("(" + "I".repeat(256) + ")V"))

        val maximumMixedSlots = "(" + "J".repeat(127) + "I)V"
        assertEquals(maximumMixedSlots, canonicalJvmMethodDescriptor(maximumMixedSlots))
        assertNull(canonicalJvmMethodDescriptor("(" + "J".repeat(128) + ")V"))

        val maximumArrayParameters = "(" + "[J".repeat(255) + ")V"
        assertEquals(maximumArrayParameters, canonicalJvmMethodDescriptor(maximumArrayParameters))
        assertNull(canonicalJvmMethodDescriptor("(" + "[J".repeat(256) + ")V"))
    }

    @Test
    fun canonicalizesNestedParameterReturnAndArrayTypes() {
        assertEquals(
            "(Lp/Outer\$Inner;)Lp/Outer\$Result;",
            canonicalJvmMethodDescriptor("(Lp/Outer.Inner;)Lp/Outer.Result;"),
        )
        assertEquals(
            "([[Lp/Outer\$Inner;)[[Lp/Outer\$Result;",
            canonicalJvmMethodDescriptor("([[Lp/Outer.Inner;)[[Lp/Outer.Result;"),
        )
    }

    @Test
    fun canonicalizesDefaultPackageNestedTypesAndPreservesBinaryNames() {
        assertEquals(
            "(LOuter\$Inner;)LOuter\$Result;",
            canonicalJvmMethodDescriptor("(LOuter.Inner;)LOuter.Result;"),
        )
        assertEquals(
            "(Lp/Outer\$Inner;Ljava/lang/String;)Lp/Outer\$Result;",
            canonicalJvmMethodDescriptor("(Lp/Outer\$Inner;Ljava/lang/String;)Lp/Outer\$Result;"),
        )
    }

    @Test
    fun rejectsMalformedObjectAndMethodDescriptors() {
        for (descriptor in listOf(
            "",
            "I",
            "(",
            "()",
            "()VV",
            "(V)V",
            "([V)V",
            "(Q)V",
            "(L;)V",
            "(Lp//Inner;)V",
            "(Lp/Outer..Inner;)V",
            "(Lp/Outer.;)V",
            "(Lp/.Inner;)V",
            "(Lp/Outer.Inner)V",
            "()L;",
            "()Vtrailing",
            "()Ljava/lang/String;;",
        )) {
            assertNull(canonicalJvmMethodDescriptor(descriptor), "accepted malformed descriptor $descriptor")
        }
    }
}
