# 4K visual comparison: P3 versus the 75% far-three variant

Requested after the 768x432 comparison looked very similar to the owner.
These are fresh 3840x2160 renders from the actual offline renderer, not enlarged copies of the earlier clips.
Both use the same take, appearance, clock interval and source binary.
The standard 4K export density is 3 pixels per logical point.

- `base-a-4k.mp4`: original P3, full frame.
- `group3-short75-4k.mp4`: complete farthest-three appearance rendered together at 75% dimensions, with shortened glow; the nearest two retain their current rendering.
- `comparison-4k-native-crops.mp4`: original left, candidate right.
  Both panels show the identical 1920x2100 crop from their respective 4K frames, at one source pixel per output pixel, below a 60-pixel label strip.
  Crop origin is (960, 30); no resizing is used.
- The `*-frame72.png` files are lossless stills at the middle of the passage.

The renderer processes 46 seconds from take time 46.766945 to 92.766945 at 24 fps.
A FIFO consumer discards the first 40 seconds while retaining normal rendering and history updates.
The final 144 frames form each six-second clip.
Viewing copies use H.264 CRF10 with 4:2:0 chroma for player compatibility.
The PNG stills retain the original RGB values.
`comparison.json` records encoding, dimensions, frame counts, whole-frame differences and movie hashes.
These descriptive differences do not decide whether the visual change is acceptable.

To reproduce, follow the parent README to apply the research patch and build both release packages.
Set `RESEARCH_WORKTREE` and `TAKE_FILE`, then run `render_4k.py` followed by `encode_4k.py`.
The helpers expect the parent directory to contain the preserved shaders, appearance file and PNG helper.
The raw RGB intermediates consume about 7.2 GB combined; they can be removed after the viewing copies and lossless stills have been verified.
The warmup stream is discarded as it arrives instead of consuming another 60 GB of disk.
