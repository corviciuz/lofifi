fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_embedded_data)");
    let path = std::path::Path::new("data/bandcamp_lofigirl.json.gz");
    if path.exists() {
        println!("cargo:rustc-cfg=has_embedded_data");
    }
    println!("cargo:rerun-if-changed=data/bandcamp_lofigirl.json.gz");
}
