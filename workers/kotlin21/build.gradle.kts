plugins {
    kotlin("jvm") version "2.1.21"
    application
}

group = "dev.semanticthread"
version = "0.1.0"

kotlin {
    jvmToolchain(21)
    sourceSets {
        main {
            kotlin.srcDir("../kotlin/src/main/kotlin")
            resources.srcDir("../kotlin/src/main/resources")
        }
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.1.21")
    implementation("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin-embeddable:2.1.21")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("org.jetbrains.kotlin:kotlin-build-tools-api:2.1.21")
    runtimeOnly("org.jetbrains.kotlin:kotlin-build-tools-impl:2.1.21") {
        isTransitive = false
    }
    runtimeOnly("org.jetbrains.kotlin:kotlin-compiler-runner:2.1.21")
    testImplementation(kotlin("test"))
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test { useJUnitPlatform() }

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    exclude { element ->
        element.file.absolutePath == file("../kotlin/src/main/kotlin/dev/semanticthread/worker/FirFactsPlugin.kt").absolutePath
    }
}
