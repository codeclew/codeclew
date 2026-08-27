package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class KotlinEngineCompatibilityTest {
    @Test
    fun projectSemanticsAndSemanticEngineRemainSeparateAuthorities() {
        val project = KotlinProjectSemantics(
            projectCompilerVersion = "2.4.10",
            compilerVersionAuthority = "KGP_COMPILER_VERSION_PROVIDER",
            languageVersion = "2.3",
            apiVersion = "2.3",
            jvmTarget = "21",
            compilerPlugins = emptyList(),
            unstableCompilerOptions = emptyList(),
        )
        val engine = currentKotlinSemanticEngine()
        val decision = kotlinEngineCompatibilityDecision(project, engine)

        assertEquals("2.3", project.languageVersion)
        assertEquals(WORKER_COMPILER_VERSION, engine.analyzerCompilerVersion)
        assertEquals("kotlin-engine-$WORKER_COMPILER_VERSION", engine.engineId)
        assertEquals("QUALIFIED", decision.status)
        assertEquals("EXACT_COMPILER_ABI", decision.kind)
        assertTrue(decision.btaEligible)
    }

    @Test
    fun unqualifiedProjectCompilerIsNotInferredFromLanguageVersion() {
        val project = KotlinProjectSemantics(
            projectCompilerVersion = "2.4.0",
            compilerVersionAuthority = "KGP_COMPILER_VERSION_PROVIDER",
            languageVersion = "2.4",
            apiVersion = "2.4",
            jvmTarget = "21",
            compilerPlugins = emptyList(),
            unstableCompilerOptions = emptyList(),
        )

        val decision = kotlinEngineCompatibilityDecision(project)
        assertEquals("REJECTED", decision.status)
        assertEquals("UNQUALIFIED", decision.kind)
        assertFalse(decision.btaEligible)
    }

    @Test
    fun gradleModelPrefersKgpCompilerVersionProviderAndReportsAuthority() {
        val script = checkNotNull(
            KotlinEngineCompatibilityTest::class.java.getResource("/semantic-thread-model.init.gradle"),
        ).readText()
        val provider = script.indexOf("KGP_COMPILER_VERSION_PROVIDER")
        val fallback = script.indexOf("KOTLIN_COMPILER_VERSION_CLASSLOADER_FALLBACK")

        assertTrue(provider >= 0)
        assertTrue(fallback > provider)
        assertTrue(script.contains("projectCompilerAuthority"))
    }
}
