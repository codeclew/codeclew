package com.acme.flow

import kotlin.test.Test
import kotlin.test.assertEquals

class MavenFlowTest {
    @Test
    fun transformsProducedValueBeforeConsumption() {
        assertEquals(8, transformAndConsume(4))
    }
}
