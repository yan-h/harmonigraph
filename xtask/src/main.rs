//! `cargo xtask bundle` — nice-plug's bundler, which packages the plugin into
//! `target/bundled/` as a CLAP and a VST3.
//!
//! Do not run it from a nested worktree: nice-plug-xtask picks the TOPMOST
//! ancestor holding a `Cargo.toml` as the workspace root, which for a worktree
//! under `.claude/worktrees/` is the main checkout — so it silently bundles
//! main. `load-plugin.sh` exists to sidestep that.

fn main() -> nice_plug_xtask::Result<()> {
    nice_plug_xtask::main()
}
