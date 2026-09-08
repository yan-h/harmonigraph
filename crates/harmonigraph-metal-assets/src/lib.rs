//! Immutable, verified build assets. No GPU objects, filesystem lookup or
//! graphics-stack dependency lives here. The renderer owns provider policy.

/// An exact compiler input and the library generated from it.
pub struct Asset {
    pub key: u64,
    pub source: &'static str,
    pub options: &'static str,
    pub library: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// A stable bucket, followed by full input equality in [`find`]. It is not an
/// integrity digest: build-time SHA-256 validation covers immutable payloads.
pub fn key(source: &str, options: &str) -> u64 {
    options
        .bytes()
        .chain(source.bytes())
        .fold(0xcbf29ce484222325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
}

/// Match only inputs which actually determine the compiler output. Device
/// handles and animated scene state are deliberately absent from this identity.
pub fn find(source: &str, options: &str) -> Option<&'static Asset> {
    let key = key(source, options);
    ASSETS
        .iter()
        .find(|asset| asset.key == key && asset.source == source && asset.options == options)
}

#[cfg(test)]
mod tests {
    #[test]
    fn exact_inputs_are_required_even_when_the_bucket_matches() {
        for asset in super::ASSETS {
            assert_eq!(super::key(asset.source, asset.options), asset.key);
            assert!(super::find(asset.source, asset.options).is_some());
            assert!(super::find(&format!("{}\n", asset.source), asset.options).is_none());
            assert!(super::find(asset.source, &format!("{}\n", asset.options)).is_none());
        }
    }
}
