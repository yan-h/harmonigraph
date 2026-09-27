// Stamp the git branch and commit in the leaf binaries as `LATTICE_BUILD_TAG`.
// Interactive shells show it in the performance overlay; the offline renderer
// rebuilds so load-plugin.sh's HEAD-based freshness check remains accurate.
// The tag names the last commit, not uncommitted source edits: a dirty marker
// would require relinking on every edit to stay honest.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn main() {
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let sha = git(&["rev-parse", "--short", "HEAD"]);

    // Re-stamp when HEAD moves — a new commit, or a branch switch. `--git-path`
    // rather than a literal `.git/...` because every session build happens in a
    // worktree, where `.git` is a FILE and HEAD lives under the main checkout's
    // `.git/worktrees/<name>/`. Asking git for the path is what makes this work
    // the same in the main checkout and in a worktree.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(resolved) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={resolved}");
        }
    }

    let tag = match (branch, sha) {
        (Some(branch), Some(sha)) => {
            // Claude session branches carry the `worktree-` prefix, which is
            // noise in a HUD. Codex's `codex/` prefix identifies the branch and
            // stays. In both cases the tag is exactly the argument
            // `./load-plugin.sh <branch>` takes.
            let name = branch.strip_prefix("worktree-").unwrap_or(&branch);
            format!("{name} @{sha}")
        }
        // A source tarball, or git missing. Say so rather than inventing a tag
        // that would later be read as naming a branch.
        _ => "unknown".to_owned(),
    };
    println!("cargo:rustc-env=LATTICE_BUILD_TAG={tag}");
}
