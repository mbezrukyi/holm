#!/usr/bin/env bash
# Runs desktop apps on a hidden Wayland display, and drives it over VNC.
set -euo pipefail

name="${LOOK_NAME:-main}"
# A Unix socket path must be short, so the files stay in the runtime folder.
dir="${XDG_RUNTIME_DIR:?}/look-$name"

usage() {
  cat >&2 <<'EOF'
Usage: look.sh <command>
  start [WIDTHxHEIGHT]   start the hidden display (default 1200x800)
  run <command...>       run an app on it, and wait for its window
  do <vncdo args...>     send keys and clicks, take screenshots
  stop                   stop the app and the display
EOF
  exit 2
}

need() {
  command -v "$1" >/dev/null || { echo "look: $1 is missing. Install it." >&2; exit 1; }
}

alive() {
  [ -f "$dir/$1.pid" ] && kill -0 "$(cat "$dir/$1.pid")" 2>/dev/null
}

use_display() {
  alive sway || { echo "look: no display. Run: look.sh start" >&2; exit 1; }
  WAYLAND_DISPLAY="$(cat "$dir/display")"
  SWAYSOCK="$(cat "$dir/swaysock")"
  export WAYLAND_DISPLAY SWAYSOCK
  unset DISPLAY
}

wait_for() {
  for _ in $(seq 50); do
    [ -e "$1" ] && [ ! -f "$1" -o -s "$1" ] && return 0
    sleep 0.1
  done
  return 1
}

case "${1:-}" in
  start)
    need sway; need swaymsg; need wayvnc; need vncdo
    if alive sway; then echo "look: $name is already started" >&2; exit 1; fi
    rm -rf "$dir"
    mkdir -p "$dir"
    # Sway gives its own socket names to the commands it runs.
    cat > "$dir/sway.conf" <<EOF
output * resolution ${2:-1200x800}
default_border none
exec echo "\$SWAYSOCK" > "$dir/swaysock"
exec echo "\$WAYLAND_DISPLAY" > "$dir/display"
EOF
    env -u DISPLAY -u WAYLAND_DISPLAY -u SWAYSOCK \
      WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
      sway -c "$dir/sway.conf" > "$dir/sway.log" 2>&1 &
    echo $! > "$dir/sway.pid"
    if ! wait_for "$dir/display" || ! wait_for "$dir/swaysock"; then
      cat "$dir/sway.log" >&2; echo "look: sway did not start" >&2; exit 1
    fi
    use_display
    wayvnc --unix-socket --socket="$dir/control.sock" "$dir/vnc.sock" > "$dir/wayvnc.log" 2>&1 &
    echo $! > "$dir/wayvnc.pid"
    if ! wait_for "$dir/vnc.sock"; then
      cat "$dir/wayvnc.log" >&2; echo "look: wayvnc did not start" >&2; exit 1
    fi
    # wayvnc adds its keyboard to the seat only while a client is connected.
    # An app misses the first keys of a new client. This idle client keeps the keyboard there.
    vncdo -s "$dir/vnc.sock" pause 100000000 > /dev/null 2>&1 &
    echo $! > "$dir/idle.pid"
    sleep 0.5
    echo "look: $name is started"
    ;;
  run)
    shift; [ $# -gt 0 ] || usage
    use_display
    "$@" > "$dir/app.log" 2>&1 &
    echo $! > "$dir/app.pid"
    for _ in $(seq 300); do
      if swaymsg -t get_tree | grep -q '"type": "con"'; then
        sleep 0.5; echo "look: the window is open"; exit 0
      fi
      alive app || { cat "$dir/app.log" >&2; echo "look: the app ended" >&2; exit 1; }
      sleep 0.1
    done
    echo "look: no window after 30 s. See $dir/app.log" >&2
    exit 1
    ;;
  do)
    shift; [ $# -gt 0 ] || usage
    alive wayvnc || { echo "look: no display. Run: look.sh start" >&2; exit 1; }
    vncdo -s "$dir/vnc.sock" "$@"
    ;;
  stop)
    for process in app idle wayvnc sway; do
      alive "$process" && kill "$(cat "$dir/$process.pid")"
    done
    rm -rf "$dir"
    echo "look: stopped"
    ;;
  *) usage ;;
esac
