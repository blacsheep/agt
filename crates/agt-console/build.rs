//! Embeds the Web console page (`web/public`) into the `agt` binary, so
//! `agt web` serves it from an installed binary without a repository checkout.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let public = manifest.join("../../web/public");
    println!("cargo:rerun-if-changed={}", public.display());

    let mut files = Vec::new();
    collect(&public, Path::new(""), &mut files);
    files.sort();

    let mut source = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for name in &files {
        let path = public.join(name);
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

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("web_assets.rs");
    fs::write(out, source).expect("write web_assets.rs");
}

fn collect(root: &Path, relative: &Path, files: &mut Vec<String>) {
    let entries = fs::read_dir(root.join(relative))
        .unwrap_or_else(|e| panic!("read {}: {e}", root.join(relative).display()));
    for entry in entries {
        let entry = entry.expect("web asset entry");
        let path = relative.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            collect(root, &path, files);
        } else {
            // Forward slashes keep the embedded names identical on every platform.
            files.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
}
