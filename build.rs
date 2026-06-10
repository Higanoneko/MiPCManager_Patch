fn main() {
    println!("cargo:rerun-if-changed=resources/mipcm_patch.rc");
    println!("cargo:rerun-if-changed=resources/mipcm_patch.exe.manifest");

    // Cargo also builds the binary as a test harness during `cargo test`; embedding a
    // requireAdministrator manifest there would make ordinary test runs require UAC.
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }

    embed_resource::compile_for(
        "resources/mipcm_patch.rc",
        ["mipcm_patch"],
        embed_resource::NONE,
    )
    .manifest_required()
    .unwrap();
}
