#!/bin/bash
P=$(cd "$(dirname "$0")" && pwd)
O=$1; mkdir -p $O; cd "$O"
for side in server client; do
RUSTUDIO_NO_AUDIO=1 timeout 120 gamescope --backend headless -- ${RS_BIN:-rustudio} --demo --play --from-play \
  --luau $side $P/$side.luau@3 --luau $side $P/extra-$side.luau@3 \
  --screenshot $O/$side.png --shot-after 9 --log $O/$side.log > $O/$side.stdout 2>&1
done
cat $O/server.log $O/client.log > $O/run.log; grep -c GLOBALS $O/run.log
