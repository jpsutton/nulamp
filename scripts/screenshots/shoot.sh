#!/bin/bash
# Runs inside the screenshot container (see take.sh). /work/music holds the audio to show,
# /work/out receives the images.
set -u
Xvfb :5 -screen 0 1280x1024x24 -nolisten tcp > /dev/null 2>&1 &
export DISPLAY=:5
sleep 1
pulseaudio -n --daemonize=yes --exit-idle-time=-1 --load="module-null-sink sink_name=null" \
  --load="module-native-protocol-unix" > /dev/null 2>&1
sleep 1
pactl set-default-sink null
mkdir -p /tmp/cfg
cat > /tmp/cfg/nulamp.ini <<INI
[nulamp]
library_folders=1|/work/music
library_rescan_on_start=1
eq_enabled=1
eq_bands=24,20,26,31,35,38,36,30,24,20
eq_preamp=28
pl_height=145
autoscroll=0
lib_width=400
lib_height=232
lib_tree_width=118
INI
# No window manager under Xvfb: find top-level windows by name.
win() { xwininfo -root -tree 2> /dev/null | grep -F -- "$1" | awk '{print $1}' | head -1; }
shot() { [ -n "$1" ] && timeout 5 import -window "$1" "$2"; }
NULAMP_CONFIG_DIR=/tmp/cfg nulamp /NEW /work/music > /dev/null 2>&1 &
sleep 14
main=$(win '- nulamp"')
# The busiest of a few frames makes the best visualizer picture.
best=0
for n in $(seq 8); do
  shot "$main" "/tmp/main$n.png"
  m=$(magick "/tmp/main$n.png" -crop 76x16+24+43 +repage -format '%[fx:mean*1000000]' info: | cut -d. -f1)
  if [ "$m" -gt "$best" ]; then best=$m; cp "/tmp/main$n.png" /tmp/main.png; fi
  sleep 0.3
done
shot "$(win 'nulamp Equalizer')" /tmp/eq.png
shot "$(win 'Playlist Editor')" /tmp/pl.png
kill %2
sleep 1
NULAMP_OPEN_DIALOG=library NULAMP_CONFIG_DIR=/tmp/cfg nulamp /NEW > /dev/null 2>&1 &
sleep 8
shot "$(win 'Media Library')" /tmp/library.png
magick /tmp/main.png /tmp/eq.png /tmp/pl.png -append -filter point -resize 200% -strip /work/out/player.png
magick /tmp/library.png -filter point -resize 200% -strip /work/out/library.png
