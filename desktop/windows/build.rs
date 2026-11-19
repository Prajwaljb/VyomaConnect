// The icon (dist/vyoma.rc) goes into the program on Windows; nothing elsewhere.
fn main() {
    println!("cargo:rerun-if-changed=dist/vyoma.rc");
    println!("cargo:rerun-if-changed=dist/vyoma.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("dist/vyoma.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
