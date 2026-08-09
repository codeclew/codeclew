package com.acme

import kotlin.test.Test
import kotlin.test.assertEquals

class RunnerTest {
    @Test
    fun `applies configured limit`() {
        assertEquals("rec", applyOptions("record", Options(3)))
    }

    @Test
    fun `transforms the produced value before consumption`() {
        assertEquals(8, transformAndConsume(4))
    }

    @Test
    fun `applies the mapping context to one value`() {
        assertEquals(6, applyMappingContext(4, mappingContext()))
    }
}
