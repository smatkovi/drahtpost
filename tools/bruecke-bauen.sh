#!/bin/sh
# Baut die SIP-Bruecke fuer Harmattan (ARMv7).
#
# Go bringt seinen eigenen Cross-Compiler mit, also braucht es hier keine
# Toolchain und kein Sysroot -- nur CGO_ENABLED=0, sonst wollte es die
# libc des Build-Rechners.
set -e
cd "$(dirname "$0")/.."
FERN=/tmp/drahtpost-bruecke
WIRT=$(sh "$HOME/ps/nfsshift-sfos/tools/buildhost.sh")
echo "== Build-Rechner: $WIRT"
rsync -a --delete bruecke/ "$WIRT:$FERN/"
ssh "$WIRT" "cd $FERN && GOFLAGS=-mod=mod GOTOOLCHAIN=local go mod tidy >/dev/null 2>&1; \
    GOOS=linux GOARCH=arm GOARM=7 CGO_ENABLED=0 go build -ldflags '-s -w' -o drahtpost-bruecke ."
mkdir -p build
rsync -a "$WIRT:$FERN/drahtpost-bruecke" build/
rsync -a "$WIRT:$FERN/go.sum" bruecke/ 2>/dev/null || true
echo "== fertig: build/drahtpost-bruecke ($(stat -c %s build/drahtpost-bruecke) B)"
