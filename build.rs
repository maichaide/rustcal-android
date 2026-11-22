use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_donate_qr)");
    let qr_path = Path::new("assets/donate_qr.png");
    println!("cargo:rerun-if-changed=assets/donate_qr.png");
    if qr_path.exists() {
        println!("cargo:rustc-cfg=has_donate_qr");
    }
}
