//! Stamps the git branch and commit into the binary as `LATTICE_BUILD_TAG`,
//! which the performance overlay shows as its `build` row.
//!
//! Bitwig loads exactly ONE bundle, and parallel sessions each build into their
//! own worktree, so "which build am I actually looking at?" is a real question
//! with a wrong answer available — `load-plugin.sh` swaps binaries that look
//! identical from inside the DAW. The overlay answering it in the picture is
//! the only check that cannot be fooled by a swap that didn't take, a
//! reactivate that didn't happen, or a build that went to a different worktree.
//!
//! The tag names the last COMMIT, not the working tree: a build made with
//! uncommitted edits carries the commit it sits on. That is the same thing
//! `load-plugin.sh`'s freshness column means, and deliberately so — a "dirty"
//! marker here could not be kept honest without re-running this script (and so
//! relinking) on every source edit, and a marker that lies is worse than none.

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
            // Every session branch carries the same `worktree-` prefix, which
            // is noise in a HUD. Stripping it also makes the tag exactly the
            // argument `./load-plugin.sh <branch>` takes.
            let name = branch.strip_prefix("worktree-").unwrap_or(&branch);
            format!("{name} @{sha}")
        }
        // A source tarball, or git missing. Say so rather than inventing a tag
        // that would later be read as naming a branch.
        _ => "unknown".to_owned(),
    };
    println!("cargo:rustc-env=LATTICE_BUILD_TAG={tag}");
}
