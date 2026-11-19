plugins {
    kotlin("jvm") version "2.4.10"
    kotlin("kapt") version "2.4.10"
    kotlin("plugin.spring") version "2.4.10"
}
repositories { mavenCentral() }
kotlin {
    jvmToolchain(25)
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_25) }
}
dependencies {
    implementation("org.springframework:spring-context:6.0.19")
    compileOnly(project(":processor"))
    kapt(project(":processor"))
}
kapt {
    arguments { arg("generationMode", file("generation-mode.txt").readText().trim()) }
}
tasks.matching { it.name == "kaptKotlin" }.configureEach { inputs.file("generation-mode.txt") }
tasks.named("compileKotlin") { doLast { file("build/selected-compile.marker").writeText("compiled") } }

val javaOutput = layout.buildDirectory.dir("generated/sources/independent/main")
val generateIndependent by tasks.registering {
    outputs.dir(javaOutput)
    doLast {
        val target = javaOutput.get().file("fixture/generated/IndependentJava.java").asFile
        target.parentFile.mkdirs()
        target.writeText("package fixture.generated; public final class IndependentJava { public static int number() { return 7; } }")
    }
}
sourceSets.named("main") { java.srcDir(javaOutput) }
// Only compileJava adds this producer to the graph. Kotlin orders itself
// after it when both tasks are already scheduled by a native :classes build.
tasks.named("compileJava") { dependsOn(generateIndependent) }
tasks.matching { it.name in setOf("kaptGenerateStubsKotlin", "compileKotlin") }.configureEach { mustRunAfter(generateIndependent) }
