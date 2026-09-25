#!/bin/sh
# Baut libwebrtc für MeeGo Harmattan (Nokia N950).
#
#   webrtc/bauen.sh <quellbaum> [sysroot]
#
# <quellbaum> ist das src-Verzeichnis eines WebRTC-Abzugs:
#   fetch --nohooks --no-history webrtc
#   python3 tools/clang/scripts/update.py
#
# Rechne mit 14 GB Quelltext und einer Stunde Bauzeit. Der Abzug gehört
# NICHT nach /tmp: der ist auf dem Baurechner ein tmpfs im Arbeitsspeicher.
set -e
SRC=${1:?Quellbaum fehlt}
SR=${2:-$HOME/QtSDK/Madde/sysroots/harmattan_sysroot_10.2011.34-1_slim}
HIER=$(cd "$(dirname "$0")" && pwd)
SCHICHT="$HIER/schicht"

[ -d "$SRC/build/config/compiler" ] || { echo "kein WebRTC-Quellbaum: $SRC" >&2; exit 1; }
[ -d "$SR/usr/include" ] || { echo "kein Sysroot: $SR" >&2; exit 1; }

# Die V4L2-Kopfdateien des Baurechners: der Sysroot-Kernel von 2009 kennt
# v4l2_capability.device_caps noch nicht (kam mit Kernel 3.3). Die Kamera
# öffnen wir nie, es geht nur ums Übersetzen.
mkdir -p "$SCHICHT/linux"
for h in videodev2.h v4l2-common.h v4l2-controls.h; do
    [ -f "$SCHICHT/linux/$h" ] || cp "/usr/include/linux/$h" "$SCHICHT/linux/$h"
done

# Die Schicht in JEDE Übersetzung ziehen -- aber nur für ARM. Für den
# Baurechner darf sie nicht gelten: bindgen (Rust) zieht sich sonst
# C++-Kopfdateien in den Parser und scheitert an libc++-Vorlagen.
GN="$SRC/build/config/compiler/BUILD.gn"
if ! grep -q harmattan-schicht "$GN"; then
    python3 - "$GN" "$HIER" "$SCHICHT" <<'PY'
import sys
gn, hier, schicht = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(gn).read()
i = s.index('config("compiler") {')
j = s.index("cflags = []", i)
neu = '''cflags = []

    # HARMATTAN: eine Kompatibilitaetsschicht fuer glibc 2.10 und Kernel
    # 2.6.32, nur fuer das Ziel.
    if (current_cpu == "arm") {
      cflags += [
        "-include",
        "%s/harmattan-schicht.h",
        "-isystem",
        "%s",
      ]
    }''' % (hier, schicht)
s = s[:j] + neu + s[j + len("cflags = []"):]
open(gn, "w").write(s)
print("Schicht eingehaengt")
PY
fi

mkdir -p "$SRC/out/harmattan"
sed "s|@SYSROOT@|$SR|" "$HIER/args.gn" > "$SRC/out/harmattan/args.gn"

cd "$SRC"
gn gen out/harmattan
ninja -C out/harmattan webrtc
ls -la out/harmattan/obj/libwebrtc.a
