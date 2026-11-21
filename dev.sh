#!/usr/bin/env bash
# Compile, installe sur l'émulateur et ouvre la vidéo de test. Usage : ./dev.sh [fichier sur l'appareil]
set -e
cd "$(dirname "$0")"
source ~/Android/env.sh
./gradlew -q --console=plain assembleDebug 2>&1 | grep -E '^e: |FAIL' || true
adb install -r -t app/build/outputs/apk/debug/app-debug.apk | tail -1
adb logcat -c
f=${1:-/data/data/app.bioplay/files/test.mkv}
adb shell am start -a android.intent.action.VIEW -d "file://$f" -t video/x-matroska -n app.bioplay/.MainActivity >/dev/null
