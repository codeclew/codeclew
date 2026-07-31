package com.acme

import kotlin.test.Test
import kotlin.test.assertEquals

class SamplesTest {
    @Test fun totalUsesPremiumBranch() { assertEquals(10, total(5, true)); assertEquals(5, total(5, false)) }
    @Test fun classificationWorks() { assertEquals("negative", classify(-1)); assertEquals("zero", classify(0)); assertEquals("positive", classify(1)) }
    @Test fun safeCallAndElvisWork() { assertEquals(3, guarded("abc")); assertEquals(0, guarded(null)) }
    @Test fun namedDefaultAndExtensionCallsWork() { assertEquals("{x]", namedCall("x")); assertEquals(4, overloaded(2)); assertEquals(2, overloaded("ab")) }
}

