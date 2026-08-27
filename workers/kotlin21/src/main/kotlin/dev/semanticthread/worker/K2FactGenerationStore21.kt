package dev.semanticthread.worker

import kotlinx.serialization.json.*
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.channels.OverlappingFileLockException
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.nio.file.StandardOpenOption.CREATE
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.READ
import java.nio.file.StandardOpenOption.WRITE
import java.security.MessageDigest
import java.util.Comparator
import java.util.UUID
import kotlin.io.path.invariantSeparatorsPathString

internal data class K2SourceFile21(val path: String, val file: Path, val sourceHash: String)
internal data class K2SourceSnapshot21(val files: List<K2SourceFile21>, val sourceSetDigest: String)
internal data class K2SourceDelta21(
    val addedOrModified: Set<String>, val removed: Set<String>, val unchanged: Set<String>,
) { val isEmpty: Boolean get() = addedOrModified.isEmpty() && removed.isEmpty() }
internal data class K2GenerationFile21(
    val path: String, val sourceHash: String, val objectHash: String, val factCount: Int,
)
internal data class K2StoredGeneration21(
    val manifestDigest: String,
    val sourceSetDigest: String,
    val graphDigest: String,
    val files: List<K2GenerationFile21>,
    val factsByFile: Map<String, List<JsonObject>>,
) { val facts: List<JsonObject> get() = factsByFile.values.flatten().sortedBy(::canonicalK2Json21) }
internal sealed interface K2Current21 {
    data object Empty : K2Current21
    data class Ready(val generation: K2StoredGeneration21) : K2Current21
    data class RecoveryRequired(val reason: String) : K2Current21
}
internal sealed interface K2StoreLockResult21<out T> {
    data object Busy : K2StoreLockResult21<Nothing>
    data class Acquired<T>(val value: T) : K2StoreLockResult21<T>
}
internal data class K2PublishResult21(
    val generation: K2StoredGeneration21, val committed: Boolean, val cleanupPending: Boolean,
)
internal data class K2StoreHooks21(
    val beforeCommit: () -> Unit = {}, val beforeCleanup: () -> Unit = {},
)

internal class K2FactGenerationStore21 private constructor(
    private val repo: Path,
    internal val stateRoot: Path,
    private val compilation: String,
    private val compilerVersion: String,
    private val configDigest: String,
    private val hooks: K2StoreHooks21,
) {
    companion object {
        fun open(
            indexRoot: Path,
            repo: Path,
            compilation: String,
            compilerVersion: String,
            configDigest: String,
            hooks: K2StoreHooks21 = K2StoreHooks21(),
        ): K2FactGenerationStore21 {
            require(compilation.isNotBlank() && compilerVersion.isNotBlank() && configDigest.isNotBlank())
            val index = checkedDirectory21(indexRoot, create = true)
            val requestedRepo = repo.toAbsolutePath().normalize()
            val canonicalRepo = repo.toRealPath()
            require(requestedRepo == canonicalRepo && Files.isDirectory(canonicalRepo, NOFOLLOW_LINKS)) {
                "repo must be a canonical non-symlink directory"
            }
            val identity = canonicalK2Bytes21(buildJsonObject {
                put("compilerVersion", compilerVersion)
                put("configDigest", configDigest)
            })
            val root = index.resolve("v2")
                .resolve(sha256K221(compilation.toByteArray(StandardCharsets.UTF_8)))
                .resolve(sha256K221(identity))
            val checkedRoot = checkedDirectory21(root, create = true)
            require(checkedRoot.startsWith(index)) { "state root escapes index" }
            return K2FactGenerationStore21(
                canonicalRepo, checkedRoot, compilation, compilerVersion, configDigest, hooks,
            )
        }
    }

    fun <T> withLock(block: Locked.() -> T): K2StoreLockResult21<T> {
        val lockPath = child21("LOCK")
        require(!Files.isSymbolicLink(lockPath)) { "LOCK must not be a symlink" }
        FileChannel.open(lockPath, CREATE, WRITE).use { channel ->
            val lock = try { channel.tryLock() } catch (_: OverlappingFileLockException) { null }
                ?: return K2StoreLockResult21.Busy
            lock.use {
                val scope = Locked()
                return try { K2StoreLockResult21.Acquired(scope.block()) } finally { scope.close() }
            }
        }
    }

    internal inner class Locked internal constructor() {
        private var live = true
        internal fun close() { live = false }
        private fun ensureLive() = check(live) { "store operation escaped lock scope" }

        fun snapshot(sources: List<Path>): K2SourceSnapshot21 {
            ensureLive()
            val seen = linkedSetOf<String>()
            val files = sources.map { requested ->
                require(requested.isAbsolute) { "source path must be absolute" }
                val absolute = requested.toAbsolutePath()
                require(requested == absolute && requested == requested.normalize()) {
                    "source path must use canonical absolute spelling"
                }
                require(absolute.startsWith(repo) && absolute != repo) { "source escapes repo" }
                requireNoSymlinkComponents21(absolute)
                val real = absolute.toRealPath()
                require(real == absolute && Files.isRegularFile(real, NOFOLLOW_LINKS)) {
                    "source must be a canonical non-symlink regular file"
                }
                val relative = repo.relativize(real).invariantSeparatorsPathString
                require(canonicalRelative21(relative) == relative && relative.endsWith(".kt"))
                require(seen.add(relative)) { "duplicate source: $relative" }
                K2SourceFile21(relative, real, sha256K221(Files.readAllBytes(real)))
            }.sortedBy(K2SourceFile21::path)
            return K2SourceSnapshot21(files, sourceSetDigest21(files.map { it.path to it.sourceHash }))
        }

        fun loadCurrent(): K2Current21 {
            ensureLive()
            if (Files.exists(child21("DIRTY"), NOFOLLOW_LINKS)) {
                return K2Current21.RecoveryRequired("DIRTY")
            }
            val current = child21("CURRENT")
            if (!Files.exists(current, NOFOLLOW_LINKS)) return K2Current21.Empty
            return try { K2Current21.Ready(readGeneration21(current)) }
            catch (_: Exception) { K2Current21.RecoveryRequired("CORRUPT") }
        }

        fun delta(snapshot: K2SourceSnapshot21, current: K2Current21): K2SourceDelta21 {
            ensureLive()
            val previous = (current as? K2Current21.Ready)?.generation
            return delta21(snapshot, previous)
        }

        fun beginDirty(snapshot: K2SourceSnapshot21, current: K2Current21): DirtyTxn {
            ensureLive()
            val recovered = current is K2Current21.RecoveryRequired
            if (recovered) recoverMutable21()
            val previous = (current as? K2Current21.Ready)?.generation?.takeUnless { recovered }
            val full = previous == null
            val delta = delta21(snapshot, previous)
            writeAtomic21(child21("DIRTY"), "dirty\n".toByteArray(StandardCharsets.UTF_8))
            return DirtyTxn(this, snapshot, previous, delta, full, recovered)
        }

        internal fun assertLive() = ensureLive()
    }

    internal inner class DirtyTxn internal constructor(
        private val owner: Locked,
        val snapshot: K2SourceSnapshot21,
        private val previous: K2StoredGeneration21?,
        val delta: K2SourceDelta21,
        val full: Boolean,
        val recovered: Boolean,
    ) {
        private var active = true

        fun publishCompiledGeneration(lines: List<String>): K2PublishResult21 {
            owner.assertLive()
            check(active) { "dirty transaction already consumed" }
            active = false
            val after = owner.snapshot(snapshot.files.map(K2SourceFile21::file))
            require(after == snapshot) { "sources changed after compiler snapshot" }
            val currentPaths = snapshot.files.map(K2SourceFile21::path).toSet()
            val required = if (full) currentPaths else delta.addedOrModified
            val batch = parseFacts21(lines, currentPaths, required, full)
            val sources = snapshot.files.associateBy(K2SourceFile21::path)
            val mergedFiles = linkedMapOf<String, K2GenerationFile21>()
            val mergedFacts = linkedMapOf<String, List<JsonObject>>()
            if (!full) {
                val oldFiles = requireNotNull(previous).files.associateBy(K2GenerationFile21::path)
                delta.unchanged.sorted().forEach { path ->
                    val old = requireNotNull(oldFiles[path])
                    require(old.sourceHash == requireNotNull(sources[path]).sourceHash)
                    mergedFiles[path] = old
                    mergedFacts[path] = requireNotNull(previous.factsByFile[path])
                }
            }
            batch.receipts.sorted().forEach { path ->
                val source = requireNotNull(sources[path])
                val facts = batch.factsByFile[path].orEmpty()
                val shard = buildJsonObject {
                    put("schema", "codeclew-k2-fact-shard/0.1")
                    put("path", path)
                    put("sourceHash", source.sourceHash)
                    put("facts", JsonArray(facts))
                }
                val bytes = canonicalK2Bytes21(shard)
                val hash = sha256K221(bytes)
                writeImmutable21(child21("objects/$hash.json"), bytes, hash)
                mergedFiles[path] = K2GenerationFile21(path, source.sourceHash, hash, facts.size)
                mergedFacts[path] = facts
            }
            require(mergedFiles.keys == currentPaths && mergedFacts.keys == currentPaths)
            val files = mergedFiles.values.sortedBy(K2GenerationFile21::path)
            val facts = mergedFacts.toSortedMap().values.flatten().sortedBy(::canonicalK2Json21)
            val graphDigest = graphDigest21(facts)
            val manifest = buildJsonObject {
                put("schema", "codeclew-k2-index-manifest/0.2")
                put("backend", "K2_FACT_GENERATION_STORE_21")
                put("compilation", compilation)
                put("compilerVersion", compilerVersion)
                put("configDigest", configDigest)
                put("sourceSetDigest", snapshot.sourceSetDigest)
                put("graphDigest", graphDigest)
                put("files", JsonArray(files.map { file -> buildJsonObject {
                    put("path", file.path); put("sourceHash", file.sourceHash)
                    put("objectHash", file.objectHash); put("factCount", file.factCount)
                } }))
            }
            val manifestBytes = canonicalK2Bytes21(manifest)
            val manifestDigest = sha256K221(manifestBytes)
            writeImmutable21(child21("generations/$manifestDigest.json"), manifestBytes, manifestDigest)
            hooks.beforeCommit()
            writeAtomic21(child21("CURRENT"), "$manifestDigest\n".toByteArray(StandardCharsets.UTF_8))
            val generation = K2StoredGeneration21(
                manifestDigest, snapshot.sourceSetDigest, graphDigest, files, mergedFacts.toSortedMap(),
            )
            val cleanupPending = try {
                hooks.beforeCleanup()
                if (Files.deleteIfExists(child21("DIRTY"))) forceDirectory21(stateRoot)
                false
            } catch (_: Exception) {
                true
            }
            return K2PublishResult21(generation, committed = true, cleanupPending = cleanupPending)
        }
    }

    private fun readGeneration21(current: Path): K2StoredGeneration21 {
        require(!Files.isSymbolicLink(current) && Files.isRegularFile(current, NOFOLLOW_LINKS))
        val pointer = Files.readString(current, StandardCharsets.UTF_8)
        require(pointer.length == 65 && pointer[64] == '\n' && HEX_K2_21.matches(pointer.substring(0, 64)))
        val manifestDigest = pointer.substring(0, 64)
        val manifest = readSealedJson21(child21("generations/$manifestDigest.json"), manifestDigest)
        require(manifest.keys == setOf(
            "schema", "backend", "compilation", "compilerVersion", "configDigest",
            "sourceSetDigest", "graphDigest", "files",
        ))
        require(string21(manifest, "schema") == "codeclew-k2-index-manifest/0.2")
        require(string21(manifest, "backend") == "K2_FACT_GENERATION_STORE_21")
        require(string21(manifest, "compilation") == compilation)
        require(string21(manifest, "compilerVersion") == compilerVersion)
        require(string21(manifest, "configDigest") == configDigest)
        val rows = manifest["files"]?.jsonArray?.map { it.jsonObject }
            ?: throw IllegalArgumentException("manifest has no files")
        val files = rows.map { row ->
            require(row.keys == setOf("path", "sourceHash", "objectHash", "factCount"))
            val path = canonicalRelative21(string21(row, "path"))
            val sourceHash = string21(row, "sourceHash").also { require(HEX_K2_21.matches(it)) }
            val objectHash = string21(row, "objectHash").also { require(HEX_K2_21.matches(it)) }
            val countPrimitive = row["factCount"]?.jsonPrimitive
                ?: throw IllegalArgumentException("manifest file has no factCount")
            require(!countPrimitive.isString)
            val factCount = requireNotNull(countPrimitive.intOrNull).also { require(it >= 0) }
            K2GenerationFile21(path, sourceHash, objectHash, factCount)
        }
        require(files.map(K2GenerationFile21::path) == files.map(K2GenerationFile21::path).sorted())
        require(files.map(K2GenerationFile21::path).distinct().size == files.size)
        val factsByFile = linkedMapOf<String, List<JsonObject>>()
        files.forEach { file ->
            val shard = readSealedJson21(child21("objects/${file.objectHash}.json"), file.objectHash)
            require(shard.keys == setOf("schema", "path", "sourceHash", "facts"))
            require(string21(shard, "schema") == "codeclew-k2-fact-shard/0.1")
            require(string21(shard, "path") == file.path && string21(shard, "sourceHash") == file.sourceHash)
            val facts = shard["facts"]?.jsonArray?.map { it.jsonObject }
                ?: throw IllegalArgumentException("shard has no facts")
            require(facts.size == file.factCount)
            require(facts.map(::canonicalK2Json21) == facts.map(::canonicalK2Json21).sorted())
            require(facts.map(::canonicalK2Json21).distinct().size == facts.size)
            facts.forEach { fact ->
                require(string21(fact, "file") == file.path)
                require(string21(fact, "recordType") != "FIR_FILE_RECEIPT")
            }
            factsByFile[file.path] = facts
        }
        val sourceSetDigest = sourceSetDigest21(files.map { it.path to it.sourceHash })
        val facts = factsByFile.values.flatten().sortedBy(::canonicalK2Json21)
        val graphDigest = graphDigest21(facts)
        require(string21(manifest, "sourceSetDigest") == sourceSetDigest)
        require(string21(manifest, "graphDigest") == graphDigest)
        return K2StoredGeneration21(manifestDigest, sourceSetDigest, graphDigest, files, factsByFile)
    }

    private fun readSealedJson21(path: Path, expectedHash: String): JsonObject {
        require(!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS))
        val bytes = Files.readAllBytes(path)
        require(sha256K221(bytes) == expectedHash) { "sealed object hash mismatch" }
        val parsed = Json.parseToJsonElement(bytes.toString(StandardCharsets.UTF_8)).jsonObject
        require(bytes.contentEquals(canonicalK2Bytes21(parsed))) { "sealed object is not canonical" }
        return parsed
    }

    private fun recoverMutable21() {
        val mutable = child21("mutable")
        if (Files.exists(mutable, NOFOLLOW_LINKS)) {
            Files.walk(mutable).use { paths ->
                paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
            }
        }
        checkedDirectory21(mutable, create = true)
    }

    private fun writeImmutable21(path: Path, bytes: ByteArray, digest: String) {
        require(sha256K221(bytes) == digest && path.fileName.toString() == "$digest.json")
        checkedDirectory21(path.parent, create = true)
        if (Files.exists(path, NOFOLLOW_LINKS)) {
            val existing = if (!Files.isSymbolicLink(path) && Files.isRegularFile(path, NOFOLLOW_LINKS)) {
                Files.readAllBytes(path)
            } else null
            if (existing != null && existing.contentEquals(bytes)) return
            if (existing != null && sha256K221(existing) == digest) {
                throw IllegalStateException("valid differing bytes at immutable digest")
            }
            quarantineOrDelete21(path)
        }
        val temporary = path.parent.resolve(".${path.fileName}.${UUID.randomUUID()}.tmp")
        try {
            writeDurable21(temporary, bytes)
            Files.move(temporary, path, ATOMIC_MOVE)
            forceDirectory21(path.parent)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    private fun writeAtomic21(path: Path, bytes: ByteArray) {
        checkedDirectory21(path.parent, create = true)
        require(!Files.isSymbolicLink(path)) { "atomic target must not be a symlink" }
        val temporary = path.parent.resolve(".${path.fileName}.${UUID.randomUUID()}.tmp")
        try {
            writeDurable21(temporary, bytes)
            Files.move(temporary, path, ATOMIC_MOVE, REPLACE_EXISTING)
            forceDirectory21(path.parent)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    private fun quarantineOrDelete21(path: Path) {
        val quarantine = checkedDirectory21(child21("quarantine"), create = true)
        val target = quarantine.resolve("${path.fileName}.${UUID.randomUUID()}")
        try {
            Files.move(path, target, ATOMIC_MOVE)
        } catch (_: Exception) {
            if (Files.deleteIfExists(path)) forceDirectory21(path.parent)
            return
        }
        forceDirectory21(path.parent)
        forceDirectory21(quarantine)
    }

    private fun child21(relative: String): Path {
        val child = stateRoot.resolve(relative).normalize()
        require(child.startsWith(stateRoot) && child != stateRoot) { "state path escapes root" }
        var cursor = stateRoot
        stateRoot.relativize(child).forEach { segment ->
            cursor = cursor.resolve(segment)
            if (Files.exists(cursor, NOFOLLOW_LINKS)) {
                require(!Files.isSymbolicLink(cursor)) { "state path contains a symlink" }
            }
        }
        return child
    }
}

private data class FactBatch21(
    val receipts: Set<String>, val factsByFile: Map<String, List<JsonObject>>,
)

private fun parseFacts21(
    lines: List<String>, current: Set<String>, required: Set<String>, full: Boolean,
): FactBatch21 {
    require(lines.all { it.isNotBlank() }) { "blank compiler fact row" }
    val receipts = linkedSetOf<String>()
    val facts = linkedMapOf<String, MutableList<JsonObject>>()
    val seenFacts = linkedSetOf<String>()
    lines.forEach { line ->
        val row = Json.parseToJsonElement(line).jsonObject
        val recordType = string21(row, "recordType")
        val path = canonicalRelative21(string21(row, "file"))
        require(path in current) { "compiler row is outside current sources" }
        if (recordType == "FIR_FILE_RECEIPT") {
            require(row.keys == setOf("recordType", "schema", "file"))
            require(string21(row, "schema") == "fir-file-receipt/0.1")
            require(receipts.add(path)) { "duplicate file receipt" }
        } else {
            val canonical = canonicalK2Json21(row)
            require(seenFacts.add(canonical)) { "duplicate compiler fact" }
            facts.getOrPut(path, ::mutableListOf).add(row)
        }
    }
    require(facts.keys.all(receipts::contains)) { "fact without file receipt" }
    if (full) require(receipts == current) { "full receipt closure is incomplete" }
    else require(receipts.containsAll(required)) { "incremental receipt closure misses source delta" }
    return FactBatch21(receipts, facts.mapValues { (_, rows) -> rows.sortedBy(::canonicalK2Json21) })
}

private fun delta21(snapshot: K2SourceSnapshot21, previous: K2StoredGeneration21?): K2SourceDelta21 {
    val current = snapshot.files.associate { it.path to it.sourceHash }
    val old = previous?.files?.associate { it.path to it.sourceHash }.orEmpty()
    val changed = current.filter { (path, hash) -> old[path] != hash }.keys
    val removed = old.keys - current.keys
    val unchanged = current.keys - changed
    return K2SourceDelta21(changed.toSortedSet(), removed.toSortedSet(), unchanged.toSortedSet())
}

private fun canonicalRelative21(raw: String): String {
    require(raw.isNotEmpty() && '\\' !in raw && !raw.startsWith('/') && !raw.matches(Regex("^[A-Za-z]:.*")))
    val segments = raw.split('/')
    require(segments.none { it.isEmpty() || it == "." || it == ".." })
    val parsed = Path.of(raw)
    require(!parsed.isAbsolute && parsed.normalize().invariantSeparatorsPathString == raw)
    return raw
}

private fun string21(value: JsonObject, key: String): String {
    val primitive = value[key] as? JsonPrimitive
        ?: throw IllegalArgumentException("$key must be a JSON string")
    require(primitive.isString) { "$key must be a JSON string" }
    return primitive.content
}

private fun sourceSetDigest21(files: List<Pair<String, String>>): String = sha256K221(canonicalK2Bytes21(
    JsonArray(files.sortedBy { it.first }.map { (path, hash) -> buildJsonObject {
        put("path", path); put("sourceHash", hash)
    } }),
))

private fun graphDigest21(facts: List<JsonObject>): String = "sha256:" + sha256K221(canonicalK2Bytes21(
    JsonArray(facts.sortedBy(::canonicalK2Json21)),
))

internal fun canonicalK2Json21(value: JsonElement): String = when (value) {
    JsonNull -> "null"
    is JsonObject -> value.entries.sortedBy { it.key }
        .joinToString(separator = ",", prefix = "{", postfix = "}") { (key, child) ->
            "${JsonPrimitive(key)}:${canonicalK2Json21(child)}"
        }
    is JsonArray -> value.joinToString(",", "[", "]", transform = ::canonicalK2Json21)
    is JsonPrimitive -> value.toString()
}

private fun canonicalK2Bytes21(value: JsonElement): ByteArray =
    (canonicalK2Json21(value) + "\n").toByteArray(StandardCharsets.UTF_8)

private fun sha256K221(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
    .digest(bytes).joinToString("") { (it.toInt() and 0xff).toString(16).padStart(2, '0') }

private fun checkedDirectory21(path: Path, create: Boolean): Path {
    val absolute = path.toAbsolutePath().normalize()
    var cursor = requireNotNull(absolute.root) { "directory must be absolute" }
    absolute.forEach { segment ->
        cursor = cursor.resolve(segment)
        if (Files.exists(cursor, NOFOLLOW_LINKS)) {
            require(!Files.isSymbolicLink(cursor) && Files.isDirectory(cursor, NOFOLLOW_LINKS)) {
                "state path component must be a non-symlink directory"
            }
        } else {
            require(create) { "state directory does not exist" }
            Files.createDirectory(cursor)
            forceDirectory21(requireNotNull(cursor.parent))
        }
    }
    require(Files.isDirectory(absolute, NOFOLLOW_LINKS) && absolute.toRealPath() == absolute) {
        "state directory must be canonical and non-symlink"
    }
    return absolute
}

private fun writeDurable21(path: Path, bytes: ByteArray) {
    FileChannel.open(path, CREATE_NEW, WRITE).use { channel ->
        val buffer = ByteBuffer.wrap(bytes)
        while (buffer.hasRemaining()) channel.write(buffer)
        channel.force(true)
    }
}

private fun forceDirectory21(path: Path) {
    require(!Files.isSymbolicLink(path) && Files.isDirectory(path, NOFOLLOW_LINKS))
    FileChannel.open(path, READ).use { it.force(true) }
}

private fun requireNoSymlinkComponents21(path: Path) {
    var cursor = requireNotNull(path.root) { "source path must be absolute" }
    path.forEach { segment ->
        cursor = cursor.resolve(segment)
        require(Files.exists(cursor, NOFOLLOW_LINKS) && !Files.isSymbolicLink(cursor)) {
            "source path contains a missing or symlink component"
        }
    }
}

private val HEX_K2_21 = Regex("[0-9a-f]{64}")
