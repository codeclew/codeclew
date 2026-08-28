plugins { kotlin("jvm") version "2.4.10" }

repositories { mavenCentral() }

val fixtureLibDir = requireNotNull(System.getProperty("codeclew.fixtureLibDir")) {
    "-Dcodeclew.fixtureLibDir must select the acceptance artifact directory"
}

dependencies {
    implementation(files("$fixtureLibDir/java-provider.jar"))
}

kotlin { jvmToolchain(21) }
