# Auditor brief

Find where one area of Harmonigraph has drifted from intent:
code that every change kept for a locally good reason, whose reason no longer holds.
Read `SKILL.md` beside this file for the kinds and the worked example.

Audit `origin/main`, read-only.
Git history, PR titles and bodies (`gh pr view N --json title,body`) and open issues are all available;
use the issues only at the end, to drop findings already filed.

## Before anything else

Pull the area's fresh defaults — the settings a new view opens on, kernels and routings included.
Which tests matter, and which behaviour is the COMMON case, follows from them:
in the first trial the shipped Gaussian shadow and the default Opacity routing were what made two findings visible,
and every tip test turned out to run only on the kernel nobody ships.

## What found things (spend the time here)

1. **Trace each mechanism to its origin.**
   Inventory the area's uniform flags, passes, special cases, constants and settings.
   For each, `git log -S` / `-G` to the PR that added it, read that PR's body for the reason, and judge whether the reason still holds on main.
   A mechanism whose reason was a case since removed or replaced is the finding this audit exists for.
2. **Producer reachability.**
   For each field the renderer or a helper reads, list the values its only caller can actually emit today.
   A branch no caller can reach, a field every stage overwrites, and a test fixture using a value the product cannot produce all come out of this step.
3. **Tests as behaviour.**
   Restate each test covering the area as the user-visible behaviour it protects.
   Flag the ones that only restate a mechanism, and the ones whose fixture cannot reach the case they name.

## Cheap sweeps (real, but noisy)

- Identifiers and labels a PR deleted that comments or docs still name. About half noise.
- Scoping lines — "keep", "preserve", "unchanged", "existing", "comments only", "byte-identical" — but only in PRs that REMOVE a setting, pass or target.
  Across every PR it was ninety hits for two findings; those two were the best evidence of intent the trial had.
  For each, name what was kept and trace it to where it was added.

Defaults at a range end found nothing in the first trial; skip it unless the area is mostly dials.

## Report

- **Findings, ranked by how much Yan would notice:**
  observable behaviour, `file:line` on main, the PRs and the step where it drifted, your confidence,
  and whether the fix is mechanical or needs Yan's call — phrased as a yes/no question when it does.
- **Sweep hits you dismissed**, one line each with why.
- **Tests flagged**, with the reason.
- **Method notes:** which steps found things, which were noise, what you would change.

Do not pad. A clean area is a result.
