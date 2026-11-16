//! Prints the text Moss reads from files: `cargo run -p moss-extract --example read -- a.pdf b.docx`.

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let started = std::time::Instant::now();
        let text = moss_extract::extract(&bytes, &path, "", &moss_extract::mac::Mac);
        println!("== {path} ({:.1}s)\n{}", started.elapsed().as_secs_f64(), text.as_deref().unwrap_or("(Moss doesn't read this file)"));
    }
}
