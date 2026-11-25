// SPDX-License-Identifier: MPL-2.0
// pa_player (PLAY-007): Android side of the player package — the system
// caption style (CaptioningManager) for the subtitle overlay.
// PLAY-AND-008: the media session (Media3 MediaSessionService over a
// SimpleBasePlayer bridge to our libmpv engine), audio focus and PiP.
// PLAY-AND-005: display mode switching (AFR) for the media_kit player, from
// clients/android/display (shared with the Media3 engine).
// PLAY-AND-004: audio passthrough (mpv `audio-spdif`) by the HDMI capabilities,
// from clients/android/audio (shared with the Media3 engine).

group = "app.kinolune.player"
version = "0.1.0"

buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.20")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

plugins {
    id("com.android.library")
}

val workspaceRoot: File = projectDir.resolve("../../../../..").canonicalFile

android {
    namespace = "app.kinolune.player"
    compileSdk = 36

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            kotlin.srcDir("src/main/kotlin")
            kotlin.srcDir(workspaceRoot.resolve("clients/android/display/src/main/kotlin"))
            // PLAY-AND-004: passthrough decisions and HDMI audio capabilities, shared with the Media3 engine.
            kotlin.srcDir(workspaceRoot.resolve("clients/android/audio/src/main/kotlin"))
        }
        getByName("test") {
            kotlin.srcDir("src/test/kotlin")
        }
    }

    defaultConfig {
        minSdk = 24
    }
}

dependencies {
    // Apache-2.0. Same Media3 version as clients/android/player (PLAY-AND-002).
    val media3 = "1.11.1"
    implementation("androidx.media3:media3-session:$media3")
    implementation("androidx.media3:media3-common:$media3")
    // Loads artwork (file, content, http) for the notification.
    implementation("androidx.media3:media3-datasource:$media3")
    testImplementation("junit:junit:4.13.2")
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}
