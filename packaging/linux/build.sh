#!/bin/bash
# Paquet autonome de bioplay pour Linux, construit dans Ubuntu 22.04 (bibliothèque C 2.35 :
# tourne sur toute distribution de 2022 ou plus récente). Compile depuis les sources ce dont
# bioplay a besoin, réduit au nécessaire : ffmpeg (décodeurs, sans encodeurs vidéo ni réseau),
# libplacebo 7.360 (Dolby Vision), mpv (libmpv seule), dav1d ; puis bioplay. Rassemble le tout
# dans /out/bioplay : bin/ (bioplay, ffmpeg, ffprobe) et lib/ (chemin relatif inscrit dans les
# programmes). Ne sont pas embarqués : bibliothèque C, pilotes graphiques, Wayland/X11, son.
#
# Usage : docker run --rm -v "$PWD":/src -v /tmp/out:/out ubuntu:22.04 /src/packaging/linux/build.sh
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
P=/opt/deps
export PKG_CONFIG_PATH=$P/lib/pkgconfig:$P/lib/x86_64-linux-gnu/pkgconfig:$P/share/pkgconfig
export LD_LIBRARY_PATH=$P/lib:$P/lib/x86_64-linux-gnu
export PATH=$P/bin:/root/.cargo/bin:$PATH
J=$(nproc)
log() { echo "=== $*"; }
mkdir -p /out/logs
# Étape déjà faite (dépendances gardées d'un essai à l'autre si /opt/deps est un volume) : sautée.
done_() { [ -e "$P/.fait-$1" ]; }
mark() { touch "$P/.fait-$1"; }

log "paquets de construction"
apt-get update -qq
apt-get install -y -qq build-essential git pkg-config nasm cmake ninja-build python3-pip curl ca-certificates \
  patchelf libass-dev libfreetype-dev libfribidi-dev libharfbuzz-dev libfontconfig-dev liblcms2-dev \
  glslang-dev glslang-tools libluajit-5.1-dev libuchardet-dev libzimg-dev libva-dev libdrm-dev libvulkan-dev \
  libxkbcommon-dev libx11-dev libxext-dev libxrandr-dev libxss-dev libxinerama-dev libxpresent-dev \
  libegl-dev libgl-dev libgbm-dev libpulse-dev libasound2-dev libspeex-dev zlib1g-dev \
  libexpat1-dev libffi-dev libxml2-dev >/dev/null
pip3 install -q 'meson>=1.4'
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal >/dev/null
cd /tmp

if ! done_ wayland; then
log "wayland 1.23 et wayland-protocols 1.41"
git clone -q --depth 1 -b 1.23.1 https://gitlab.freedesktop.org/wayland/wayland.git
meson setup wayland/build wayland --prefix=$P -Ddocumentation=false -Dtests=false >/dev/null && ninja -C wayland/build install >/dev/null
git clone -q --depth 1 -b 1.41 https://gitlab.freedesktop.org/wayland/wayland-protocols.git
meson setup wp/build wayland-protocols --prefix=$P -Dtests=false >/dev/null && ninja -C wp/build install >/dev/null

mark wayland
fi
if ! done_ entetes; then
log "en-têtes Vulkan 1.3.290 et NVIDIA (ffnvcodec)"
git clone -q --depth 1 -b v1.3.290 https://github.com/KhronosGroup/Vulkan-Headers.git
cmake -S Vulkan-Headers -B vh/build -DCMAKE_INSTALL_PREFIX=$P >/dev/null && cmake --install vh/build >/dev/null
git clone -q --depth 1 -b n13.0.19.0 https://github.com/FFmpeg/nv-codec-headers.git
make -C nv-codec-headers PREFIX=$P install >/dev/null

mark entetes
fi
# Chargeur Vulkan récent pour la compilation seulement (mpv en exige un plus récent qu'Ubuntu
# 22.04) ; à l'exécution, c'est celui du système (non embarqué).
if ! done_ loader; then
log "chargeur Vulkan 1.3.290"
git clone -q --depth 1 -b v1.3.290 https://github.com/KhronosGroup/Vulkan-Loader.git
cmake -S Vulkan-Loader -B vl/build -DCMAKE_INSTALL_PREFIX=$P -DCMAKE_BUILD_TYPE=Release -DVULKAN_HEADERS_INSTALL_DIR=$P \
  -DBUILD_TESTS=OFF -DBUILD_WSI_XCB_SUPPORT=OFF -DBUILD_WSI_XLIB_SUPPORT=ON -DBUILD_WSI_WAYLAND_SUPPORT=ON >/out/logs/vkloader.log 2>&1
cmake --build vl/build -j$J >>/out/logs/vkloader.log 2>&1 && cmake --install vl/build >>/out/logs/vkloader.log 2>&1 || { tail -20 /out/logs/vkloader.log; exit 1; }
mark loader
fi
if ! done_ dav1d; then
log "dav1d 1.5.1"
git clone -q --depth 1 -b 1.5.1 https://code.videolan.org/videolan/dav1d.git
meson setup dav1d/build dav1d --prefix=$P --buildtype=release -Denable_tools=false -Denable_tests=false >/dev/null && ninja -C dav1d/build install >/dev/null

mark dav1d
fi
if ! done_ dovi; then
log "libdovi (Dolby Vision)"
curl -sSfL https://github.com/lu-zero/cargo-c/releases/latest/download/cargo-c-x86_64-unknown-linux-musl.tar.gz | tar -xz -C /root/.cargo/bin
git clone -q --depth 1 https://github.com/quietvoid/dovi_tool.git
(cd dovi_tool/dolby_vision && cargo cinstall --release --prefix=$P --libdir=$P/lib)

mark dovi
fi
if ! done_ placebo; then
log "libplacebo 7.360.1"
git clone -q --depth 1 --recurse-submodules -b v7.360.1 https://code.videolan.org/videolan/libplacebo.git
meson setup libplacebo/build libplacebo --prefix=$P --libdir=lib --buildtype=release -Dvulkan=enabled -Dglslang=enabled \
  -Dlcms=enabled -Dlibdovi=enabled -Ddemos=false -Dtests=false -Dopengl=enabled >/dev/null
ninja -C libplacebo/build install >/dev/null

mark placebo
fi
if ! done_ ffmpeg; then
log "ffmpeg 9.0.2 réduit"
git clone -q --depth 1 -b n9.0.2 https://github.com/FFmpeg/FFmpeg.git ffmpeg
(cd ffmpeg && ./configure --prefix=$P --enable-shared --disable-static --disable-doc --disable-debug --enable-gpl \
  --disable-autodetect --disable-encoders --enable-encoder=png,mjpeg --disable-muxers --enable-muxer=image2,image2pipe \
  --disable-network --disable-protocols --enable-protocol=file,pipe --disable-devices \
  --enable-zlib --enable-iconv --enable-libdav1d --enable-libspeex --enable-vaapi --enable-vulkan --enable-libdrm \
  --enable-ffnvcodec --enable-nvdec --enable-cuvid >/dev/null && make -j$J >/dev/null 2>&1 && make install >/dev/null)

mark ffmpeg
fi
if ! done_ mpv; then
# Son : PulseAudio (accepté aussi par PipeWire) et ALSA ; la sortie PipeWire native de mpv veut
# une PipeWire plus récente qu'Ubuntu 22.04.
log "mpv 0.41.0 (libmpv seule, réduite)"
git clone -q --depth 1 -b v0.41.0 https://github.com/mpv-player/mpv.git
meson setup mpv/build mpv --prefix=$P --libdir=lib --buildtype=release -Dlibmpv=true -Dcplayer=false \
  -Dcdda=disabled -Ddvdnav=disabled -Dlibbluray=disabled -Dlibarchive=disabled -Djavascript=disabled \
  -Drubberband=disabled -Dsdl2-audio=disabled -Dsdl2-video=disabled -Dsdl2-gamepad=disabled -Dvapoursynth=disabled \
  -Ddvbin=disabled -Dlibavdevice=disabled -Dcplugins=disabled -Djpeg=disabled -Dmanpage-build=disabled \
  -Dhtml-build=disabled -Dpdf-build=disabled -Dtests=false -Dxv=disabled -Doss-audio=disabled -Djack=disabled \
  -Dopenal=disabled -Dsndio=disabled -Dvdpau=disabled -Dsixel=disabled -Dcaca=disabled -Ddrm=disabled -Dgbm=disabled \
  -Dlua=luajit -Duchardet=enabled -Dvulkan=enabled -Dwayland=enabled -Dx11=enabled -Dpipewire=disabled \
  -Dpulse=enabled -Dalsa=enabled -Dcuda-hwaccel=enabled -Dcuda-interop=enabled >/out/logs/mpv-setup.log 2>&1 || { tail -30 /out/logs/mpv-setup.log; exit 1; }
ninja -C mpv/build install >/out/logs/mpv-build.log 2>&1 || { tail -30 /out/logs/mpv-build.log; exit 1; }

mark mpv
fi
log "bioplay"
cp -r /src /tmp/bioplay && cd /tmp/bioplay && cargo build -q --release -p bioplay

log "assemblage"
B=/out/bioplay; rm -rf $B; mkdir -p $B/bin $B/lib $B/share/applications $B/share/icons/hicolor/scalable/apps
cp target/release/bioplay $P/bin/ffmpeg $P/bin/ffprobe $B/bin/
cp -L $P/lib/libmpv.so.2 $B/lib/
cp desktop/data/bioplay.desktop $B/share/applications/
cp desktop/data/bioplay.svg $B/share/icons/hicolor/scalable/apps/
# Ce qui vient toujours du système : bibliothèque C, C++, pilotes graphiques, affichage, son,
# polices, et quelques bibliothèques universelles et stables.
EXCLUDE='/(ld-linux|libc|libm|libdl|libpthread|librt|libresolv|libutil|libgcc_s|libstdc\+\+|libmvec)\.so|libGL|libEGL|libGLX|libGLdispatch|libOpenGL|libvulkan|libdrm|libgbm|libva|libvdpau|libcuda|libnvidia|libwayland|libX|libxcb|libxkbcommon|libpipewire|libpulse|libasound|libjack|libsystemd|libdbus|libfontconfig|libfreetype|libharfbuzz|libexpat|libz\.so|libffi|libglib|libgio|libgobject|libgmodule|libbz2|libpng|libbrotli|libgraphite'
# Dépendances directes (NEEDED) des programmes et bibliothèques embarqués, de proche en proche,
# sans descendre dans celles laissées au système (leurs propres dépendances viennent avec lui).
python3 - "$B" "$EXCLUDE" <<'PY'
import os, re, shutil, subprocess, sys
B, ex = sys.argv[1], re.compile(sys.argv[2])
def needed(f):
    out = subprocess.run(["readelf", "-d", f], capture_output=True, text=True).stdout
    return re.findall(r"\(NEEDED\).*\[(.+?)\]", out)
def paths(f):
    out = subprocess.run(["ldd", f], capture_output=True, text=True).stdout
    return dict(m.groups() for m in re.finditer(r"^\s*(\S+) => (/\S+)", out, re.M))
todo = [os.path.join(B, "bin", x) for x in os.listdir(os.path.join(B, "bin"))] + [os.path.join(B, "lib", "libmpv.so.2")]
seen = set(os.listdir(os.path.join(B, "lib")))
while todo:
    f = todo.pop()
    where = paths(f)
    for n in needed(f):
        p = where.get(n)
        if n in seen or not p or ex.search(p):
            continue
        seen.add(n)
        dst = os.path.join(B, "lib", n)
        shutil.copy(os.path.realpath(p), dst)
        todo.append(dst)
PY
for f in $B/bin/*; do patchelf --set-rpath '$ORIGIN/../lib' "$f"; done
for f in $B/lib/*; do patchelf --set-rpath '$ORIGIN' "$f"; done
strip --strip-unneeded $B/lib/* $B/bin/ffmpeg $B/bin/ffprobe 2>/dev/null || true
tar -C /out -czf /out/bioplay-linux-x86_64.tar.gz bioplay
log "fini : $(du -sh $B | cut -f1) décompressé, $(du -h /out/bioplay-linux-x86_64.tar.gz | cut -f1) compressé, $(ls $B/lib | wc -l) bibliothèques"
