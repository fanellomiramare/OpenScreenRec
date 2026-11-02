fn main() {
    // Swift-linked macOS dependencies (ScreenCaptureKit) need the OS
    // Swift runtime on the rpath — see libs/capture/build.rs.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
