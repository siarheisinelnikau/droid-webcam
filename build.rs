use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=DROID_CAMERA_SOURCE_DLL");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("camera-source.dll");
    if let Some(source) = env::var_os("DROID_CAMERA_SOURCE_DLL") {
        println!(
            "cargo:rerun-if-changed={}",
            PathBuf::from(&source).display()
        );
        fs::copy(source, output).expect("could not embed the Camera Frame Server DLL");
    } else {
        fs::write(output, []).expect("could not create the empty DLL placeholder");
    }
}
