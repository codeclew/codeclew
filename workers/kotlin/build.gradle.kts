plugins {
    kotlin("jvm") version "2.4.10"
    application
}

group = "dev.semanticthread"
version = "0.1.0"

kotlin { jvmToolchain(21) }

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.4.10")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.10.0")
    testImplementation(kotlin("test"))
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test { useJUnitPlatform() }

