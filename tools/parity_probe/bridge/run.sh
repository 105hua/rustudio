#!/bin/sh
# Runs the bridge corpus headless in rustudio as Studio's was measured: a
# Script in ServerScriptService and a LocalScript in PlayerScripts, both
# named BridgeProbe (game-script identity 2, not the harness's 1).
# usage: run.sh OUTDIR   (writes client.log server.log)
set -u
D=$(cd "$(dirname "$0")" && pwd); O=$1; mkdir -p "$O"
{ printf 'local s=Instance.new("Script"); s.Name="BridgeProbe"; s.Source=[====['; cat "$D/probe.luau"
  printf ']====]; s.Parent=game:GetService("ServerScriptService"); s.Disabled=true; s.Disabled=false\n'; } > "$O/harness-server.luau"
{ printf 'local s=Instance.new("LocalScript"); s.Name="BridgeProbe"; s.Source=[====['; cat "$D/probe.luau"
  printf ']====]; s.Parent=game:GetService("Players").LocalPlayer:WaitForChild("PlayerScripts"); s.Disabled=true; s.Disabled=false\n'; } > "$O/harness-client.luau"
for side in client server; do
RUSTUDIO_NO_AUDIO=1 timeout 120 gamescope --backend headless -- rustudio --demo --play --from-play \
  --luau $side "$O/harness-$side.luau@3" --screenshot "$O/$side.png" --shot-after 12 --log "$O/$side.log" >/dev/null 2>&1
done
