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

cat > "$STAGE/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
for p in $(pidof tonprozess 2>/dev/null); do
    kill "$p" 2>/dev/null || true
done
exit 0
PRERM
chmod 755 "$STAGE/DEBIAN/prerm"

DEB="drahtpost-ton_${VERSION}_armel.deb"
python3 paket/mkdeb.py "$STAGE" "$DEB"
