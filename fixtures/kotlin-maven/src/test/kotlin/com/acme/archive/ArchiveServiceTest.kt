package com.acme.archive

import org.junit.jupiter.api.DisplayName
import org.junit.jupiter.api.Test
import kotlin.test.assertEquals

class ArchiveServiceTest {
    @Test
    @DisplayName("The archive event contains the product identity")
    fun archiveEventContainsProductIdentity() {
        val event = ArchiveService().archiveEvent(ProductIdentity("42", "SKU-42", "Product"))

        assertEquals("42:SKU-42:Product", event)
    }
}
