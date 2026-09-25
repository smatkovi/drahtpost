#!/bin/sh
# Baut tg_owt (Telegrams WebRTC-Abzug) fuer Harmattan.
#
#   tgowt/bauen.sh <tg_owt-Baum> <webrtc-src-Baum> <openssl-prefix>
#
# Warum tg_owt und nicht das Upstream-WebRTC, das schon gebaut ist: siehe
# README, Abschnitt "tgcalls: gegen welches WebRTC?". Kurz: tgcalls spricht
# rtc::, cricket:: und sigslot, und die gibt es upstream nicht mehr.
#
# Vom Upstream-Baum bleibt trotzdem alles Wichtige in Gebrauch: das clang,
# die libc++ und die Kopfdateien von ffmpeg und opus.
set -e
T=${1:?tg_owt-Baum fehlt}
W=${2:?WebRTC-src-Baum fehlt}
SSL=${3:?OpenSSL-Prefix fehlt}
SR=${SR:-$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim}
HIER=$(cd "$(dirname "$0")" && pwd)
AUS=${AUS:-$T/bau-harmattan}

[ -f "$T/CMakeLists.txt" ] || { echo "kein tg_owt-Baum: $T" >&2; exit 1; }
[ -f "$SSL/include/openssl/ssl.h" ] || { echo "kein OpenSSL: $SSL" >&2; exit 1; }

cmake -S "$T" -B "$AUS" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$HIER/harmattan.cmake" \
    -DWEBRTC_SRC="$W" -DSCHICHT="$HIER/../webrtc/schicht" -DSYSROOT="$SR" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_CXX_STANDARD=20 \
    -DBUILD_SHARED_LIBS=OFF \
    -DTG_OWT_BUILD_AUDIO_BACKENDS=OFF \
    -DTG_OWT_USE_X11=OFF \
    -DTG_OWT_USE_PIPEWIRE=OFF \
    -DTG_OWT_USE_PROTOBUF=OFF \
    -DTG_OWT_ARCH_ARMV7_USE_NEON=ON \
    -DTG_OWT_OPENSSL_INCLUDE_PATH="$SSL/include" \
    -DTG_OWT_OPUS_INCLUDE_PATH="$W/third_party/opus/src/include" \
    -DTG_OWT_FFMPEG_INCLUDE_PATH="$W/third_party/ffmpeg" \
    -DTG_OWT_LIBJPEG_INCLUDE_PATH="$SR/usr/include"

cmake --build "$AUS" -- -k 0
echo "== fertig:"; ls -la "$AUS/libtg_owt.a"
