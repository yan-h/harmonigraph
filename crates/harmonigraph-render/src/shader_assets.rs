//! Install immutable Metal assets before device creation. The backend retains
//! all translation, reflection, GPU-object and worker-lifetime responsibilities.

/// Initialize once per linked image, before any graphics device is requested.
/// Other platforms keep their ordinary backend path.
pub fn initialize() {
    #[cfg(target_os = "macos")]
    metal::initialize();
}

/// Actual backend library requests, useful for strict coverage and benchmarks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    pub loaded: usize,
    pub source: usize,
    pub load_failed: usize,
    pub rejected: usize,
}

pub fn statistics() -> Statistics {
    #[cfg(target_os = "macos")]
    return metal::statistics();
    #[cfg(not(target_os = "macos"))]
    Statistics::default()
}

#[cfg(target_os = "macos")]
mod metal {
    use std::borrow::Cow;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    use crate::wgpu::hal::metal::shader_library::{
        self as hal, Event, Provider, Request, Resolution,
    };

    enum Mode {
        Embedded,
        Source,
        Strict,
        #[cfg(feature = "shader-assets-tools")]
        Export(std::path::PathBuf),
    }

    struct Assets {
        mode: Mode,
        // Only suppress repeated diagnostics; never caches native objects.
        reported: Mutex<HashSet<u64>>,
        loaded: AtomicUsize,
        source: AtomicUsize,
        load_failed: AtomicUsize,
        rejected: AtomicUsize,
    }

    static ASSETS: OnceLock<Assets> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();

    pub(super) fn initialize() {
        INSTALLED.get_or_init(|| {
            let mode = match std::env::var("HARMONIGRAPH_SHADER_ASSETS").as_deref() {
                Err(_) | Ok("embedded") => Mode::Embedded,
                Ok("source") => Mode::Source,
                Ok("strict") => Mode::Strict,
                #[cfg(feature = "shader-assets-tools")]
                Ok("export") => Mode::Export(
                    std::env::var_os("HARMONIGRAPH_SHADER_EXPORT_DIR")
                        .expect("HARMONIGRAPH_SHADER_EXPORT_DIR is required")
                        .into(),
                ),
                Ok(mode) => panic!("unknown HARMONIGRAPH_SHADER_ASSETS mode: {mode}"),
            };
            let provider = ASSETS.get_or_init(|| Assets {
                mode,
                reported: Mutex::default(),
                loaded: AtomicUsize::new(0),
                source: AtomicUsize::new(0),
                load_failed: AtomicUsize::new(0),
                rejected: AtomicUsize::new(0),
            });
            // SAFETY: the asset builder compiles the exact backend-generated
            // inputs. Build-time SHA-256 checks validate immutable embedded
            // payloads; lookup requires full source/options equality. Provider
            // code and bytes share this backend's linked-image lifetime.
            assert!(unsafe { hal::install(provider) }, "Metal provider already installed");
        });
    }

    fn options(request: &Request<'_>) -> String {
        let o = request.options;
        let optional = |value: Option<u64>| value.map_or("unavailable".into(), |v| v.to_string());
        format!(
            "schema=1\nwgpu-hal={}\nlanguage={}\nfast_math={}\ninvariance={}\nmath_mode={}\nmath_functions={}\n",
            hal::BACKEND_VERSION,
            o.language_version,
            o.fast_math,
            o.preserve_invariance.map_or("unavailable".into(), |v| v.to_string()),
            optional(o.math_mode),
            optional(o.math_functions),
        )
    }

    impl Provider for Assets {
        fn resolve(&self, request: &Request<'_>) -> Resolution {
            if matches!(self.mode, Mode::Source) {
                return Resolution::Source;
            }
            let options = options(request);
            let key = harmonigraph_metal_assets::key(request.source, &options);
            #[cfg(feature = "shader-assets-tools")]
            if let Mode::Export(directory) = &self.mode {
                // Serialize duplicate exports, and reject even an unlikely hash
                // collision rather than overwriting a different compiler input.
                let _guard = self.reported.lock().expect("asset export lock");
                let result = (|| -> std::io::Result<()> {
                    std::fs::create_dir_all(directory)?;
                    for (extension, contents) in [("metal", request.source), ("options", &options)]
                    {
                        let path = directory.join(format!("{key:016x}.{extension}"));
                        if path.exists() && std::fs::read_to_string(&path)? != contents {
                            return Err(std::io::Error::other("Metal artifact key collision"));
                        }
                        std::fs::write(path, contents)?;
                    }
                    Ok(())
                })();
                return match result {
                    Ok(()) => Resolution::Source,
                    Err(error) => Resolution::Reject(format!("Metal asset export: {error}")),
                };
            }
            if let Some(asset) = harmonigraph_metal_assets::find(request.source, &options) {
                return Resolution::Library {
                    bytes: Cow::Borrowed(asset.library),
                    fallback_on_load_error: !matches!(self.mode, Mode::Strict),
                };
            }
            if matches!(self.mode, Mode::Strict) {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Resolution::Reject(format!(
                    "missing Metal asset {key:016x}; regenerate the production catalog"
                ));
            }
            if self.reported.lock().expect("asset diagnostic lock").insert(key) {
                eprintln!(
                    "Harmonigraph: Metal asset {key:016x} unavailable; compiling from source"
                );
            }
            Resolution::Source
        }

        fn event(&self, request: &Request<'_>, event: Event<'_>) {
            match event {
                Event::Loaded => {
                    self.loaded.fetch_add(1, Ordering::Relaxed);
                }
                Event::Source => {
                    self.source.fetch_add(1, Ordering::Relaxed);
                }
                Event::LoadFailed(message) => {
                    self.load_failed.fetch_add(1, Ordering::Relaxed);
                    let key = harmonigraph_metal_assets::key(request.source, &options(request));
                    eprintln!("Harmonigraph: Metal asset {key:016x} could not load: {message}");
                }
            }
        }
    }

    pub(super) fn statistics() -> super::Statistics {
        ASSETS.get().map_or_else(super::Statistics::default, |assets| super::Statistics {
            loaded: assets.loaded.load(Ordering::Relaxed),
            source: assets.source.load(Ordering::Relaxed),
            load_failed: assets.load_failed.load(Ordering::Relaxed),
            rejected: assets.rejected.load(Ordering::Relaxed),
        })
    }
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
mod catalog;
