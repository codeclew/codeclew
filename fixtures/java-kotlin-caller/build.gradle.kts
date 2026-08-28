plugins { java }

val fixtureLibDir = requireNotNull(System.getProperty("codeclew.fixtureLibDir")) {
    "-Dcodeclew.fixtureLibDir must select the acceptance artifact directory"
}

dependencies {
    implementation(files("$fixtureLibDir/kotlin-provider.jar"))
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 21
}
