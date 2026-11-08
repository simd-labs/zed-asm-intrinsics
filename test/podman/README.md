# Podman test rig

The container runs Zed and headless Sway in rootless podman. No host display is necessary.

## Build the image

Make the image one time. It must have network access and about 1.3 GB of disk.

```sh
podman build -t zed-shot test/podman
```

## Build the extension

```sh
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/zed_asm_intrinsics.wasm extension.wasm
```

## Install check

The container has no `simdref` on the `PATH`, so the extension installs it. The default wait is 600 seconds.

```sh
sh test/podman/run-install.sh [FILE [OUT.png [WAIT_SECS]]]
```

`FILE` defaults to `test/podman/ws/demo.s`. Use `test/podman/ws/demo.cpp` for C++. `OUT.png` defaults to `test/podman/out/install.png`.

The script also copies `OUT.Zed.log` and `OUT.install.simdref.log`, the extension `install.log`, adjacent to `OUT.png`.

The script prints this line on success and exits with 0:

```text
starting language server process. binary path: .../extensions/work/asm-intrinsics-lsp/bin/simdref-lsp
OK: all N new paths in $HOME outside the Zed data dir are on the Zed, mesa, fontconfig and asm-lsp allowlist
.../extensions/work/asm-intrinsics-lsp/home/.local/share/simdref
OK: isa update wrote the catalog dir inside the work dir
```

The script fails when a new path appears in the container `$HOME` outside the Zed data dir and the path is not from Zed, mesa, fontconfig or the `assembly` extension. `outside.txt` adjacent to `OUT.png` lists every new path there.

Set `EXT_DIR` to a directory with `extension.toml` and `extension.wasm` to test another build.

## Screenshot check

`shot.sh` opens a file in Zed and writes a screenshot of the inlay hints.

```sh
sh test/podman/shot.sh WORKSPACE FILE OUT.png [DEV_EXT_DIR ...]
```

`WORKSPACE` and `FILE` are host paths. `FILE` is below `WORKSPACE`. Each `DEV_EXT_DIR` holds `extension.toml` and `extension.wasm`. Its name must be the extension id, `asm-intrinsics-lsp`.

Environment settings:

- `WAIT_SECS`: seconds to wait before the screenshot, default 12.
- `SETTINGS_JSON`: Zed settings file, default `test/podman/settings.json`.
- `PODMAN_NETWORK`: podman network mode, default `slirp4netns`.

For hints, the image must have a new `simdref-lsp` on the `PATH`. The base image does not.

## Screenshot session

The hover shots use a live session. The pointer must move. Each capture waits for the screen to be quiet. `in-container.sh session` starts sway and Zed and keeps them running. The host runs the session with `podman exec`. End the session with `podman rm -f`.

```sh
podman run -d --name zed-session \
  -v $WORKSPACE:/work -v $DEV_EXT:/dev-ext-in/asm-intrinsics-lsp:ro \
  -v $PRESTAGE:/prestage-in:ro -v $HERE/settings.json:/rig/settings.json:ro \
  -v $HERE/in-container.sh:/rig/in-container.sh:ro \
  -v $HERE/mouse.py:/rig/mouse.py:ro -v $HERE/poll.py:/rig/poll.py:ro \
  -v $HERE/measure.py:/rig/measure.py:ro -v $SHOTDIR:/shot \
  -e ZED_ALLOW_ROOT=1 zed-shot sleep infinity
podman exec zed-session sh /rig/in-container.sh session 12 /rig/settings.json /work/demo.s /work/demo.cpp
```

The helpers start in the container with the wayland env:

```sh
XDG="podman exec -e XDG_RUNTIME_DIR=/tmp/xdg-shot -e WAYLAND_DISPLAY=wayland-1 zed-session"
$XDG python3 /rig/poll.py /shot/asm-hints.png
$XDG python3 /rig/mouse.py <x> <y>
$XDG python3 /rig/poll.py /shot/asm-hover.png
```

- `mouse.py X Y` moves the pointer to screen pixels X Y.
- `poll.py OUT.png` writes OUT.png when the screen is quiet.
- `measure.py IN.png` prints the editor line y centers, to point the hover.

The image adds the helper dependencies wayland-automation and pillow. A pinned `/prestage-in` (the extension work dir layout: `bin/`, `tools/`, `home/`) keeps the session off the network. The script copies it into the extension work dir and plants today's `update-` marker.

## Update the screenshots

To update the screenshots in the README, commit the new PNGs on top of the current commit of `refs/assets/screenshots`. Push that ref fast-forward only.

Point the README links at the new commit sha.
Do not commit PNGs to `main`.

## Exit status

The container kills Zed and Sway. An exit status of 137 or a kill message is usual. Read the logs, not the podman exit status.
