# Adaptive tuning musical policy engine

This is the independent pure-core component of [#621](https://github.com/yan-h/harmonigraph/issues/621), based on reviewed recovery head `5ebe666b6eae6e29cd5e24d9468132047b2c217f`.
This document records the pure-core component and its original evidence.
The later [musical integration stage](adaptive-tuning-musical-integration.md) wires it into adaptive playback over the ordinary recovery chain;
combined review and host qualification still remain before #621 is complete.

## Named initial constants

The [current musical contract](adaptive-tuning.md#musical-policy-boundary), current #621 and [accepted decisions](https://github.com/yan-h/harmonigraph/pull/633) were checked before implementation.
The preparation note was analytical input, not executed evidence.
The policy uses existing `positions_within`, `LatticePos::respell`, resolved `Tuning` integer axes and `PitchClass`.
The narrow new `PitchClass::signed_microcents_from` API avoids a float round trip.

| Constant | Initial value |
| --- | --- |
| Raw domain | threes −6 through 6, fives −2 through 2, sevens 0 |
| Maximum raw/canonical nodes and candidates | 65 |
| Candidate radius | 50,000,000 microcents, inclusive and circular |
| Context projection radius | 50,000,000 microcents, inclusive and circular |
| Context L1 weight | 4 per usable voice |
| Key history L1 weight | 1 when usable context exists |
| Empty-context origin L1 weight | 1, ignoring history |
| Tie order | absolute threes, fives, sevens; then signed threes, fives, sevens |
| Maximum context input | 256 emitted voices and scheduled predecessors together |
| Maximum central cohort | 256 onsets, one sequential call each |

These values are the version-one `policy::CONFIG` descriptor, not additional controls.
The production configuration reducer now binds that descriptor into requests through the [musical integration stage](adaptive-tuning-musical-integration.md).
The function takes `MusicalConfig`, which copies only resolved origin, axes and respelling flags from `ResolvedConfig`.
Its input type has no display tolerance, camera, reach, pane, resolution, auto-detection or learning state.
Revision identity belongs to the caller's bound request, not the score.

Enumeration visits the complete raw box, respells every position and deduplicates canonical coordinates.
It does not clip those coordinates back to the raw box or merge unrelated coordinates at equal pitch.
The independent Just domain has 65 coordinates.
Syntonic locking produces the 29-coordinate chain from −14 through 14 fifths;
the septimal lock adds no further change because the raw domain has no sevens.

This deliberately chooses a 5-limit first policy.
Independent septimal alternatives would change the required D–F–A trajectory;
they are not silently admitted as a larger version of the same fixture.
An actual septimal held pitch still remains authoritative and follows the ordinary projection rule.

## Function and ownership boundary

`assign_new_note(config, context, history, onset, scratch)` returns a `Decision` or explicit input error.
The onset contains the MIDI key already selected by the central order.
The owner retains source, channel, lifetime, input time and decision identity, and supplies only that source/channel/key's optional transient history cell.
`HistoryUpdate::Set(node)` or `Clear` must be applied in the appropriate prospective or confirmed owner.
The function does not own a map or infer resets from an empty slice.

Each `ContextPitch` contains the actual emitted pitch class or an explicitly scheduled predecessor's pitch, including current player expression, separately from optional attack-node metadata.
The owner retains absolute pitch and provenance.
Metadata is reused only when that exact coordinate belongs to the effective canonical domain and represents the authoritative pitch exactly under the current configuration.
A configuration revision alone would not prove either condition.
Otherwise the engine chooses the nearest node within the fixed context radius, using the coordinate tie order.
An unprojectable voice contributes no L1 term;
the function cannot delete it or change its pitch.
Context multiplicity is preserved:
two voices at one pitch contribute twice.

Candidates lie within the fixed key-relative radius.
With usable context the score is `4 * sum(L1(candidate, context_node)) + L1(candidate, history)` when history exists.
With none it is only `L1(candidate, origin)`.
Origin preference can select a non-origin coordinate for a non-C key.
Scores use `u64`, with widened coordinate subtraction so even malformed external history cannot overflow.
Candidate pitch and corrections use exact core microcents throughout.
The selected correction is folded into the nearest octave;
the general pitch accessor chooses −600 cents for an exact half-octave tie, which lies outside this policy's candidate radius.

`Assignment::NoCandidate` is a completed result with zero adaptive correction and a cleared key history.
Player expression is still composed by the output owner.
It is distinct from `InvalidMidiKey`, `TooMuchContext`, an absent reply or a deadline failure.
More than 256 context inputs is rejected without truncation.
The caller must admit cohorts and held/prospective voices within the central limits;
the single-onset function cannot enforce a multi-call cohort count.

The scratch is caller-owned fixed arrays for domain nodes, candidate indices and projected context coordinates.
Every call rebuilds all used entries and lengths, so there is no cache key or carry-forward state to invalidate.
Direct enumeration is the initial implementation.
No timing measurements or optimization claims are part of this stage without a separate exclusive-machine grant.

## Musical fixtures

The focused tests call the production function.
Just fixtures explicitly set independent Just axes and disable comma locks/automatic engagement;
the ordinary default configuration is locked 12-TET and yields zero corrections for every MIDI key.
The musical input copied from a resolved configuration is unchanged by display tolerance or irrelevant mode/revision changes.

Held C `(0,0,0)` and G `(1,0,0)` admit both E candidates:
5/4 `(0,1,0)` scores 12 and 81/64 `(4,0,0)` scores 28. The selected E is 5/4.

Sequential empty-context D–F–A incorporates D before F and both before A.
The expected placement is D `(2,0,0)` = 9/8, F `(3,−1,0)` = 27/20 and A `(3,0,0)` = 27/16. Thus F/D = 6/5 and A/D = 3/2. The test checks real candidates and exact differences of core axes;
rational names describe ideal Just ratios rather than claiming float logarithms equal quantized microcents.
Independent selection of F from empty context gives a different coordinate, so the fixture detects omission of predecessors.

The ii–V–I fixture starts D/F/A at MIDI keys 50/53/57, keeps D into G/B/D, then keeps G into C/E/G.
Each phase's new notes are evaluated sequentially in ascending key order.
It asserts exact coordinates and corrections reconstructed from the real core axes, and every selected correction remains within ±50 cents of its key.
This is one observed trajectory, not an anti-drift guarantee or a promise that incompatible held context always forms a pure chord.

With held D `(2,0,0)`, a fresh E prefers 81/64 with score 8 versus 12 for 5/4. Separate history at 5/4 changes those scores to 13 and 12, selecting 5/4. Applying `Clear` removes that preference on the next call.
Empty usable context ignores the previous phrase's history.
Ordinary release survival and the required withdrawal/configuration/session/epoch/recovery clears still need central lifecycle wiring;
these tests exercise the pure consequences of supplied/cleared cells only.

The context fixtures cover stale 12-TET attack metadata for a held 400-cent E under a new independent Just configuration:
its projection is `(−4,−1,0)`, while the authoritative pitch remains exactly 400 cents.
A zero-correction voice with the same pitch gets the same decision without metadata.
A bent C attack sounding at G projects as G.
Exact valid metadata is retained, but even equal-pitch metadata outside the canonical domain is rejected and projected.

Supported custom axes 720/360/960 cents form a 120-cent pitch grid in the enumerated domain.
A 300-cent target is at least 60 cents from every node, actually reaching `NoCandidate` and clearing history.
The same pitch as context has no projection while C still has candidates, exercising empty usable context and history suppression separately.
These custom values lie within the existing ±40-cent Just-axis parameter ranges.

Other fixtures cover inclusive ±50-cent boundaries and exclusion one microcent beyond, octave wrap, respelling duplicates, equal-pitch coordinate preservation, exact score/projection ties and explicit input bounds.
A deliberately coincident all-zero-axis core input reaches all 65 candidates at once for scratch stress;
it is not a host parameter configuration or the supported custom-axis emptiness fixture.

## Executed evidence and remaining work

On Apple Silicon with Rust 1.92, the 13 focused policy fixtures passed.
The complete core suite passed 128 tests with one existing ignored test;
core clippy with all targets and warnings denied passed.
The allocation guard counts alloc, zeroed alloc, realloc and free on the calling test thread.
It reaches all 256 sequential onsets with predecessor incorporation and direct history access/update, then the full 65-candidate/256-projected-context path and explicit NoCandidate/input-error paths.
It reports zero heap calls.
The cohort harness uses four source rows of 64 unique channel-zero keys each;
it supplies pure inputs and is not a production source/history owner.

Executed `size_of`/alignment output:

| Compiled object | Bytes |
| --- | ---: |
| `PolicyScratch` (alignment 8) | 4,208 |
| Domain array: 65 × 16-byte node/pitch cells | 1,040 |
| Candidate index array | 65 |
| Projected context array: 256 coordinates | 3,072 |
| `MusicalConfig` | 20 |
| `ContextPitch` / 256 input cells | 20 / 5,120 |
| `OrderedOnset` | 1 |
| `Assignment` / `HistoryUpdate` / `Decision` | 20 / 16 / 36 |
| Optional history node / four-row harness table | 16 / 8,192 |

The scratch includes lengths and padding beyond its three arrays, and has a compile-time 4,224-byte ceiling in the storage fixture.
It fits within the contract's 4-MiB policy reservation.
That comparison excludes central ownership/provenance storage, which still needs production accounting.

Executed candidate counts for keys C through B:

| Configuration | Canonical domain | Counts by key class |
| --- | ---: | --- |
| Independent Just | 65 | 5, 5, 7, 5, 5, 5, 6, 5, 5, 5, 7, 5 |
| Default locked 12-TET | 29 | 3, 2, 3, 2, 2, 3, 2, 3, 2, 2, 3, 2 |
| Independent 720/360/960 | 65 | 9, 5, 7, 0, 6, 8, 4, 8, 6, 0, 7, 5 |
| All-zero-axis core stress input | 65 | 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 |

The storage ceiling is 65 candidates for any core input.
The count table does not claim an exhaustive maximum over the supported continuous host parameter ranges.

The executed ii–V–I trajectory uses core axes `three = 701,955,017` and `five = 386,313,721` microcents:

| Phase | New MIDI key | Selected node (3,5,7) | Correction, microcents |
| --- | ---: | --- | ---: |
| ii, empty then sequential | 50 (D) | (2,0,0) | +3,910,034 |
| ii | 53 (F) | (3,−1,0) | +19,551,330 |
| ii | 57 (A) | (3,0,0) | +5,865,051 |
| V, retain D | 55 (G) | (1,0,0) | +1,955,017 |
| V | 59 (B) | (1,1,0) | −11,731,262 |
| I, retain G | 48 (C) | (0,0,0) | 0 |
| I | 52 (E) | (0,1,0) | −13,686,279 |

The held D and G keep their original corrections through those transitions.
Only each new key's ±50-cent correction bound is enforced.
No timing benchmark has been run;
neither the allocation fixture nor this progression is a production callback or WCET result.
Final exact-head CI and release evidence is recorded in the draft PR handoff, so recording that result does not move the commit the binaries identify.

The original component left the following owner/integration work for later stages;
the [musical integration report](adaptive-tuning-musical-integration.md) records the current implementation and remaining checks:

- Complete #617 aggregation and #616 artificial-policy central sequencing,
including callback-order independence and off/retrigger/expression precedence.
- Restack this component onto that chain and bind the production descriptor/configuration.
- Wire separate prospective/confirmed history with provenance,
release survival, required resets and suffix revocation/recovery.
- Compose player expression at actual output,
preserving held corrections through configuration changes and failures.
- Validate complete callback storage and work at real voice/cohort/candidate limits,
D512 boundaries and the accepted Bitwig live/offline topology.
- Run combined independent review,
exact-head CI and both release builds before the final integrated handoff.

This component does not claim manual Bitwig or listening evidence, overall #621 acceptance, or completion of the adaptive tuning goal.
