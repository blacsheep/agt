//! Embeds the Web console (`web/`) into the `agt` binary so `agt web` works
//! from an installed binary without a repository checkout.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Runtime files of the console; tests and docs are not shipped.
const ROOT_FILES: &[&str] = &[
    "server.mjs",
    "activity.mjs",
    "tool-details.mjs",
    "package.json",
];
const PUBLIC_DIR: &str = "public";

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let web = manifest.join("../../web");
    println!("cargo:rerun-if-changed={}", web.display());

    let mut files = Vec::new();
    for name in ROOT_FILES {
        files.push(name.to_string());
    }
    collect(&web, Path::new(PUBLIC_DIR), &mut files);
    files.sort();

    // FNV-1a over names and contents: identifies the asset set without a crypto dependency.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut source = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for name in &files {
        let path = web.join(name);
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for byte in name.bytes().chain([0]).chain(bytes.iter().copied()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let absolute = path.canonicalize().expect("canonical web asset path");
        writeln!(
            source,
            "    ({name:?}, include_bytes!({:?})),",
            absolute.display().to_string()
        )
        .unwrap();
        println!("cargo:rerun-if-changed={}", path.display());
    }
    source.push_str("];\n");
    writeln!(source, "pub const HASH: &str = \"{hash:016x}\";").unwrap();

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("web_assets.rs");
    fs::write(out, source).expect("write web_assets.rs");
}

fn collect(web: &Path, relative: &Path, files: &mut Vec<String>) {
    let entries = fs::read_dir(web.join(relative))
        .unwrap_or_else(|e| panic!("read {}: {e}", web.join(relative).display()));
    for entry in entries {
        let entry = entry.expect("web asset entry");
        let path = relative.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            collect(web, &path, files);
        } else {
            // Forward slashes keep the embedded names identical on every platform.
            files.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
}
