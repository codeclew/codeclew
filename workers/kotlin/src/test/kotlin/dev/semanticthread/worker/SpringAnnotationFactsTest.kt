package dev.semanticthread.worker

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import java.nio.file.Path
import kotlinx.serialization.json.*
import org.jetbrains.kotlin.cli.jvm.K2JVMCompiler
import kotlin.test.*

class SpringAnnotationFactsTest {
    @Test
    fun realSpringAnnotationsResolveToExactComputationRoots() {
        val source = """
            package roots
            import org.springframework.web.bind.annotation.*
            import org.springframework.web.bind.annotation.GetMapping as Fetch
            import org.springframework.kafka.annotation.*
            import org.springframework.scheduling.annotation.*
            import org.springframework.core.annotation.AliasFor
            import java.util.concurrent.TimeUnit

            const val PREFIX = "/api"
            const val TOPIC = "orders"
            @Target(AnnotationTarget.FUNCTION, AnnotationTarget.ANNOTATION_CLASS)
            @Retention(AnnotationRetention.RUNTIME)
            @RequestMapping(method = [RequestMethod.GET], produces = ["application/json"])
            annotation class JsonGet(
                @get:AliasFor(annotation = RequestMapping::class, attribute = "path")
                val route: Array<String> = ["/default"]
            )
            @Target(AnnotationTarget.FUNCTION)
            @JsonGet
            annotation class Routed(
                @get:AliasFor(annotation = JsonGet::class, attribute = "route")
                val url: Array<String> = ["/nested"]
            )
            @Target(AnnotationTarget.FUNCTION)
            @RequestMapping(path = ["/original"])
            annotation class ValueRoute(
                @get:AliasFor(annotation = RequestMapping::class, attribute = "value")
                val route: Array<String> = []
            )

            interface Contract {
                @Fetch("/inherited") fun inherited(): String
            }
            @RestController
            @RequestMapping(path = [PREFIX, "/v2"], headers = ["X-Tenant"])
            class Api : Contract {
                @Fetch(path = ["/items", "/products"], params = ["active=true"])
                fun items(): String = "ok"
                @PostMapping("/items", consumes = ["application/json"])
                fun items(value: String): String = value
                @JsonGet(route = ["/composed"]) fun composed(): String = "ok"
                @JsonGet fun defaultRoute(): String = "ok"
                @Routed(url = ["/deep"]) fun nestedRoute(): String = "ok"
                @ValueRoute(route = ["/override"]) fun aliasedValue(): String = "ok"
                @RequestMapping("/any") fun any(): String = "ok"
                override fun inherited(): String = "ok"
            }

            class Worker {
                @KafkaListener(topics = [TOPIC], groupId = "readers")
                @KafkaListener(topicPattern = "audit.*", id = "audit")
                fun consume(value: String) { println(value) }
                @KafkaListener(topicPartitions = [TopicPartition(topic = "fixed", partitions = ["0", "1"])])
                fun partitioned(value: String) { println(value) }
                @Scheduled(fixedDelay = 5, timeUnit = TimeUnit.SECONDS)
                @Scheduled(cron = "0 0 * * * *", zone = "UTC")
                fun refresh() {}
                @Scheduled(fixedRateString = "\${'$'}{rate:1000}", initialDelayString = "100")
                fun configured() {}
                @Scheduled(cron = "-") fun disabled() {}
                @Schedules(value = [Scheduled(fixedRate = 100), Scheduled(fixedDelay = 200)])
                fun contained() {}
            }
            @KafkaListener(topics = ["multi"], id = "multi")
            class Multi {
                @KafkaHandler fun handle(value: String) { println(value) }
                @KafkaHandler(isDefault = true) fun fallback(value: Any) { println(value) }
            }
        """.trimIndent()
        compile(source).let { rows ->
            fun spring(name: String) = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == name }["spring"]!!.jsonObject
            fun entries(name: String) = spring(name)["entries"]!!.jsonArray.map { it.jsonObject }
            val items = rows.filter { it["compilerCallableId"]?.jsonPrimitive?.content == "roots/Api.items" }
            assertEquals(2, items.size)
            assertEquals(2, items.map { it["symbolIdentity"] }.distinct().size)
            val get = items.single { it["jvmDescriptor"]!!.jsonPrimitive.content.startsWith("()") }["spring"]!!.jsonObject["entries"]!!.jsonArray.single().jsonObject
            assertEquals("HTTP_ENDPOINT", get["kind"]!!.jsonPrimitive.content)
            assertEquals(listOf("GET"), strings(get["attributes"]!!.jsonObject["method"]))
            assertEquals(listOf("/items", "/products"), strings(get["attributes"]!!.jsonObject["path"]))
            assertEquals(listOf("/api", "/v2"), strings(get["classAttributes"]!!.jsonArray.single().jsonObject["path"]))
            assertEquals(true, get["controller"]!!.jsonPrimitive.boolean)
            assertEquals(listOf("/composed"), strings(entries("roots/Api.composed").single()["attributes"]!!.jsonObject["path"]))
            assertEquals(listOf("/default"), strings(entries("roots/Api.defaultRoute").single()["attributes"]!!.jsonObject["path"]))
            assertEquals(listOf("/deep"), strings(entries("roots/Api.nestedRoute").single()["attributes"]!!.jsonObject["path"]))
            assertEquals(listOf("/override"), strings(entries("roots/Api.aliasedValue").single()["attributes"]!!.jsonObject["path"]))
            assertEquals(listOf("/inherited"), strings(entries("roots/Api.inherited").single()["attributes"]!!.jsonObject["path"]))
            assertNull(entries("roots/Api.any").single()["attributes"]!!.jsonObject["method"])
            assertEquals(2, entries("roots/Worker.consume").size)
            assertTrue(entries("roots/Worker.consume").any { strings(it["attributes"]!!.jsonObject["topics"]) == listOf("orders") })
            assertEquals(2, entries("roots/Worker.refresh").size)
            assertEquals(2, entries("roots/Worker.contained").size)
            val partitions = entries("roots/Worker.partitioned").single()["attributes"]!!.jsonObject["topicPartitions"]!!.jsonArray.single().jsonObject["attributes"]!!.jsonObject
            assertEquals("fixed", partitions["topic"]!!.jsonPrimitive.content)
            assertEquals(listOf("0", "1"), strings(partitions["partitions"]))
            assertTrue("RUNTIME_EXPRESSION" in strings(spring("roots/Worker.configured")["boundaries"]))
            assertEquals("-", entries("roots/Worker.disabled").single()["attributes"]!!.jsonObject["cron"]!!.jsonPrimitive.content)
            assertEquals("KAFKA_LISTENER", entries("roots/Multi.handle").single()["kind"]!!.jsonPrimitive.content)
            assertEquals(true, entries("roots/Multi.fallback").single()["handlerAttributes"]!!.jsonObject["isDefault"]!!.jsonPrimitive.boolean)
            assertTrue(items.all { strings(it["spring"]!!.jsonObject["boundaries"]).isEmpty() })
        }
    }

    @Test
    fun annotationNamesAloneAreNotFrameworkEvidence() {
        val rows = compile("""
            package impostors
            annotation class GetMapping(val value: String)
            annotation class KafkaListener(val topics: Array<String>)
            annotation class Scheduled(val fixedDelay: Long)
            class Worker {
                @GetMapping("/fake") @KafkaListener(["fake"]) @Scheduled(10)
                fun fake() {}
            }
        """.trimIndent())
        val row = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == "impostors/Worker.fake" }
        assertTrue(row["spring"]!!.jsonObject["entries"]!!.jsonArray.isEmpty())
    }

    @Test
    fun inheritedImplementationsKeepConcreteBeanContextAndDeclaringIdentity() {
        val rows = compile("""
            package inherited
            import org.springframework.web.bind.annotation.*
            import org.springframework.kafka.annotation.*
            open class Base {
                @GetMapping("/item") fun item(): String = "ok"
                @KafkaHandler fun consume(value: String) { println(value) }
            }
            @RestController @RequestMapping("/child")
            @KafkaListener(topics = ["child-events"])
            class Child: Base()
        """.trimIndent())
        val child = rows.single { it["compilerClassId"]?.jsonPrimitive?.content == "inherited/Child" && it["declarationKind"]?.jsonPrimitive?.content == "CLASS" }
        val entries = child["spring"]!!.jsonObject["entries"]!!.jsonArray.map { it.jsonObject }
        assertEquals(2, entries.size, child.toString())
        val http = entries.single { it["kind"]!!.jsonPrimitive.content == "HTTP_ENDPOINT" }
        assertEquals("class:inherited/Child", http["beanClass"]!!.jsonPrimitive.content)
        assertEquals("callable:inherited/Base.item#jvm:()Ljava/lang/String;", http["targetSymbol"]!!.jsonPrimitive.content)
        assertEquals(listOf("/child"), strings(http["classAttributes"]!!.jsonArray.single().jsonObject["path"]))
        assertEquals(listOf("child-events"), strings(entries.single { it["kind"]!!.jsonPrimitive.content == "KAFKA_LISTENER" }["attributes"]!!.jsonObject["topics"]))
    }

    @Test
    fun resolvedFeignClientsAreOutboundButControllerImplementationsRemainEntrypoints() {
        val rows = compile("""
            package clientroles
            import org.springframework.web.bind.annotation.*
            import org.springframework.cloud.openfeign.FeignClient as RemoteClient
            @RemoteClient(name = "remote") interface Client {
                @GetMapping("/remote") fun read(): String
            }
            @RestController class Server : Client {
                override fun read(): String = "local"
            }
            @RemoteClient(name = "base") interface DefaultClient {
                @GetMapping("/default") fun defaultRead(): String = "local"
            }
            @RestController class InheritedServer : DefaultClient
            annotation class FeignClient
            @FeignClient class Unregistered {
                @GetMapping("/unknown") fun read(): String = "unknown"
            }
        """.trimIndent())
        fun metadata(id: String) = rows.single { it["compilerCallableId"]?.jsonPrimitive?.content == id }["spring"]!!.jsonObject
        val client = metadata("clientroles/Client.read")
        assertTrue(client["entries"]!!.jsonArray.isEmpty())
        assertTrue("OUTBOUND_FEIGN_CLIENT_NOT_SERVER_ENTRYPOINT" in strings(client["boundaries"]))
        assertEquals("HTTP_ENDPOINT", metadata("clientroles/Server.read")["entries"]!!.jsonArray.single().jsonObject["kind"]!!.jsonPrimitive.content)
        val inherited = rows.single { it["declarationKind"]?.jsonPrimitive?.content == "CLASS" && it["compilerClassId"]?.jsonPrimitive?.content == "clientroles/InheritedServer" }
        assertEquals("HTTP_ENDPOINT", inherited["spring"]!!.jsonObject["entries"]!!.jsonArray.single().jsonObject["kind"]!!.jsonPrimitive.content)
        val unknown = metadata("clientroles/Unregistered.read")
        assertEquals(1, unknown["entries"]!!.jsonArray.size)
        assertTrue("CONTROLLER_REGISTRATION_UNPROVEN" in strings(unknown["boundaries"]))
    }

    private fun strings(value: JsonElement?): List<String> = when (value) {
        is JsonArray -> value.map { it.jsonPrimitive.content }
        null -> emptyList()
        else -> listOf(value.jsonPrimitive.content)
    }

    private fun compile(source: String): List<JsonObject> {
        val root = Files.createTempDirectory("spring-k2-facts").toRealPath()
        try {
            val input = Files.writeString(root.resolve("Roots.kt"), source)
            val facts = root.resolve("facts.jsonl")
            val plugin = Path.of(FirFactsCompilerPluginRegistrar::class.java.protectionDomain.codeSource.location.toURI())
            val output = ByteArrayOutputStream()
            val status = PrintStream(output).use { stream ->
                synchronized(K2JVMCompiler::class.java) {
                    K2JVMCompiler().exec(stream, "-no-stdlib", "-no-reflect", "-jvm-target", "21",
                        "-classpath", System.getProperty("java.class.path"), "-d", root.resolve("classes").toString(),
                        "-Xplugin=$plugin", "-P", "plugin:semantic-thread-facts:output=$facts", input.toString())
                }
            }
            assertEquals(0, status.code, output.toString())
            return Files.readAllLines(facts).map { Json.parseToJsonElement(it).jsonObject }
                .filter { it["recordType"]?.jsonPrimitive?.content == "DECLARATION_DESCRIPTOR" }
        } finally { root.toFile().deleteRecursively() }
    }
}
