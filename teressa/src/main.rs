fn main() {
    if let Err(error) = teressa::uci::run() {
        eprintln!("teressa: {error}");
        std::process::exit(1);
    }
}
