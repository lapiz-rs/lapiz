import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.OutputDirectory
import java.util.Properties

abstract class SyncGeneratedDirectory : Sync() {
    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty
}

plugins {
    id("com.android.application")
}

val repoRoot = file("../..")
val toolchain = Properties().apply {
    file("../toolchain.properties").inputStream().use { load(it) }
}
val arch = providers.gradleProperty("androidArch").orNull ?: mapOf(
    "amd64" to "x86_64",
    "x86_64" to "x86_64",
    "aarch64" to "aarch64",
    "arm64" to "aarch64",
)[System.getProperty("os.arch")]
val abi = mapOf(
    "aarch64" to "arm64-v8a",
    "x86_64" to "x86_64",
)[arch] ?: throw GradleException("Specify -PandroidArch=aarch64 or -PandroidArch=x86_64")
val rustTarget = "$arch-linux-android"

android {
    namespace = "dev.lapiz.app"
    compileSdk = toolchain.getProperty("ANDROID_PLATFORM").toInt()
    buildToolsVersion = toolchain.getProperty("ANDROID_BUILD_TOOLS")
    ndkVersion = toolchain.getProperty("ANDROID_NDK")

    packaging {
        jniLibs {
            if (!providers.gradleProperty("stripRustSymbols").isPresent) {
                keepDebugSymbols += "**/liblapiz_app.so"
            }
        }
    }

    defaultConfig {
        applicationId = "dev.lapiz.app"
        minSdk = 28
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += abi
        }
    }

    flavorDimensions += "channel"
    productFlavors {
        create("dev") {
            dimension = "channel"
            applicationId = "dev.lapiz.dbg"
        }
        create("prod") {
            dimension = "channel"
            applicationId = "dev.lapiz.app"
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            // TODO Use real certificate when I can finally afford it
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

dependencies {
    implementation("androidx.activity:activity:1.10.1")
    implementation("androidx.core:core:1.16.0")
}

val syncAssets = tasks.register<SyncGeneratedDirectory>("syncAssets") {
    outputDirectory.set(layout.buildDirectory.dir("generated/assets"))
    from(repoRoot.resolve("assets/builtin_assets")) {
        into("builtin_assets")
    }
    into(outputDirectory)
}

androidComponents {
    beforeVariants(selector().all()) { variant ->
        variant.enable = (variant.buildType == "debug") == (variant.productFlavors[0].second == "dev")
    }

    onVariants(selector().all()) { variant ->
        val taskSuffix = variant.name.replaceFirstChar { it.uppercaseChar() }
        val profile = if (variant.buildType == "release") "release" else "dev"
        // The built-in dev profile is built into target/debug.
        val outDir = if (variant.buildType == "release") "release" else "debug"
        val buildRust = tasks.register<Exec>("buildRust$taskSuffix") {
            workingDir = repoRoot
            doFirst {
                environment("ANDROID_NDK_HOME", sdkComponents.ndkDirectory.get().asFile.absolutePath)
            }
            commandLine(
                "cargo", "ndk", "-P", "28", "-t", abi, "build", "--locked",
                "-p", "lapiz_app", "--lib", "--profile", profile,
            )
        }
        val syncRust = tasks.register<SyncGeneratedDirectory>("syncRust$taskSuffix") {
            dependsOn(buildRust)
            outputDirectory.set(layout.buildDirectory.dir("generated/jniLibs/${variant.name}"))
            from(repoRoot.resolve("target/$rustTarget/$outDir/liblapiz_app.so")) {
                into(abi)
            }
            into(outputDirectory)
        }
        variant.sources.assets?.addGeneratedSourceDirectory(syncAssets, SyncGeneratedDirectory::outputDirectory)
        variant.sources.jniLibs?.addGeneratedSourceDirectory(syncRust, SyncGeneratedDirectory::outputDirectory)
    }
}
