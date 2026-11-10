# Material camera probes

`regression-camera.luau` builds four material swatches in memory (left to
right: Slate, Rock, Cardboard, Leather) and pins a Scriptable camera
(FieldOfView 70) on them after the normal camera step. It changes only the
running Play session and leaves MaterialService.Use2022Materials alone.

```sh
RUSTUDIO_NO_GUI=1 RUSTUDIO_NO_AUDIO=1 target/release/rustudio PLACE.rbxl --play --from-play \
  --window 1600x1000 --luau client tools/parity_probe/materials/regression-camera.luau@3 \
  --screenshot /tmp/swatches.png --shot-after 12
```

Per-instance randomized UV offsets use different seeds from Studio, so a
capture won't match Studio's nail or grain positions exactly.
