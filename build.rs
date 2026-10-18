//! Embed the launcher icon into the Windows executable. Other targets need nothing.

fn main() {
    println!("cargo:rerun-if-changed=assets/spotlight.rc");
    println!("cargo:rerun-if-changed=assets/spotlight.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/spotlight.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the Windows icon");
    }
}
