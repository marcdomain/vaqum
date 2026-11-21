#!/usr/bin/env bash
# Renders demo.gif: records each demo/scenes/*.tape with VHS, puts its
# numbered caption underneath, and joins them into one GIF.
#
#   ./demo/render.sh
#
# Needs: vhs, ffmpeg, Node.js, and Google Chrome or Chromium (captions and
# the browser scene; override with CHROME=/path/to/chrome). The browser scene
# also needs the viewer built: see web/README.md.
#
# To change the demo, edit a scene file: `.tape` scenes are ordinary VHS
# tape commands; `.web` scenes record the browser viewer opening the
# `# archive:` with the `# password:`. The `# caption:` line is the text
# shown under each (HTML allowed, <b> for the highlighted word). Scenes run
# in file-name order and share one sandbox, so later scenes can use earlier
# output.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="/tmp/vaqum-demo-render"
OUT="$ROOT/demo.gif"
WIDTH=960        # must match `Set Width` in demo/common.tape
CAPTION_H=76
BG="#EEF0FB"     # must match `Set MarginFill`
FPS=24

for tool in vhs ffmpeg node; do
  command -v "$tool" >/dev/null || { echo "render.sh: $tool is required" >&2; exit 1; }
done
if [ -z "${CHROME:-}" ]; then
  for c in "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "/Applications/Chromium.app/Contents/MacOS/Chromium" \
    google-chrome google-chrome-stable chromium chromium-browser; do
    if [ -x "$c" ] || command -v "$c" >/dev/null 2>&1; then CHROME="$c"; break; fi
  done
fi
[ -n "${CHROME:-}" ] || { echo "render.sh: Chrome/Chromium not found; set CHROME=/path/to/chrome" >&2; exit 1; }

echo "==> building vaqum (release)"
cargo build --release --quiet
export VAQUM_DEMO_BIN="$ROOT/target/release"

echo "==> preparing sandbox"
bash demo/setup.sh
rm -rf "$WORK"
mkdir -p "$WORK"

caption_png() { # $1 = number, $2 = caption html, $3 = output png
  cat > "$WORK/caption.html" <<EOF
<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;width:${WIDTH}px;height:${CAPTION_H}px;background:${BG};overflow:hidden}
body{display:flex;align-items:center;justify-content:center;gap:12px;padding-bottom:10px;box-sizing:border-box;
  font:500 18px -apple-system,"SF Pro Text",Inter,"Segoe UI",Helvetica,Arial,sans-serif;color:#3b3f5c;-webkit-font-smoothing:antialiased}
.n{width:28px;height:28px;border-radius:50%;background:#5b4ee8;color:#fff;display:grid;place-items:center;font-size:14px;font-weight:700}
b{color:#4338ca;font-weight:700}
</style></head><body><span class="n">$1</span><span>$2</span></body></html>
EOF
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size="${WIDTH},${CAPTION_H}" --screenshot="$3" "file://$WORK/caption.html" >/dev/null 2>&1
  [ -s "$3" ] || { echo "render.sh: failed to render caption $1" >&2; exit 1; }
}

if [ ! -f web/site/vaqum-viewer.wasm ]; then
  echo "render.sh: web/site/vaqum-viewer.wasm is missing; build the viewer first (web/README.md)" >&2
  exit 1
fi

parts=()
n=0
for tape in demo/scenes/*.tape demo/scenes/*.web; do
  [ -e "$tape" ] || continue
  n=$((n + 1))
  name="$(basename "${tape%.*}")"
  caption="$(sed -n 's/^# caption: *//p' "$tape" | head -n 1)"
  echo "==> scene $n: $name"

  if [ "${tape##*.}" = web ]; then
    archive="$(sed -n 's/^# archive: *//p' "$tape" | head -n 1)"
    password="$(sed -n 's/^# password: *//p' "$tape" | head -n 1)"
    node demo/web-scene.mjs "$CHROME" web/site "/tmp/vaqum-demo/$archive" "$password" "$WORK/$name" >/dev/null
    ffmpeg -v error -y -f concat -safe 0 -i "$WORK/$name/frames.txt" \
      -vf "fps=$FPS,format=yuv420p" -c:v libx264 -crf 12 -preset veryfast "$WORK/$name.mp4"
    caption_png "$n" "$caption" "$WORK/$name.png"
    ffmpeg -v error -y -i "$WORK/$name.mp4" -loop 1 -i "$WORK/$name.png" \
      -filter_complex "[1:v]format=yuv420p[c];[0:v]fps=$FPS,format=yuv420p[v];[v][c]vstack=shortest=1" \
      -c:v libx264 -crf 12 -preset veryfast -pix_fmt yuv420p "$WORK/$name-captioned.mp4"
    parts+=("$WORK/$name-captioned.mp4")
    continue
  fi

  # Full tape = shared settings + output path + the scene's own commands
  # (VHS only honours `Set` lines at the very top of a tape).
  {
    echo "Output \"$WORK/$name.mp4\""
    cat demo/common.tape
    echo
    cat "$tape"
  } > "$WORK/$name.tape"
  vhs -q "$WORK/$name.tape"
  caption_png "$n" "$caption" "$WORK/$name.png"
  # Recording on top, caption strip below.
  ffmpeg -v error -y -i "$WORK/$name.mp4" -loop 1 -i "$WORK/$name.png" \
    -filter_complex "[1:v]format=yuv420p[c];[0:v]fps=$FPS,format=yuv420p[v];[v][c]vstack=shortest=1" \
    -c:v libx264 -crf 12 -preset veryfast -pix_fmt yuv420p "$WORK/$name-captioned.mp4"
  parts+=("$WORK/$name-captioned.mp4")
done

echo "==> joining ${#parts[@]} scenes"
: > "$WORK/list.txt"
for p in "${parts[@]}"; do echo "file '$p'" >> "$WORK/list.txt"; done
ffmpeg -v error -y -f concat -safe 0 -i "$WORK/list.txt" -c copy "$WORK/all.mp4"

echo "==> encoding demo.gif"
ffmpeg -v error -y -i "$WORK/all.mp4" \
  -vf "fps=$FPS,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
  "$OUT"

# The big sample file is only needed while recording.
rm -f /tmp/vaqum-demo/server-logs.txt /tmp/vaqum-demo/server-logs.txt.vqm

echo "==> wrote $OUT ($(du -h "$OUT" | cut -f1))"
