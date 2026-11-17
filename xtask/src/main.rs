//! `cargo xtask bundle harmonigraph-plugin --release`: builds the plugin and
//! lays out `target/bundled/Harmonigraph.clap`, the macOS bundle a CLAP host
//! loads.
//!
//! The workspace root is this crate's parent, which is the workspace by
//! construction: `xtask` is a member of it. Not the topmost ancestor holding
//! a `Cargo.toml`, as `nice-plug-xtask` took it: worktrees live at
//! `.claude/worktrees/<name>`, inside the main checkout, so from a worktree
//! that walks past the worktree's own `Cargo.toml` and bundles the main
//! checkout, saying nothing about it.
//!
//! The bundle is what `nice-plug-xtask` 0.1.1 made, byte for byte less the
//! binary and its signature: the dylib at `Contents/MacOS/Harmonigraph`,
//! `PkgInfo`, a fixed `Info.plist` and an ad-hoc signature. The identifier
//! stays `com.nice-plug.harmonigraph-plugin`, since whether a host keys a
//! plugin's saved state or its scan cache on it is not established.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

const USAGE: &str =
    "usage: cargo xtask bundle harmonigraph-plugin [--bundle-dir <dir>] [cargo build flags]";

/// The one package that bundles.
const PACKAGE: &str = "harmonigraph-plugin";
/// The bundle's name and the binary's inside it, so what shows in Bitwig's
/// browser and in `~/Library/Audio/Plug-Ins`.
const NAME: &str = "Harmonigraph";

/// `Contents/Info.plist`, as `nice-plug-xtask` 0.1.1 wrote it for [`PACKAGE`]
/// named [`NAME`]. Bitwig caches a bundle's class list by this file's mtime
/// and size (`load-plugin.sh`), so its bytes stay as they were.
const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>

<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist>
  <dict>
    <key>CFBundleExecutable</key>
    <string>Harmonigraph</string>
    <key>CFBundleIconFile</key>
    <string></string>
    <key>CFBundleIdentifier</key>
    <string>com.nice-plug.harmonigraph-plugin</string>
    <key>CFBundleName</key>
    <string>Harmonigraph</string>
    <key>CFBundleDisplayName</key>
    <string>Harmonigraph</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>NSHumanReadableCopyright</key>
    <string></string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
"#;
/// `Contents/PkgInfo`: the package type and creator signature, as above.
const PKG_INFO: &[u8] = b"BNDL????";

fn main() -> Result<()> {
    let root = workspace_root()?;
    std::env::set_current_dir(&root)?;
    let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("target"),
    };

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("bundle") => {
            let (packages, mut build_args) = split_bundle_args(args)?;
            if !cfg!(target_os = "macos") {
                bail!("bundling makes a macOS .clap bundle; build the plugin with cargo here");
            }
            if let Some(package) = packages.iter().find(|package| *package != PACKAGE) {
                bail!("'{package}' is not a plugin package; only {PACKAGE} bundles");
            }
            // Elsewhere than `target/bundled` for a bundle that must not
            // replace the one there: in the main checkout that is the slot a
            // host loads (`load-plugin.sh`).
            let bundled = match build_args.iter().position(|arg| arg == "--bundle-dir") {
                Some(at) if at + 1 < build_args.len() => {
                    build_args.remove(at);
                    PathBuf::from(build_args.remove(at))
                }
                Some(_) => bail!("'--bundle-dir' needs a path\n\n{USAGE}"),
                None => target_dir.join("bundled"),
            };
            build(&build_args)?;
            let dylib = target_dir
                .join(profile(&build_args)?)
                .join(format!("lib{}.dylib", PACKAGE.replace('-', "_")));
            bundle(&dylib, &bundled)
        }
        Some(command) => bail!("unknown command '{command}'\n\n{USAGE}"),
        None => bail!("missing command\n\n{USAGE}"),
    }
}

fn build(build_args: &[String]) -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "-p", PACKAGE])
        .args(build_args)
        .status()
        .context("could not run cargo build")?;
    if !status.success() {
        bail!("cargo build failed for {PACKAGE}");
    }
    Ok(())
}

/// The directory under the target directory that `cargo build` with these
/// flags writes to: the profile's name, except `dev`, which cargo writes to
/// `debug`. A `--target` build lands elsewhere and is not a macOS bundle this
/// knows how to make.
fn profile(build_args: &[String]) -> Result<&str> {
    let mut profile = "debug";
    let mut args = build_args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" | "-r" => profile = "release",
            "--profile" => profile = args.next().context("'--profile' needs a name")?,
            "--target" => bail!("cross-compiled bundles are not supported"),
            arg if arg.starts_with("--target=") => {
                bail!("cross-compiled bundles are not supported")
            }
            arg => profile = arg.strip_prefix("--profile=").unwrap_or(profile),
        }
    }
    Ok(if profile == "dev" { "debug" } else { profile })
}

/// Lay out `<bundled>/Harmonigraph.clap` around `dylib` and sign it ad hoc.
fn bundle(dylib: &Path, bundled: &Path) -> Result<()> {
    let bundle = bundled.join(format!("{NAME}.clap"));
    let contents = bundle.join("Contents");
    let binary = contents.join("MacOS").join(NAME);
    fs::create_dir_all(binary.parent().expect("a file under MacOS/"))?;
    // Replaced rather than overwritten: macOS caches a binary's signature
    // check by its file, so one rewritten in place fails that check at its
    // next load and the loading process is killed, while a host that has the
    // old binary mapped keeps its file.
    if binary.exists() {
        fs::remove_file(&binary)
            .with_context(|| format!("could not remove {}", binary.display()))?;
    }
    fs::copy(dylib, &binary)
        .with_context(|| format!("could not copy {} to {}", dylib.display(), binary.display()))?;
    fs::write(contents.join("PkgInfo"), PKG_INFO)?;
    fs::write(contents.join("Info.plist"), INFO_PLIST)?;
    // The linker's ad-hoc signature covers the binary alone; signing the
    // bundle seals `Info.plist` and `PkgInfo` in with it, which Apple
    // silicon's stricter loading may ask of a plugin bundle.
    let status = Command::new("codesign")
        .args(["-f", "-s", "-"])
        .arg(&bundle)
        .status()
        .context("could not run codesign")?;
    if !status.success() {
        bail!("codesign failed on {}", bundle.display());
    }
    eprintln!("Created a CLAP bundle at '{}'", bundle.display());
    Ok(())
}

/// The directory holding the workspace `Cargo.toml`, taken from cargo's own
/// `CARGO_MANIFEST_DIR` at *run* time rather than `env!` at compile time: a
/// binary compiled once and served back out of `sccache` would otherwise
/// carry whichever worktree first built it.
fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| anyhow!("'$CARGO_MANIFEST_DIR' is unset; run this through `cargo xtask`"))?;
    Path::new(&manifest_dir)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("'{manifest_dir}' has no parent directory"))
}

/// Split `<package>` or a series of `-p <package>` off the front, leaving
/// everything else for `cargo build`.
fn split_bundle_args(args: impl Iterator<Item = String>) -> Result<(Vec<String>, Vec<String>)> {
    let mut packages = Vec::new();
    let mut build_args = Vec::new();
    let mut args = args.peekable();

    // A bare first argument is the package, which is the common case.
    if args.peek().is_some_and(|arg| !arg.starts_with('-')) {
        packages.push(args.next().expect("just peeked"));
    }
    while let Some(arg) = args.next() {
        if arg == "-p" || arg == "--package" {
            packages.push(
                args.next().ok_or_else(|| anyhow!("'{arg}' needs a package name\n\n{USAGE}"))?,
            );
        } else {
            build_args.push(arg);
        }
    }

    if packages.is_empty() {
        bail!("no package to bundle\n\n{USAGE}");
    }
    Ok((packages, build_args))
}
