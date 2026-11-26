// Re-run the build when migrations change so sqlx::migrate! embeds the new files.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
