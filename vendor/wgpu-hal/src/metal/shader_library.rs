//! An optional source-to-library provider. Translation, reflection and native
//! pipeline ownership stay in the normal Metal backend.

use alloc::{borrow::Cow, format, string::String};
use std::sync::OnceLock;

use objc2::{available, rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{MTLCompileOptions, MTLDevice, MTLLibrary};

/// Backend version belongs to artifact identity, even if generated text matches.
pub const BACKEND_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Resolved native compilation options. `None` means the API is unavailable.
#[derive(Clone, Copy, Debug)]
pub struct CompileOptions {
    pub language_version: u64,
    pub fast_math: bool,
    pub preserve_invariance: Option<bool>,
    pub math_mode: Option<u64>,
    pub math_functions: Option<u64>,
}

/// The exact source and options supplied to the normal Metal compiler.
pub struct Request<'a> {
    pub source: &'a str,
    pub options: CompileOptions,
}

/// Provider decision; validation can reject misses without silently compiling.
pub enum Resolution {
    Source,
    Library { bytes: Cow<'static, [u8]>, fallback_on_load_error: bool },
    Reject(String),
}

/// Reports the actual native path, including failed bytecode loads.
pub enum Event<'a> {
    Loaded,
    LoadFailed(&'a str),
    Source,
}

/// Must be immutable after installation and safe for concurrent device creation.
pub trait Provider: Send + Sync {
    fn resolve(&self, request: &Request<'_>) -> Resolution;
    fn event(&self, _request: &Request<'_>, _event: Event<'_>) {}
}

static PROVIDER: OnceLock<&'static dyn Provider> = OnceLock::new();

/// Install before creating any devices. Returns false if already installed.
///
/// # Safety
/// Every returned library must implement exactly the requested generated source
/// and compiler semantics, including bounds instrumentation. The caller must
/// verify its artifact provenance and integrity; wgpu cannot validate bytecode.
/// The provider and its code must outlive every device using this backend.
pub unsafe fn install(provider: &'static dyn Provider) -> bool {
    PROVIDER.set(provider).is_ok()
}

#[allow(deprecated)]
pub(super) fn load(
    device: &ProtocolObject<dyn MTLDevice>,
    source: &str,
    options: &MTLCompileOptions,
) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>, String> {
    let Some(provider) = PROVIDER.get() else {
        return device
            .newLibraryWithSource_options_error(&NSString::from_str(source), Some(options))
            .map_err(|error| format!("{error}"));
    };
    let modern = available!(macos = 15.0, ios = 18.0, tvos = 18.0, visionos = 2.0);
    let request = Request {
        source,
        options: CompileOptions {
            language_version: options.languageVersion().0 as u64,
            fast_math: options.fastMathEnabled(),
            preserve_invariance: available!(macos = 11.0, ios = 13.0, tvos = 14.0, visionos = 1.0)
                .then(|| options.preserveInvariance()),
            math_mode: modern.then(|| options.mathMode().0 as u64),
            math_functions: modern.then(|| options.mathFloatingPointFunctions().0 as u64),
        },
    };
    match provider.resolve(&request) {
        Resolution::Reject(message) => return Err(message),
        Resolution::Source => {}
        Resolution::Library { bytes, fallback_on_load_error } => {
            match super::library_from_metallib::new_library_from_metallib_bytes(device, &bytes) {
                Ok(library) => {
                    provider.event(&request, Event::Loaded);
                    return Ok(library);
                }
                Err(error) => {
                    let message = format!("{error}");
                    provider.event(&request, Event::LoadFailed(&message));
                    if !fallback_on_load_error {
                        return Err(message);
                    }
                }
            }
        }
    }
    provider.event(&request, Event::Source);
    device
        .newLibraryWithSource_options_error(&NSString::from_str(source), Some(options))
        .map_err(|error| format!("{error}"))
}
