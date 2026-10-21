plugins { kotlin("jvm") version "2.3.21" }
kotlin { jvmToolchain(25); compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_25) } }
val generatedJava = layout.buildDirectory.dir("generated/sources/owned/main")
val generateOwnedJava by tasks.registering {
    inputs.file("input.txt")
    outputs.dir(generatedJava)
    doLast {
        val output = generatedJava.get().file("fixture/generated/OwnedValue.java").asFile
        output.parentFile.mkdirs()
        when (val token = file("input.txt").readText().trim()) {
            "missing" -> output.delete()
            "number" -> output.writeText("package fixture.generated; public final class OwnedValue { public static int label() { return 42; } }")
            else -> output.writeText("package fixture.generated; public final class OwnedValue { public static String label() { return \"$token\"; } }")
        }
        file("build/generator-ran.marker").writeText("generated")
    }
}
sourceSets.named("main") { java.srcDir(generateOwnedJava) }
tasks.named("compileKotlin") { doLast { file("build/kotlin-compiled.marker").writeText("compiled") } }
