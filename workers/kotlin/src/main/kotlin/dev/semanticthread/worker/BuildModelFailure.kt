package dev.semanticthread.worker

/** Interpret known build failures without copying private build logs into the public protocol. */
internal fun buildModelFailure(tool: ProjectModelBuildTool, output: String): WorkerFailure {
    val text = output.lowercase()
    val (reason, action) = when {
        listOf("status code: 401", "status code 401", "status code: 403", "status code 403", "401 unauthorized", "403 forbidden", "authentication failed").any(text::contains) ->
            "BUILD_REPOSITORY_AUTHENTICATION" to
                "Restore access to the project's artifact repositories in the same environment (credentials, VPN and Maven settings or Gradle properties), then retry."
        listOf("pkix path building failed", "unable to find valid certification path", "sslhandshakeexception").any(text::contains) ->
            "BUILD_REPOSITORY_TLS" to
                "Configure the project JDK trust store for the artifact repository certificate, then retry; do not disable TLS verification."
        listOf("could not resolve", "could not find artifact", "could not transfer artifact", "non-resolvable parent pom", "in offline mode", "unknownhostexception", "could not get resource").any(text::contains) ->
            "BUILD_DEPENDENCY_RESOLUTION" to
                "Verify repository access and project dependency versions; if offline, populate the project's dependency cache using its normal build, then retry."
        listOf("java_home is not defined correctly", "invalid source release", "error: release version", "unsupported class file major version", "requires java", "no matching toolchains found").any(text::contains) ->
            "BUILD_JDK_CONFIGURATION" to
                "Select the JDK required by the project through JAVA_HOME or its configured toolchain; verify java -version and the build tool's --version output, then retry."
        Regex("task '[^'\\n]+' not found").containsMatchIn(text) || "selected project in the reactor" in text ->
            "BUILD_COMPILATION_NOT_FOUND" to
                "Run clew doctor repository again and select a reported module/compilation; verify that module exists in the native build, then retry."
        else -> "BUILD_MODEL_EXTRACTION_FAILED" to
            "Run the native diagnostic command below, resolve its first failure, then retry Codeclew. If that command succeeds, report the Codeclew error code and selected compilation."
    }
    val diagnostic = when (tool) {
        ProjectModelBuildTool.GRADLE -> "From the repository root run ./gradlew --stacktrace tasks --all."
        ProjectModelBuildTool.MAVEN -> "From the selected module directory run mvn -e -DskipTests help:effective-pom dependency:build-classpath (use the repository's executable mvnw instead of mvn when present)."
    }
    return WorkerFailure(
        "UNSUPPORTED_PROJECT_CONFIGURATION",
        "$reason: ${tool.name} could not supply the project model. $action $diagnostic Build output was omitted because it may contain private repository data or credentials.",
    )
}
