#!/bin/sh
# Packt den Tonprozess als eigenes .deb.
#
# Eigenes Paket, nicht in die Drahtpost hinein: das Programm ist mit
# 10 MB siebenmal so gross wie sie, und wer nicht telefoniert, braucht es
# nicht. Ausserdem haengt es an einer Bauverabredung (tg_owt, OpenSSL,
# Opus), die sich viel seltener aendert als die Drahtpost selbst.
set -e
cd "$(dirname "$0")/.."

VERSION=${1:-0.1}
STAGE=build/stage-ton
rm -rf "$STAGE"

[ -x build/tonprozess ] || { echo "build/tonprozess fehlt -- erst tools/ton-bauen.sh" >&2; exit 1; }

mkdir -p "$STAGE/opt/drahtpost" "$STAGE/DEBIAN"
cp build/tonprozess "$STAGE/opt/drahtpost/"
chmod 755 "$STAGE/opt/drahtpost/tonprozess"
sed "s/@VERSION@/$VERSION/" paket/control-ton.in > "$STAGE/DEBIAN/control"

cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
# Die Regel gilt erst fuer neu angelegte Knoten -- die Kameraknoten gibt
# es seit dem Start. Also einmal nachziehen.
udevadm trigger --subsystem-match=video4linux 2>/dev/null || true
udevadm trigger --subsystem-match=media 2>/dev/null || true
exit 0
POSTINST
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
for p in $(pidof tonprozess 2>/dev/null); do
    kill "$p" 2>/dev/null || true
done
exit 0
PRERM
chmod 755 "$STAGE/DEBIAN/prerm"

# Der Aegis-Antrag: ohne GRP::video kommt der Tonprozess nicht an die
# Kamera. /dev/media0 und die Subdev-Knoten gehoeren root:video, und der
# Benutzer der Sitzung ist nicht in dieser Gruppe -- gst-launch scheitert
# sonst mit "Initializing the media controller failed: Permission
# denied". telepathy-stream-engine beantragt dieselbe Gruppe fuer
# dieselbe Sache.
cat > "$STAGE/DEBIAN/_aegis" <<'AEGIS'
<aegis name="drahtpost-ton">
  <request>
    <credential name="GRP::video" />
    <for path="/opt/drahtpost/tonprozess" />
  </request>
</aegis>
AEGIS

# Die Kamera erreichbar machen.
#
# /dev/media0 und die Subdev-Knoten gehoeren root:video, und der Benutzer
# der Sitzung ist nicht in dieser Gruppe. Der saubere Weg waere der
# Aegis-Antrag oben -- der wird einem unsignierten Paket aber nicht
# gewaehrt (restok.conf fuehrt GRP::video als ~GRP::video, also
# geschuetzt). Bleibt die Regel: Gruppe users, Modus 0660. Damit kommt
# jede App des Benutzers an die Kamera -- und das ist eine bewusste
# Lockerung, keine Nebenwirkung. Wer sie zurueck will, loescht die Datei
# und startet neu.
#
# Die Nummer muss ueber 91 liegen: 91-permissions.rules setzt
# SUBSYSTEM=="video4linux" GROUP="video", und wer davor kommt, wird
# ueberschrieben. Das hat beim ersten Versuch (60-) genau so ausgesehen,
# als greife die Regel gar nicht.
mkdir -p "$STAGE/etc/udev/rules.d"
cat > "$STAGE/etc/udev/rules.d/92-drahtpost-kamera.rules" <<'REGEL'
# Videoanrufe brauchen den Media-Controller des OMAP3-ISP.
KERNEL=="media[0-9]*", GROUP="users", MODE="0660"
SUBSYSTEM=="video4linux", GROUP="users", MODE="0660"
REGEL

DEB="drahtpost-ton_${VERSION}_armel.deb"
python3 paket/mkdeb.py "$STAGE" "$DEB"
