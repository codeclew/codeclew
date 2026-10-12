package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment
import org.jetbrains.kotlin.com.intellij.openapi.util.Disposer
import org.jetbrains.kotlin.com.intellij.psi.util.PsiTreeUtil
import org.jetbrains.kotlin.config.CompilerConfiguration
import org.jetbrains.kotlin.psi.KtNamedFunction
import org.jetbrains.kotlin.psi.KtPsiFactory
import kotlin.test.*
import org.jetbrains.org.objectweb.asm.ClassReader
import org.jetbrains.org.objectweb.asm.ClassVisitor
import org.jetbrains.org.objectweb.asm.MethodVisitor
import org.jetbrains.org.objectweb.asm.Opcodes

/** Real K2 2.4 compiler occurrences; no manufactured name-to-symbol bindings. */
class KotlinDocumentationVariablesTest {
    private fun compile(source: String): List<JsonObject> = compileAll(source).filter {
        it["recordType"]?.jsonPrimitive?.content in setOf("DOCUMENTATION_VARIABLE", "DOCUMENTATION_DATA_BOUNDARY")
    }
    private fun compileAll(source: String): List<JsonObject> = compileSources(mapOf("Variables.kt" to source))
    private fun compileSources(sources: Map<String, String>, verifyBytecode: ((Path, List<JsonObject>) -> Unit)? = null): List<JsonObject> {
        val root = Files.createTempDirectory("kotlin-documentation-variables").toRealPath()
        try {
            val inputs = sources.map { (name, text) -> Files.writeString(root.resolve(name), text).toString() }
            val facts = root.resolve("facts.jsonl")
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream ->
                synchronized(K2JVMCompiler::class.java) {
                    K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "21",
                        "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(),
                        "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", *inputs.toTypedArray())
                }
            }
            assertEquals(0, status.code, output.toString())
            val rows = Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
            verifyBytecode?.invoke(root.resolve("classes"), rows)
            return rows
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    private fun extractData(source: String): Map<String, JsonObject> = extractDataSources(mapOf("Variables.kt" to source), "Variables.kt")
    private fun extractDataSources(sources: Map<String, String>, selectedFile: String): Map<String, JsonObject> {
        val source = sources[selectedFile]!!
        val rows = compileSources(sources)
        val descriptors = rows.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" }.map { row ->
            val name = Path.of(row["file"]!!.jsonPrimitive.content).fileName.toString()
            val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.fromCompilerInput(sources[name]!!))
            val range = assertNotNull(coordinates.range(row["start"]!!.jsonPrimitive.int, row["end"]!!.jsonPrimitive.int))
            JsonObject(row + mapOf("start" to JsonPrimitive(range.first), "end" to JsonPrimitive(range.last + 1), "file" to JsonPrimitive("src/$name")))
        }
        val disposable = Disposer.newDisposable("kotlin-documentation-data-test")
        try {
            val environment = KotlinCoreEnvironment.createForProduction(disposable, CompilerConfiguration(), EnvironmentConfigFiles.JVM_CONFIG_FILES)
            val file = KtPsiFactory(environment.project, markGenerated = false).createFile(selectedFile, source)
            val documentation = KotlinDocumentationSource("src/$selectedFile", file, source, ":/main", descriptors.filter { it["file"]!!.jsonPrimitive.content == "src/$selectedFile" },
                rows.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL" && Path.of(it["file"]!!.jsonPrimitive.content).fileName.toString() == selectedFile },
                rows.filter { it["recordType"]?.jsonPrimitive?.content in setOf("DOCUMENTATION_VARIABLE", "DOCUMENTATION_VARIABLE_RECEIPT", "DOCUMENTATION_DATA_BOUNDARY", "DOCUMENTATION_MEMBER", "DOCUMENTATION_ACCESSOR_ATTEMPT", "DOCUMENTATION_STRING_CONSTANT", "DOCUMENTATION_OPERATION", "DOCUMENTATION_ENUM_ENTRY") && Path.of(it["file"]!!.jsonPrimitive.content).fileName.toString() == selectedFile }, descriptors)
            return PsiTreeUtil.collectElementsOfType(file, KtNamedFunction::class.java).filter { it.name != null && it.bodyExpression != null && it.receiverTypeReference == null }.associate { function ->
                val enriched = documentation.enrich(function, buildJsonObject { put("name", function.name) })
                function.name!! to assertNotNull(enriched["documentation"]?.jsonObject?.get("dataInput"), enriched.toString()).jsonObject
            }
        } finally { Disposer.dispose(disposable) }
    }

    @Test
    fun normalizedInputPreservesExactByteSyntaxAndCompilerNamedDefaultArgumentSlots() {
        val source = "\uFEFF" + """
            package docs
            // π🙂 before exact spans
            fun helper(input: String): String { val chosen = input; return chosen }
            fun format(input: String, flag: Boolean): String {
                var chosen = input
                if (flag) { chosen = "π🙂" }
                else { chosen = input }
                val transformed = helper(chosen)
                return transformed
            }
            fun pair(first: String, second: String = "default"): String { val copy = first; return copy + second }
            fun named(input: String): String = pair(second = input, first = "prefix")
            fun omitted(input: String): String = pair(first = input)
            fun constant(): String = "literal"
        """.trimIndent().replace("\n", "\r\n") + "\r\n"
        val data = extractData(source)
        assertEquals(setOf("helper", "format", "pair", "named", "omitted", "constant"), data.keys)
        val bytes = source.toByteArray(Charsets.UTF_8)
        for ((_, input) in data) {
            assertEquals("codeclew-kotlin-documentation-data/1.3", input["schema"]!!.jsonPrimitive.content)
            assertEquals("ORIGINAL_UTF8_BYTES", input["coordinateDomain"]!!.jsonPrimitive.content)
            val nodes = input["nodes"]!!.jsonArray
            assertTrue(nodes.isNotEmpty())
            assertTrue(input["body"]!!.jsonPrimitive.int in nodes.indices)
            for (node in nodes.map { it.jsonObject }) {
                val start = node["byteStart"]!!.jsonPrimitive.int; val end = node["byteEnd"]!!.jsonPrimitive.int
                assertTrue(start >= input["ownerByteStart"]!!.jsonPrimitive.int && end <= input["ownerByteEnd"]!!.jsonPrimitive.int && start < end)
                assertTrue(bytes.copyOfRange(start, end).decodeToString().isNotEmpty())
                assertTrue(node["children"]!!.jsonArray.all { it.jsonPrimitive.int in nodes.indices })
                if (node["kind"]!!.jsonPrimitive.content == "LOCAL") {
                    assertTrue(input["variables"]!!.jsonArray.map { it.jsonObject }.any {
                        it["kind"]!!.jsonPrimitive.content == "VARIABLE_DECLARATION" && it["byteStart"] == node["byteStart"] && it["byteEnd"] == node["byteEnd"]
                    }, "local syntax requires the exact FIR declaration span")
                }
            }
        }
        val formatKinds = data["format"]!!["nodes"]!!.jsonArray.map { it.jsonObject["kind"]!!.jsonPrimitive.content }
        assertTrue(setOf("LOCAL", "IF", "ASSIGNMENT", "CALL", "RETURN").all(formatKinds::contains), formatKinds.toString())
        val branch = data["format"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "IF" }
        assertTrue(branch["roles"]!!.jsonObject.keys.containsAll(setOf("CONDITION", "THEN", "ELSE")), branch.toString())
        assertEquals(2, formatKinds.count { it == "ASSIGNMENT" })
        for (token in data["format"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.filter { it["kind"]!!.jsonPrimitive.content == "UNSUPPORTED" }) {
            assertEquals("=", bytes.copyOfRange(token["byteStart"]!!.jsonPrimitive.int, token["byteEnd"]!!.jsonPrimitive.int).decodeToString(),
                "only assignment operator leaves are opaque; both branches must normalize")
        }
        val named = data["named"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "CALL" }
        assertEquals(listOf(1, 0), named["actuals"]!!.jsonArray.map { it.jsonObject["formalSlot"]!!.jsonPrimitive.int })
        val namedArguments = named["actuals"]!!.jsonArray.map { it.jsonObject["expression"]!!.jsonPrimitive.int }
            .map { data["named"]!!["nodes"]!!.jsonArray[it].jsonObject }
            .map { bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() }
        assertEquals(listOf("input", "\"prefix\""), namedArguments)
        val omitted = data["omitted"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "CALL" }
        assertEquals(listOf(1), omitted["defaultArguments"]!!.jsonArray.map { it.jsonPrimitive.int })
        assertTrue(data["constant"]!!["variables"]!!.jsonArray.isEmpty(), "a compiler receipt qualifies even an empty variable set")
    }

    @Test
    fun normalizedInputDoesNotTreatCustomOperatorsPropertiesOrExternalCallsAsPureTransfers() {
        val source = """
            package docs
            class Box(private val input: String) { val value: String get() = input; operator fun plus(other: Box): Box = this }
            fun operator(value: Box): String { val result = value + value; return "later" }
            fun property(value: Box): String { val result = value.value; return "later" }
            fun external(value: String): String { val result = value.trim { it == ' ' }; return "later" }
            fun trigger(): Boolean = true
            val enabled: Boolean get() = trigger()
            operator fun Boolean?.not(): Boolean = trigger()
            fun getter(): String { if (enabled) {}; return "later" }
            fun nullable(flag: Boolean?): String { if (!flag) {}; return "later" }
            fun variable(vararg value: String): String = "result"
            fun zero(): String = variable()
        """.trimIndent()
        // Class methods using `this` are outside this bounded pure-expression slice.
        val data = extractData(source)
        for (name in listOf("operator", "external", "nullable", "zero")) {
            assertTrue(data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.any { it["kind"]!!.jsonPrimitive.content == "UNSUPPORTED" }, name)
        }
        val bytes = source.toByteArray(Charsets.UTF_8)
        for ((name, text) in listOf("getter" to "enabled", "nullable" to "!flag", "zero" to "variable()")) {
            val matched = data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.filter {
                it["kind"]!!.jsonPrimitive.content != "RETURN" && bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() == text
            }
            assertEquals(1, matched.size, "$name must retain its exact unsupported expression")
            assertEquals(if (name == "getter") "ACCESSOR" else "UNSUPPORTED", matched.single()["kind"]!!.jsonPrimitive.content)
        }
    }

    @Test
    fun primaryConstructorStorageBindsResolvedParameterObjectsToOrdinaryProperties() {
        val source = """
            package docs
            class Queue
            class Endpoint(private val submitted: Queue)
            class Worker(private val pending: Queue, val other: String) { var lastState = "idle" }
            class PairQueues(q: Queue) { val submitted = q; val pending = q }
            class Custom(private val input: Queue) { val pending: Queue get() = input }
            class Changed(var pending: Queue) { init { pending = Queue() } }
            class SideEffects(var pending: Queue) { val changed = run { pending = Queue(); "changed" } }
        """.trimIndent()
        val facts = compileAll(source)
        val constructors = facts.filter { it["declarationKind"]?.jsonPrimitive?.content == "CONSTRUCTOR" }
        for ((name, properties) in listOf("Endpoint" to listOf("submitted"), "Worker" to listOf("pending", "other"))) {
            val constructor = constructors.single { it["ownerIdentity"]?.jsonPrimitive?.content == "class:docs/$name" }
            val proof = assertNotNull(constructor["documentationConstructorStorage"]?.jsonObject, name)
            assertEquals("K2_PRIMARY_PARAMETER_PROPERTY_INITIALIZER", proof["authority"]!!.jsonPrimitive.content)
            assertEquals(constructor["symbolIdentity"], proof["constructorIdentity"])
            val bindings = proof["bindings"]!!.jsonArray.map { it.jsonObject }
            assertEquals(properties.indices.toList(), bindings.map { it["parameterIndex"]!!.jsonPrimitive.int })
            assertEquals(properties.map { "property:docs/$name.$it" }, bindings.map { it["propertyIdentity"]!!.jsonPrimitive.content })
            assertEquals(properties.indices.map { constructor["symbolIdentity"]!!.jsonPrimitive.content + "/parameter/$it" }, bindings.map { it["parameterIdentity"]!!.jsonPrimitive.content })
        }
        val custom = constructors.single { it["ownerIdentity"]?.jsonPrimitive?.content == "class:docs/Custom" }
        assertEquals(listOf("property:docs/Custom.input"), custom["documentationConstructorStorage"]!!.jsonObject["bindings"]!!.jsonArray.map {
            it.jsonObject["propertyIdentity"]!!.jsonPrimitive.content
        })
        val pair = constructors.single { it["ownerIdentity"]?.jsonPrimitive?.content == "class:docs/PairQueues" }
        val sharedBindings = pair["documentationConstructorStorage"]!!.jsonObject["bindings"]!!.jsonArray.map { it.jsonObject }
        assertEquals(listOf(0, 0), sharedBindings.map { it["parameterIndex"]!!.jsonPrimitive.int })
        assertEquals(listOf("property:docs/PairQueues.submitted", "property:docs/PairQueues.pending"), sharedBindings.map { it["propertyIdentity"]!!.jsonPrimitive.content })
        assertEquals(1, sharedBindings.map { it["parameterIdentity"] }.toSet().size)
        for (name in listOf("Changed", "SideEffects")) {
            assertTrue(constructors.single { it["ownerIdentity"]?.jsonPrimitive?.content == "class:docs/$name" }["documentationConstructorStorage"] == null, name)
        }
    }

    @Test
    fun memberExtensionAndLabelledThisNeverCollapseDistinctReceiverStorage() {
        val labelledThis = "this@" + "Box"
        val source = """
            package docs
            class Box(var value: String) {
                fun Box.copyValue(): String { value = "extension"; $labelledThis.value = "dispatch"; return value }
                fun labelled(): String = $labelledThis.value
            }
        """.trimIndent()
        val facts = compileAll(source)
        assertTrue(facts.none { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_VARIABLE_RECEIPT"
            && ".copyValue#jvm:" in it["ownerSymbolIdentity"]!!.jsonPrimitive.content })
        val labelled = extractData(source)["labelled"]!!
        val bytes = source.toByteArray(Charsets.UTF_8)
        val receiver = labelled["nodes"]!!.jsonArray.map { it.jsonObject }.single {
            bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() == "this@Box"
        }
        assertEquals("UNSUPPORTED", receiver["kind"]!!.jsonPrimitive.content)
    }

    @Test
    fun ordinaryPropertyIdentitiesCloseAcrossFilesWithoutPromotingCustomOrVirtualAccessors() {
        val data = extractDataSources(mapOf(
            "Task.kt" to "package docs\nclass Task(val name: String, val eligible: Boolean)",
            "Config.kt" to "package docs\nclass Config(val prefix: String)\nclass Custom(private val input: String) { val value: String get() = input }\nopen class Virtual { open val value: String = \"initial\" }",
            "Pipeline.kt" to ("\uFEFF" + "package docs\r\nfun ordinary(task: Task, config: Config): String { val chosen = task.name; return config.prefix + chosen }\r\nfun custom(other: Custom): String = other.value\r\nfun virtual(other: Virtual): String = other.value\r\n")
        ), "Pipeline.kt")
        val members = data["ordinary"]!!["members"]!!.jsonArray.map { it.jsonObject }
        assertEquals(setOf("property:docs/Task.name", "property:docs/Config.prefix"), members.map { it["propertyIdentity"]!!.jsonPrimitive.content }.toSet())
        assertEquals(setOf("src/Task.kt", "src/Config.kt"), members.map { it["propertyFile"]!!.jsonPrimitive.content }.toSet())
        assertEquals(2, data["ordinary"]!!["nodes"]!!.jsonArray.count { it.jsonObject["kind"]!!.jsonPrimitive.content == "MEMBER" })
        for (name in listOf("custom", "virtual")) {
            assertTrue(data[name]!!["members"]!!.jsonArray.isEmpty(), name)
            val body = data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.filter { it["kind"]!!.jsonPrimitive.content != "RETURN" }
            assertEquals(2, body.size)
            assertEquals(1, body.count { it["kind"]!!.jsonPrimitive.content == "ACCESSOR" })
            val attempt = data[name]!!["accessorAttempts"]!!.jsonArray.single().jsonObject
            assertEquals("READ", attempt["accessMode"]!!.jsonPrimitive.content)
            assertEquals("K2_RESOLVED_PROPERTY_SYMBOL_NOT_ACCESSOR_DISPATCH", attempt["authority"]!!.jsonPrimitive.content)
            assertFalse(attempt["storageProof"]!!.jsonObject["qualified"]!!.jsonPrimitive.boolean)
        }
    }

    @Test
    fun primaryPipelineRetainsOrdinaryStorageNormalExternalResultsAndCompilerIntEquality() {
        val fixture = generateSequence(Path.of("").toAbsolutePath()) { it.parent }
            .map { it.resolve("fixtures/documentation-common-pipeline/kotlin/src/main/kotlin/paired/Pipeline.kt") }
            .first { Files.isRegularFile(it) }
        val data = extractData(Files.readString(fixture))
        val assembly = data["assemble"]!!["nodes"]!!.jsonArray.map { it.jsonObject }
        assertEquals(3, assembly.count { it["kind"]!!.jsonPrimitive.content == "CONSTRUCT" }, assembly.toString())
        assertTrue(assembly.none { it["kind"]!!.jsonPrimitive.content == "UNSUPPORTED" }, assembly.toString())
        val allocation = assembly.single { it["kind"]!!.jsonPrimitive.content == "CONSTRUCT" && it["actuals"]!!.jsonArray.isEmpty() }
        assertEquals("constructor:java/util/concurrent/LinkedBlockingQueue.LinkedBlockingQueue#jvm:()V", allocation["targetIdentity"]!!.jsonPrimitive.content)
        assertEquals(1, data["submit"]!!["nodes"]!!.jsonArray.count { it.jsonObject["kind"]!!.jsonPrimitive.content == "CONSTRUCT" })
        val helper = data["chooseName"]!!
        assertTrue(helper["members"]!!.jsonArray.isNotEmpty(), helper.toString())
        val run = data["runOnce"]!!
        val members = run["members"]!!.jsonArray.map { it.jsonObject }
        assertTrue(setOf("property:paired/ProcessingLoop.pending", "property:paired/ProcessingLoop.config",
            "property:paired/Config.enabled", "property:paired/Config.prefix", "property:paired/Task.eligible",
            "property:paired/ProcessingLoop.gateway", "property:paired/ProcessingLoop.lastState").all { id -> members.any { it["propertyIdentity"]!!.jsonPrimitive.content == id } }, members.toString())
        assertEquals(2, members.count { it["accessMode"]!!.jsonPrimitive.content == "WRITE" })
        val nodes = run["nodes"]!!.jsonArray.map { it.jsonObject }
        assertEquals(4, nodes.count { it["kind"]!!.jsonPrimitive.content == "CALL" }, run.toString())
        assertEquals(2, nodes.count { it["kind"]!!.jsonPrimitive.content == "ASSIGNMENT" })
        assertTrue(nodes.any { it["kind"]!!.jsonPrimitive.content == "BINARY" }, nodes.toString())
        val bytes = Files.readString(fixture).toByteArray(Charsets.UTF_8)
        assertTrue(nodes.filter { it["kind"]!!.jsonPrimitive.content == "UNSUPPORTED" }.all {
            bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString() in setOf("=", "==", "!=", "+", "!")
        }, nodes.toString())
    }

    @Test
    fun enumIdentitiesAndGuardedQualifiedWritesComeFromK2Symbols() {
        val source = "\uFEFF" + """
            package docs
            import docs.TaskStatus.COMPLETED as Finished
            // π🙂 before the guarded writes
            fun finalize(task: Task, failedChunks: Int, failedRows: Int, hasResult: Boolean) {
                if (failedChunks > 0) { task.status = TaskStatus.FAILED }
                else if (failedRows > 0) {
                    if (hasResult) { task.status = TaskStatus.COMPLETED_WITH_ERRORS }
                    else { task.status = TaskStatus.IN_PROGRESS }
                } else { task.status = Finished }
            }
            fun shadow(): String = Other.FAILED
            fun custom(value: Counter): Boolean = value > 0
        """.trimIndent().replace("\n", "\r\n")
        val data = extractDataSources(mapOf(
            "Status.kt" to "package docs\nenum class TaskStatus { FAILED, COMPLETED_WITH_ERRORS, IN_PROGRESS, COMPLETED }\nclass Task(var status: TaskStatus)\nobject Other { val FAILED = \"failed\" }\nclass Counter { operator fun compareTo(other: Int): Int = 0 }",
            "Process.kt" to source,
        ), "Process.kt")
        val process = data["finalize"]!!
        val entries = process["enumEntries"]!!.jsonArray.map { it.jsonObject }
        assertEquals(setOf("FAILED", "COMPLETED_WITH_ERRORS", "IN_PROGRESS", "COMPLETED"), entries.map { it["entryName"]!!.jsonPrimitive.content }.toSet())
        val bytes = source.toByteArray(Charsets.UTF_8)
        for (entry in entries) {
            assertEquals("class:docs/TaskStatus", entry["enumClassIdentity"]!!.jsonPrimitive.content)
            assertEquals("enum-entry:docs/TaskStatus." + entry["entryName"]!!.jsonPrimitive.content, entry["entryIdentity"]!!.jsonPrimitive.content)
            assertEquals("K2_RESOLVED_ENUM_ENTRY_SYMBOL", entry["authority"]!!.jsonPrimitive.content)
            val syntax = bytes.copyOfRange(entry["byteStart"]!!.jsonPrimitive.int, entry["byteEnd"]!!.jsonPrimitive.int).decodeToString()
            assertEquals(if (entry["entryName"]!!.jsonPrimitive.content == "COMPLETED") "Finished" else "TaskStatus." + entry["entryName"]!!.jsonPrimitive.content, syntax)
        }
        val nodes = process["nodes"]!!.jsonArray.map { it.jsonObject }
        assertEquals(4, nodes.count { it["kind"]!!.jsonPrimitive.content == "ENUM_CONSTANT" })
        assertEquals(4, nodes.count { it["kind"]!!.jsonPrimitive.content == "ASSIGNMENT" })
        assertEquals(2, nodes.count { it["kind"]!!.jsonPrimitive.content == "BINARY" })
        val writes = process["members"]!!.jsonArray.map { it.jsonObject }.filter { it["accessMode"]!!.jsonPrimitive.content == "WRITE" }
        assertEquals(4, writes.size)
        assertTrue(writes.all { it["propertyIdentity"]!!.jsonPrimitive.content == "property:docs/Task.status" })
        assertTrue(data["shadow"]!!["enumEntries"]!!.jsonArray.isEmpty())
        assertTrue(data["custom"]!!["nodes"]!!.jsonArray.none { it.jsonObject["kind"]!!.jsonPrimitive.content == "BINARY" })
    }

    @Test
    fun virtualAndThrowingOverriddenAccessorsKeepStaticIdentityAndExactAttemptOrder() {
        val source = "\uFEFF" + """
            package docs
            // π🙂 preserves the original coordinate domain
            enum class Status { FAILED, COMPLETED }
            const val FINALIZED = "FINALIZED"
            fun constant(): String = FINALIZED
            fun opaque(task: Task): Boolean = task.status == Status.FAILED
            open class Task { open var status: Status = Status.COMPLETED; open val failed: Int = 0 }
            class Throwing : Task() {
                override var status: Status
                    get() = error("getter")
                    set(value) { error("setter") }
                override val failed: Int get() = error("counter")
            }
            fun finalize(task: Task) {
                if (task.failed > 0) { task.status = Status.FAILED }
                else { task.status = Status.COMPLETED }
                val after = task.status
                val message = "failed ${'$'}{task.failed}"
            }
        """.trimIndent().replace("\n", "\r\n")
        val extracted = extractData(source)
        assertTrue(extracted["constant"]!!["accessorAttempts"]!!.jsonArray.isEmpty())
        assertEquals("FINALIZED", extracted["constant"]!!["stringConstants"]!!.jsonArray.single().jsonObject["stringConstant"]!!.jsonPrimitive.content)
        assertEquals(1, extracted["constant"]!!["nodes"]!!.jsonArray.count { it.jsonObject["kind"]!!.jsonPrimitive.content == "STRING_CONSTANT" })
        assertTrue(extracted["opaque"]!!["accessorAttempts"]!!.jsonArray.isEmpty())
        assertTrue(extracted["opaque"]!!["enumEntries"]!!.jsonArray.isEmpty())
        val data = extracted["finalize"]!!
        assertTrue(data["members"]!!.jsonArray.isEmpty())
        val attempts = data["accessorAttempts"]!!.jsonArray.map { it.jsonObject }
        assertEquals(listOf("READ", "WRITE", "WRITE", "READ"), attempts.map { it["accessMode"]!!.jsonPrimitive.content })
        assertEquals(listOf("property:docs/Task.failed", "property:docs/Task.status", "property:docs/Task.status", "property:docs/Task.status"), attempts.map { it["propertyIdentity"]!!.jsonPrimitive.content })
        val bytes = source.toByteArray(Charsets.UTF_8)
        assertEquals(listOf("task.failed", "task.status", "task.status", "task.status"), attempts.map {
            bytes.copyOfRange(it["byteStart"]!!.jsonPrimitive.int, it["byteEnd"]!!.jsonPrimitive.int).decodeToString()
        })
        assertTrue(attempts.all { !it["storageProof"]!!.jsonObject["qualified"]!!.jsonPrimitive.boolean })
        assertEquals(4, data["nodes"]!!.jsonArray.count { it.jsonObject["kind"]!!.jsonPrimitive.content == "ACCESSOR" })
        assertEquals(1, data["nodes"]!!.jsonArray.count { it.jsonObject["kind"]!!.jsonPrimitive.content == "BINARY" })
    }

    @Test
    fun staticClassQualifierRequiresResolvedJavaStaticFunctionAndNeverObjectOrShadowedReceiver() {
        val source = "\uFEFF" + """
            package docs
            import java.time.LocalDateTime
            // π🙂 precedes the exact class qualifier
            object Clock { fun now(): Int = 1; @JvmStatic fun staticNow(): Int = 3 }
            class InstanceClock { fun now(): Int = 2 }
            fun javaStatic(): LocalDateTime = LocalDateTime.now()
            fun objectCall(): Int = Clock.now()
            fun kotlinStatic(): Int = Clock.staticNow()
            fun shadow(LocalDateTime: InstanceClock): Int = LocalDateTime.now()
        """.trimIndent().replace("\n", "\r\n")
        val data = extractData(source)
        val call = data["javaStatic"]!!["nodes"]!!.jsonArray.map { it.jsonObject }.single { it["kind"]!!.jsonPrimitive.content == "CALL" }
        val qualifier = assertNotNull(call["staticQualifier"]).jsonObject
        assertEquals("class:java/time/LocalDateTime", qualifier["classIdentity"]!!.jsonPrimitive.content)
        assertEquals("K2_STATIC_FUNCTION_RESOLVED_CLASS_QUALIFIER", qualifier["authority"]!!.jsonPrimitive.content)
        assertTrue(call["roles"]!!.jsonObject.isEmpty())
        val bytes = source.toByteArray(Charsets.UTF_8)
        assertEquals("LocalDateTime", bytes.copyOfRange(qualifier["byteStart"]!!.jsonPrimitive.int, qualifier["byteEnd"]!!.jsonPrimitive.int).decodeToString())
        for (name in listOf("objectCall", "shadow", "kotlinStatic")) {
            val calls = data[name]!!["nodes"]!!.jsonArray.map { it.jsonObject }.filter { it["kind"]!!.jsonPrimitive.content == "CALL" }
            assertEquals(1, calls.size)
            assertNull(calls.single()["staticQualifier"])
            assertNotNull(calls.single()["roles"]!!.jsonObject["RECEIVER"])
        }
    }

    @Test
    fun extensionReceiverPositionAndNullablePrimitiveSlotsMatchEmittedBytecode() {
        compileSources(mapOf("Extensions.kt" to """
            package docs
            class Extensions {
                fun String.join(input: Int?): String = this
                fun Int?.echo(input: Int?): Int? = input
                fun stringCaller(input: Int?): String = "prefix".join(input)
                fun intCaller(input: Int?): Int? = input.echo(input)
            }
        """.trimIndent()), verifyBytecode = { classes, rows ->
            val methods = mutableSetOf<String>()
            ClassReader(Files.readAllBytes(classes.resolve("docs/Extensions.class"))).accept(object : ClassVisitor(Opcodes.ASM9) {
                override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor? {
                    methods += "$name#jvm:$descriptor"
                    return null
                }
            }, ClassReader.SKIP_CODE or ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
            val declarations = rows.filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR"
                && it["symbolIdentity"]?.jsonPrimitive?.content?.let { id -> ".join#" in id || ".echo#" in id } == true }
            assertEquals(2, declarations.size)
            for (row in declarations) assertTrue(row["symbolIdentity"]!!.jsonPrimitive.content.substringAfter("Extensions.") in methods, row.toString())
            val calls = rows.filter { it["recordType"]?.jsonPrimitive?.content == "DOCUMENTATION_CALL"
                && it["target"]?.jsonPrimitive?.content?.let { id -> ".join#" in id || ".echo#" in id } == true }
            assertEquals(2, calls.size)
            for (row in calls) assertTrue(row["target"]!!.jsonPrimitive.content.substringAfter("Extensions.") in methods, row.toString())
            assertTrue("join#jvm:(Ljava/lang/String;Ljava/lang/Integer;)Ljava/lang/String;" in methods)
            assertTrue("echo#jvm:(Ljava/lang/Integer;Ljava/lang/Integer;)Ljava/lang/Integer;" in methods)
        })
    }

    @Test
    fun constructorActualsKeepSourceOrderAndResolvedFormalSlots() {
        val source = """
            package docs
            class Queue
            class Owner(val first: Queue, val second: Queue)
            fun assemble() {
                val one = Queue()
                val two = Queue()
                val owner = Owner(second = two, first = one)
            }
        """.trimIndent()
        val data = extractData(source)["assemble"]!!
        val nodes = data["nodes"]!!.jsonArray.map { it.jsonObject }
        val constructors = nodes.filter { it["kind"]!!.jsonPrimitive.content == "CONSTRUCT" }
        assertEquals(3, constructors.size, data.toString())
        val owner = constructors.single { it["targetIdentity"]!!.jsonPrimitive.content.startsWith("constructor:docs/Owner.Owner#jvm:") }
        assertEquals(listOf(1, 0), owner["actuals"]!!.jsonArray.map { it.jsonObject["formalSlot"]!!.jsonPrimitive.int })
        assertEquals(listOf("two", "one"), owner["actuals"]!!.jsonArray.map {
            val n = nodes[it.jsonObject["expression"]!!.jsonPrimitive.int]
            source.toByteArray().copyOfRange(n["byteStart"]!!.jsonPrimitive.int, n["byteEnd"]!!.jsonPrimitive.int).decodeToString()
        })
        assertTrue(constructors.all { it["defaultArguments"]!!.jsonArray.isEmpty() })
    }

    @Test
    fun parametersAndShadowedLocalsBindExactCompilerSymbolsAcrossOriginalByteCoordinates() {
        val source = "\uFEFF" + """
            package docs
            // π🙂 before source occurrences
            fun select(input: String, flag: Boolean = false): String {
                var chosen = input
                if (flag) { val input = "inner π🙂"; chosen = input }
                else { val input = "other"; chosen = input }
                return chosen
            }
        """.trimIndent().replace("\n", "\r\n") + "\r\n"
        val rows = compile(source)
        val variables = rows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_VARIABLE" }
        assertTrue(variables.isNotEmpty())
        val declarations = variables.filter { it["kind"]!!.jsonPrimitive.content == "VARIABLE_DECLARATION" }
        val sameNames = declarations.filter { it["name"]!!.jsonPrimitive.content == "input" }
        assertEquals(3, sameNames.size, declarations.toString())
        assertEquals(3, sameNames.map { it["variableIdentity"]!!.jsonPrimitive.content }.toSet().size)
        val owner = "callable:docs/select#jvm:(Ljava/lang/String;Z)Ljava/lang/String;"
        val coordinates = assertNotNull(CompilerUtf16ToUtf8ByteMap.fromCompilerInput(source))
        val bytes = source.toByteArray(Charsets.UTF_8)
        fun text(row: JsonObject, start: String = "start", end: String = "end"): String {
            val range = assertNotNull(coordinates.range(row[start]!!.jsonPrimitive.int, row[end]!!.jsonPrimitive.int))
            return bytes.copyOfRange(range.first, range.last + 1).decodeToString()
        }
        val byIdentity = declarations.associateBy { it["variableIdentity"]!!.jsonPrimitive.content }
        for (row in variables) {
            assertEquals(owner, row["ownerSymbolIdentity"]!!.jsonPrimitive.content)
            assertEquals("COMPILER_UTF16_OFFSETS", row["coordinateDomain"]!!.jsonPrimitive.content)
            assertEquals("COMPILER_EXACT", row["resolution"]!!.jsonPrimitive.content)
            assertEquals("K2_RESOLVED_VARIABLE_SYMBOL", row["authority"]!!.jsonPrimitive.content)
            val declaration = assertNotNull(byIdentity[row["variableIdentity"]!!.jsonPrimitive.content])
            assertEquals(declaration["start"], row["declarationStart"])
            assertEquals(declaration["end"], row["declarationEnd"])
            assertEquals(text(declaration), text(row, "declarationStart", "declarationEnd"))
            if (row["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS") {
                assertEquals(row["name"]!!.jsonPrimitive.content, text(row))
            }
        }
        val parameter = sameNames.single { it["variableKind"]!!.jsonPrimitive.content == "PARAMETER" }
        assertEquals(0, parameter["parameterIndex"]!!.jsonPrimitive.int)
        assertFalse(parameter["hasDefault"]!!.jsonPrimitive.boolean)
        val flag = declarations.single { it["name"]!!.jsonPrimitive.content == "flag" }
        assertEquals(1, flag["parameterIndex"]!!.jsonPrimitive.int)
        assertTrue(flag["hasDefault"]!!.jsonPrimitive.boolean)
        val accesses = variables.filter { it["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS" }
        val inputReads = accesses.filter { it["name"]!!.jsonPrimitive.content == "input" }
        assertEquals(sameNames.map { it["variableIdentity"] }.toSet(), inputReads.map { it["variableIdentity"] }.toSet())
        val chosen = declarations.single { it["name"]!!.jsonPrimitive.content == "chosen" }
        val chosenAccesses = accesses.filter { it["variableIdentity"] == chosen["variableIdentity"] }
        assertEquals(2, chosenAccesses.count { it["accessMode"]!!.jsonPrimitive.content == "WRITE" })
        assertEquals(1, chosenAccesses.count { it["accessMode"]!!.jsonPrimitive.content == "READ" })
        assertTrue(rows.none { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_DATA_BOUNDARY" }, rows.toString())
        // Compare complete retained compiler records after removing only the temp path.
        assertEquals(rows.map { JsonObject(it - "file") }, compile(source).map { JsonObject(it - "file") })
    }

    @Test
    fun fieldsDelegatedPropertiesAndDeferredBodiesKeepExplicitBoundaries() {
        val source = """
            package docs
            class Box(val value: String)
            fun opaque(box: Box, input: String): String {
                val property = box.value
                val delegated by lazy { input }
                val deferred = { input }
                fun nested(): String = input
                return input
            }
        """.trimIndent()
        val rows = compile(source)
        val owner = "callable:docs/opaque#jvm:(Ldocs/Box;Ljava/lang/String;)Ljava/lang/String;"
        val ownerRows = rows.filter { it["ownerSymbolIdentity"]?.jsonPrimitive?.content == owner }
        val variables = ownerRows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_VARIABLE" }
        assertTrue(variables.isNotEmpty())
        assertTrue(variables.none { it["variableKind"]!!.jsonPrimitive.content == "FIELD" })
        assertTrue(variables.none { it["name"]!!.jsonPrimitive.content in setOf("value", "delegated") })
        val inputReads = variables.filter { it["name"]!!.jsonPrimitive.content == "input" && it["kind"]!!.jsonPrimitive.content == "VARIABLE_ACCESS" }
        assertEquals(1, inputReads.size, "deferred/local-function captures do not become immediate reads: $variables")
        val codes = ownerRows.filter { it["recordType"]!!.jsonPrimitive.content == "DOCUMENTATION_DATA_BOUNDARY" }
            .map { it["code"]!!.jsonPrimitive.content }.toSet()
        assertTrue("DOCUMENTATION_PROPERTY_STORAGE_OPAQUE" in codes, codes.toString())
        assertTrue("DOCUMENTATION_NESTED_BODY_DEFERRED" in codes, codes.toString())
    }
}
