fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match storyfella_lsp::serve(stdin.lock(), stdout.lock()) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("storyfella-lsp: {error}");
            std::process::exit(1);
        }
    }
}
