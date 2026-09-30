#!/bin/sh
# Re-render every near-2x2 still with the current (exp -1.5) glow and rebuild both montages.

cd /Users/yan/projects/harmonigraph/.claude/worktrees/stars-cheap-proto
E=docs/evidence/spectrogram-stars/near-2x2-2026-09-30
rm -f /tmp/near2x2/*-000*.png
HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_OFFLINE_SPECTRAL_ONLY=1 HARMONIGRAPH_STARS_PROTO=off \
  ./target/release/harmonigraph-offline "/Users/yan/Music/Harmonigraph Takes/take-2026-09-22_15-35-06.take" \
  --appearance docs/evidence/spectrogram-stars/cheap-proto-2026-09-30/appearance.ron \
  --start 28 --end 40 --fps 1 -s 1920x1080 -o /tmp/near2x2/off.png > /dev/null 2>&1
sh $E/render.sh
for n in off n1 n2 n1-glow n2-glow n1-core05 n2-core05; do cp /tmp/near2x2/$n-00011.png $E/long-take-medium-$n.png; done
sh $E/montage.sh 1000 800 /tmp/near2x2/crops-2x.png
W=160 H=120 sh $E/montage.sh 1000 850 /tmp/near2x2/crops-2x-tight.png
cp /tmp/near2x2/crops-2x.png /tmp/near2x2/crops-2x-tight.png $E/
echo done
