# Historical change inventory

Read-only inventory for the [change-cost audit](../../historical-change-cost.md).
Inspected on 2026-09-19.
The 100 most recent first-parent changes ending at `8b4edf4ea63669c6baf55f1878d917470b0ce4fa`,
from `723f9f2d6adb401a7b572eae16ea263ac461bad8` inclusive,
span 2026-09-09 through 2026-09-19.
The ending revision was also the remote main revision when checked.
Each row compares a commit to its first parent;
a merged PR counts once even when it contains multiple commits.
The rows include feature work,
fixes,
refactors,
tests and documentation;
they are not a sample consisting only of bugs or only of ordinary changes.

Counts use `git diff --numstat --find-renames=50% PARENT COMMIT`.
“All files” includes binary/generated files and recognizes a detected rename as one entry.
“Crate files” selects paths under `crates/` ending in `.rs` or `.wgsl`,
including test code and comments.
“Crate +/−” sums inserted and deleted text lines in those paths;
it is churn rather than net growth.
It excludes browser JavaScript,
vendor code,
fixtures,
build tooling and generated Metal sources,
which may still carry real maintenance obligations.
Moves between several files can count the same lines again even with rename detection.
These counts are locators for semantic review,
not estimates of time,
quota,
complexity or defect rates.

Reproduce the population and inspect any row with:

```sh
git rev-list --first-parent -n 100 8b4edf4ea63669c6baf55f1878d917470b0ce4fa
git diff --numstat --find-renames=50% e57fd575^1 e57fd575
git show e57fd575 -- crates/harmonigraph-render/src/shaders/spectrogram.wgsl
```

| Change | Subject | All files | Crate files | Crate +/− |
| --- | --- | ---: | ---: | ---: |
| [`8b4edf4e`](https://github.com/yan-h/harmonigraph/commit/8b4edf4ea63669c6baf55f1878d917470b0ce4fa) | Let a stale Metal corpus go red locally, instead of printing nothing (#965) | 3 | 1 | 9 |
| [`129b7503`](https://github.com/yan-h/harmonigraph/commit/129b7503bf9bb3d87e4a4e2312ec04fdd9a3bda5) | Carry every declared setting into the parity replay, and reach the unsnapped branch (#956) | 6 | 1 | 28 |
| [`188a171f`](https://github.com/yan-h/harmonigraph/commit/188a171f592f927de8e13d2c732d06831ded9328) | Split view.rs into the six shapes it was already holding apart (#958) | 7 | 7 | 1883 |
| [`1786c5ba`](https://github.com/yan-h/harmonigraph/commit/1786c5ba1a4e2994c20dc2672eb7d200fd605d9b) | Trade the outage bookkeeping back for the three lines that do the work (#953) | 1 | 1 | 216 |
| [`d28b58e5`](https://github.com/yan-h/harmonigraph/commit/d28b58e55917303fe1cbf0d3dda368086233a8da) | Stop the heatmap where the data stops, not at the now-line (#948) | 12 | 7 | 301 |
| [`c5aad48c`](https://github.com/yan-h/harmonigraph/commit/c5aad48cecb9e2402be72a5afa000fa0491fab4e) | Let a resuming baseline withdraw the release its outage handed out (#946) | 1 | 1 | 329 |
| [`9124f12b`](https://github.com/yan-h/harmonigraph/commit/9124f12b9085757f2e1254f9b995188108db74a5) | Document the long-term maintainability plan (#949) | 2 | 0 | 0 |
| [`e57fd575`](https://github.com/yan-h/harmonigraph/commit/e57fd5754833fc0eee1170edb279d7cd89fe951c) | Let the wash keep the one base cell in 1024 its hash lands on the edge (#945) | 15 | 1 | 11 |
| [`34718d41`](https://github.com/yan-h/harmonigraph/commit/34718d418397eb38e0e2260163f5bf9ce921f232) | Let the picture refuse a number it cannot draw, instead of shipping it (#944) | 6 | 6 | 600 |
| [`37157f9d`](https://github.com/yan-h/harmonigraph/commit/37157f9d97381d91815c1aff8f9d4fac6fb672e5) | Key the audio thread's map bank on the revision that already exists (#942) | 3 | 3 | 90 |
| [`83dffb75`](https://github.com/yan-h/harmonigraph/commit/83dffb75e3301d8e54ac5deb1a857286c29a8787) | Stop running harmonigraph-render's tests twice, and let a merge be asked for before it is green (#941) | 2 | 0 | 0 |
| [`6a42ea0b`](https://github.com/yan-h/harmonigraph/commit/6a42ea0b5bfd3912355cd098988e6a017aaf5b59) | Let the picture's floor be the gradient's floor, and retire the dial that said otherwise (#934) | 28 | 7 | 314 |
| [`27d83d2d`](https://github.com/yan-h/harmonigraph/commit/27d83d2d502e1f1c968d8b31e5d55c2582090560) | Grant worktree sessions the commands their contracts already require (#935) (#939) | 1 | 0 | 0 |
| [`e8af6c75`](https://github.com/yan-h/harmonigraph/commit/e8af6c752f3531c3a1868dcbfa2e7c527b9c4189) | Pin shared skills at the audit-merges signal changes (#938) | 1 | 0 | 0 |
| [`a505cdcb`](https://github.com/yan-h/harmonigraph/commit/a505cdcb0c69d0096c04c4a3c787ba51d7b3f5f7) | Audit merges #880-#933: restore an assertion a hand-resolved merge reverted, and end a departure's poses with its ink (#937) | 15 | 8 | 144 |
| [`24952475`](https://github.com/yan-h/harmonigraph/commit/24952475b0912732da82fb7afaf032a76f88a381) | Move both cloud size dials down to the band that gets used (#933) | 4 | 4 | 175 |
| [`0c242167`](https://github.com/yan-h/harmonigraph/commit/0c242167a0f47a6defc0ae1985ef40f641a26679) | Give the six pass-aged pane caches one owner (#843) (#932) | 8 | 8 | 802 |
| [`7dc20aa0`](https://github.com/yan-h/harmonigraph/commit/7dc20aa009c43ba4cb76ebdfce0933b233088318) | Bring the sevens depth back to nine sheets, and measure the demand not the trim (#916) (#931) | 4 | 4 | 122 |
| [`232d750e`](https://github.com/yan-h/harmonigraph/commit/232d750e8397aa5b19e9a8d885da69ec2a920a72) | Make the spectrogram's effects independent dials, and retire the settings that repeated each other (#928) | 57 | 11 | 802 |
| [`128a4102`](https://github.com/yan-h/harmonigraph/commit/128a4102acf9567d267c80d9ea83484322d617f3) | Repair ViewConfig::spacing at the load door, so marker_unit is always a size (#912) (#927) | 4 | 4 | 104 |
| [`b03ad2df`](https://github.com/yan-h/harmonigraph/commit/b03ad2df9e02d5342ec350b776bb754601e11261) | Say which depth the cabinet crossing was measured at, and what count() reads (#897) (#917) | 2 | 2 | 45 |
| [`b637c28f`](https://github.com/yan-h/harmonigraph/commit/b637c28fb784cebcd759addff103646b0f28db1f) | Repair the startup probe's three stale state reads, and gate it in CI (#875) (#925) | 2 | 1 | 10 |
| [`7241b525`](https://github.com/yan-h/harmonigraph/commit/7241b5252fdddcadc3c63418afb76eb203f07095) | Count lines, not newlines, in reclaim's dirty-file note (#900) (#915) | 2 | 0 | 0 |
| [`57896e63`](https://github.com/yan-h/harmonigraph/commit/57896e630b542633ea9bff0034ff8eab0f871210) | Retire the mosaic's duplicate size dial, make Variety mean something, and fold Shade floor into Relief (#918) | 46 | 6 | 351 |
| [`ff9bf009`](https://github.com/yan-h/harmonigraph/commit/ff9bf00944328f48810b27e0c84188ebfd05b019) | Decide a released voice once, at its release (#905) (#919) | 2 | 2 | 108 |
| [`af28cf47`](https://github.com/yan-h/harmonigraph/commit/af28cf479da4fd8124df3116c41af1ea96160154) | Add the spiral's dot-shadow callback on every frame, and let it decline (#923) | 2 | 2 | 160 |
| [`d7e0f6df`](https://github.com/yan-h/harmonigraph/commit/d7e0f6dff1eb4debe92c7750ad0b6291edd6cad9) | Stop one workflow minting a per-branch copy of a cache everyone shares (#921) | 3 | 0 | 0 |
| [`b990a183`](https://github.com/yan-h/harmonigraph/commit/b990a183a6d4ed4c27197e865791469dee977ab3) | Keep the unknown-map refusal in Lattice Map instead of spending it on Adaptive (#920) | 2 | 2 | 86 |
| [`abfef085`](https://github.com/yan-h/harmonigraph/commit/abfef08519000acf474d5067551a87378f56fb7a) | Give the watercolour a black point, and rename the two cloud textures Mosaic and Watercolor (#913) | 42 | 8 | 300 |
| [`446c80f2`](https://github.com/yan-h/harmonigraph/commit/446c80f232f917dd430798c647591cd140e9ba8d) | Add the watercolour Wash as a second cloud texture beside the water shader (#909) | 45 | 9 | 1182 |
| [`58889c5d`](https://github.com/yan-h/harmonigraph/commit/58889c5d2a272c67430712c36eb505ff1f7c18ce) | Refract the spectrogram through a pile of soft scales (#888) | 51 | 9 | 1023 |
| [`9da6b572`](https://github.com/yan-h/harmonigraph/commit/9da6b572fb481ac8fb08ac85a4bc38dbe1461ea4) | Guard the view's load door: three Layers widths, a NaN that panics the derive, and a poison list that checks itself (#895 row 2A) (#910) | 7 | 7 | 421 |
| [`f92c6133`](https://github.com/yan-h/harmonigraph/commit/f92c6133a346fe2465c2029980e1a130da727b7c) | Give every voice one way out of the held set, and say which sources are in doubt once (#911) | 1 | 1 | 278 |
| [`44246369`](https://github.com/yan-h/harmonigraph/commit/442463694f460a1b3c5411240cafa72a414a4b77) | Row 2B of #895: the spiral's halo callback, the mark cache key, and two per-frame allocations (#907) | 5 | 5 | 328 |
| [`58b07ad1`](https://github.com/yan-h/harmonigraph/commit/58b07ad15f9eb69cee2d1b4864c34fcfd6bf876f) | Add the look-prototype skill: contact sheets before a port (#904) | 7 | 0 | 0 |
| [`d57fdd28`](https://github.com/yan-h/harmonigraph/commit/d57fdd285343c39274f8e7a456be82217ed88854) | Delete the offline audio aligner, and say where an export's time goes (#903) | 11 | 10 | 1187 |
| [`f1a8c0ff`](https://github.com/yan-h/harmonigraph/commit/f1a8c0ff3451bc47b8efa36c355c7729b0bc1276) | Binary-search the audio map history, hold the allocation guard's feature pairing, and key two memos on what they measure (#902) | 6 | 5 | 496 |
| [`58a641ab`](https://github.com/yan-h/harmonigraph/commit/58a641ab67f966d965043bd035c9bb573c34ac95) | Unify the writer pump, the take's latch reset and its incomplete rule (#901) | 6 | 6 | 657 |
| [`0d91fca3`](https://github.com/yan-h/harmonigraph/commit/0d91fca3dd307ce2766ef1bd5b70687509e2bd13) | Force past git's submodule refusal when removing a worktree (#898) (#899) | 2 | 0 | 0 |
| [`77785ec0`](https://github.com/yan-h/harmonigraph/commit/77785ec0e36398f1a2201ec6e4bb512a81b4b07f) | Give every staggered slice one animation length from its own start (#894) | 5 | 4 | 136 |
| [`4dc90e27`](https://github.com/yan-h/harmonigraph/commit/4dc90e27e1ac7993362fcb4a577648b98af8d4d5) | Merge the sevens extent and center into one three-handle layer strip (#896) | 25 | 24 | 1200 |
| [`c6566c39`](https://github.com/yan-h/harmonigraph/commit/c6566c3918a5b05200e5b122a996d7606dbfc55f) | Add lattice map shapes with independent generator automation (#890) | 22 | 17 | 1699 |
| [`cd4a1a2e`](https://github.com/yan-h/harmonigraph/commit/cd4a1a2ef55cecabb82fdfe301a905647ec93863) | Add reversible lattice animation with independent order and starting pose (#885) | 183 | 23 | 1882 |
| [`46e6a3b1`](https://github.com/yan-h/harmonigraph/commit/46e6a3b166cc911fba2e55dd25f5337e184ddc2e) | Average spectrogram power and add musical blur and lava contours (#884) | 85 | 18 | 1262 |
| [`95278984`](https://github.com/yan-h/harmonigraph/commit/9527898410a5a802c78f230b1ec43f089a678efc) | Keep a spectrogram note name until its own box has scrolled off (#882) | 1 | 1 | 176 |
| [`916c2429`](https://github.com/yan-h/harmonigraph/commit/916c2429c211dd31009a23d06b3a406f05dc068f) | Audit merges #857–#879: retire prose the glow, spectral and tuning merges outran (#881) | 11 | 6 | 21 |
| [`08e963ad`](https://github.com/yan-h/harmonigraph/commit/08e963adb36371041c5cb76a52df7f6257521d05) | Render glow at half resolution and cache angular blur weights (#879) | 123 | 12 | 238 |
| [`9d4f76d6`](https://github.com/yan-h/harmonigraph/commit/9d4f76d6abca99ca1fc55ee73e2fd40bdfca8584) | Rasterize directional glow with blended overlap statistics (#877) | 85 | 12 | 1367 |
| [`514e0a50`](https://github.com/yan-h/harmonigraph/commit/514e0a50680808844490f9d730182e800f4b3642) | Simplify directional glow sizing and ink timing (#876) | 46 | 15 | 474 |
| [`eaaa46d8`](https://github.com/yan-h/harmonigraph/commit/eaaa46d8a8ca89a86d0c3309c3cbf46553041022) | Diffuse spectral visuals with independent effect controls (#872) | 66 | 18 | 1685 |
| [`1084b19f`](https://github.com/yan-h/harmonigraph/commit/1084b19f205375cf1c33c56f3beb54717090d606) | Remove wide lattice glow and record the performance audit (#873) | 86 | 13 | 385 |
| [`53d55b97`](https://github.com/yan-h/harmonigraph/commit/53d55b97ffff3fc6f85c283ee4066949467ba994) | Prototype textured, breathing lattice glow with a wider light component (#871) | 100 | 25 | 771 |
| [`be46ba19`](https://github.com/yan-h/harmonigraph/commit/be46ba19f784c1e09f93461b81a02aa2767536d5) | Layer spectral note names by onset and pitch (#870) | 3 | 3 | 171 |
| [`8a79aeb5`](https://github.com/yan-h/harmonigraph/commit/8a79aeb5f5eb6fce0ffd1b91ea0d065a8d18d8b3) | Blend flat piano-roll colors gently over the spectrogram (#869) | 19 | 18 | 351 |
| [`14a3205c`](https://github.com/yan-h/harmonigraph/commit/14a3205c8ec3e41c5340ad56b9468a2e54806bea) | Strengthen Gaussian occlusion of rear lattice nodes and text (#868) | 14 | 4 | 52 |
| [`46c74bd2`](https://github.com/yan-h/harmonigraph/commit/46c74bd23cad2626ca955a7e84805566f52455fb) | Rebind a name's shadow atlas after another name's fill (#867) | 10 | 3 | 102 |
| [`a7865845`](https://github.com/yan-h/harmonigraph/commit/a7865845a4b87d22e5adc3fe3e44823f4c22d071) | Prototype shared linear occlusion for lattice nodes and labels (#866) | 150 | 19 | 981 |
| [`829caaf8`](https://github.com/yan-h/harmonigraph/commit/829caaf8fc797a175fe736c65b31881b1fd733c2) | Use exponential pitch cost for adaptive tuning (#865) | 20 | 11 | 473 |
| [`7b80c5f5`](https://github.com/yan-h/harmonigraph/commit/7b80c5f59808ae436cafc69829cf94710d509e27) | Initialize shared skills in fresh worktrees (#863) | 8 | 0 | 0 |
| [`79531920`](https://github.com/yan-h/harmonigraph/commit/79531920a2ab919a2fa1a7f4be828e14022179ed) | Guard loaded settings against bar ranges (#862) | 8 | 8 | 379 |
| [`bf7a6f69`](https://github.com/yan-h/harmonigraph/commit/bf7a6f6934b13772c585953c503358e3b1c3a7d7) | Released notes count only when nothing is held (#861) | 16 | 8 | 133 |
| [`75c8b33c`](https://github.com/yan-h/harmonigraph/commit/75c8b33c1689074a7469f199a0903741564de61e) | Rework adaptive-tuning context: drop the new-note factor, hold repeats, one vote per note (#860) | 16 | 7 | 287 |
| [`0afb949f`](https://github.com/yan-h/harmonigraph/commit/0afb949f83667584898ea00698d67cc2b543c50b) | Decay tuning context by time and new notes, and weigh register per octave (#858) | 18 | 8 | 432 |
| [`44f49866`](https://github.com/yan-h/harmonigraph/commit/44f498668e64a5bb446f90e50c61fa48d31a4e92) | Size a drawn mark's bitmap off its ink, not its design box (#857) | 2 | 1 | 54 |
| [`de2e887a`](https://github.com/yan-h/harmonigraph/commit/de2e887ac3db6ff962d1840ff53cf67f7febabd5) | Audit merges #776–#854: pin the goldens' window, end the render bar full (#856) | 19 | 9 | 60 |
| [`3d38a86d`](https://github.com/yan-h/harmonigraph/commit/3d38a86d64247a92d271677c3a5cd3e3c4ea163e) | Replace Keep first tuning with a declared keyboard tuning (#852) (#854) | 18 | 14 | 561 |
| [`1dcc03ec`](https://github.com/yan-h/harmonigraph/commit/1dcc03ecaa11bfd999b937f7b09bf39692500963) | Let Learn hear sources with Retune off (#853) | 3 | 2 | 42 |
| [`52571d1a`](https://github.com/yan-h/harmonigraph/commit/52571d1a2072c90638b0dfae9fa2800923df788e) | Keep first tuning: a pitch class replays its first adaptive tuning (#851) | 8 | 6 | 152 |
| [`bb732a6c`](https://github.com/yan-h/harmonigraph/commit/bb732a6c17733a156a5d8ad31e74d5bcfff4b1fe) | Model recorder lifecycle and deferred transitions explicitly (#842) (#850) | 7 | 7 | 721 |
| [`cb824074`](https://github.com/yan-h/harmonigraph/commit/cb8240749f60156cd77ca0ecda9721babf0663a6) | Extract private recorder writer and render-job modules (#842) (#849) | 9 | 9 | 10028 |
| [`4904160f`](https://github.com/yan-h/harmonigraph/commit/4904160f4a04e236b062898733c08eda24b07a73) | Merge pull request #847 from yan-h/codex/issue-845-lattice-stages | 11 | 11 | 3512 |
| [`cb2728ab`](https://github.com/yan-h/harmonigraph/commit/cb2728ab6f9423f658711340bf0aefd4d229ae75) | Gate stopped recorder progress on callback continuity (#839) (#848) | 9 | 8 | 791 |
| [`80841ea4`](https://github.com/yan-h/harmonigraph/commit/80841ea489edf63661b68a9e696157bd5cd9683c) | Remove obsolete source-movement guidance (#846) | 1 | 0 | 0 |
| [`eb8e05e0`](https://github.com/yan-h/harmonigraph/commit/eb8e05e0f8d4b9ac9a733ca546a2319ba54fed67) | Version 0.4.0 (#841) | 2 | 0 | 0 |
| [`b26f366a`](https://github.com/yan-h/harmonigraph/commit/b26f366a84df7859bd8066add9de36a93eeb37bd) | Light the thumb a press would take on multi-handle bars (#840) | 6 | 6 | 399 |
| [`8c1a8e35`](https://github.com/yan-h/harmonigraph/commit/8c1a8e35bf13ed18206b18e66470d674f9bb6f20) | End an On-stop take only once it has captured a note, by either path (#838) | 8 | 6 | 245 |
| [`cc5ea4f4`](https://github.com/yan-h/harmonigraph/commit/cc5ea4f4025c1b6e3740ef3b38409c92a1af2ad0) | Hold the reload lock across the roll's multi-prepare retention tests (#675) (#837) | 1 | 1 | 6 |
| [`017016cf`](https://github.com/yan-h/harmonigraph/commit/017016cfb50a40e7ab624f97061f08fcf2feeadc) | Pin what the layout tests measure on instead of inheriting the fresh look (#836) | 10 | 10 | 262 |
| [`4242843d`](https://github.com/yan-h/harmonigraph/commit/4242843d54284f9796d8e107d9d948c366b4b768) | Refresh README-linked documentation (#835) | 10 | 0 | 0 |
| [`ac22ff11`](https://github.com/yan-h/harmonigraph/commit/ac22ff11d8e6fa3f484828e30edd6ff1f6bbfe75) | Capture the DAW look of 2026-09-10 as the fresh view's defaults (#834) | 30 | 14 | 264 |
| [`f8789b91`](https://github.com/yan-h/harmonigraph/commit/f8789b91c4f9a4139464b30e249885b68cf54f1c) | Rewrite README around Harmonigraph's musical purpose (#829) | 8 | 0 | 0 |
| [`73475921`](https://github.com/yan-h/harmonigraph/commit/73475921fb40eccdbf2886876856b8e6d6979eb3) | Encode renders to YouTube's recommended upload settings (#833) | 2 | 2 | 78 |
| [`272ffc11`](https://github.com/yan-h/harmonigraph/commit/272ffc118e832edf112b139afcdf201ebfa243fc) | Bed every picture pane on black, like the spectrogram (#832) | 31 | 18 | 140 |
| [`b1f27907`](https://github.com/yan-h/harmonigraph/commit/b1f27907bd556142523b48deb1e6cac63e602446) | Keep the render bar moving until the video is written, and show a percentage (#831) | 4 | 4 | 187 |
| [`2153be1d`](https://github.com/yan-h/harmonigraph/commit/2153be1dc2413e7103f7b11763325cc12e350706) | Add a Whole video spectrogram choice to video renders (#830) | 9 | 9 | 176 |
| [`793ab14b`](https://github.com/yan-h/harmonigraph/commit/793ab14b10ce0383dcbbfbe0c4d32e78f815b84a) | Expand video preview arrangement controls (#827) | 6 | 6 | 863 |
| [`c5be24b9`](https://github.com/yan-h/harmonigraph/commit/c5be24b93b0b4098629739c117b95767b59da2de) | Let Stop finish a take whose playhead moved back before it rolled (#828) | 2 | 2 | 63 |
| [`b3697f83`](https://github.com/yan-h/harmonigraph/commit/b3697f836115f71fd08b5747d2bf9078b44c2fa2) | Keep a roll note's name while its ribbon still reaches into the pitch range (#826) | 1 | 1 | 75 |
| [`b69328da`](https://github.com/yan-h/harmonigraph/commit/b69328da13d728e9e9ce1b3d8d6c67cb21173e54) | Hold note names still through a zoom: snap the thinning grid to powers of two (#824) | 1 | 1 | 150 |
| [`4cba72c7`](https://github.com/yan-h/harmonigraph/commit/4cba72c75a4b4ee2e7f7be89b559c5ec2e3a4544) | Keep enlarged preview ribbons proportional (#823) | 2 | 2 | 107 |
| [`cfa65611`](https://github.com/yan-h/harmonigraph/commit/cfa65611950309ac78a44f4cae28482f8ba668ae) | Keep adaptive tuning controls still during edits (#822) | 2 | 2 | 50 |
| [`7139bacf`](https://github.com/yan-h/harmonigraph/commit/7139bacfdac44c31a33de72a85c6c21d544ca425) | Scale piano roll hairline floor in video preview (#821) | 6 | 6 | 152 |
| [`5938548a`](https://github.com/yan-h/harmonigraph/commit/5938548aeffacba50a55b659077c036eb25519e2) | Move video layout controls into the preview (#820) | 5 | 5 | 300 |
| [`2fb4423e`](https://github.com/yan-h/harmonigraph/commit/2fb4423e6be5219107717b293234af137ab5541f) | End a take at a bar the transport plays through (#817) | 8 | 8 | 698 |
| [`143e8aa9`](https://github.com/yan-h/harmonigraph/commit/143e8aa93276f234ea2a44153182fb27e4a8ca2f) | Disable adaptive tuning and the reachable-neighbourhood display by default (#819) | 4 | 4 | 39 |
| [`b13dcab9`](https://github.com/yan-h/harmonigraph/commit/b13dcab98eb33cd2327f22d00ac1af5f428d0595) | Make a bar's fill the part of the track left of the frontier (#816) | 2 | 2 | 312 |
| [`cfe4771a`](https://github.com/yan-h/harmonigraph/commit/cfe4771a3846134035205bbea60404d8b79406df) | Align video preview shadow scale and timing with export (#815) | 7 | 6 | 373 |
| [`689e464d`](https://github.com/yan-h/harmonigraph/commit/689e464dc9960b1f3588ae0333f3c16085adc9e7) | Delete the nice-plug output scheduler (#813) | 14 | 7 | 165 |
| [`3b4999d2`](https://github.com/yan-h/harmonigraph/commit/3b4999d27e97a441c4afd4c469c41a217872e313) | Centralize adaptive tuning instance controls (#812) | 18 | 16 | 1176 |
| [`723f9f2d`](https://github.com/yan-h/harmonigraph/commit/723f9f2d6adb401a7b572eae16ea263ac461bad8) | Keep the audio allocation guard live under the release profile (#810) | 3 | 1 | 41 |
