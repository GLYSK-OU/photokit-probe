// Embed Info.plist into `photokit-probe-plist` only, so macOS has a usage description for a
// command-line tool that asks for Photos access.
fn main() {
    let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    println!("cargo:rustc-link-arg-bin=photokit-probe-plist=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
}
