fn main() {
    println!("cargo:rerun-if-changed=native/macos.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/macos.m")
            .flag("-fobjc-arc")
            .flag("-fobjc-arc-exceptions")
            .flag("-std=gnu11")
            .flag("-Wall")
            .flag("-Wextra")
            .flag("-Werror")
            .compile("discimage_macos");
        for framework in ["Foundation", "DiscRecording"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rustc-link-lib=objc");
    }
}
