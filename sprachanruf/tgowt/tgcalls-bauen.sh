#!/bin/sh
# Baut tgcalls gegen das gebaute tg_owt, fuer Harmattan.
#
#   tgowt/tgcalls-bauen.sh <tgcalls-Baum> <tg_owt-Baum> <webrtc-src-Baum> \
#                          <openssl-prefix> [schicht] [ausgabe]
#
# tgcalls bringt kein Bausystem mit -- Telegram Desktop uebersetzt es in
# seinem eigenen CMake-Baum mit. Wir uebersetzen es mit denselben Fahnen
# wie tg_owt: derselbe clang, dieselbe libc++, dieselben Hartungsschalter.
# Weicht davon etwas ab, passen die Vorlagennamen beim Binden nicht.
#
# Nicht dabei: group/ -- Gruppenanrufe, die ffmpeg gebunden brauchen. Fuer
# ein Telefonat mit einem Gegenueber ist das ueberfluessig.
#
# desktop_capturer/ dagegen muss mit: die Kameraanbindung von
# platform/tdesktop verweist darauf. Ohne X11 findet sie nichts, aber sie
# gehoert zum Uebersetzungsbild.
set -e
TG=${1:?tgcalls-Baum fehlt}
O=${2:?tg_owt-Baum fehlt}
W=${3:?WebRTC-src-Baum fehlt}
SSL=${4:?OpenSSL-Prefix fehlt}
HIER=$(cd "$(dirname "$0")" && pwd)
SCHICHT=${5:-$HIER/../webrtc/schicht}
AUS=${6:-$O/bau-harmattan/tgcalls}
SR=${SR:-$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim}

CXX=$W/third_party/llvm-build/Release+Asserts/bin/clang++
AR=$W/third_party/llvm-build/Release+Asserts/bin/llvm-ar

HART="--target=arm-linux-gnueabihf -march=armv7-a -mfloat-abi=hard -mfpu=neon -mthumb"
LIBCPP="-nostdinc++ -isystem $W/third_party/libc++/src/include -isystem $W/third_party/libc++abi/src/include -I$W/buildtools/third_party/libc++ -D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE -D_LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS -D_LIBCXXABI_DISABLE_VISIBILITY_ANNOTATIONS"

# Die Vorgaben von tg_owt fuer die Benutzer der Bibliothek stehen in
# tg_owtConfig.cmake; hier stehen sie von Hand, damit das Skript ohne
# CMake auskommt.
DEFS="-DWEBRTC_POSIX -DWEBRTC_LINUX -DWEBRTC_ARCH_ARM -DWEBRTC_ARCH_ARM_V7 -DWEBRTC_HAS_NEON -DWEBRTC_USE_H264 -DWEBRTC_APM_DEBUG_DUMP=0 -DRTC_ENABLE_VP9 -DWEBRTC_HAVE_SCTP -DABSL_ALLOCATOR_NOTHROW=1 -DHAVE_SCTP -D__STDC_FORMAT_MACROS -D__STDC_CONSTANT_MACROS"

IZUS="-I$TG -I$TG/tgcalls -I$O/src -I$O/src/third_party/abseil-cpp -I$O/src/third_party/libyuv/include -I$O/src/third_party/crc32c/src/include -I$O/src/third_party/libsrtp/include -I$O/src/third_party/libsrtp/crypto/include -I$O/src/rtc_base/third_party -isystem $SSL/include -isystem $W/third_party/opus/src/include -isystem $W/third_party/ffmpeg/chromium/config/Chrome/linux/arm-neon -isystem $W/third_party/ffmpeg"

FAHNEN="$HART --sysroot=$SR -include $SCHICHT/../harmattan-schicht.h -isystem $SCHICHT $LIBCPP -std=c++20 -O2 -fPIC -fno-strict-aliasing -fvisibility=hidden $DEFS $IZUS -Wno-everything"

mkdir -p "$AUS"
QUELLEN=$(ls $TG/tgcalls/*.cpp $TG/tgcalls/utils/*.cpp $TG/tgcalls/v2/*.cpp $TG/tgcalls/platform/tdesktop/*.cpp $TG/tgcalls/desktop_capturer/*.cpp $TG/tgcalls/third-party/json11.cpp 2>/dev/null | grep -v Test)

gut=0; schlecht=0
for q in $QUELLEN; do
    o="$AUS/$(basename "$(dirname "$q")")-$(basename "$q" .cpp).o"
    if $CXX $FAHNEN -c "$q" -o "$o" > "$o.log" 2>&1; then
        gut=$((gut+1)); rm -f "$o.log"
    else
        schlecht=$((schlecht+1))
        echo "!! $(basename "$q"): $(sed -e 's/\x1b\[[0-9;]*m//g' "$o.log" | grep -m1 'error:' | sed "s|$TG/||" | cut -c1-130)"
    fi
done
echo "== $gut uebersetzt, $schlecht gescheitert"
[ "$schlecht" -eq 0 ] || exit 1
$AR rcs "$AUS/libtgcalls.a" "$AUS"/*.o
ls -la "$AUS/libtgcalls.a"
