plugins { base }

val springFixtureTarget = providers.environmentVariable("CARGO_TARGET_DIR")
    .map { file(it) }.getOrElse(file("target"))
val springFixture = tasks.register<Exec>("prepareSpringFrameworkFixture") {
    workingDir(rootDir)
    environment("CARGO_INCREMENTAL", "0")
    commandLine("cargo", "build", "--locked", "-p", "clew-framework-spring", "--example", "derive")
    inputs.files(fileTree("crates/clew-facts"), fileTree("crates/clew-framework-spring"), file("Cargo.toml"), file("Cargo.lock"))
    outputs.file(springFixtureTarget.resolve("debug/examples/derive"))
}

subprojects {
    tasks.withType<org.gradle.jvm.application.tasks.CreateStartScripts>().configureEach {
        doLast {
            // Select the analyzer JVM without changing JAVA_HOME inherited by
            // project-native Maven/Gradle children of the worker.
            val marker = "if [ -n \"\$JAVA_HOME\" ] ; then"
            val script = unixScript.readText()
            check(script.contains(marker)) { "Worker launcher Java selection template changed" }
            unixScript.writeText(script.replace(marker,
                "if [ -n \"\$CODECLEW_WORKER_JAVA_HOME\" ] ; then\n" +
                "    JAVACMD=\$CODECLEW_WORKER_JAVA_HOME/bin/java\n" +
                "elif [ -n \"\$JAVA_HOME\" ] ; then"))
        }
    }
    tasks.withType<org.gradle.api.tasks.testing.Test>().configureEach {
        dependsOn(springFixture)
        systemProperty("codeclew.test.springInterpreter", springFixtureTarget.resolve("debug/examples/derive").absolutePath)
        // Unit fixtures construct workers without the managed launcher, which
        // supplies this snapshot namespace in production.
        environment("CODECLEW_K1_BUILD_STATE_NAMESPACE", "sha256:" + "a".repeat(64))
    }
}
