package dev.semanticthread.worker

import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.HexFormat
import kotlin.io.path.invariantSeparatorsPathString
import kotlin.io.path.readBytes

internal data class ProjectModelInput(val path: String, val hash: String)

/** One request's input inventory; discarded after build-tool execution and at the next RPC. */
internal data class ProjectModelInventory(
    val modelInputs: List<ProjectModelInput>,
    val sourcePaths: List<String>,
) {
    private val rawHash by lazy { computeInputHash("RAW") }
    private val canonicalHash by lazy { computeInputHash("CANONICAL") }

    fun inputHash(view: String): String = when (view) {
        "RAW" -> rawHash
        "CANONICAL" -> canonicalHash
        else -> error("unknown project model view")
    }

    private fun computeInputHash(view: String): String = digest((
        listOf("projectModelSchema=8", "projectModelView=$view") +
            modelInputs.map { "${it.path}:${it.hash}" } + sourcePaths
        ).joinToString("\n").toByteArray())

    companion object {
        private fun digest(bytes: ByteArray): String =
            "sha256:" + HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes))
        private val excludedDirectories = setOf("build", "target", ".gradle", ".kotlin", ".semantic-thread", ".git")
        private val modelNames = setOf(
            "settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts", "gradle.properties",
            "libs.versions.toml", "gradle-wrapper.properties", "gradle-wrapper.jar", "gradlew", "gradlew.bat",
            "pom.xml", "mvnw", "mvnw.cmd",
        )

        fun capture(repo: Path): ProjectModelInventory {
            val modelFiles = mutableListOf<Path>()
            val sourcePaths = mutableListOf<String>()
            Files.walkFileTree(repo, object : SimpleFileVisitor<Path>() {
                override fun preVisitDirectory(dir: Path, attrs: BasicFileAttributes): FileVisitResult =
                    if (dir != repo && dir.fileName.toString() in excludedDirectories) FileVisitResult.SKIP_SUBTREE
                    else FileVisitResult.CONTINUE

                override fun visitFile(file: Path, attrs: BasicFileAttributes): FileVisitResult {
                    if (!attrs.isRegularFile) return FileVisitResult.CONTINUE
                    val relative = repo.relativize(file).invariantSeparatorsPathString
                    if (file.fileName.toString().endsWith(".kt")) sourcePaths.add(relative)
                    if (file.fileName.toString() in modelNames ||
                        listOf(".mvn/", "buildSrc/", "build-logic/", "gradle/").any(relative::startsWith)) {
                        modelFiles.add(file)
                    }
                    return FileVisitResult.CONTINUE
                }
            })
            return ProjectModelInventory(
                modelFiles.sorted().map { ProjectModelInput(repo.relativize(it).invariantSeparatorsPathString, digest(it.readBytes())) },
                sourcePaths.sorted(),
            )
        }
    }
}
