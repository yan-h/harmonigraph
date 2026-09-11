# Adaptive tuning: a moving harmonic neighbourhood

## Status and scope

Design discussion recorded on 2026-09-09 for Yan and future sessions.
This document records requirements and proposals, not implemented behaviour or a settled algorithm.
Yan explicitly asked to refine the design before implementation.
Continue the discussion and update this document as decisions are made.
The current implementation is described in [adaptive-tuning.md](adaptive-tuning.md).
The [standalone tuning laboratory](../tools/adaptive-tuning-simulator/README.md) implements a first experimental model and the musical fixtures for inspection.
Its documented scoring and reference choices remain hypotheses to refine against this design.
The [plugin implementation record](adaptive-tuning-plugin.md) describes the subsequent Rust port, controls, expression handling and integration limits.

Yan later clarified that toy-example answers are not fixed at every parameter setting, and that different answers may implicitly assume different settings.
Read their expected outcomes as musical targets to reproduce under explicitly stated settings, not a demand for one unconditional answer across the entire parameter space.
The explicitly immutable prohibition on automatic retuning of sounding notes is unchanged.
Do not satisfy the examples with chord-specific exceptions; first test whether a small coherent family of scoring rules and settings can express them.

## Guiding principle: few hard rules, tunable shared heuristics

Yan explicitly prefers few hard rules and a simple combination of heuristics that can be adjusted to produce intuitive outcomes.
This is the governing design direction, not a request to encode a hierarchy of mandatory decisions for every toy example.
Treat sounding-note attraction, release recency, register relationships and precise input as influences in the shared selection mechanism where possible.
Words such as “precedence” in the examples describe desired musical influence; they do not by themselves require lexicographic priorities that defeat every other consideration.

The explicitly agreed fixed adaptive correction and hard exclusion of harmonically remote candidates remain structural constraints.
Within eligible candidates, seek a small shared score or comparably simple heuristic combination, with parameters explaining changes in behaviour.
The number of heuristic terms need not equal the number of exposed controls; keep both understandable.
Specific chord names and example identities must not become branches in that mechanism.
If satisfying an example demands a new special rule, present the tradeoff to Yan and consider revising or dropping the example instead.

## Musical objective

Allow indefinite travel through a just-intonation lattice using locally sensible harmonic steps, even when the controller repeats a fixed set of pitches.
There must be no automatic pull toward the original tuning reference and no fixed absolute lattice box that eventually stops the journey.

The motivating progression is repeated C major → E major → A♭ major → C major on a 12-TET keyboard.
With a just lattice, harmonic continuity should allow this to become C major → E major → G♯ major → B♯ major → …,
translating the major-triad shape one step right along the major-third axis at each transition.
The shared tones E, G♯ and B♯ provide a musical route for those translations.

Three just major thirds have ratio `(5/4)^3 = 125/64`, less than an octave.
Relative to the repeated keyboard register, each three-transition cycle therefore accumulates one diesis of downward pitch drift: `1200 log2(128/125) ≈ 41.059 cents`.
Rightward lattice movement and downward accumulated tuning displacement are both intended.
Returning to the same input chord must not reset its lattice position or wrap away the accumulated displacement.

This example is a requirement for general harmonic heuristics, not a request to recognize and special-case these three chords.
MIDI alone cannot distinguish an ordinary return to C from continuing enharmonically to B♯;
the desired musical convention favours continuity along the lattice.

Yan later asked for the opposite as an option, for a Wicki-Hayden keyboard where a key should keep its pitch once established.
It is the [keyboard tuning](adaptive-tuning-plugin.md#keyboard-tuning):
a key only becomes a node the keyboard itself renders at that key, so the travel above continues only among nodes the keyboard cannot tell apart.
The default 12-TET keyboard tells none of them apart, so the travel stays the default.

## Structural requirements and musical targets

### Continuous input pitch

Do not model the input as twelve keyboard pitch classes mapped to twelve lattice nodes.
Any arbitrary pitch can arrive, including from a controller with fine divisions of the octave.
Finer input tuning should give the player more control over nearby harmonic alternatives.
The concrete example is choosing between the minor sevenths 16/9 (approximately 996.09 cents) and 9/5 (approximately 1017.60 cents) above a reference.
When both are harmonically admissible, a more precise input should help select the intended one.

Yan refined the example of a sounding just E and a deliberately played Pythagorean E.
If the new input is close enough to the Pythagorean E to light that node on the lattice, treat it as an intentional Pythagorean E and allow it.
Otherwise the new note may or may not match the sounding 5/4 E, depending on distance and other parameters.
This supersedes the earlier unconditional sounding-pitch-class matching rule.
An intentional pitch still cannot admit a node outside the hard harmonic boundary.

The precision criterion refers to the existing lattice note-matching tolerance, not automatically to the separately proposed tolerance for refreshing memory.
How that criterion relates to the moving tuning reference must be resolved before implementation.
Matching raw input against the original absolute lattice could pull a returning 12-TET C back home whenever the old C node remains eligible, conflicting with the required drift.
The display's current matching semantics alone therefore do not settle the adaptive reference semantics.
Do not silently equate intentional input with whichever output node the algorithm has already chosen; that would make the criterion circular.

### Context determines the minor-seventh preference

The 9/5 versus 16/9 example is not solely a test of a finely tuned controller selecting either interval.
Yan specified the following contextual expectations for a subsequent B♭, with ratios measured from C:

| Existing context | Expected B♭ | Musical relationship |
|---|---|---|
| C–E♭–G | 9/5 | With E♭ at 6/5, B♭ at 9/5 is a just fifth above E♭ |
| C–E–G | No fixed answer | Yan is unsure and accepts that algorithm parameters may decide |
| C–F | 16/9 | With F at 4/3, B♭ at 16/9 is a just fourth above F, equivalently a fifth below it modulo octaves |

Use these as examples of context-dependent selection, not hard-coded chord-recognition rules.
Compare the contexts with the same incoming B♭ pitch to isolate the effect of harmony from finer controller input.
The ratio explanations assume just context pitches; a fixture must establish their actual tuned onset pitches rather than infer them from keyboard labels alone.
In particular, the C–E♭–G expectation uses E♭ at 6/5, and the C–F expectation uses F at 4/3.
These examples constrain the harmonic metric alongside the ascending-third travel requirement.
Fine input control remains useful, but does not replace the requirement for harmony to influence the choice.

Register distance should weaken a context note's influence, but it must not overturn the 9/5 preference in this specific minor-triad example.
Yan's musical target is 9/5 over 16/9 for C–E♭–G regardless of the registers occupied by those context pitch classes, because of lattice distance.
The later parameter clarification qualifies this target: the register comparison should hold under stated suitable settings, rather than every possible pitch-fidelity setting or deliberate input alteration.
Moving E♭3 to E♭6 while retaining the just pitch class therefore does not make 16/9 plausible for B♭3.
This rules out a register-weighting heuristic that simply discards the distant E♭ and thereby reverses this preference.
Different pitch input and parameters may change the winner; this contextual example does not categorically prohibit choosing 16/9 elsewhere in the parameter space.
Determine that behaviour through the shared selection rule, not a special-case exception for B♭ or this chord.

### Register determines which E fits a fifth chain

Yan supplied a concrete constraint for the register-aware algorithm, which is now required in the initial version.
Establish C–G–D–A as successive ascending just perfect fifths, preserving their registers rather than folding them into one octave.
For example, the context is C3–G3–D4–A4, with frequency ratios `1, 3/2, 9/4, 27/8` relative to C3.
Then compare two alternative next onsets against that same context:

| Incoming E register | Required tuning | Lattice interpretation |
|---|---|---|
| Roughly a perfect fifth above A4: E5 | `81/16` above C3, or `3/2` above A4 | Continue the Pythagorean chain to four fifth steps above C |
| Roughly a major third above C3: E3 | `5/4` above C3 | Use one just major-third step above C |

The high E is two octaves plus `81/64` above C, whose octave-reduced interval is approximately 407.820 cents.
The low E is approximately 386.314 cents above C.
After accounting for their two-octave register difference, the two E placements differ by the syntonic comma `81/80`, approximately 21.506 cents.
This is a choice between distinct lattice nodes, not merely putting one fixed E node in a different octave.

A comparison may use the same nominal input pitch class in those two registers to isolate register's effect.
Start each alternative from the same established context; do not let the first E contaminate the context used for the second.
Preserve the actual context pitches and registers in the fixture, rather than inferring a Pythagorean chain from key labels alone.
All already sounding context notes retain their fixed adaptive tuning.

This constrains how an incoming note's register changes the relevance of individual context notes and intervals.
It does not prescribe a specific weighting formula or a special-case rule for E.
Both outcomes are requirements for the initial version.
A register-blind simplification cannot satisfy them and is no longer an accepted initial scope.

### Sounding counterparts and the revised precision rule

Extend the fifth-chain example by first playing E3 at 5/4 above C3, then adding E5.
Yan initially required the new high E to match a still-sounding low E in pitch class, giving E5 at `5/1` above C3.
The later precision answer supersedes that universal requirement: deliberately precise Pythagorean input is allowed, and other input may or may not match the low E depending on distance and parameters.
Matching the sounding low E remains an intended available behaviour, not an unconditional lock that requires chord-specific exemptions.
The existing E is never retuned.

If the low E has instead been recently released, the high E should continue the fifth chain at `81/16` above C3.
The released E remains eligible to contribute to memory, but loses the stronger status of a sounding note.
Distinguishing held from released context is therefore a musical decision, not merely storage bookkeeping or a slow decay of influence.
An arbitrarily long configured memory must not categorically force the high E to match that released low E; the example targets settings that continue the fifth chain.

How to score correspondence with a sounding note under continuous input, handle multiple sounding alternatives, and relate precision to memory tolerance and candidate eligibility remain design questions.
Do not turn those questions into a fixed twelve-key mapping.
The examples use unbent notes; the separate onset-context simplification for player bends remains in force.

### Further agreed examples

- **Comma-pump progression.** Play C major → F major → D minor → G major → C major, preserving common tones between successive chords. If the original C is no longer sounding on the return, Yan explicitly wants the resulting syntonic-comma drift rather than a return to its original reference. If the original C remains sounding, the outcome may depend on parameters. In the just common-tone construction the final C is `80/81` of the original C at the same nominal register.
- **A lone reference in widely separated registers.** With only C3 sounding, compare separate incoming E3 and E6 onsets against identical context. Both must select the 5/4 pitch class above C; the high placement alone does not justify a Pythagorean E without the additional fifth-chain context.
- **An old pedal released last.** Hold C through changing harmony, then release it. Its released contribution takes precedence over notes released before it, but not over still-sounding notes. Release recency matters even when that C's original attack was much older; the formal meaning of precedence remains to be designed.

### Context and memory

Released notes must continue to contribute to harmonic context, favouring more recently released notes.
Forget old contributions when they cease to be useful, using straightforward bounded memory rather than an elaborate analysis of whether a note could ever affect a future choice.
**New musical activity replaces old context; elapsed time alone does not gradually weaken it.** If Yan plays and leaves a note, later notes must still use that context for as long as the configured memory/reset setting permits.
The exact replacement and weighting rules remain open.
Releases count as activity for ordering released memory: a newly released old pedal precedes previously released notes without outranking notes still sounding.

Repeating a note refreshes its existing contribution rather than accumulating copies or increasing its weight merely through repetition.
If the same keyboard C returns one diesis lower, it is an entirely separate new pitch and must be treated as such.
Identity follows the resulting pitch/lattice placement, not controller-key identity.
Yan suggested a tolerance setting for deciding when a new note counts as the same as an old one.
The proposed tolerance concerns memory matching, not permission to retune a sounding note.
Its comparison domain, units, default, interaction with lattice spelling and register-sensitive matching still need to be specified.
Memory matching must preserve the registers needed by harmonic context rather than silently collapsing octave-separated occurrences into a pitch-class-only record.
Do not choose a default that conflates the motivating diesis-separated pitches or the desired minor-seventh alternatives.

For the initial version, Yan accepts remembering the last N distinct contributions, refreshing repeats and evicting the least recently used released contribution when capacity is exceeded.
The later pedal-release answer requires this recency bookkeeping to account for release events as well as attacks and repeat refreshes.
Held notes remain represented rather than being evicted to meet the remembered-note budget.
The capacity may be configurable; its value and accounting for held versus released entries remain open.
**This is explicitly a temporary simplification.** Yan wants to move away from this fixed recent-note scheme eventually, toward a more musically informed replacement policy.
Do not turn the initial capacity rule into a permanent musical contract or implement the more elaborate successor before it is designed.

Sounding notes should have stronger influence than released memory; a sounding counterpart is a preference subject to the revised precision and parameter rules, not an unconditional pitch-class lock.
Express that influence through the shared heuristics rather than assume an absolute held-versus-released decision hierarchy.
The formal weighting and interaction with harmonic and register distance remain open.
A sustained bass may keep the neighbourhood nearby or be left behind by the newer harmony, depending on the sequence of notes and harmonic heuristics.
Yan accepts either outcome; do not impose a universal pedal anchor or universal escape rule.
Forgetting individual old notes must not itself erase the accumulated tuning displacement.
Ordinary releases between chords must not destroy continuity.

**Register awareness is required in the initial version.** Yan explicitly reversed the earlier permission to defer it after supplying the fifth-chain example.
Harmonic context must store notes by register, and evaluate an incoming note using its register relative to those context notes.
This is more than giving bass notes a fixed extra weight: relationships between input and context registers belong in the decision.
The exact register-sensitive heuristic remains to be designed, but its inclusion is no longer deferred.
Do not declare octave-equivalent inputs interchangeable against unchanged context or use a pitch-class-only context model.
Output register and accumulated tuning displacement must also be preserved.

Transposing the entire performance and its context by an octave must preserve relative tuning decisions and lattice travel, with output pitches shifted by that octave.
Register influence must therefore depend on relationships among registers rather than absolute octave numbers.
This whole-performance transposition invariant does not make an isolated incoming note octave-equivalent against unchanged context.

Provide a configurable time before reset after silence, with a proposed “Never” option.
The working proposal measures silence from the release of the last sounding note and clears both harmonic memory and accumulated drift at timeout.
Whether transport events reset the context must be configurable, rather than always resetting or always preserving it.
This answers the discussion of stopping/restarting playback and looping;
the exact event triggers, whether they share one control or have separate controls, and defaults remain open.
Exact pedal and reset behaviour, timeout range and the treatment of accumulated drift under transport reset still need to be specified.

### Frozen notes and sequential decisions

**No adaptive retuning of an already sounding note, ever, under the present design.** Yan explicitly made this an immutable rule unless he later changes his mind.
Choose the adaptive tuning correction at attack and retain it for the note's lifetime, regardless of subsequent harmony or neighbourhood movement.
Do not reselect or glide a held note to improve a later chord.
Player-authored pitch bends after attack must remain audible, composed with the fixed adaptive correction.
The no-retuning rule forbids automatic changes by the algorithm, not expressive bends by the player.

For the initial version, harmonic context uses each note's tuned onset pitch and does not follow its subsequent bends.
This is an intentional simplification: a bent note's current audible pitch may differ from the pitch contributing to future tuning decisions.
Whether and how harmonic context should follow player bends remains an open question for later design.
Do not silently implement bend-following context or suppress the player's bends to make those two pitches agree.
Onset-based policy context does not redefine actual emitted-pitch reporting as onset-only reporting.

Prefer a simple algorithm and accept that the route through the notes affects the result.
A chord played together, rolled or arpeggiated need not arrive at the same tuning, and changing note order may change the path.
Do not introduce joint chord optimization or extra grouping latency just to make those cases equivalent.
The implementation still needs a deterministic order for simultaneous attacks.

### Hard harmonic boundary

Determine a finite eligible neighbourhood from harmonic context before considering the incoming pitch.
Distance means harmonic distance along the lattice, not acoustic distance in cents.
A sufficiently distant lattice node is ineligible even if it perfectly matches the input pitch.
The incoming pitch must not expand the boundary to reach such a node.

The neighbourhood can move as context changes, allowing unlimited cumulative travel through individually local steps.
The distance metric, treatment of multiple context notes and boundary size are not yet chosen.
In particular, a simple radius of two fifth/major-third steps around a lone root admits 16/9 but excludes 9/5, which requires two fifth steps and one reverse major-third step.
Check the desired harmonic vocabulary before choosing a radius or node-count limit.

Provide a setting controlling which lattice axes or interval families the algorithm may use.
Yan requested configurability rather than a fixed choice between fifth/major-third relationships and including seventh-based relationships such as 7/4.
The UI shape, available combinations, default and behaviour when that setting changes remain open.
Changing the allowed vocabulary must still respect the prohibition on retuning existing notes.

### Preference within the boundary

Among eligible candidates, balance closeness to the incoming pitch against harmonic distance or preference.
Yan requested a setting controlling their relative importance.
Pitch fidelity must be measured relative to a moving tuning reference so that accumulated drift does not turn fidelity into an attraction back to the original pitch.
How that reference is represented and updated remains a central open question.

The proposed balance is “Harmonic preference ↔ Pitch fidelity”.
Stronger harmonic preference should make a coarse controller useful for following just relationships;
stronger pitch fidelity should give a finely tuned controller more explicit choice.
Neither end can admit nodes outside the hard boundary.
The proposal retains meaningful pitch sensitivity even at the strongest harmonic setting, since ignoring pitch entirely could make every input choose the same node.
No score formula, weights or slider endpoints have been agreed.

### Lattice reachability indicator

Show the nodes that can actually be selected by any arbitrary next input pitch with the present context held fixed.
Distinguish eligible candidates from reachable winners: an eligible node may never win for any input pitch.
The desired indicator shows the latter set.
It does not show everything reachable after arbitrary future sequences, which can extend indefinitely.

Yan imagines a reasonably small reachable set, fewer than twenty nodes.
Treat that as a design target to validate, not an agreed hard cap that may silently remove useful interval choices.
Register awareness means a sweep through just one octave can miss nodes reachable from other input registers.
The indicator's “any arbitrary next input” meaning must account for those registers, for example by taking the union of reachable lattice nodes across the intended input-register range.
The input range, how to compute that union and whether an optional register-specific view helps remain open.
Do not present a single-register slice as the complete reachable set.

Proposed presentation: subtle outlines on reachable nodes, distinct from the appearance of sounding notes.
A faint connecting region and an optional view of each node's input-pitch range are possibilities, not approved UI specifications.
Context and balance changes may move those ranges and make nodes enter or leave the reachable set.

## Proposed controls

| Control | Intended purpose | Status |
|---|---|---|
| Neighbourhood size | Set the maximum permitted harmonic remoteness | Proposed; metric, range and default open |
| Harmonic preference ↔ Pitch fidelity | Balance eligible candidates' harmonic suitability and input-pitch closeness | Requested; scoring, range and default open |
| Reset after silence | Set when context and accumulated drift reset, optionally never | Requested; timing semantics, range and default open |
| Reset on transport events | Choose whether stopping/restarting playback and looping reset context | Requested; event triggers, one versus separate controls, reset scope and defaults open |
| Allowed lattice axes / interval families | Control which harmonic relationships may supply candidates, including whether seventh-based relationships are allowed | Requested; UI, combinations and default open |
| Same-note tolerance | Decide when a new pitch refreshes an existing memory contribution | Suggested by Yan; matching semantics, units, range and default open |
| Remembered-note capacity | Set N for the temporary recent-note replacement scheme | Scheme accepted for initial version; exposing N as a setting remains proposed |

The initial memory replacement scheme is intentionally temporary; its successor remains to be designed.
Do not add gradual time decay as the default interpretation of forgetting.
Replacement must not be conflated with the silence reset: one lets new harmony take over, while the other intentionally ends the journey.

## Open decisions for the next discussion

1. **Moving reference.** What establishes and advances the input-to-lattice reference? How are register and accumulated unwrapped displacement carried separately? Note-order dependence is accepted, but updates still need a coherent musical interpretation.
2. **Harmonic distance and vocabulary setting.** Which axis combinations should the setting offer, and how are permitted connections weighted? Is remoteness measured from one centre, the whole context or nearby individual voices? How is a widely spread context handled without admitting arbitrary bridges or leaving no candidates?
3. **Memory matching and weights.** Different resulting pitches remain distinct even when played by the same key. How should the proposed same-note tolerance work, and what is N for the accepted temporary recent-note scheme? How are sounding-note preference and newly released memory precedence expressed without categorical pitch-class locking? Recency must include release events; general weights and chord-density effects remain open. A held bass has no categorical anchoring privilege over unrelated incoming notes. The eventual successor to fixed-capacity recency remains open.
4. **Reset.** What counts as silence under sustain? Which transport transitions trigger the configurable reset, and should stop/start and loop reset share a control? Does a transport reset clear accumulated displacement along with context? What timeout range and defaults feel useful?
5. **Continuous pitch and expression.** Which incoming pitch controls describe the pitch to quantize at attack? Post-attack player bends remain audible, while the initial policy context retains tuned onset pitch. Bend-aware context is deferred; automatic reselection of sounding notes is forbidden. The current participating path does not use incoming expression to choose its assignment, so continuous input requires an explicit change to that contract.
6. **Selection and indicator.** How strong can pitch fidelity become within the boundary, and how are ties and simultaneous attacks ordered stably? How should the incoming note's register relative to register-bearing context influence selection from the first version, and how should the indicator express register-dependent reachability?

Joint chord selection was an earlier suggestion; the subsequent decision favours simplicity and accepts route dependence.
A fixed twelve-node keyboard mapping was explicitly rejected in favour of continuous pitch input.
Do not restore that assumption merely to simplify the indicator.

## Constraint consistency and simplicity

Yan explicitly asked to flag contradictions or complexity and reconsider constraints rather than build a convoluted algorithm to satisfy every accumulated answer.
Record revisions as revisions, not an ever-growing list of exception rules.

The latest answers revise absolute sounding-pitch matching into a preference that allows precise intentional alternatives and parameter-dependent outcomes.
The no-automatic-retuning rule is unchanged.
Release precedence and octave-transposition invariance fit the existing context model without requiring separate chord rules.
The two register examples do not by themselves contradict each other: register can distinguish the fifth-chain Es while lattice preference still preserves 9/5 in the minor-triad example.
Whether a simple metric actually satisfies both must be demonstrated rather than assumed.

Yan then explicitly confirmed that parameters may change the toy-example answers and that different answers may have assumed different parameters.
This removes the apparent demand for incompatible unconditional outputs, including treating the minor-triad answer as a prohibition on 16/9 at every precision setting.
Which moving or absolute reference the node-lighting precision criterion uses remains unresolved.
An algorithm draft must state input pitches, relevant settings, and sounding/released context for each example.
Prefer a small number of coherent parameter choices rather than rescuing every example with an unrelated setting or exception.
No mathematical contradiction has been established, and no need for chord-specific rules has been demonstrated; a concrete draft and probes must establish whether the shared rules are sufficient.
If identical inputs, context and settings are later required to produce incompatible outcomes, flag that exact conflict to Yan rather than layering on exceptions.

## Behaviour to validate when an algorithm is proposed

Apply the parameter clarification above to musical-output examples.
Record the settings used for each comparison; changing only register in a register comparison must not silently change the settings too.

- Repeated ascending-third major chords continue rightward for many cycles, beyond one diesis and beyond the old absolute pitch-correction window.
- Ordinary gaps between chords preserve the journey until the configured reset.
- A lone remembered note remains useful after waiting, provided the silence timeout has not expired; new musical activity, rather than gradual time decay, replaces its influence.
- Repetition refreshes one remembered contribution without multiplying its weight by the number of attacks.
- Replaying a keyboard pitch at a diesis-shifted output creates a distinct contribution; any memory tolerance must have deliberately stated matching behaviour.
- In the temporary N-entry scheme, new distinct activity evicts the least recently used released contribution, while held notes remain represented.
- Later notes never cause an already sounding note to be adaptively retuned.
- Player bends after attack remain audible with the adaptive correction fixed; the initial harmonic context continues to use tuned onset pitch.
- Transport-triggered context reset follows its setting rather than an unconditional preserve/reset policy.
- Fine pitch input can distinguish 9/5 and 16/9 when both are eligible.
- With the same subsequent B♭ input, just C–E♭–G context favours 9/5 and just C–F context favours 16/9; C–E–G has no prescribed answer and may depend on parameters.
- In the initial version, an established ascending C3–G3–D4–A4 fifth chain yields E5 at `81/16` above C3 when extending the chain, but E3 at `5/4` above C3 when played near C; compare separate next onsets from identical context.
- With E3 at `5/4` still sounding after that fifth chain, deliberately precise Pythagorean E5 input may select `81/16`; otherwise matching at `5/1` depends on distance and parameters. If E3 was recently released instead, E5 should continue the chain at `81/16` despite the remembered E3.
- The just common-tone C major → F major → D minor → G major → C major progression drifts when the original C is no longer sounding; retaining that original sounding C makes the outcome parameter-dependent.
- With only C3 sounding, separate E3 and E6 inputs both select the 5/4 pitch class above C.
- Releasing an old pedal moves its contribution ahead of notes released earlier without giving it precedence over still-sounding notes.
- Redistributing the just C–E♭–G context across registers, including moving E♭ far above an incoming B♭, must still favour 9/5 over 16/9 because of lattice distance.
- Transposing the entire input performance and context by an octave preserves relative tuning decisions and lattice travel, and transposes outputs by that octave.
- An exact pitch match outside the harmonic boundary never wins.
- The allowed-vocabulary setting controls candidate generation without retuning notes already sounding.
- Reachable nodes shown in the indicator agree with actual next-note selection under the same context.
- Note-order dependence is accepted for inversions and arpeggios; simultaneous attacks must still have a deterministic order, and sustained-anchor and register behaviour need explicit heuristics.
- The initial version stores context notes by register and evaluates incoming notes in relation to those registers.

These are design checks, not claims of implemented or tested behaviour.
The existing policy's fixed origin domain, 50-cent candidate window and per-note sequential decisions are useful comparison points, not constraints on this redesign.
