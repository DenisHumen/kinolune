import java.util.Base64

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// PKG-018 (ADR-NNNN): the build flavor (ADR-0041) is `--dart-define=KINOLUNE_FLAVOR`, which Flutter hands to Gradle base64-encoded
// in the `dart-defines` property. Full builds get the in-app updater (src/full: REQUEST_INSTALL_PACKAGES and PackageInstaller); store
// builds get a no-op stub (src/store) instead, so neither the permission nor the installer code is in them. Default: store, as in Dart.
val dartDefines: List<String> = (project.findProperty("dart-defines") as String?).orEmpty()
    .split(",")
    .filter { it.isNotEmpty() }
    .mapNotNull { runCatching { String(Base64.getDecoder().decode(it), Charsets.UTF_8) }.getOrNull() }
fun dartDefine(name: String): String? = dartDefines.lastOrNull { it.startsWith("$name=") }?.removePrefix("$name=")
val kinoluneFlavor: String = dartDefine("KINOLUNE_FLAVOR") ?: "store"
val kinoluneFull = kinoluneFlavor == "full"

// PLAY-012 (ADR-0153): the Google Cast sender. The Cast SDK is proprietary, so F-Droid builds pass
// `--dart-define=KINOLUNE_GOOGLE_CAST=false`: then neither the SDK nor its options provider is in the APK (src/nocast is a
// no-op bridge with the same API). Every other build of both flavors has it, as in Dart (AppFlavor.googleCast).
val kinoluneCast = dartDefine("KINOLUNE_GOOGLE_CAST") != "false"

android {
    namespace = "app.kinolune"
    // permission_handler_android needs API 37 to compile against.
    compileSdk = maxOf(flutter.compileSdkVersion, 37)
    ndkVersion = flutter.ndkVersion

    compileOptions {
        // flutter_local_notifications (TWI-009) needs java.time desugaring.
        isCoreLibraryDesugaringEnabled = true
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "app.kinolune"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    sourceSets {
        getByName("main") {
            java.srcDir(if (kinoluneFull) "src/full/kotlin" else "src/store/kotlin")
            java.srcDir(if (kinoluneCast) "src/cast/kotlin" else "src/nocast/kotlin")
        }
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

// PKG-018: the full flavor's manifest (REQUEST_INSTALL_PACKAGES, the install result receiver) is merged into full builds only.
androidComponents {
    onVariants { variant ->
        if (kinoluneFull) variant.sources.manifests.addStaticManifestFile("src/full/AndroidManifest.xml")
        // PLAY-012: the Cast options provider (builds with the Cast SDK only).
        if (kinoluneCast) variant.sources.manifests.addStaticManifestFile("src/cast/AndroidManifest.xml")
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    // TWI-009: java.time for flutter_local_notifications on old Android.
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.4")
    // NET-019 (ADR-NNNN): background downloads — the notification and the WorkManager kick (Apache-2.0; the
    // workmanager plugin of TWI-009 already brings the same WorkManager).
    implementation("androidx.core:core:1.19.0")
    implementation("androidx.work:work-runtime:2.11.2")
    // PLAY-AND-009 (ADR-NNNN): the system Watch Next row of Android TV and Google TV (Apache-2.0).
    implementation("androidx.tvprovider:tvprovider:1.1.0")
    // PLAY-012 (ADR-0153): Google Cast sender with the Default Media Receiver (proprietary Google APIs terms; not in F-Droid
    // builds) and the media router that lists the devices (Apache-2.0).
    if (kinoluneCast) {
        implementation("com.google.android.gms:play-services-cast-framework:22.1.0")
        implementation("androidx.mediarouter:mediarouter:1.7.0")
    }
    testImplementation("junit:junit:4.13.2")
}
