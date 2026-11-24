// Edit-back probe: changes only the single photo in the album "LC-probe".
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    photokit_probe::edit::run(&args);
}
