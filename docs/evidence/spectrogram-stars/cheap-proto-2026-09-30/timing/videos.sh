#!/bin/sh
# 15 s of the long take per variant, then a 2x2 of 1:1 centre crops.
cd /Users/yan/projects/harmonigraph/.claude/worktrees/stars-cheap-proto || exit 1
TAKE="/Users/yan/Music/Harmonigraph Takes/take-2026-09-22_15-35-06.take"
RON=docs/evidence/spectrogram-stars/cheap-proto-2026-09-30/appearance.ron
for v in off p1 p1x p2; do
  HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_OFFLINE_SPECTRAL_ONLY=1 HARMONIGRAPH_STARS_PROTO=$v \
    ./target/release/harmonigraph-offline "$TAKE" --appearance "$RON" --start 30 --end 45 --fps 60 \
    -s 1920x1080 -o /tmp/stars-rec/stars-$v.mp4 > /tmp/stars-rec/render-$v.log 2>&1 || echo "render $v failed"
done
cd /tmp/stars-rec || exit 1
ffmpeg -loglevel error -y -i stars-off.mp4 -i stars-p1.mp4 -i stars-p1x.mp4 -i stars-p2.mp4 -filter_complex \
  "[0:v]crop=960:540:480:270[a];[1:v]crop=960:540:480:270[b];[2:v]crop=960:540:480:270[c];[3:v]crop=960:540:480:270[d];\
[a][b][c][d]xstack=inputs=4:layout=0_0|w0_0|0_h0|w0_h0[v]" \
  -map "[v]" -c:v libx264 -crf 14 -pix_fmt yuv420p -preset slow stars-compare-crops.mp4
echo done
