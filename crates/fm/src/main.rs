fn main() -> std::process::ExitCode {
    fm::cli(std::env::args().skip(1).collect())
}
