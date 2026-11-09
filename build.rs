fn main() {
    println!("cargo:rerun-if-changed=native/windows.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cc::Build::new()
            .cpp(true)
            .file("native/windows.cpp")
            .flag_if_supported("/std:c++17")
            .flag_if_supported("/EHsc")
            .warnings_into_errors(true)
            .compile("discimage_windows");
        for library in ["ole32", "oleaut32", "uuid"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    println!("cargo:rerun-if-changed=native/macos.m");
    println!("cargo:rerun-if-changed=native/probe.c");
    println!("cargo:rerun-if-changed=native/registry.h");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .files(["native/macos.m", "native/probe.c"])
            .flag("-fobjc-arc")
            .flag("-fobjc-arc-exceptions")
            .flag("-std=gnu11")
            .flag("-Wall")
            .flag("-Wextra")
            .flag("-Werror")
            .compile("discimage_macos");
        for framework in ["Foundation", "CoreFoundation", "DiscRecording", "IOKit"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rustc-link-lib=objc");
    }
}
