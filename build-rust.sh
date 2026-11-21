#!/usr/bin/env bash
# Compile le cœur Rust (index des sous-titres) pour Android.
# Usage : build-rust.sh <dossier jniLibs de sortie>
set -euo pipefail
out=$1
cd "$(dirname "$0")"
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-${ANDROID_HOME:?ANDROID_HOME manquant}/ndk/30.0.16248370}
cargo ndk -t arm64-v8a -t x86_64 --platform 30 -o "$out" build --release -p bioplay-core
