plugins {
    kotlin("jvm") version "2.4.20"
    kotlin("plugin.allopen") version "2.4.20"
}
repositories { mavenCentral() }
kotlin {
    jvmToolchain(21)
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21) }
}
allOpen { annotation("fixture.OpenForExtension") }
