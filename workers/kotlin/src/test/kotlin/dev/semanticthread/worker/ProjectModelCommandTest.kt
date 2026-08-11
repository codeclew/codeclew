package dev.semanticthread.worker

import java.nio.file.Files
import kotlin.io.path.createFile
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class ProjectModelCommandTest {
    @Test
    fun gradlePlanIsOfflineAndUsesOnlyRepoOwnedHome() {
        val repo = Files.createTempDirectory("worker-gradle-plan").toRealPath()
        try {
            val wrapper = repo.resolve("gradlew").createFile()
            val init = repo.resolve("model.init.gradle").createFile()
            val home = repoOwnedStateDirectory(repo, ".gradle")
            val command = gradleModelCommand(
                wrapper,
                repo,
                home,
                init,
                "compileKotlin",
                ":semanticThreadModel",
            )
            assertEquals(1, command.count { it == "--offline" })
            assertEquals(1, command.count { it == "--gradle-user-home" })
            assertEquals(home.toString(), command[command.indexOf("--gradle-user-home") + 1])
            assertTrue(home.startsWith(repo))
            assertEquals(".gradle", repo.relativize(home).toString())
            val process = sanitizedProjectModelProcess(command, repo)
            for (key in listOf("GRADLE_OPTS", "GRADLE_USER_HOME", "MAVEN_OPTS", "MAVEN_ARGS", "MAVEN_CONFIG")) {
                assertFalse(process.environment().containsKey(key))
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun mavenPlanIsOfflineAndUsesOnlyRepoOwnedRepository() {
        val repo = Files.createTempDirectory("worker-maven-plan").toRealPath()
        try {
            val command = mavenModelCommand(listOf(repo.resolve("mvnw").toString()), repo, listOf("help:effective-pom"))
            assertEquals(1, command.count { it == "-o" })
            assertEquals(1, command.count { it.startsWith("-Dmaven.repo.local=") })
            val configured = command.single { it.startsWith("-Dmaven.repo.local=") }
                .substringAfter('=')
            val localRepository = java.nio.file.Path.of(configured).toRealPath()
            assertTrue(localRepository.startsWith(repo))
            assertEquals(
                ".semantic-thread/maven-repository",
                repo.relativize(localRepository).toString(),
            )
            val process = sanitizedProjectModelProcess(command, repo)
            for (key in listOf("GRADLE_OPTS", "GRADLE_USER_HOME", "MAVEN_OPTS", "MAVEN_ARGS", "MAVEN_CONFIG")) {
                assertFalse(process.environment().containsKey(key))
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun repoOwnedStateRejectsSymlinkAndPathEscape() {
        val repo = Files.createTempDirectory("worker-state-containment").toRealPath()
        val external = Files.createTempDirectory("worker-state-external").toRealPath()
        try {
            Files.createSymbolicLink(repo.resolve(".gradle"), external)
            assertFailsWith<WorkerFailure> {
                repoOwnedStateDirectory(repo, ".gradle")
            }
            assertFailsWith<IllegalArgumentException> {
                repoOwnedStateDirectory(repo, "..", "outside")
            }
        } finally {
            repo.toFile().deleteRecursively()
            external.toFile().deleteRecursively()
        }
    }
}
