plugins {
    kotlin("jvm") version "2.4.10"
    application
}

group = "dev.semanticthread"
version = "0.1.0"

kotlin {
    jvmToolchain(21)
    sourceSets {
        main { kotlin.srcDir("../kotlin21/src/main/kotlin") }
        test { kotlin.srcDir("../kotlin21/src/test/kotlin") }
    }
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    compilerOptions.freeCompilerArgs.addAll(
        "-opt-in=org.jetbrains.kotlin.K1Deprecation",
        "-opt-in=org.jetbrains.kotlin.config.CompilerConfiguration.Internals",
    )
    exclude { element ->
        element.file.absolutePath == file("../kotlin21/src/main/kotlin/dev/semanticthread/worker/FirFactsPlugin21.kt").absolutePath
    }
}

val pluginQualification = configurations.create("pluginQualification")

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.4.10")
    implementation("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin-embeddable:2.4.10")
    implementation("org.jetbrains.kotlin:kotlin-allopen-compiler-plugin-embeddable:2.4.10")
    implementation("org.jetbrains.kotlin:kotlin-noarg-compiler-plugin-embeddable:2.4.10")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.10.0")
    implementation("org.jetbrains.kotlin:kotlin-build-tools-api:2.4.10")
    runtimeOnly("org.jetbrains.kotlin:kotlin-build-tools-impl:2.4.10") {
        isTransitive = false
    }
    runtimeOnly("org.jetbrains.kotlin:kotlin-compiler-runner:2.4.10")
    add(pluginQualification.name, "org.jetbrains.kotlin:kotlin-maven-allopen:2.3.0") { isTransitive = false }
    add(pluginQualification.name, "org.jetbrains.kotlin:kotlin-maven-noarg:2.3.0") { isTransitive = false }
    testImplementation(kotlin("test"))
    // Test artifacts only: extraction recognizes stable annotation FQNs, not library versions.
    testImplementation("org.springframework:spring-web:6.1.2")
    testImplementation("jakarta.persistence:jakarta.persistence-api:3.1.0")
    testImplementation("org.springframework.kafka:spring-kafka:3.0.16")
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test {
    inputs.files(pluginQualification)
    doFirst { systemProperty("codeclew.test.projectCompilerPlugins", pluginQualification.asPath) }
    useJUnitPlatform()
    dependsOn(tasks.jar)
    classpath = files(tasks.jar.flatMap { it.archiveFile }) +
        (classpath - sourceSets.main.get().output)
}
