#!/usr/bin/env bash
# Linux only: run the GPUI client under a virtual X display and save a screenshot.
# For agents and CI without a screen. Needs Xvfb, ImageMagick (`import`) and a
# Vulkan or GL driver (Mesa llvmpipe works; see client/PORTING.md).
#
# usage: client/scripts/headless-shot.sh PROJECT_DIR [RELATIVE_FILE] [OUT.png] [WAIT_SECONDS]
set -euo pipefail

project=${1:?usage: headless-shot.sh PROJECT_DIR [RELATIVE_FILE] [OUT.png] [WAIT_SECONDS]}
file=${2:-}
out=${3:-/tmp/openidea-gpui.png}
wait=${4:-20}
here=$(cd "$(dirname "$0")/.." && pwd)
display=${DISPLAY_NUM:-:99}

if ! xdpyinfo -display "$display" >/dev/null 2>&1; then
  Xvfb "$display" -screen 0 1400x900x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
  sleep 2
fi

for pid in $(pgrep -x openidea-gpui || true); do kill "$pid"; done
sleep 1

export DISPLAY=$display
unset WAYLAND_DISPLAY
export OPENIDEA_LOG=${OPENIDEA_LOG:-info,wgpu=warn,naga=warn}
args=("$project")
[ -n "$file" ] && args+=(--open "$file")
nohup "$here/target/debug/openidea-gpui" "${args[@]}" >/tmp/openidea-gpui.log 2>&1 &
sleep "$wait"
import -window root "$out"
echo "$out (log: /tmp/openidea-gpui.log)"
