use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let firmware = manifest.join("../../../firmware");
    let lvgl = firmware.join("managed_components/lvgl__lvgl");
    let mut sources: Vec<PathBuf> = glob::glob(lvgl.join("src/**/*.c").to_str().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    for core in [
        "timefmt.c",
        "clock_source.c",
        "template_fields.c",
        // Task 12: the runtime asset store, compiled unmodified so the
        // simulator's digest -> bytes lookup uses the identical format and
        // logic the device's link/asset_flash.c backs with real flash
        // I/O -- see csrc/sim_shim.c's RAM-backed asset_flash_io_t.
        "asset_store.c",
        // Task 8 (stage 2a): the scene model, its CBOR decoder and its
        // binding evaluator, compiled unmodified for the same reason
        // ui/scene_view.c below is -- the stage-2 parity gate compares a
        // device framebuffer against a simulator framebuffer, and that is
        // only meaningful while both run the same translation units. All
        // four are free of ESP-IDF includes by design (see ui/scene_view.h's
        // header comment), so any compile failure here is a violated
        // constraint, not a simulator problem.
        "scene_model.c",
        "scene_decode.c",
        "scene_binding.c",
    ] {
        sources.push(firmware.join("main/core").join(core));
    }
    // Task 12: free of ESP-IDF includes by design (see its own header
    // comment) specifically so it can compile into this host binary.
    sources.push(firmware.join("main/ui/font_registry.c"));
    // Task 8: the scene interpreter. Its header states the no-ESP-IDF rule
    // and names this simulator as the reason for it.
    sources.push(firmware.join("main/ui/scene_view.c"));
    for font in [
        "deskmate_font_18.c",
        "deskmate_font_28.c",
        "deskmate_font_56.c",
        "deskmate_font_96.c",
    ] {
        sources.push(firmware.join("main/ui/fonts").join(font));
    }
    sources.push(manifest.join("csrc/sim_shim.c"));

    let cbor = firmware.join("managed_components/espressif__cbor/tinycbor/src");
    for cbor_source in [
        "cborencoder.c",
        "cborencoder_close_container_checked.c",
        "cborparser.c",
        "cborparser_float.c",
        "cborvalidation.c",
    ] {
        sources.push(cbor.join(cbor_source));
    }

    let mut build = cc::Build::new();
    build
        .files(&sources)
        .include(&lvgl) // lvgl.h
        .include(firmware.join("main"))
        .include(&firmware) // lv_conf.h
        .include(&cbor)
        .define("LV_CONF_INCLUDE_SIMPLE", None)
        .define("CBOR_PARSER_MAX_RECURSIONS", "8")
        .flag_if_supported("-std=c11")
        .warnings(false); // LVGL's own sources are not our warning surface
    build.compile("deskmate_sim");

    println!(
        "cargo:rerun-if-changed={}",
        firmware.join("lv_conf.h").display()
    );
    println!("cargo:rerun-if-changed={}", firmware.join("main").display());
    println!("cargo:rerun-if-changed={}", manifest.join("csrc").display());
}
