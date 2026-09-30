#!/bin/sh
# Near-2x2 stills: Medium (the appearance's fresh profile), long take, last frame of 28..40 s.
cd /Users/yan/projects/harmonigraph/.claude/worktrees/stars-cheap-proto || exit 1
r() {
  env HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_OFFLINE_SPECTRAL_ONLY=1 HARMONIGRAPH_STARS_PROTO=$1 $3 \
    ./target/release/harmonigraph-offline "/Users/yan/Music/Harmonigraph Takes/take-2026-09-22_15-35-06.take" \
    --appearance docs/evidence/spectrogram-stars/cheap-proto-2026-09-30/appearance.ron \
    --start 28 --end 40 --fps 1 -s 1920x1080 -o /tmp/near2x2/$2.png 2>&1 | grep -E "^done|error|panic"
}
r n1 n1
r n2 n2
r n1 n1-glow HARMONIGRAPH_STARS_NEAR_CORE=1.0
r n2 n2-glow HARMONIGRAPH_STARS_NEAR_CORE=1.0
r n1 n1-core05 HARMONIGRAPH_STARS_NEAR_CORE=0.5
r n2 n2-core05 HARMONIGRAPH_STARS_NEAR_CORE=0.5
