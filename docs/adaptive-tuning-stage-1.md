# Adaptive tuning stage 1: bundle discovery

Stage 1 addresses [#631](https://github.com/yan-h/harmonigraph/issues/631) only.
The committed base is merged main `8d1ce70b26cd6805947c862591b7fd23be3eddd4`, incorporating [#630](https://github.com/yan-h/harmonigraph/pull/630) and [#633](https://github.com/yan-h/harmonigraph/pull/633).
The implementation branch is `codex/631-clap-discovery` in the Codex-managed worktree `/Users/yan/.codex/worktrees/7f30/harmonigraph`.
The draft PR and final handoff record the exact stage head, avoiding a self-referential commit hash here.

## Requirement and decision

Installing a bundle with an added CLAP class must invalidate Bitwig's class-discovery fingerprint while preserving the signed bundle and the live executable's inode.
Both installation entry points must carry this behavior.
No adaptive tuning implementation, plugin descriptor, persisted state or Rust code changes in this stage.

`load-plugin.sh` retains the existing order:
sign and verify a staging bundle, then copy the signed executable through the live inode and copy its resource seal.
It now refreshes `Contents/Info.plist`'s modification time before verifying the live bundle.
The plist bytes stay identical, so the refresh does not invalidate their code signature.
If the timestamp still has the previous whole-second value, the loader waits one second and retries until it differs.
This deliberately covers timestamp precision coarser than Bitwig's measured millisecond fingerprint, including rapid installs, without manufacturing future dates or changing bundle versions.
Every install refreshes metadata, including switching back to a build with fewer classes.
`update-plugin.sh` continues to delegate the swap to that loader.

## Evidence and rejected approaches

On Bitwig 6.1 and macOS 26.6.2, #631 measured a cached timestamp/size pair of `1785094189049` milliseconds and `881` bytes matching the unchanged plist.
The installed factory already enumerated both classes, excluding a missing class in the executable as the cause.
Updating the scan location, touching the bundle root and Contents directory, and restarting Bitwig did not discover the companion.
Touching the plist and then restarting Bitwig refreshed both metadata and the location index.
Cache deletion, descriptor changes and fake audio ports were unnecessary.
These are the original issue's host measurements, not a new live-host trial performed for this stage.

Changing the plist's contents or bundle version would require installing newly signed metadata and adds no benefit to this timestamp-based fix.
Signing the live bundle is rejected because codesign replaces its executable inode, leaving a surviving host mapped to the old file.
Duplicating the swap in `update-plugin.sh` would restore a previously observed source of drift.
A bare touch without checking the resulting timestamp could collide with a rapid preceding install, so the retry is part of the contract.

## Files and validation

- `load-plugin.sh`: refresh discovery metadata without changing the existing staging and inode-preserving copy sequence.
- `update-plugin.sh`: document that its delegated swap includes discovery metadata refresh.
- `.claude/tests/plugin-swap.sh`: extend the existing temporary-repository fixture with metadata, unchanged-plist and signature assertions for both paths, a deterministic timestamp collision, and direct-loader inode assertions.
- `docs/adaptive-tuning.md`: link this stage's installation contract.
- `docs/adaptive-tuning-stage-1.md`: retain requirements, decisions, evidence and the handoff.

The metadata regression failed before the fix for CLAP and VST3 on both entry points:
four failures.
After the fix, `.claude/tests/plugin-swap.sh` passes, including the injected initial-touch collision for each bundle on the direct-loader path.
The fixture uses real Mach-O binaries and macOS codesign, with a throwaway Git repository, bundle slot and HOME;
Cargo is stubbed because compilation is outside the behavior under test.
Existing checks retain registered worktree discovery, build-tag reporting and installed-byte verification.
The additional checks require a changed plist timestamp, byte-identical plist contents, valid final signatures and preserved executable inodes.

Shell syntax, Rust formatting, Markdown semantic breaks and diff whitespace checks pass.
Full CI runs in GitHub Actions, with its result attached to the exact pushed head.
No local release compilation is required for these scripts and documentation changes.
Neither installation script was run against the shared Bitwig slot, and no new binary tag or loadable release build is claimed.

## Remaining work and review boundary

This stage ends at a committed, pushed draft PR ready for independent review;
it is not merged.
The original implementer validates and fixes subsequent independent review findings.
A live Bitwig rediscovery trial for this revision remains a user-controlled check:
unload the plugin host, install the desired existing build using the updated loader, then restart or rescan Bitwig and inspect the available classes.
The original issue established restart after plist refresh;
this stage does not claim that a rescan alone reloads an already running sandbox.

Subsequent stages implement the production contracts in the [adaptive tuning design](adaptive-tuning.md), including #632's cancellation and retiming requirements.
The accepted initial delay candidate remains 512 samples at 44.1 kHz with a 512-sample engine buffer, subject to implementation validation;
#630's 2,048-sample measurement remains evidence rather than a production default.
