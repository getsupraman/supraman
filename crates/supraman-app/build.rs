fn main() {
    println!("cargo:rerun-if-changed=assets/supraman.rc");
    println!("cargo:rerun-if-changed=assets/supraman.ico");
    println!("cargo:rerun-if-changed=assets/tray-on-dark.ico");
    println!("cargo:rerun-if-changed=assets/tray-on-light.ico");
    println!("cargo:rerun-if-changed=assets/supraman.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/supraman.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
