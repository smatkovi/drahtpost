#!/bin/sh
# Baut libopus fuer Harmattan (armv7, softfp) und misst es auf dem Geraet.
#
# Ohne autotools: die Quelllisten stehen in den *.mk, und uebersetzt wird
# von Hand -- so hat man die Fahnen wirklich in der Hand.
set -e
SRC=$HOME/ps/external/libopus
SR=$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim
OUT=${1:-/tmp/opusbau}
ART=${2:-fixed}     # fixed oder float

CC="clang --target=armv7-unknown-linux-gnueabi --sysroot=$SR -mfloat-abi=softfp -march=armv7-a -mfpu=neon"

mkdir -p "$OUT"
cd "$SRC"

liste() { sed -n "/^$1[ =]/,/^$/p" "$2" | sed "s/^$1 *= *//; s/\\\\$//" | tr -s ' \n' '\n' | grep '\.c$' | grep -v '/x86/'; }

QUELLEN="$(liste CELT_SOURCES celt_sources.mk) $(liste SILK_SOURCES silk_sources.mk) $(liste OPUS_SOURCES opus_sources.mk) $(liste OPUS_SOURCES_FLOAT opus_sources.mk) $(liste CELT_SOURCES_ARM_NEON_INTR celt_sources.mk) $(liste SILK_SOURCES_ARM_NEON_INTR silk_sources.mk)"
if [ "$ART" = fixed ]; then
    QUELLEN="$QUELLEN $(liste SILK_SOURCES_FIXED silk_sources.mk)"
    ARITHMETIK="-DFIXED_POINT=1"
    IZUS="-Isilk/fixed"
else
    QUELLEN="$QUELLEN $(liste SILK_SOURCES_FLOAT silk_sources.mk)"
    ARITHMETIK="-DFLOAT_APPROX"
    IZUS="-Isilk/float"
fi

# NEON ohne Laufzeiterkennung: der Cortex-A8 hat es, also PRESUME.
# Nur die NEON-Intrinsics, nicht der handgeschriebene Assembler: der
# liegt als .s vor und muesste erst durch arm2gnu.pl. Die Intrinsics sind
# das, was "--enable-intrinsics" einschaltet, und decken die heissen
# Stellen ab (celt_pitch_xcorr, NSQ).
NEON="-DOPUS_ARM_MAY_HAVE_NEON_INTR -DOPUS_ARM_PRESUME_NEON_INTR"
FAHNEN="-O3 -fno-math-errno $ARITHMETIK $NEON -DOPUS_BUILD -DUSE_ALLOCA -DHAVE_LRINTF \
        -I. -Iinclude -Icelt -Isilk $IZUS"

rm -f "$OUT"/*.o
n=0
for q in $QUELLEN; do
    o="$OUT/$(echo "$q" | tr '/' '_' | sed 's/\.c$/.o/')"
    $CC $FAHNEN -c "$q" -o "$o" 2>&1 | head -5
    n=$((n+1))
done
echo "== $n Dateien uebersetzt ($ART)"
