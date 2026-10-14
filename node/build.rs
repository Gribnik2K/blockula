fn main() {
    // Embed the git commit (if built from a checkout) for /api/status and /metrics.
    if let Ok(out) = std::process::Command::new("git").args(["rev-parse", "--short=12", "HEAD"]).output() {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            println!("cargo:rustc-env=BLOCKULA_GIT_SHA={sha}");
        }
    }
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-env-changed=BLOCKULA_GIT_SHA");
}
