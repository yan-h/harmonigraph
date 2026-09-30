#!/bin/sh
# Paired near-2x2 timings: production / n1 / n2 at High and Medium interleaved
# in one process, 1080p@2 and 4K@4, each twice (ABAB).
cd /Users/yan/projects/harmonigraph/.claude/worktrees/stars-cheap-proto || exit 1
mkdir -p /tmp/near2x2/timing
run() {
  HARMONIGRAPH_SHADER_ASSETS=source PROBE_CASE="stars-near,blur only" PROBE_FRAMES=240 PROBE_FILLS=1 \
    PROBE_SIZE=$1 PROBE_PPP=$2 \
    cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial -- --ignored --nocapture --test-threads=1 \
    > /tmp/near2x2/timing/$1-$3.log 2>&1
}
run 1920x1080 2 a
run 3840x2160 4 a
run 1920x1080 2 b
run 3840x2160 4 b
echo done
