#!/bin/sh
# 2x nearest-neighbour crops, two rows:
#   off | n1 | n1 glow core 1.0 | n1 core 0.5
#   off | n2 | n2 glow core 1.0 | n2 core 0.5
cd /tmp/near2x2 || exit 1
X=${1:-1000}; Y=${2:-800}; W=${W:-320}; H=${H:-240}
OUT=${3:-montage.png}
FONT=/System/Library/Fonts/Supplemental/Arial.ttf
tile() { echo "[$1:v]crop=$W:$H:$X:$Y,scale=iw*2:ih*2:flags=neighbor,drawtext=fontfile=$FONT:text='$2':x=8:y=8:fontsize=26:fontcolor=white:box=1:boxcolor=black@0.6[$3]"; }
ffmpeg -loglevel error -y \
  -i off-00011.png -i n1-00011.png -i n1-glow-00011.png -i n1-core05-00011.png \
  -i n2-00011.png -i n2-glow-00011.png -i n2-core05-00011.png \
  -filter_complex "\
$(tile 0 off a0);$(tile 1 n1 a1);$(tile 2 'n1 glow core 1.0' a2);$(tile 3 'n1 glow core 0.5' a3);\
$(tile 0 off b0);$(tile 4 n2 b1);$(tile 5 'n2 glow core 1.0' b2);$(tile 6 'n2 glow core 0.5' b3);\
[a0][a1][a2][a3]hstack=4[ra];[b0][b1][b2][b3]hstack=4[rb];[ra][rb]vstack=2" \
  -frames:v 1 "$OUT"
