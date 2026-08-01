pluginManagement { repositories { gradlePluginPortal(); mavenCentral() } }
dependencyResolutionManagement { repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { mavenCentral() } }
rootProject.name = "semantic-thread"
include(":workers:kotlin")
include(":workers:kotlin21")
include(":workers:kotlin23")
