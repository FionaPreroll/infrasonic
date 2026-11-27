// SPDX-License-Identifier: GPL-3.0-only
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

// The release version. versionCode = MAJOR * 10000 + MINOR * 100 + PATCH; tags are android-vX.Y.Z.
// One universal APK per channel (no ABI splits); the store build is the same on Play and F-Droid.
val appVersion = "0.1.0"
val appVersionCode = appVersion.split(".").map(String::toInt).let { (a, b, c) -> a * 10000 + b * 100 + c }

// The GitHub build (release android-latest) comes from every commit on main that changes the app.
// CI sets INFRASONIC_BUILD to the number of commits on main, which only grows and is the same for
// anyone who builds that commit. Its version code is above every store build's, so a store build
// updates to it, and the update check compares it with the release's version.json.
val githubBuild = System.getenv("INFRASONIC_BUILD")?.toInt()
val githubVersionCode = githubBuild?.let { 1_000_000 + it } ?: appVersionCode
val githubVersionName = githubBuild?.let { "$appVersion-$it-" + (System.getenv("INFRASONIC_COMMIT") ?: "").take(7) }
    ?: appVersion

// Android ABIs for the Rust library; `-Pinfrasonic.abis=arm64-v8a` builds fewer.
val abis = (findProperty("infrasonic.abis") as String? ?: "arm64-v8a,armeabi-v7a,x86_64")
    .split(",").map(String::trim).filter(String::isNotEmpty)
val repoRoot: File = rootDir.parentFile
val rustSources = fileTree(repoRoot) {
    include("crates/*/src/**", "crates/*/Cargo.toml", "crates/ffi/uniffi.toml", "vendor/**", "Cargo.toml", "Cargo.lock")
    exclude("**/target/**")
}

/** Runs `android/build-rust.sh`; the output directory is the task's only output. */
abstract class RustTask : Exec() {
    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty
}

// The NDK that builds the Rust library is the one pinned below, for reproducible builds.
val ndkVersionPinned = "29.0.14206865"
val ndkDir: String = (System.getenv("ANDROID_HOME") ?: rootDir.resolve("local.properties").takeIf { it.exists() }
    ?.readLines()?.firstOrNull { it.startsWith("sdk.dir=") }?.substringAfter("="))
    ?.let { "$it/ndk/$ndkVersionPinned" } ?: ""

/** A Rust task writing to `build/<dir>`; `args` gets that directory's path. */
fun rustTask(name: String, dir: String, args: (String) -> List<String>) =
    tasks.register<RustTask>(name) {
        group = "rust"
        inputs.files(rustSources).withPathSensitivity(PathSensitivity.RELATIVE)
        inputs.file(rootDir.resolve("build-rust.sh"))
        val out = layout.buildDirectory.dir(dir)
        outputDir.set(out)
        workingDir = repoRoot
        environment("ANDROID_NDK_HOME", ndkDir)
        executable = rootDir.resolve("build-rust.sh").path
        args(args(out.get().asFile.path))
    }

val rustBindings = rustTask("rustBindings", "generated/uniffi") { listOf("bindings", it) }
val rustDebug = rustTask("rustAndroidDebug", "rust/debug") { listOf("android", "debug", it) + abis }
val rustRelease = rustTask("rustAndroidRelease", "rust/release") { listOf("android", "release", it) + abis }
// The library for this machine, loaded by the JVM unit tests through JNA (in target/debug).
val rustHost = rustTask("rustHost", "rust/host") { listOf("host") }

android {
    namespace = "io.github.fionapreroll.infrasonic"
    ndkVersion = ndkVersionPinned
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }

    defaultConfig {
        applicationId = "io.github.fionapreroll.infrasonic"
        minSdk = 26
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Screenshots from the device tests land in build/outputs/connected_android_test_additional_output.
        testInstrumentationRunnerArguments["useTestStorageService"] = "true"
        ndk { abiFilters += abis }
    }

    // Release signing from the environment (CI secrets); without it the release APK is unsigned,
    // as F-Droid builds it before comparing with the published APK.
    val keystore = System.getenv("INFRASONIC_KEYSTORE")
    if (keystore != null) {
        signingConfigs.create("release") {
            storeFile = file(keystore)
            storePassword = System.getenv("INFRASONIC_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("INFRASONIC_KEY_ALIAS")
            keyPassword = System.getenv("INFRASONIC_KEY_PASSWORD")
            enableV1Signing = false
            enableV2Signing = true
            enableV3Signing = true
        }
    }

    // "github" installs new versions of itself from the release android-latest, if the user agrees
    // (src/github/). "store" is for Google Play and F-Droid, which bring updates themselves: no
    // update check and no permission to install apps (src/store/).
    flavorDimensions += "channel"
    productFlavors {
        create("github") {
            dimension = "channel"
            versionCode = githubVersionCode
            versionName = githubVersionName
            buildConfigField(
                "String",
                "UPDATES_URL",
                "\"" + (System.getenv("INFRASONIC_UPDATES_URL")
                    ?: "https://github.com/FionaPreroll/infrasonic/releases/download/android-latest/") + "\"",
            )
        }
        create("store") {
            dimension = "channel"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
            vcsInfo { include = false }
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // No Google-signed dependency block in the APK (F-Droid, reproducible builds).
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    packaging {
        jniLibs.useLegacyPackaging = false
        resources.excludes += setOf("META-INF/*.version", "META-INF/**/LICENSE*", "DebugProbesKt.bin")
    }

    lint {
        abortOnError = true
        warningsAsErrors = false
        checkReleaseBuilds = true
        disable += setOf("GradleDependency", "NewerVersionAvailable", "AndroidGradlePluginVersion")
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

androidComponents {
    onVariants { variant ->
        variant.sources.kotlin?.addGeneratedSourceDirectory(rustBindings, RustTask::outputDir)
        val rust = if (variant.buildType == "release") rustRelease else rustDebug
        variant.sources.jniLibs?.addGeneratedSourceDirectory(rust, RustTask::outputDir)
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(rustHost)
    systemProperty("jna.library.path", repoRoot.resolve("target/debug").path)
    // Servers for the end-to-end tests, from scripts/e2e-servers.sh.
    System.getenv("INFRASONIC_E2E_SERVERS")?.let { environment("INFRASONIC_E2E_SERVERS", it) }
    testLogging {
        events("failed", "skipped")
        showStandardStreams = true
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
    }
}

dependencies {
    implementation(libs.androidx.core)
    implementation(libs.androidx.appcompat)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.navigation.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.icons)
    implementation(libs.media3.session)
    implementation(libs.media3.common)
    implementation(libs.androidx.media)
    implementation(libs.work.runtime)
    implementation(libs.coroutines.android)
    implementation(libs.coroutines.guava)
    implementation(libs.jna) { artifact { type = "aar" } }
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    testImplementation(libs.json)
    testImplementation(libs.coroutines.test)
    testImplementation(libs.jna)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.uiautomator)
    androidTestImplementation(libs.coroutines.test)
    androidTestImplementation(libs.androidx.test.storage)
    androidTestUtil(libs.androidx.test.services)
}
