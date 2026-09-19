package dev.semanticthread.worker

import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

class CompilerConfigurationAuthorityTest {
    @Test
    fun sourceBytesChangeFullIdentityButKeepBtaConfigurationIdentity() {
        val repo = Files.createTempDirectory("compiler-configuration-authority").toRealPath()
        try {
            val source = repo.resolve("A.kt").also { it.writeText("class A") }
            val before = manifest(repo, listOf(source), listOf(source))
            val beforeFull = fullKey(before)
            val beforeConfiguration = btaSemanticConfigurationDigest(before)

            source.writeText("class A { val changed = true }")
            val after = manifest(repo, listOf(source), listOf(source))

            assertNotEquals(beforeFull, fullKey(after))
            assertEquals(beforeConfiguration, btaSemanticConfigurationDigest(after))
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun sourceMembershipChangesFullIdentityButKeepBtaConfigurationIdentity() {
        val repo = Files.createTempDirectory("compiler-configuration-membership").toRealPath()
        try {
            val a = repo.resolve("A.kt").also { it.writeText("class A") }
            val b = repo.resolve("B.kt").also { it.writeText("class B") }
            val before = manifest(repo, listOf(a), listOf(a))
            val added = manifest(repo, listOf(a, b), listOf(a, b))
            val removed = manifest(repo, listOf(b), listOf(b))
            val configuration = btaSemanticConfigurationDigest(before)

            assertNotEquals(fullKey(before), fullKey(added))
            assertNotEquals(fullKey(added), fullKey(removed))
            assertEquals(configuration, btaSemanticConfigurationDigest(added))
            assertEquals(configuration, btaSemanticConfigurationDigest(removed))
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun compilerConfigurationInputsInvalidateBtaIdentity() {
        val repo = Files.createTempDirectory("compiler-configuration-fields").toRealPath()
        try {
            val source = repo.resolve("A.kt").also { it.writeText("class A") }
            val baseline = manifest(repo, listOf(source), listOf(source))
            val expected = btaSemanticConfigurationDigest(baseline)
            assertNotEquals(
                expected,
                btaSemanticConfigurationDigest(
                    baseline.withField("sourceSelectionSchema", "kotlin-source-selection/1.1"),
                ),
            )

            val changed = listOf(
                baseline.withField(
                    "orderedCompileClasspath",
                    JsonArray(listOf(JsonPrimitive("lib/b.jar"), JsonPrimitive("lib/a.jar"))),
                ),
                baseline.withField(
                    "classpathAuthority",
                    buildJsonObject {
                        put("orderedDigest", "sha256:${"b".repeat(64)}")
                        put("bytesDigest", "sha256:${"c".repeat(64)}")
                    },
                ),
                baseline.withField("jdkHomeFingerprint", "sha256:${"b".repeat(64)}"),
                baseline.withField("analysisJvmFingerprint", "sha256:${"c".repeat(64)}"),
                baseline.withField(
                    "orderedCompilerPlugins",
                    JsonArray(listOf(JsonPrimitive("plugin-b.jar"))),
                ),
                baseline.withField(
                    "orderedCompilerPluginOptions",
                    JsonArray(listOf(JsonPrimitive("plugin:changed=true"))),
                ),
                baseline.withField(
                    "orderedFreeCompilerArguments",
                    JsonArray(listOf(JsonPrimitive("-Xchanged"))),
                ),
                baseline.withField(
                    "orderedOptIns",
                    JsonArray(listOf(JsonPrimitive("changed.OptIn"))),
                ),
                baseline.withField("target", "17"),
                baseline.withField("languageVersion", "2.3"),
                baseline.withField("apiVersion", "2.3"),
                baseline.withField("unknownFutureConfiguration", "v2"),
            )

            changed.forEach { variant ->
                assertNotEquals(expected, btaSemanticConfigurationDigest(variant))
            }
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    @Test
    fun unknownOrLegacySourceAuthorityNeverStripsSourceFields() {
        val repo = Files.createTempDirectory("compiler-configuration-versioning").toRealPath()
        try {
            val source = repo.resolve("A.kt").also { it.writeText("class A") }
            val beforeAuthority = sourceSelectionAuthority(repo, model(listOf(source), listOf(source)))
            val before = manifest(repo, listOf(source), listOf(source))
            source.writeText("class A { val changed = true }")
            val afterAuthority = sourceSelectionAuthority(repo, model(listOf(source), listOf(source)))
            val after = replaceSourceAuthority(before, afterAuthority)

            val unknownBefore = before.withField("sourceSelectionSchema", "kotlin-source-selection/9.9")
            val unknownAfter = replaceSourceAuthority(unknownBefore, afterAuthority)
                .withField("sourceSelectionSchema", "kotlin-source-selection/9.9")
            assertNotEquals(
                btaSemanticConfigurationDigest(unknownBefore),
                btaSemanticConfigurationDigest(unknownAfter),
            )

            val legacyBefore = withoutField(replaceSourceAuthority(before, beforeAuthority), "sourceSelectionSchema")
            val legacyAfter = withoutField(replaceSourceAuthority(before, afterAuthority), "sourceSelectionSchema")
            assertNotEquals(
                btaSemanticConfigurationDigest(legacyBefore),
                btaSemanticConfigurationDigest(legacyAfter),
            )

            val malformedBefore = malformedSourceRole(before, beforeAuthority["sourceFiles"]!!.jsonObject["digest"]!!.jsonPrimitive.content)
            val malformedAfter = malformedSourceRole(after, afterAuthority["sourceFiles"]!!.jsonObject["digest"]!!.jsonPrimitive.content)
            assertNotEquals(
                btaSemanticConfigurationDigest(malformedBefore),
                btaSemanticConfigurationDigest(malformedAfter),
            )
        } finally {
            repo.toFile().deleteRecursively()
        }
    }

    private fun manifest(repo: Path, sourceFiles: List<Path>, analysisSourceFiles: List<Path>): JsonObject =
        buildJsonObject {
            put("schema", "kotlin-semantic-input-manifest/0.1")
            put("compilation", ":app/main")
            put("declaredCompilerVersion", "2.4.10")
            put("analyzerCompilerVersion", "2.4.10")
            sourceSelectionAuthority(repo, model(sourceFiles, analysisSourceFiles)).forEach(::put)
            putJsonArray("orderedCompileClasspath") {
                add(JsonPrimitive("lib/a.jar"))
                add(JsonPrimitive("lib/b.jar"))
            }
            putJsonObject("classpathAuthority") {
                put("orderedDigest", "sha256:${"a".repeat(64)}")
                put("bytesDigest", "sha256:${"a".repeat(64)}")
            }
            put("jdkHomeFingerprint", "sha256:${"a".repeat(64)}")
            putJsonArray("orderedCompilerPlugins") { add(JsonPrimitive("plugin-a.jar")) }
            putJsonArray("orderedCompilerPluginOptions") { add(JsonPrimitive("plugin:mode=stable")) }
            putJsonArray("orderedFreeCompilerArguments") { add(JsonPrimitive("-Xstable")) }
            putJsonArray("orderedOptIns") { add(JsonPrimitive("stable.OptIn")) }
            put("target", "21")
            put("languageVersion", "2.4")
            put("apiVersion", "2.4")
        }

    private fun model(sourceFiles: List<Path>, analysisSourceFiles: List<Path>) = buildJsonObject {
        putJsonArray("sourceFiles") { sourceFiles.forEach { add(JsonPrimitive(it.toString())) } }
        putJsonArray("analysisSourceFiles") { analysisSourceFiles.forEach { add(JsonPrimitive(it.toString())) } }
    }

    private fun fullKey(manifest: JsonObject): String = semanticK2CacheKey(
        buildJsonObject {
            put("extractorSchema", FIR_FACTS_EXTRACTOR_SCHEMA)
            put("pluginArtifactFingerprint", "sha256:${"e".repeat(64)}")
            put("workerCompilerVersion", "2.4.10")
            put("workerVersion", "0.1.0")
            put("workerProtocolVersion", "1.0")
        },
        canonicalJsonForTest(manifest),
    )

    private fun JsonObject.withField(name: String, value: kotlinx.serialization.json.JsonElement): JsonObject =
        JsonObject(this + (name to value))

    private fun JsonObject.withField(name: String, value: String): JsonObject =
        withField(name, JsonPrimitive(value))

    private fun withoutField(value: JsonObject, name: String): JsonObject =
        JsonObject(value.filterKeys { it != name })

    private fun replaceSourceAuthority(value: JsonObject, authority: JsonObject): JsonObject = buildJsonObject {
        value.forEach { (key, entry) ->
            if (key !in setOf("sourceSelectionSchema", "sourceFiles", "analysisSourceFiles")) put(key, entry)
        }
        authority.forEach(::put)
    }

    private fun malformedSourceRole(value: JsonObject, digest: String): JsonObject = buildJsonObject {
        value.forEach { (key, entry) -> if (key != "sourceFiles") put(key, entry) }
        putJsonObject("sourceFiles") { put("digest", digest) }
    }

    private fun canonicalJsonForTest(value: JsonElement): String = when (value) {
        is JsonObject -> value.entries
            .sortedBy { it.key }
            .joinToString(",", prefix = "{", postfix = "}") { (key, entry) ->
                "${JsonPrimitive(key)}:${canonicalJsonForTest(entry)}"
            }
        is JsonArray -> value.joinToString(",", prefix = "[", postfix = "]", transform = ::canonicalJsonForTest)
        else -> value.toString()
    }
}
