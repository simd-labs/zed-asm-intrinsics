#!/bin/sh
# run-install.sh [FILE [OUT.png [WAIT_SECS]]]
# Install check. Run Zed with this extension as a dev extension, no simdref
# on PATH, network on. The extension downloads uv, runs "uv tool install
# simdref", then "isa update". Needs extension.wasm in the repo root:
#   cargo build --release --target wasm32-wasip2
#   cp target/wasm32-wasip2/release/zed_simdref.wasm extension.wasm
# Exit 0 if Zed.log shows the language server starting from the work dir, the
# catalog is in the work dir, and every new path in $HOME outside the Zed data
# dir is on the allowlist below.
# EXT_DIR (default: the repo root) holds extension.toml and extension.wasm.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
FILE=$(realpath "${1:-$HERE/ws/demo.s}")
OUT=${2:-$HERE/out/install.png}
export WAIT_SECS=${3:-600}

EXT=${EXT_DIR:-$ROOT}
[ -f "$EXT/extension.wasm" ] || { echo "missing $EXT/extension.wasm, build it first" >&2; exit 2; }
ID=$(sed -n 's/^id = "\(.*\)"$/\1/p' "$EXT/extension.toml")
[ -n "$ID" ] || { echo "no id in $EXT/extension.toml" >&2; exit 2; }

# Zed reads the extension as <id>, so stage the two files under that name.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir "$STAGE/$ID"
cp "$EXT/extension.toml" "$EXT/extension.wasm" "$STAGE/$ID/"

sh "$HERE/shot.sh" "$(dirname "$FILE")" "$FILE" "$OUT" "$STAGE/$ID" || true

grep -E "starting language server process.*extensions/work/$ID/bin/simdref-lsp" "${OUT%.png}.Zed.log"

# The extension must write only inside its work dir. Every other new path in
# $HOME must come from Zed, mesa, fontconfig or the assembly extension.
OUTSIDE="${OUT%.png}.outside.txt"
[ -s "$OUTSIDE" ] || { echo "FAIL: $OUTSIDE missing or empty, the write check did not run" >&2; exit 1; }
ALLOWED='^/home/shot(/\.cache(/(zed|mesa_shader_cache|fontconfig)(/.*)?)?|/\.config(/asm-lsp(/.*)?)?)?$'
if grep -vE "$ALLOWED" "$OUTSIDE"; then
  echo "FAIL: the paths above are new in \$HOME outside the Zed data dir" >&2
  exit 1
fi
echo "OK: all $(wc -l <"$OUTSIDE") new paths in \$HOME outside the Zed data dir are on the Zed, mesa, fontconfig and asm-lsp allowlist"

grep -E "extensions/work/$ID/home/\.local/share/simdref$" "${OUT%.png}.workdir.txt"
echo "OK: isa update wrote the catalog dir inside the work dir"
