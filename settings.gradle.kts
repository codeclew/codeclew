pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }
dependencyResolutionManagement { repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { mavenCentral() } }
rootProject.name = "semantic-thread"
listOf("kotlin", "kotlin21", "kotlin23").forEach { worker ->
    if (file("workers/$worker").isDirectory) include(":workers:$worker")
}
