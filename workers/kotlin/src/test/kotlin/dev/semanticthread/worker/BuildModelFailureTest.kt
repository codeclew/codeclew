package dev.semanticthread.worker

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class BuildModelFailureTest {
    @Test
    fun publicFailuresGiveCausalActionsWithoutDisclosingBuildOutput() {
        val cases = listOf(
            "Could not transfer artifact; status code: 401" to "BUILD_REPOSITORY_AUTHENTICATION",
            "Could not get resource: PKIX path building failed" to "BUILD_REPOSITORY_TLS",
            "Non-resolvable parent POM" to "BUILD_DEPENDENCY_RESOLUTION",
            "invalid source release: 21" to "BUILD_JDK_CONFIGURATION",
            "Task 'compileKotlin' not found" to "BUILD_COMPILATION_NOT_FOUND",
            "Something unexpected" to "BUILD_MODEL_EXTRACTION_FAILED",
            "Task :compileKotlin failed; cannot access class Example; symbol not found" to "BUILD_MODEL_EXTRACTION_FAILED",
        )
        for ((output, reason) in cases) {
            for (tool in ProjectModelBuildTool.entries) {
                val message = buildModelFailure(tool, "$output https://private.invalid/?token=secret /private/service/pom.xml").message.orEmpty()
                assertTrue(message.startsWith(reason), message)
                assertTrue("then retry" in message || "then retry Codeclew" in message, message)
                assertTrue(if (tool == ProjectModelBuildTool.MAVEN) "mvn -e" in message else "./gradlew --stacktrace" in message)
                assertFalse("private.invalid" in message)
                assertFalse("secret" in message)
                assertFalse("/private/service" in message)
            }
        }
    }
}
