#!/bin/sh
# Baut libtgvoip fuer Harmattan (armv7, softfp, NEON).
#
# Ohne den mitgelieferten WebRTC-Ton-Teil (TGVOIP_NO_DSP): Echo,
# Rauschen und Aussteuerung macht der Sprachpfad des Geraets, sobald der
# Ton ueber die SIP-Bruecke laeuft. Das spart 276 Dateien.
#
# Ohne Video (TGVOIP_NO_VIDEO) fuer den ersten Durchgang.
# Krypto liefert der Aufrufer (TGVOIP_USE_CUSTOM_CRYPTO), damit wir nicht
# an der uralten OpenSSL des Sysroots haengen.
set -e
SRC=/tmp/libtgvoip
SR=$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim
OUT=${1:-/tmp/tgvoip-bau}
OPUS=$HOME/ps/external/libopus
GCCXX=/tmp/xgcc-harmattan/arm-none-linux-gnueabi/include/c++

# Die GCC-14-Kreuzkette statt clang: sie bringt eine C++11-Bibliothek mit,
# die dem Sysroot von 2009 fehlt. Eingebunden wird sie statisch -- genau
# so, wie die Qt-Oberflaeche dieses Projekts gebaut wird.
CXX="/tmp/xgcc-harmattan/bin/arm-none-linux-gnueabi-g++ --sysroot=$SR -march=armv7-a -mfpu=neon"

mkdir -p "$OUT"
cd "$SRC"

QUELLEN="$(ls *.cpp) $(ls audio/*.cpp) $(ls video/*.cpp) os/posix/NetworkSocketPosix.cpp"

FAHNEN="-std=c++11 -O3 -ffast-math -fno-strict-aliasing -fexceptions -frtti \
  -DTGVOIP_NO_DSP -DTGVOIP_NO_VIDEO -DTGVOIP_USE_CUSTOM_CRYPTO -DUSE_NEON \
  -D__STDC_LIMIT_MACROS -D__STDC_FORMAT_MACROS -Wno-c++11-narrowing -DWEBRTC_POSIX -DTGVOIP_USE_CALLBACK_AUDIO_IO \
  -I. -I$OPUS/include -Wno-unknown-pragmas -Wno-unused-variable \
  -include /tmp/tgvoip-kompat.h"

rm -f "$OUT"/*.o
fehler=0
for q in $QUELLEN; do
    o="$OUT/$(echo "$q" | tr '/' '_' | sed 's/\.cpp$/.o/')"
    if ! $CXX $FAHNEN -c "$q" -o "$o" 2> "$OUT/$(basename "$q").log"; then
        fehler=$((fehler+1))
        echo "!! $q"
        head -4 "$OUT/$(basename "$q").log"
    fi
done
echo "== $(ls "$OUT"/*.o 2>/dev/null | wc -l) Dateien uebersetzt, $fehler gescheitert"
