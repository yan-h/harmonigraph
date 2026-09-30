#!/bin/sh
# Timing first (quiet GPU), then videos: 15 s of the long take, rendered from
# 12 s earlier so the spectrogram history fills, trimmed; plus a 2x2 of 1:1 crops.
cd /Users/yan/projects/harmonigraph/.claude/worktrees/stars-cheap-proto || exit 1
sh docs/evidence/spectrogram-stars/near-2x2-2026-09-30/timing.sh
cargo build --release -p harmonigraph-offline > /tmp/near2x2/build.log 2>&1 || { echo "build failed"; exit 1; }
TAKE="/Users/yan/Music/Harmonigraph Takes/take-2026-09-22_15-35-06.take"
RON=docs/evidence/spectrogram-stars/cheap-proto-2026-09-30/appearance.ron
v() {
  env HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_OFFLINE_SPECTRAL_ONLY=1 HARMONIGRAPH_STARS_PROTO=$1 $3 \
    ./target/release/harmonigraph-offline "$TAKE" --appearance "$RON" --start 18 --end 45 --fps 60 \
    -s 1920x1080 -o /tmp/near2x2/raw-$2.mp4 > /tmp/near2x2/render-$2.log 2>&1 || echo "render $2 failed"
  ffmpeg -loglevel error -y -ss 12 -i /tmp/near2x2/raw-$2.mp4 -an -c:v libx264 -crf 18 -pix_fmt yuv420p /tmp/near2x2/near-$2.mp4
  rm -f /tmp/near2x2/raw-$2.mp4
}
v off off
v n2 n2
v n2 n2-glow HARMONIGRAPH_STARS_NEAR_CORE=1.0
v n2 n2-core05 HARMONIGRAPH_STARS_NEAR_CORE=0.5
cd /tmp/near2x2 || exit 1
ffmpeg -loglevel error -y -i near-off.mp4 -i near-n2.mp4 -i near-n2-glow.mp4 -i near-n2-core05.mp4 -filter_complex \
  "[0:v]crop=960:540:480:270[a];[1:v]crop=960:540:480:270[b];[2:v]crop=960:540:480:270[c];[3:v]crop=960:540:480:270[d];\
[a][b][c][d]xstack=inputs=4:layout=0_0|w0_0|0_h0|w0_h0[v]" \
  -map "[v]" -c:v libx264 -crf 24 -pix_fmt yuv420p -preset slow near-compare-crops.mp4
ffmpeg -loglevel error -y -ss 8 -i near-compare-crops.mp4 -frames:v 1 near-compare-8s.png
echo done
