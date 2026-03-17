// Build script for typechoai
// Fixes C++ linking issues with whisper-rs-sys

fn main() {
    // Tell Cargo to re-run this script if any of these change
    println!("cargo:rerun-if-changed=build.rs");

    // For Linux, we need to link the C++ standard library
    // This is typically needed for whisper-rs-sys
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
    }
}
