//! Included only in an isolated copy of wgpu-hal by backend.py.
//! This is experimental file-based plumbing, not the proposed production API.

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSError, NSString};
use objc2_metal::{MTLCompileOptions, MTLDevice, MTLLibrary};

#[allow(deprecated)] // Read the pinned backend's legacy setting without changing it.
pub(super) fn library(
    device: &ProtocolObject<dyn MTLDevice>,
    source: &str,
    options: &MTLCompileOptions,
) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>, Retained<NSError>> {
    let Ok(mode) = std::env::var("HARMONIGRAPH_METAL_ASSETS") else {
        return device
            .newLibraryWithSource_options_error(&NSString::from_str(source), Some(options));
    };
    assert!(matches!(mode.as_str(), "export" | "strict" | "fallback"));
    let directory = std::path::PathBuf::from(
        std::env::var_os("HARMONIGRAPH_METAL_ASSET_DIR").expect("asset directory required"),
    );
    let settings = format!(
        "wgpu-hal=29.0.4\nlanguage={}\nfast_math={}\ninvariance={}\nmath_mode={}\nmath_functions={}\n",
        options.languageVersion().0,
        options.fastMathEnabled(),
        options.preserveInvariance(),
        options.mathMode().0,
        options.mathFloatingPointFunctions().0,
    );
    // This probe uses FNV only to choose a file bucket. Exact source/options
    // equality is mandatory before loading, so a collision cannot serve stale
    // code. The builder additionally records SHA-256 for every artifact file.
    let key = settings.bytes().chain(source.bytes()).fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    let base = directory.join(format!("{key:016x}"));
    if mode == "export" {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(base.with_extension("metal"), source).unwrap();
        std::fs::write(base.with_extension("options"), &settings).unwrap();
    } else {
        let matching = std::fs::read_to_string(base.with_extension("metal"))
            .is_ok_and(|saved| saved == source)
            && std::fs::read_to_string(base.with_extension("options"))
                .is_ok_and(|saved| saved == settings);
        if matching {
            if let Ok(bytes) = std::fs::read(base.with_extension("metallib")) {
                if !bytes.is_empty() {
                    let loaded = super::library_from_metallib::new_library_from_metallib_bytes(
                        device, &bytes,
                    );
                    if loaded.is_ok() {
                        eprintln!("METAL_ASSET hit {key:016x}");
                        return loaded;
                    }
                }
            }
        }
        assert_ne!(mode, "strict", "missing, mismatched or invalid Metal asset {key:016x}");
    }
    eprintln!("METAL_ASSET source {key:016x}");
    device.newLibraryWithSource_options_error(&NSString::from_str(source), Some(options))
}
