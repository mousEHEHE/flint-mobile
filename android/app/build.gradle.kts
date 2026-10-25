// build.sh builds the Rust library into src/main/jniLibs; Gradle only packages it.

plugins {
    id("com.android.application")
}

android {
    namespace = "com.mouse.flint"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.mouse.flint"
        minSdk = 26 // Vulkan 1.0 is there from API 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += listOf("arm64-v8a")
        }

        // NativeActivity loads lib<name>.so; the manifest reads the name.
        manifestPlaceholders["nativeLibraryName"] = "flint_mobile"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            isDebuggable = true
            isJniDebuggable = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    packaging {
        // cargo strips release builds itself; stripping a debug build breaks its backtraces.
        jniLibs {
            keepDebugSymbols += listOf("*/arm64-v8a/libflint_mobile.so")
        }
    }

    lint {
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    // The Java half of gpui-mobile (src/main/java/dev/gpui/mobile) uses these.
    implementation("androidx.core:core:1.12.0")
    implementation("androidx.core:core-splashscreen:1.0.1")
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("androidx.media:media:1.7.1")
}
