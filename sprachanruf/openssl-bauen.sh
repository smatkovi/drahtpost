#!/bin/sh
# Baut OpenSSL 1.1.1w statisch fuer Harmattan.
#
# tg_owt setzt OpenSSL 1.1 voraus; im Sysroot liegt 0.9.8k von 2009. Auf
# dem Geraet ist 1.1.1w zwar nachinstalliert, aber statisch zu binden ist
# hier das Richtige: dann haengt der Anrufdienst an keiner Bibliothek, die
# jemand spaeter wegraeumen kann.
set -e
QUELLE=${QUELLE:-$HOME/sfos-browser-next-140/esr115/rust182/vendor/openssl-src-111.28.2+1.1.1w/openssl}
SR=$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim
BAU=${1:-/run/media/sebastian/2e638a7f-26db-4e89-9446-81d688464798/webrtc-bau/openssl-bau}
ZIEL=$BAU/fertig
# Die Kreuz-GCC; der Tarball dazu liegt auf dem Laptop in
# ~/ps/toolchains -- /tmp ist auf dem Baurechner ein tmpfs.
X=/tmp/xgcc-harmattan/bin/arm-none-linux-gnueabi

rm -rf "$BAU"; mkdir -p "$BAU"
cp -a "$QUELLE"/. "$BAU/quelle"
cd "$BAU/quelle"
git clean -xfd 2>/dev/null || true

# linux-armv4 ist OpenSSLs Name fuer 32-bit-ARM; die Assembler-Teile
# (auch die NEON-Varianten von AES und SHA) kommen damit mit.
CC="$X-gcc --sysroot=$SR -march=armv7-a -mfpu=neon" \
AR="$X-ar" RANLIB="$X-ranlib" \
./Configure linux-armv4 no-shared no-tests no-dso no-engine \
    --prefix="$ZIEL" --openssldir=/etc/ssl > "$BAU/konfig.log" 2>&1

make -j"$(nproc)" > "$BAU/bau.log" 2>&1
make install_sw > "$BAU/inst.log" 2>&1

echo "== fertig:"
ls -la "$ZIEL/lib/libcrypto.a" "$ZIEL/lib/libssl.a"
$X-readelf -A "$ZIEL/lib/libcrypto.a" 2>/dev/null | grep -m2 "VFP"
