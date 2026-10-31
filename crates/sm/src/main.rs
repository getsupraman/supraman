fn main() -> std::process::ExitCode {
    sm::cli(std::env::args().skip(1).collect())
}
