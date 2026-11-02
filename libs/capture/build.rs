fn main() {
    // The macOS ScreenCaptureKit dependencies are Swift-bridged: binaries
    // that link them need the OS Swift runtime directory on their rpath,
    // or dyld aborts with "Library not loaded: @rpath/libswift_Concurrency.dylib"
    // when the tests or the app run outside Xcode. The runtime ships with
    // macOS 12+, so this also holds for end users of release builds.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
