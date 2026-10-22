#!/bin/sh
# Starts the kappa.yashi saver after N idle minutes on an X11 session (needs xprintidle: sudo apt install xprintidle).
# Wayland/Hyprland: use hypridle instead (see README).
MINUTES="${1:-5}"
LIMIT=$((MINUTES * 60 * 1000))
BIN="$(dirname "$0")/kappayashi-saver"
while sleep 10; do
  if [ "$(xprintidle)" -ge "$LIMIT" ] && ! pgrep -x kappayashi-saver >/dev/null 2>&1; then
    "$BIN" --saver
  fi
done
