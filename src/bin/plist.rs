// The same probe with Info.plist embedded in the binary (build.rs).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    photokit_probe::run("plist", &args);
}
