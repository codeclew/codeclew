package com.acme.archive

import org.junit.jupiter.api.DisplayName
import org.junit.jupiter.api.Test
import kotlin.test.assertEquals

class ArchiveServiceTest {
    @Test
    @DisplayName("Событие архивации содержит идентичность товара")
    fun archiveEventContainsProductIdentity() {
        val event = ArchiveService().archiveEvent(ProductIdentity("42", "SKU-42", "Товар"))

        assertEquals("42:SKU-42:Товар", event)
    }
}
