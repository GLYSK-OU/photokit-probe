// Plain command-line tool (also the executable inside PhotoKitProbe.app).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // inside an .app bundle the report goes to its own file
    let variant = if std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")) { "app" } else { "plain" };
    photokit_probe::run(variant, &args);
}
