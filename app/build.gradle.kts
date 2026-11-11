import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// The release key lives outside the repo, and only on the machine that makes releases. Without it a
// release build falls back to the debug key, which can't update a phone running the real one.
val releaseSigning = Properties().apply {
    val file = File(System.getenv("VYOMA_SIGNING") ?: "${System.getProperty("user.home")}/.local/share/vyoma-signing/signing.properties")
    if (file.exists()) file.inputStream().use { load(it) }
}

android {
    // No Google-encrypted dependency list in the APK: only Google can read it, and F-Droid and IzzyOnDroid flag it.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }
    namespace = "dev.vyoma.connect"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.vyoma.connect"
        minSdk = 31
        targetSdk = 37
        versionCode = 11
        // One version for the phone app and the PC app (desktop/daemon reads it too).
        versionName = rootProject.file("VERSION").readText().trim()
        buildConfigField("boolean", "DEMO", "false")
    }

    signingConfigs {
        if (!releaseSigning.isEmpty) create("release") {
            storeFile = File(releaseSigning.getProperty("storeFile"))
            storePassword = releaseSigning.getProperty("storePassword")
            keyAlias = releaseSigning.getProperty("keyAlias")
            keyPassword = releaseSigning.getProperty("keyPassword")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
        debug {
            applicationIdSuffix = ".debug"
        }
        // Self-contained showcase: a simulated PC inside the app, no network. Installs beside the real app.
        create("demo") {
            initWith(getByName("release"))
            // For showing around: anyone can build one, so never the real key.
            signingConfig = signingConfigs.getByName("debug")
            applicationIdSuffix = ".demo"
            versionNameSuffix = "-demo"
            matchingFallbacks += "release"
            buildConfigField("boolean", "DEMO", "true")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources {
            excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "/META-INF/DEPENDENCIES", "/META-INF/LICENSE*", "/META-INF/NOTICE*", "META-INF/versions/9/OSGI-INF/MANIFEST.MF")
        }
    }
}

dependencies {
    implementation(project(":core:protocol"))
    implementation(project(":core:designsystem"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.profileinstaller)
    implementation(libs.androidx.glance.appwidget)
    implementation(libs.androidx.glance.material3)
    implementation(libs.androidx.palette)
    implementation(libs.sshd.core)
    implementation(libs.sshd.sftp)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.zxing.core)

    testImplementation(libs.junit)
    testImplementation(kotlin("test-junit"))
    // Android's org.json is a stub on the JVM; the real one for rule (de)serialisation tests.
    testImplementation("org.json:json:20231013")
}
