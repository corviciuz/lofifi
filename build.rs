fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_embedded_data)");
    
    let mut found_path = None;
    if let Ok(entries) = std::fs::read_dir("data") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if (name.starts_with("bandcamp_presave_") || name.starts_with("bandcamp_lofigirl")) && name.ends_with(".json.gz") {
                found_path = Some(entry.path());
                break;
            }
        }
    }

    if let Some(path) = found_path {
        println!("cargo:rustc-cfg=has_embedded_data");
        let abs_path = std::fs::canonicalize(path).unwrap();
        let path_str = abs_path.to_str().unwrap().replace("\\", "/");
        println!("cargo:rustc-env=BANDCAMP_EMBEDDED_PATH={}", path_str);
    }

    println!("cargo:rerun-if-changed=data");
}
