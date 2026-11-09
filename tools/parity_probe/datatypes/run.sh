#!/bin/sh
# Runs the full datatype corpus (direct calls included) headless in rustudio.
# usage: run.sh OUTDIR   (writes client.log server.log extras.log actor.log)
set -u
D=$(cd "$(dirname "$0")" && pwd); O=$1; mkdir -p "$O"
run() { RUSTUDIO_NO_AUDIO=1 timeout 120 gamescope --backend headless -- rustudio --demo --play --from-play \
  --luau "$1" "$D/$2@3" --screenshot "$O/$3.png" --shot-after "$4" --log "$O/$3.log" >/dev/null 2>&1; }
run client harness-client.luau client 12
run server harness-server.luau server 12
run client extras-client.luau extras 8
run server actor.luau actor 8
