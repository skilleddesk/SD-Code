#!/usr/bin/env bash
# The frameless window on Linux, driven with a real mouse (0.22) - run inside `xvfb-run` by window-check.yml.
#
# A window manager (openbox) runs, the real app starts, and xdotool presses the title bar's own buttons and drags
# its caption the way a person would. Then the app's self-test (SDC_WINDOW_SELFTEST) checks the window API.
# Results: out/real-input.txt and out/report.json, screenshots in out/*.png, the window tree in out/windows.txt.
set -u

out="$1"
app="$2"
mkdir -p "$out"

openbox >/dev/null 2>&1 &
sleep 2

SDC_WINDOW_SELFTEST="$out/report.json" SDC_WINDOW_SELFTEST_WAIT=60 "$app" >"$out/app.log" 2>&1 &
app_pid=$!

result() { echo "$1" | tee -a "$out/real-input.txt"; }

shot() { xwd -root -silent | convert xwd:- "$out/$1" 2>/dev/null || import -window root "$out/$1"; }

# GTK makes small helper windows with the same title; the app's window is the largest visible one.
pick() {
  best=""
  best_area=0
  for id in $(xdotool search --onlyvisible --name 'SDC' 2>/dev/null); do
    eval "$(xdotool getwindowgeometry --shell "$id")"
    area=$((WIDTH * HEIGHT))
    if [ "$area" -gt "$best_area" ]; then
      best=$id
      best_area=$area
    fi
  done
  echo "$best"
}

wid=""
for _ in $(seq 1 60); do
  wid=$(pick)
  if [ -n "$wid" ]; then
    eval "$(xdotool getwindowgeometry --shell "$wid")"
    if [ "$WIDTH" -gt 200 ]; then break; fi
  fi
  sleep 1
done
xwininfo -root -tree > "$out/windows.txt" 2>&1 || true

if [ -z "$wid" ]; then
  result "FAIL no SDC window appeared"
  shot no-window.png || true
  wait "$app_pid"
  exit 1
fi

sleep 12
shot 1-start.png

geometry() { eval "$(xdotool getwindowgeometry --shell "$wid")"; }

geometry
result "start: x=$X y=$Y w=$WIDTH h=$HEIGHT (window $wid)"
start_w=$WIDTH

# The maximise button: the middle of the three 46 px buttons at the right end of the 48 px title bar.
xdotool mousemove --sync $((X + WIDTH - 69)) $((Y + 24)) click 1
sleep 3
geometry
shot 2-maximised.png
if [ "$WIDTH" -gt "$start_w" ]; then result "PASS maximise button: w=$WIDTH"; else result "FAIL maximise button: w=$WIDTH"; fi

# The same button again restores.
xdotool mousemove --sync $((X + WIDTH - 69)) $((Y + 24)) click 1
sleep 3
geometry
if [ "$WIDTH" -eq "$start_w" ]; then result "PASS restore button: w=$WIDTH"; else result "FAIL restore button: w=$WIDTH"; fi

# Drag the window by an empty stretch of the title bar.
before_x=$X
xdotool mousemove --sync $((X + 300)) $((Y + 24)) mousedown 1
for step in $(seq 1 10); do
  xdotool mousemove --sync $((X + 300 + step * 15)) $((Y + 24 + step * 6))
  sleep 0.05
done
xdotool mouseup 1
sleep 2
geometry
shot 3-dragged.png
if [ "$X" -ne "$before_x" ]; then result "PASS drag by the title bar: x $before_x -> $X"; else result "FAIL drag by the title bar: x stayed $X"; fi

# A double-click on the caption maximises; a second one restores.
xdotool mousemove --sync $((X + 300)) $((Y + 24)) click --repeat 2 --delay 80 1
sleep 3
geometry
if [ "$WIDTH" -gt "$start_w" ]; then result "PASS double-click maximises: w=$WIDTH"; else result "FAIL double-click maximises: w=$WIDTH"; fi
xdotool mousemove --sync $((X + 300)) $((Y + 24)) click --repeat 2 --delay 80 1
sleep 2

# The minimise button.
geometry
xdotool mousemove --sync $((X + WIDTH - 115)) $((Y + 24)) click 1
sleep 2
state=$(xprop -id "$wid" _NET_WM_STATE 2>/dev/null)
if echo "$state" | grep -q HIDDEN; then result "PASS minimise button"; else result "FAIL minimise button: $state"; fi
xdotool windowactivate "$wid" 2>/dev/null || xdotool windowmap "$wid"
sleep 2

# Now the app's own self-test (it waits SDC_WINDOW_SELFTEST_WAIT seconds from start), then it quits.
wait "$app_pid"
shot 4-end.png || true

if grep -q FAIL "$out/real-input.txt"; then exit 1; fi
exit 0
