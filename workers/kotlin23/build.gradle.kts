plugins {
    kotlin("jvm") version "2.3.0"
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
    implementation("org.jetbrains.kotlin:kotlin-compiler-embeddable:2.3.0")
    implementation("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin-embeddable:2.3.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.10.0")
    testImplementation(kotlin("test"))
}

application { mainClass.set("dev.semanticthread.worker.MainKt") }
tasks.test { useJUnitPlatform() }

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    compilerOptions.freeCompilerArgs.add("-Xcontext-parameters")
    exclude { element ->
        element.file.name == "SpringAnnotationFacts24.kt" ||
        element.file.absolutePath == file("../kotlin/src/main/kotlin/dev/semanticthread/worker/FirFactsPlugin.kt").absolutePath
    }
}
