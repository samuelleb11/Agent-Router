plugins {
    alias(libs.plugins.android.application)
    // No `kotlin.android` plugin: AGP 9 builds Kotlin itself, and applying it is an error.
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Firebase is optional at build time, exactly as the push entitlement is on iOS: without
// `google-services.json` the app still builds, installs and drives sessions — it simply
// says notifications are not set up. Applying the plugin unconditionally would fail the
// build outright for anyone who has not done the console step yet.
val hasFirebase = file("google-services.json").exists()
if (hasFirebase) apply(plugin = "com.google.gms.google-services")

android {
    namespace = "ca.whnet.agentrouter"
    compileSdk = 37

    defaultConfig {
        applicationId = "ca.whnet.agentrouter"
        // The test phone is a Galaxy Note 9, which Samsung left on Android 10.
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        buildConfigField("boolean", "HAS_FIREBASE", hasFirebase.toString())
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // The messaging service imports Firebase, so it can only be compiled when Firebase is
    // actually a dependency. Keeping it in its own source dir is what lets the rest of the
    // app build and run before the console step has been done.
    sourceSets.getByName("main") {
        if (hasFirebase) kotlin.srcDir("src/firebase/java")
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// SharedFixtureTest reads the fixtures the web console and both phones share, at the
// repository's root. They are an input of every unit test run, so a changed fixture runs the
// tests again instead of reusing a cached result that knew nothing of it.
tasks.withType<Test>().configureEach {
    inputs.dir(rootProject.file("../fixtures/live"))
        .withPropertyName("sharedFixtures")
        .withPathSensitivity(PathSensitivity.RELATIVE)
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.activity.compose)
    implementation(libs.navigation.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.process)
    implementation(libs.okhttp)
    implementation(libs.serialization.json)
    implementation(libs.vico.compose.m3)

    if (hasFirebase) {
        implementation(platform(libs.firebase.bom))
        implementation(libs.firebase.messaging)
    }

    debugImplementation(libs.compose.ui.tooling)
    testImplementation(libs.junit)
}
