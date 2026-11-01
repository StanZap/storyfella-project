use std::process::Command;

fn main() {
    for path in ["ui-js/writing-diff.js", "package.json", "package-lock.json"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let status = Command::new("npm")
        .args(["run", "editor:build"])
        .status()
        .expect("Studio's diff editor needs Node.js and npm; run npm ci in apps/studio");
    assert!(
        status.success(),
        "Could not bundle Studio's diff editor; run npm ci in apps/studio"
    );
}
