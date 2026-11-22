plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "app.bioplay"
    compileSdk = 37
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "app.bioplay"
        // Android 11 : Surface.setFrameRate (fréquence de l'écran calée sur celle du film).
        minSdk = 30
        targetSdk = 37
        versionCode = 1
        versionName = "0.1"
        // Téléphones : arm64 seulement ; la version debug ajoute x86_64 pour l'émulateur.
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        debug {
            ndk { abiFilters += "x86_64" }
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs["debug"]
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Polices rangées sans compression : projetées en mémoire au lieu d'être décompressées.
    androidResources { noCompress += listOf("ttf") }

    // Le shader du verre est commun avec le bureau.
    sourceSets["main"].assets.directories.add("$rootDir/desktop/shaders")
    // Drapeaux des langues, communs avec le bureau.
    sourceSets["main"].assets.directories.add("$rootDir/shared")
    sourceSets["main"].jniLibs.directories.add(layout.buildDirectory.dir("generated/jniLibs").get().asFile.path)

    packaging {
        // Le système charge les bibliothèques natives directement depuis l'APK.
        jniLibs.useLegacyPackaging = false
    }
}

val rust = tasks.register<Exec>("buildRust") {
    workingDir = rootDir
    commandLine("./build-rust.sh", layout.buildDirectory.dir("generated/jniLibs").get().asFile.path)
    inputs.dir("$rootDir/core/src")
    inputs.file("$rootDir/core/Cargo.toml")
    outputs.dir(layout.buildDirectory.dir("generated/jniLibs"))
}

tasks.named("preBuild") { dependsOn(rust) }

dependencies {
    val compose = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(compose)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.activity:activity-compose:1.13.0")
    // Lecteur : mpv 0.42 + FFmpeg compilés pour Android.
    implementation("dev.jdtech.mpv:libmpv:1.1.0")
    // Partages réseau (SMB 2/3, disque de la box) ; liste des partages par DCE/RPC.
    implementation("com.hierynomus:smbj:0.14.0")
    implementation("com.rapid7.client:dcerpc:0.12.13")
}
