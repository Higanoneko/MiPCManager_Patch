fn main() {
    println!("cargo:rerun-if-changed=resources/mipcm_patch.rc");
    println!("cargo:rerun-if-changed=resources/mipcm_patch.exe.manifest");
    println!("cargo:rerun-if-changed=resources/mipcm_gui.rc");
    println!("cargo:rerun-if-changed=resources/mipcm_gui.exe.manifest");
    println!("cargo:rerun-if-changed=resources/mipcm_gui_test.rc");
    println!("cargo:rerun-if-changed=resources/mipcm_gui_test.exe.manifest");
    println!("cargo:rerun-if-changed=assets/MiPCManager.ico");

    // Cargo also builds the binaries as test harnesses during `cargo test`; embedding a
    // requireAdministrator manifest there would make ordinary test runs require UAC.
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }

    // CLI：仅 requireAdministrator。
    embed_resource::compile_for(
        "resources/mipcm_patch.rc",
        ["mipcm_patch"],
        embed_resource::NONE,
    )
    .manifest_required()
    .unwrap();

    // GUI：requireAdministrator + Common Controls v6（现代主题）。
    // 设置 MIPCM_SKIP_GUI_MANIFEST=1 可跳过（便于本机无 UAC 冒烟测试）。
    if std::env::var_os("MIPCM_SKIP_GUI_MANIFEST").is_none() {
        embed_resource::compile_for("resources/mipcm_gui.rc", ["mipcm_gui"], embed_resource::NONE)
            .manifest_required()
            .unwrap();
    } else {
        // 仍嵌入图标 + Common Controls v6，但不强制管理员，便于自动化冒烟。
        embed_resource::compile_for(
            "resources/mipcm_gui_test.rc",
            ["mipcm_gui"],
            embed_resource::NONE,
        )
        .manifest_required()
        .unwrap();
    }
}