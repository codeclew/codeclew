package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals

class ConstructorOwnerAuthorityProjectModelCommandTest {
    @Test
    fun exactCompilerOwnerIsTheFinalCanonicalContainment() {
        val authority = constructorOwnerAuthority(
            "p/Outer.Inner",
            listOf("class:p/Outer.Inner", "class:p/Outer", "callable:p/build"),
        )

        assertEquals("p/Outer.Inner", authority.compilerClassId)
        assertEquals("class:p/Outer.Inner", authority.ownerIdentity)
        assertEquals(
            listOf("class:p/Outer", "callable:p/build", "class:p/Outer.Inner"),
            authority.containment,
        )
    }
}
