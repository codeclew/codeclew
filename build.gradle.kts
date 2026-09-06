plugins { base }

subprojects {
    tasks.withType<org.gradle.api.tasks.testing.Test>().configureEach {
        // Unit fixtures construct workers without the managed launcher, which
        // supplies this snapshot namespace in production.
        environment("CODECLEW_K1_BUILD_STATE_NAMESPACE", "sha256:" + "a".repeat(64))
    }
}
