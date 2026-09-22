#!/bin/sh
# Baut die Drahtpost fuer das N9/N950.
set -e
cd "$(dirname "$0")/.."
FERN=/tmp/drahtpost-src
HOST=$(sh "$HOME/ps/nfsshift-sfos/tools/buildhost.sh")
echo "== Build-Rechner: $HOST"
rsync -a --delete --exclude build --exclude target --exclude .git ./ "$HOST:$FERN/"
ssh "$HOST" 'sh /tmp/drahtpost-src/tools/remote-build.sh'
mkdir -p build
scp -q "$HOST:$FERN/build/drahtpost" build/
echo "== drahtpost fertig ($(stat -c %s build/drahtpost) B)"
