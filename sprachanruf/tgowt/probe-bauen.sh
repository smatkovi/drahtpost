#!/bin/sh
# Bindet die Probe: tgcalls + tg_owt + libc++ + OpenSSL + Opus.
#
#   tgowt/probe-bauen.sh <tgcalls-Baum> <tg_owt-Baum> <webrtc-src> \
#                        <openssl-prefix> <opus-prefix> [schicht]
set -e
TG=${1:?}; O=${2:?}; W=${3:?}; SSL=${4:?}; OPUS=${5:?}
HIER=$(cd "$(dirname "$0")" && pwd)
SCHICHT=${6:-$HIER/../webrtc/schicht}
# Der Lader heisst auf Harmattan /lib/ld-linux.so.3, nicht
# ld-linux-armhf.so.3: die Distribution nennt sich armel, ist aber in
# Wahrheit hart gleitkommig. Ohne diese Zeile meldet die Shell "No such
# file or directory" fuer eine Datei, die sichtbar da ist.
#
# -static-libgcc, weil das Sysroot nur libgcc_s.so.1 hat, nicht den
# Entwicklungsverweis libgcc_s.so -- die statische libgcc.a tut dasselbe.
#
# clang bringt keine Anlaufdateien und keine libgcc mit -- die kommen aus
# der Kreuz-GCC. Ohne sie fehlen crtbeginS.o und -lgcc.
# Der Tarball dazu liegt auf dem Laptop in ~/ps/toolchains -- auf dem
# Baurechner ist /tmp ein tmpfs und nach dem Neustart leer.
GCC=${GCC:-/tmp/xgcc-harmattan/lib/gcc/arm-none-linux-gnueabi/14.2.0}
SR=${SR:-$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim}
AUS=${AUS:-$O/bau-harmattan/probe}
# Welche Probe gebaut wird -- tgcalls-probe.cpp beweist das Binden,
# audio-messen.cpp misst, was die Tonaufbereitung kostet.
QUELLE=${QUELLE:-$HIER/tgcalls-probe.cpp}
NAME=$(basename "$QUELLE" .cpp)

CXX=$W/third_party/llvm-build/Release+Asserts/bin/clang++
CPP=$W/out/harmattan/obj/buildtools/third_party/libc++/libc++.a
# libvpx kommt aus dem Upstream-Bau: dort ist es fuer armv7+NEON fertig
# eingerichtet (ninja third_party/libvpx:libvpx). Die Archive sind duenn,
# die Objekte muessen also stehen bleiben.
VPX=$W/out/harmattan/obj/third_party/libvpx
CPPABI=$W/out/harmattan/obj/buildtools/third_party/libc++abi/libc++abi.a

HART="--target=arm-linux-gnueabihf -march=armv7-a -mtune=cortex-a8 -mfloat-abi=hard -mfpu=neon -mthumb"
LIBCPP="-nostdinc++ -isystem $W/third_party/libc++/src/include -isystem $W/third_party/libc++abi/src/include -I$W/buildtools/third_party/libc++ -D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE -D_LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS -D_LIBCXXABI_DISABLE_VISIBILITY_ANNOTATIONS"

mkdir -p "$AUS"
# -x c: clang++ wuerde die Datei sonst als C++ uebersetzen und die Namen
# verzieren -- der Binder faende die Attrappen dann nicht.
$CXX -x c $HART --sysroot=$SR -O2 -fPIC -c "$HIER/video-attrappen.c" -o "$AUS/video-attrappen.o"
# --whole-archive fuer tgcalls: die Fassungen tragen sich ueber statische
# Objekte selbst ein (tgcalls::Meta). Ohne das holt der Binder die
# betreffenden Objekte nie herein, und Versions() bliebe leer.
$CXX $HART --sysroot=$SR -include $SCHICHT/../harmattan-schicht.h -isystem $SCHICHT \
    $LIBCPP -std=c++20 -O2 \
    -DWEBRTC_POSIX -DWEBRTC_LINUX -DWEBRTC_ARCH_ARM -DWEBRTC_ARCH_ARM_V7 -DWEBRTC_HAS_NEON \
    -I$TG -I$TG/tgcalls -I$O/src -I$O/src/third_party/abseil-cpp \
    -I$W/third_party/opus/src/include \
    "$QUELLE" -o "$AUS/$NAME" \
    -fuse-ld=lld -Wl,--error-limit=0 -Wl,--dynamic-linker=/lib/ld-linux.so.3 -nostdlib++ -static-libgcc -B"$GCC" -L"$GCC" \
    -Wl,--whole-archive "$O/bau-harmattan/tgcalls/libtgcalls.a" -Wl,--no-whole-archive \
    "$O/bau-harmattan/libtg_owt.a" \
    "$VPX/libvpx.a" "$VPX/libvpx_assembly_arm.a" $VPX/libvpx_intrinsics_neon/*.o \
    "$AUS/video-attrappen.o" \
    "$OPUS/libopus.a" "$SSL/lib/libssl.a" "$SSL/lib/libcrypto.a" \
    "$CPP" "$CPPABI" \
    -L"$SR/usr/lib" -lz -ljpeg -lpulse-simple -lpulse -lpthread -ldl -lm -lrt

ls -la "$AUS/$NAME"
