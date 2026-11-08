#!/bin/sh
# in-container.sh MODE WAIT SETTINGS FILE...
# MODE shot: open the files, wait, one grim, copy the logs, kill everything.
# MODE session: open the files and keep sway and Zed running. The host
# moves the pointer and captures the PNG with podman exec mouse.py and
# poll.py. End the session with podman rm -f.
set -eu

MODE="$1"
WAIT="$2"
SETTINGS="$3"
shift 3

# Set the runtime dir so the host's podman exec commands find the wayland
# socket without the script pid.
export XDG_RUNTIME_DIR=/tmp/xdg-shot
rm -rf "$XDG_RUNTIME_DIR"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

export WLR_BACKENDS=headless
export WLR_RENDERER=pixman
export WLR_LIBINPUT_NO_DEVICES=1
export LIBGL_ALWAYS_SOFTWARE=1
export ZED_ALLOW_EMULATED_GPU=1
# mesa vulkan software
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export WAYLAND_DISPLAY=wayland-1

# A new user data dir on each launch. Zed session restore reopens
# in-memory buffers, so a used dir re-types earlier text into each shot.
UD=/home/shot/zed-user-data
rm -rf "$UD"
mkdir -p "$UD/config"
cp "$SETTINGS" "$UD/config/settings.json"

# prestage dev extensions into user data dir
for d in /dev-ext-in/*; do
  [ -d "$d" ] || continue
  name=$(basename "$d")
  target=$(readlink -f "$d")
  # Remove the install dir first, or the relink does not work when the
  # dir is on the read-only dev directory (EROFS).
  rm -rf "$UD/extensions/installed/$name"
  mkdir -p "$UD/extensions/installed"
  ln -s "$target" "$UD/extensions/installed/$name"
done

# The mounted /prestage-in has the pinned simdref install (the extension
# work dir layout: bin/, tools/, home/). Copy it in and plant today's
# update marker, so the session starts the server without a download and
# the daily upgrade check is a no-op.
if [ -d /prestage-in ]; then
  WORK="$UD/extensions/work/asm-intrinsics-lsp"
  mkdir -p "$WORK"
  cp -a /prestage-in/. "$WORK/"
  touch "$WORK/update-$(( $(date +%s) / 86400 ))"
fi

# Orphaned wayland sockets from an interrupted start break new sessions.
rm -f "$XDG_RUNTIME_DIR"/wayland-*
sway --unsupported-gpu >/shot/sway.log 2>&1 &
SWAY_PID=$!

i=0
while [ $i -lt 50 ]; do
  if [ -S "$XDG_RUNTIME_DIR/wayland-1" ]; then break; fi
  i=$((i+1)); sleep 0.1
done
[ -S "$XDG_RUNTIME_DIR/wayland-1" ] || { echo "sway socket missing"; cat /shot/sway.log; exit 3; }

for f in "$@"; do
  mkdir -p "$(dirname "$f")"
done
touch /tmp/zed-start
# setsid keeps Zed alive after the launch call. zed.log is the debug record.
setsid /opt/zed.app/libexec/zed-editor --user-data-dir "$UD" "$@" >/shot/zed.log 2>&1 < /dev/null &
ZED_PID=$!

if [ "$MODE" = session ]; then
  echo "started zed with $*"
  # Keep the container alive. The host kills it to end the session.
  tail -f /shot/zed.log
fi

sleep "$WAIT"

grim /shot/shot.png
rc=$?
find "$UD/extensions" -maxdepth 3 >/shot/extensions.txt 2>&1 || true
find "$UD/extensions/work" -maxdepth 5 >/shot/workdir.txt 2>&1 || true
{ ls -la /home/shot/.cache/uv /home/shot/.local/share/uv; } >/shot/home-uv.txt 2>&1 || true
cp "$UD/extensions/index.json" /shot/index.json 2>/dev/null || true
cp "$UD/logs/Zed.log" /shot/Zed.log 2>/dev/null || true
find "$UD/extensions/work" -name install.log -exec sh -c 'cp "$1" "/shot/install.$(basename $(dirname $1)).log"' _ {} \; 2>/dev/null || true

kill $ZED_PID 2>/dev/null || true
kill $SWAY_PID 2>/dev/null || true
wait 2>/dev/null || true
# Every path in $HOME that changed while Zed ran, outside the Zed data dir.
find /home/shot -newer /tmp/zed-start ! -path "$UD" ! -path "$UD/*" >/shot/outside.txt 2>&1 || true
exit $rc
