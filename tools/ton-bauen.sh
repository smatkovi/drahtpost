#!/bin/sh
# Baut den Tonprozess (tgcalls) auf dem Baurechner und holt ihn her.
#
# Die Bibliotheken dahinter -- tg_owt, tgcalls, OpenSSL, Opus -- werden
# nicht bei jedem Lauf neu gebaut; die Skripte dafuer stehen in
# sprachanruf/tgowt/ und laufen einmal. Hier geht es nur um das Programm.
set -e
cd "$(dirname "$0")/.."
HOST=$(sh "$HOME/ps/nfsshift-sfos/tools/buildhost.sh")
B=/run/media/sebastian/2e638a7f-26db-4e89-9446-81d688464798/webrtc-bau
FERN=/tmp/tgowt

echo "== Build-Rechner: $HOST"
rsync -a sprachanruf/tgowt/ "$HOST:$FERN/"
rsync -a sprachanruf/webrtc/ "$HOST:/tmp/webrtc-schicht/"
ssh "$HOST" "QUELLE=$FERN/tonprozess.cpp sh $FERN/probe-bauen.sh \
    $B/tgcalls $B/tg_owt $B/src $B/openssl-bau/fertig $B/opus-bau $B/schicht" \
    | tail -2

mkdir -p build
scp -q "$HOST:$B/tg_owt/bau-harmattan/probe/tonprozess" build/
# Ohne die Fehlersuchtabellen: 15 MB davon liegen sonst in jedem Paket
# und auf einem Geraet mit 2 GB /home.
ssh "$HOST" "$B/src/third_party/llvm-build/Release+Asserts/bin/llvm-strip \
    $B/tg_owt/bau-harmattan/probe/tonprozess -o /tmp/tonprozess-schlank"
scp -q "$HOST:/tmp/tonprozess-schlank" build/tonprozess
chmod 755 build/tonprozess
echo "== tonprozess fertig ($(stat -c %s build/tonprozess) B)"
