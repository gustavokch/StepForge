// Tell rustc this crate depends on CoreMIDI.framework. A staticlib performs no
// final link, so this mainly matters for host `cargo test` (rlib) and any Rust
// binary consuming the crate. The iOS app additionally links CoreMIDI.framework
// via its target's "Link Binary With Libraries" (design decision D8).
fn main() {
    println!("cargo:rustc-link-lib=framework=CoreMIDI");
    println!("cargo:rerun-if-changed=build.rs");
}
