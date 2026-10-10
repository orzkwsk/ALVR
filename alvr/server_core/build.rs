use std::{env, path::Path, process::Command};
fn main() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let result = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .expect("Git build identity required");
    assert!(result.status.success(), "Git build identity required");
    let commit = String::from_utf8(result.stdout).unwrap();
    let commit = commit.trim();
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "full source commit required"
    );
    println!("cargo:rustc-env=MONAKA_ALVR_BUILD_COMMIT={commit}");
    println!(
        "cargo:rerun-if-changed={}",
        root.join(".git/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join(".git/refs").display()
    );
}
