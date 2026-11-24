//! The offline renderer: `harmonigraph_offline` does the work, and this binary
//! lives in the plugin's package so that a plugin build always builds the
//! renderer of the same commit (#1954).
//!
//! It must never name `harmonigraph_plugin`. Cargo hands a binary its package's
//! library, but rustc loads and links one only when the binary names it, so
//! the plugin's `#[global_allocator]` and everything else in it stay out and
//! the renderer keeps the system allocator.

fn main() -> std::process::ExitCode {
    // Stamped by this package's build script, the same one that stamps the
    // plugin, so the two always carry one tag.
    harmonigraph_offline::main(env!("LATTICE_BUILD_TAG"))
}
