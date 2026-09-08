use std::{collections::BTreeSet, env, fs, path::PathBuf};

use sha2::{Digest, Sha256};

fn main() {
    let directory = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    println!("cargo:rerun-if-changed=assets");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 1, "unknown Metal asset manifest schema");
    let hashes = manifest["sha256"].as_object().unwrap();
    let files: BTreeSet<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "manifest.json")
        .collect();
    assert_eq!(files, hashes.keys().cloned().collect(), "Metal artifact file set changed");
    for (name, hash) in hashes {
        assert_eq!(PathBuf::from(name).file_name().unwrap(), name.as_str());
        let bytes = fs::read(directory.join(name)).unwrap();
        assert!(!bytes.is_empty(), "empty Metal asset: {name}");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            hash.as_str().unwrap(),
            "changed Metal asset: {name}"
        );
    }
    let mut generated = String::from("pub static ASSETS: &[Asset] = &[\n");
    for name in files.iter().filter(|name| name.ends_with(".metal")) {
        let stem = name.strip_suffix(".metal").unwrap();
        let key = u64::from_str_radix(stem, 16).unwrap();
        let path = |extension: &str| directory.join(format!("{stem}.{extension}"));
        assert!(
            files.contains(&format!("{stem}.options"))
                && files.contains(&format!("{stem}.metallib"))
        );
        generated.push_str(&format!(
            "Asset {{ key: {key}, source: include_str!({:?}), options: include_str!({:?}), library: include_bytes!({:?}) }},\n",
            path("metal"), path("options"), path("metallib"),
        ));
    }
    generated.push_str("];\n");
    fs::write(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("assets.rs"), generated).unwrap();
}
