fn main() {
    slint_build::compile("ui/app.slint").unwrap();
    // On Windows the program carries Vyoma's icon (the agent's resource file, desktop/windows/dist).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../windows/dist/vyoma.rc");
        println!("cargo:rerun-if-changed=../windows/dist/vyoma.ico");
        embed_resource::compile("../windows/dist/vyoma.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
