#!/bin/sh
# Baut tgcalls gegen das gebaute libwebrtc, fuer Harmattan.
#
# tgcalls bringt kein eigenes Bausystem mit -- Telegram Desktop uebersetzt
# es in seinem CMake-Baum. Wir uebersetzen selbst, aber nicht mit selbst
# ausgedachten Fahnen: die Fahnen kommen aus WebRTCs eigener
# compile_commands.json. Das ist keine Kosmetik. tgcalls und libwebrtc
# muessen dieselbe libc++ mit denselben Hartungs-Schaltern benutzen, sonst
# passen die Symbole beim Binden nicht zusammen, und die Schicht fuer die
# Luecken von 2009 muss auch hier vorgeschaltet sein.
set -e
B=/run/media/sebastian/2e638a7f-26db-4e89-9446-81d688464798/webrtc-bau
W=$B/src
T=$B/tgcalls
O=$W/out/harmattan
AUS=${1:-$B/tgcalls-bau}

# Die Fahnen einer echten WebRTC-Uebersetzungseinheit herausziehen: alles
# ausser dem Uebersetzer selbst, den Abhaengigkeitsdateien und der Quelle.
python3 - "$O/compile_commands.json" > /tmp/tgcalls-fahnen.txt <<'PY'
import json, shlex, sys
for e in json.load(open(sys.argv[1])):
    if e["file"].endswith("pc/peer_connection.cc"):
        w = shlex.split(e["command"])
        aus, i = [], 1
        while i < len(w):
            if w[i] in ("-MF", "-o"):
                i += 2; continue
            if w[i] in ("-MD", "-c") or w[i].endswith(".cc"):
                i += 1; continue
            aus.append(w[i]); i += 1
        print(" ".join(shlex.quote(x) for x in aus))
        break
PY
FAHNEN=$(cat /tmp/tgcalls-fahnen.txt)
CXX=$W/third_party/llvm-build/Release+Asserts/bin/clang++

# Aus dem Bauverzeichnis heraus uebersetzen: die Pfade in den Fahnen sind
# relativ dazu (../.., gen). tgcalls-Pfade deshalb absolut dazu.
cd "$O"
EIGEN="-I$T -I$T/tgcalls -I$W/third_party/libsrtp/include -I$W/third_party/opus/src/include"

mkdir -p "$AUS"
QUELLEN="$(ls $T/tgcalls/*.cpp) $(ls $T/tgcalls/utils/*.cpp) $(ls $T/tgcalls/v2/*.cpp) $(ls $T/tgcalls/platform/fake/*.cpp)"

gut=0; schlecht=0
for q in $QUELLEN; do
    o="$AUS/$(basename "$(dirname "$q")")-$(basename "$q" .cpp).o"
    if eval $CXX $FAHNEN $EIGEN -c "$q" -o "$o" > "$o.log" 2>&1; then
        gut=$((gut+1)); rm -f "$o.log"
    else
        schlecht=$((schlecht+1))
        echo "!! $(basename "$q"): $(grep -m1 "error:" "$o.log" | sed "s|$B||g" | cut -c1-140)"
    fi
done
echo "== $gut uebersetzt, $schlecht gescheitert"
