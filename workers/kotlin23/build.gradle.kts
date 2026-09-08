plugins {
    kotlin("jvm") version "2.3.0"
    application
}

group = "dev.semanticthread"
version = "0.1.0"

kotlin {
    jvmToolchain(21)
    sourceSets {
        test { kotlin.srcDir("../kotlin/src/test/kotlin") }
        main {
            kotlin.srcDir("../kotlin/src/main/kotlin")
            resources.srcDir("../kotlin/src/main/resources")
        }
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.3.0")
    implementation("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin-embeddable:2.3.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.10.0")
    testImplementation(kotlin("test"))
    testImplementation("org.springframework:spring-web:6.1.2")
    testImplementation("org.springframework.cloud:spring-cloud-openfeign-core:4.1.0") { isTransitive = false }
    testImplementation("org.springframework.kafka:spring-kafka:3.0.16")
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test {
    useJUnitPlatform()
    dependsOn(tasks.jar)
    classpath = files(tasks.jar.flatMap { it.archiveFile }) +
        (classpath - sourceSets.main.get().output)
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    compilerOptions.freeCompilerArgs.add("-Xcontext-parameters")
    exclude { element ->
        (!element.isDirectory && element.file.absolutePath.startsWith(file("../kotlin/src/test/kotlin").absolutePath) &&
            element.file.name !in setOf("SpringAnnotationFactsTest.kt", "KotlinDocumentationFlowTest.kt")) ||
        element.file.name == "SpringAnnotationFacts24.kt" ||
        element.file.name == "JvmAnnotationFacts24.kt" ||
        element.file.absolutePath == file("../kotlin/src/main/kotlin/dev/semanticthread/worker/FirFactsPlugin.kt").absolutePath
    }
}
