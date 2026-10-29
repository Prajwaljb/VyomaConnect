import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.jvm)
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_17
    }
}

dependencies {
    api(libs.kotlinx.coroutines.core)
    api(libs.kotlinx.serialization.json)
    implementation(libs.bouncycastle.pkix)

    testImplementation(libs.junit)
    testImplementation(kotlin("test-junit"))
    testImplementation(libs.kotlinx.coroutines.test)
}

tasks.test {
    // Interop tests talk to the real kdeconnectd on this machine; opt in with VYOMA_INTEROP=1.
    environment("VYOMA_INTEROP", System.getenv("VYOMA_INTEROP") ?: "0")
    // kdeconnectd sends the peer's device id as SNI; ids contain '_', which the JDK's TLS server
    // rejects as a hostname. Android's Conscrypt accepts it, so only the JVM test needs this.
    systemProperty("jdk.tls.server.disableExtensions", "server_name")
    testLogging {
        events("passed", "skipped", "failed", "standardOut", "standardError")
        showStandardStreams = true
    }
}
