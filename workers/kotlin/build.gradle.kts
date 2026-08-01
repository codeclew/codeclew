plugins {
    kotlin("jvm") version "2.4.10"
    application
}

group = "dev.semanticthread"
version = "0.1.0"

kotlin { jvmToolchain(21) }

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    compilerOptions.freeCompilerArgs.addAll(
        "-opt-in=org.jetbrains.kotlin.K1Deprecation",
        "-opt-in=org.jetbrains.kotlin.config.CompilerConfiguration.Internals",
    )
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.4.10")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.10.0")
    testImplementation(kotlin("test"))
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test { useJUnitPlatform() }
