fn main() {
    if let Err(error) = terastockfish::uci::run_stdio() {
        eprintln!("terastockfish: {error}");
        std::process::exit(1);
    }
}
