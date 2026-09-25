#!/bin/sh
# Baut, packt und installiert auf dem Geraet -- und startet neu.
#
#   tools/installieren.sh [drahtpost-version] [ton-version]
#
# Warum das Neustarten hier steht und nicht in postinst: unter aegis
# greift der kill aus den Paketskripten nicht. Der Aufruf laeuft durch,
# pidof findet die laufende Fassung, kill meldet keinen Fehler -- und der
# alte Prozess lebt weiter, mit der geloeschten Datei im Speicher. Von
# aussen sieht die Installation gelungen aus, und die Aenderung fehlt.
# Sichtbar wird es nur an /proc/<pid>/exe -> ... (deleted).
set -e
cd "$(dirname "$0")/.."
VER=${1:-0.4}
TONVER=${2:-}
GERAET=${GERAET:-192.168.1.8}
SSH="ssh -oHostKeyAlgorithms=+ssh-rsa -oPubkeyAcceptedAlgorithms=+ssh-rsa -i $HOME/.ssh/id_rsa_n9 user@$GERAET"
SCP="scp -q -oHostKeyAlgorithms=+ssh-rsa -oPubkeyAcceptedAlgorithms=+ssh-rsa -i $HOME/.ssh/id_rsa_n9"

# Nie waehrend eines Gespraechs: ein Austausch mitten im Anruf legt ihn.
# Gefragt wird die Drahtpost selbst -- ein laufender Tonprozess ist noch
# kein Gespraech, der wird auch fuer die Tonprobe gestartet.
STAND=$($SSH 'python2.6 -c "
import socket, json
s = socket.socket(socket.AF_UNIX)
try:
    s.connect(\"/home/user/.pytelegram/daemon.sock\")
    s.send(json.dumps({\"cmd\": \"call_status\", \"args\": {}}) + \"\n\")
    print s.recv(400)
except Exception:
    print \"{}\"
"' 2>/dev/null)
case "$STAND" in
    *'"active":true'*)
        echo "✗ es laeuft gerade ein Gespraech -- spaeter" >&2
        exit 1
        ;;
esac

sh tools/build.sh
sh tools/bruecke-bauen.sh > /dev/null
sh paket/build-deb.sh "$VER" > /dev/null
PAKETE="drahtpost_${VER}_armel.deb"
if [ -n "$TONVER" ]; then
    sh tools/ton-bauen.sh > /dev/null
    sh paket/build-ton-deb.sh "$TONVER" > /dev/null
    PAKETE="$PAKETE drahtpost-ton_${TONVER}_armel.deb"
fi

# shellcheck disable=SC2086
$SCP $PAKETE "user@$GERAET:/home/user/"
FERN=""
for p in $PAKETE; do FERN="$FERN /home/user/$p"; done
$SSH "sudo dpkg -i $FERN 2>&1 | grep -iE 'Setting up|error'"
# Die Ausgabe gehoert in eine Datei, nicht nach /dev/null. Beim ersten
# echten Anruf ging genau das schief: warum er nicht hinausging, stand in
# einer Zeile, die nirgends landete.
$SSH 'sudo sh -c "kill $(pidof drahtpost tonprozess drahtpost-bruecke 2>/dev/null)" 2>/dev/null; sleep 2
      cd /home/user && nohup /opt/drahtpost/drahtpost >> /home/user/.pytelegram/drahtpost.log 2>&1 &
      sleep 8; pidof drahtpost > /dev/null && echo "== Drahtpost laeuft" || echo "✗ Drahtpost startet nicht"
      pidof drahtpost-bruecke > /dev/null && echo "== SIP-Bruecke laeuft" || echo "⚠ SIP-Bruecke laeuft nicht"'
$SSH 'for p in $(pidof drahtpost); do ls -la /proc/$p/exe; done'
