# Headless environment for README screenshots: Xvfb, a PulseAudio null sink (real-time pacing so
# playback and the visualizer run, but no sound reaches any device) and ImageMagick.
FROM archlinux:base
RUN pacman -Syu --noconfirm --needed xorg-server-xvfb xorg-xwininfo imagemagick pulseaudio \
      pulseaudio-alsa alsa-lib ffmpeg libopenmpt freetype2 fontconfig libxkbcommon-x11 libxcursor \
      libxrandr libxi libx11 ttf-dejavu > /dev/null \
 && pacman -Scc --noconfirm > /dev/null
