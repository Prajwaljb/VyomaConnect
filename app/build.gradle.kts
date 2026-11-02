plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "dev.vyoma.connect"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.vyoma.connect"
        minSdk = 31
        targetSdk = 37
        versionCode = 3
        versionName = "0.2.1"
        buildConfigField("boolean", "DEMO", "false")
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Sideload builds: sign with the debug key until a release key is set up.
            signingConfig = signingConfigs.getByName("debug")
        }
        debug {
            applicationIdSuffix = ".debug"
        }
        // Self-contained showcase: a simulated PC inside the app, no network. Installs beside the real app.
        create("demo") {
            initWith(getByName("release"))
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
            excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "/META-INF/LICENSE*", "/META-INF/NOTICE*", "META-INF/versions/9/OSGI-INF/MANIFEST.MF")
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

    testImplementation(libs.junit)
    testImplementation(kotlin("test-junit"))
    // Android's org.json is a stub on the JVM; the real one for rule (de)serialisation tests.
    testImplementation("org.json:json:20231013")
}
