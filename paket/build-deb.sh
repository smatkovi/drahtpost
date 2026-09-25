#!/bin/sh
# Packt drahtpost als Harmattan-.deb.
#
#   paket/build-deb.sh [version]
#
# Keine versionierten Abhaengigkeiten -- die Begruendung steht im
# WhatsApp-Port: "~" sortiert in Debian unter der leeren Zeichenkette,
# und eine Bedingung auf 4.7.4 waere gegen 4.7.4~git nie erfuellbar.
set -e
cd "$(dirname "$0")/.."

VERSION=${1:-0.1}
STAGE=build/stage
rm -rf "$STAGE"

[ -x build/drahtpost ] || { echo "build/drahtpost fehlt -- erst tools/build.sh" >&2; exit 1; }

[ -x build/drahtpost-bruecke ] ||
    { echo "build/drahtpost-bruecke fehlt -- erst tools/bruecke-bauen.sh" >&2; exit 1; }

mkdir -p "$STAGE/opt/drahtpost" "$STAGE/DEBIAN" "$STAGE/usr/bin"
cp build/drahtpost "$STAGE/opt/drahtpost/"
chmod 755 "$STAGE/opt/drahtpost/drahtpost"
# Die SIP-Bruecke gehoert zu drahtpost, nicht zum Tonpaket: sie laeuft
# auch ohne Gespraech weiter (das Telefon meldet sich bei ihr an), und
# drahtpost startet sie.
cp build/drahtpost-bruecke "$STAGE/opt/drahtpost/"
chmod 755 "$STAGE/opt/drahtpost/drahtpost-bruecke"
cp paket/telegram-anrufe-einrichten "$STAGE/usr/bin/"
chmod 755 "$STAGE/usr/bin/telegram-anrufe-einrichten"
cp paket/postinst paket/prerm "$STAGE/DEBIAN/"
chmod 755 "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/prerm"
sed "s/@VERSION@/$VERSION/" paket/control.in > "$STAGE/DEBIAN/control"

DEB="drahtpost_${VERSION}_armel.deb"
python3 paket/mkdeb.py "$STAGE" "$DEB"
