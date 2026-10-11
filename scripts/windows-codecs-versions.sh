# vcpkg release and ports of the Windows codec libraries (sourced by windows-codecs.sh).
# 2026.07.29: FFmpeg 8.1.2, libopenmpt 0.8.7.
VCPKG_TAG=2026.07.29
# The ports built (the cache key of the vcpkg binary cache covers this file only).
VCPKG_PORTS=("ffmpeg[core,avcodec,avformat,swresample,swscale,zlib]" libopenmpt)
