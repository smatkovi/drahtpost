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

# Harmattan ist hart gleitkommig (readelf -A auf libc/libm/libstdc++:
# "Tag_ABI_VFP_args: VFP registers"). Fuer C und C++ gilt also die harte
# Kette; weich war nur noetig, wo Go mitspielt -- dessen ARM-Konvention
# reicht Gleitkommazahlen in Kernregistern.
# Kreuz-GCC aus /tmp; der Tarball liegt auf dem Laptop in
# ~/ps/toolchains (auf dem Baurechner ist /tmp ein tmpfs).
CC="${CC:-/tmp/xgcc-harmattan/bin/arm-none-linux-gnueabi-gcc --sysroot=$SR -march=armv7-a -mfpu=neon}"

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
# -fPIC, weil alles andere (tg_owt, libc++) lageunabhaengig ist und der
# Binder sonst R_ARM_MOVW_ABS_NC in einem PIE ablehnt.
FAHNEN="-O3 -fPIC -fno-math-errno $ARITHMETIK $NEON -DOPUS_BUILD -DUSE_ALLOCA -DHAVE_LRINTF \
        -I. -Iinclude -Icelt -Isilk $IZUS"

rm -f "$OUT"/*.o
n=0
for q in $QUELLEN; do
    o="$OUT/$(echo "$q" | tr '/' '_' | sed 's/\.c$/.o/')"
    # Nicht durch head leiten: schliesst der die Leitung, faengt der
    # Uebersetzer SIGPIPE und stirbt mitten in der Ausgabe -- bei zwei
    # warnungsreichen Dateien fehlte danach stillschweigend das Objekt.
    if ! $CC $FAHNEN -c "$q" -o "$o" > "$o.log" 2>&1; then
        echo "!! $q"
        grep -m3 "error" "$o.log"
    fi
    n=$((n+1))
done
echo "== $n Dateien uebersetzt ($ART)"
