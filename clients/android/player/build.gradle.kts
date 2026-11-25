// SPDX-License-Identifier: MPL-2.0
// PLAY-AND-002: Media3/ExoPlayer engine behind the core PlayerEngine contract
// (ADR-0017), talking to the core through the UniFFI bindings of pa-ffi.
//
// The core comes from the CORE-004 build (tools/bindings/build-aar.sh), as in
// pa-core-android: generated Kotlin bindings and libpa_ffi.so from `paGenDir`
// (default <workspace>/target/android). Instrumented-test media come from
// tools/make-fixtures.sh in `paFixturesDir` (default build/fixtures). The instrumented tests of
// CoreDataSource (PLAY-AND-003) need the core built with the `testing` feature
// (`PA_TESTING=1 tools/bindings/build-aar.sh`).
// AGP 9 compiles Kotlin itself (built-in Kotlin): no Kotlin plugin.

plugins {
    id("com.android.library") version "9.5.1"
}

val workspaceRoot = rootDir.resolve("../../..").canonicalFile
val genDir: File = providers.gradleProperty("paGenDir").orNull?.let { file(it) }
    ?: workspaceRoot.resolve("target/android")
// PLAY-AND-003: extra native libraries for the test APK only (libsmb2.so per ABI for the Samba streaming test).
val testJniDir: File? = providers.gradleProperty("paTestJniDir").orNull?.let { file(it) }
val fixturesDir: File = providers.gradleProperty("paFixturesDir").orNull?.let { file(it) }
    ?: layout.buildDirectory.dir("fixtures").get().asFile
// PLAY-AND-004: the Media3 FFmpeg audio extension built by tools/media3-ffmpeg from our LGPL FFmpeg (`java/` and
// `jniLibs/<abi>/libffmpegJNI.so`). Optional: without it TrueHD and DTS play only by passthrough or a platform
// decoder. libffmpegJNI.so needs libavcodec/libswresample/libavutil.so in the APK (the app's libmpv set).
val ffmpegExtDir: File? = providers.gradleProperty("paFfmpegExtDir").orNull?.let { file(it) }

android {
    namespace = "dev.projectalpha.player"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // JNA and the UniFFI bindings are found by reflection (same rules as pa-core-android).
        consumerProguardFiles(workspaceRoot.resolve("tools/bindings/pa-core-android/consumer-rules.pro"))
        // The extension's native methods are registered by name from libffmpegJNI.so.
        ffmpegExtDir?.let { consumerProguardFiles(it.resolve("proguard-rules.txt")) }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // JUnit runs on the JVM: android.jar stubs return defaults (PlaybackException reads SystemClock).
    testOptions.unitTests.isReturnDefaultValues = true

    sourceSets {
        getByName("main") {
            // PLAY-AND-005: display modes, HDR and Dolby Vision decisions, shared with the media_kit player plugin.
            kotlin.srcDir(workspaceRoot.resolve("clients/android/display/src/main/kotlin"))
            // PLAY-AND-004: passthrough decisions and HDMI capabilities, shared with the media_kit player plugin.
            kotlin.srcDir(workspaceRoot.resolve("clients/android/audio/src/main/kotlin"))
            kotlin.srcDir(genDir.resolve("kotlin"))
            jniLibs.srcDir(genDir.resolve("jniLibs"))
            ffmpegExtDir?.let {
                java.srcDir(it.resolve("java"))
                jniLibs.srcDir(it.resolve("jniLibs"))
            }
        }
        getByName("test") {
            kotlin.srcDir(workspaceRoot.resolve("clients/android/display/src/test/kotlin"))
            kotlin.srcDir(workspaceRoot.resolve("clients/android/audio/src/test/kotlin"))
        }
        getByName("androidTest") {
            assets.srcDir(fixturesDir)
            testJniDir?.let { jniLibs.srcDir(it) }
        }
    }
}

dependencies {
    val media3 = "1.11.1"
    api("androidx.media3:media3-exoplayer:$media3")
    // PLAY-AND-006: SubtitleView (Media3 cues) and AspectRatioFrameLayout of the video picture.
    api("androidx.media3:media3-ui:$media3")
    implementation("androidx.annotation:annotation:1.9.1")
    // The FFmpeg extension's sources use Checker Framework annotations (MIT; compile time only).
    if (ffmpegExtDir != null) compileOnly("org.checkerframework:checker-qual:3.49.0")
    // UniFFI's Kotlin runtime. JNA is dual-licensed Apache-2.0 / LGPL-2.1; we use it under Apache-2.0.
    api("net.java.dev.jna:jna:5.19.1@aar")

    testImplementation("junit:junit:4.13.2")

    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
