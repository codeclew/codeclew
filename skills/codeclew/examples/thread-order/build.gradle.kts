plugins {
    kotlin("jvm") version "2.4.10"
    application
}
repositories { mavenCentral() }
kotlin { jvmToolchain(21) }
application { mainClass.set("sample.ThreadExampleKt") }
