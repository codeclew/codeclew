package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class KotlinEngineCompatibilityTest {
    @Test
    fun kotlin19OptionsAreQualifiedOnlyWithinTheMeasuredScope() {
        for (version in listOf("1.9.24", "1.9.25")) {
            val project = KotlinProjectSemantics(version, "TEST", "1.9", "1.9", "17", emptyList(), emptyList())
            for (jsr in listOf("strict", "warn", "ignore")) for (defaults in listOf("disable", "all", "all-compatibility")) {
                val options = listOf("-Xjsr305=$jsr", "-Xjvm-default=$defaults")
                val decision = kotlinEngineCompatibilityDecision(project.copy(unstableCompilerOptions = options))
                assertEquals("QUALIFIED", decision.status)
                assertEquals("COMPATIBLE_ANALYSIS", decision.kind)
                assertFalse(decision.btaEligible)
                assertEquals(options, kotlinAnalysisCompilerArguments(version, options))
            }
            for (options in listOf(listOf("-Xjsr305"), listOf("-Xjsr305=under-migration:strict"), listOf("-Xjsr305=@private.Annotation:warn"), listOf("-Xjvm-default=enable"), listOf("-Xjsr305=strict", "-Xjsr305=ignore"))) {
                assertEquals("REJECTED", kotlinEngineCompatibilityDecision(project.copy(unstableCompilerOptions = options)).status)
            }
            val configured = project.copy(unstableCompilerOptions = listOf("-Xjsr305=strict"))
            for (outside in listOf(configured.copy(projectCompilerVersion = "1.9.23"), configured.copy(jvmTarget = "21"), configured.copy(languageVersion = "2.0"), configured.copy(apiVersion = "1.8"))) {
                assertEquals("REJECTED", kotlinEngineCompatibilityDecision(outside).status)
            }
        }
    }

    @Test
    fun kotlin19AnnotationTargetNormalizationPreservesNativeAndUnknownArguments() {
        val original = listOf("-Xannotation-default-target=param-property", "-Xunknown=keep", "-java-parameters")
        assertEquals(listOf("-Xunknown=keep", "-java-parameters"), kotlinAnalysisCompilerArguments("1.9.25", original))
        assertEquals(original, kotlinAnalysisCompilerArguments("2.3.0", original))
        assertEquals(3, original.size)
    }

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
    fun qualifiedPatchLineUsesTheProductionK24Route() {
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
        assertEquals("QUALIFIED", decision.status)
        assertEquals("QUALIFIED_PATCH_LINE", decision.kind)
        assertTrue(decision.btaEligible)
    }

    @Test
    fun olderProjectCompilerHasReadOnlyBaseline() {
        val project = KotlinProjectSemantics(
            projectCompilerVersion = "2.1.21",
            compilerVersionAuthority = "KOTLIN_COMPILER_VERSION_CLASSLOADER_FALLBACK",
            languageVersion = "2.1",
            apiVersion = "2.1",
            jvmTarget = "21",
            compilerPlugins = emptyList(),
            unstableCompilerOptions = emptyList(),
        )

        val decision = kotlinEngineCompatibilityDecision(project)
        assertEquals("QUALIFIED", decision.status)
        assertEquals("COMPATIBLE_ANALYSIS", decision.kind)
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
