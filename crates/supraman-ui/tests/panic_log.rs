//! Its own process: the logger and the panic hook are global.

#[test]
fn a_panic_is_in_the_log_with_its_location_and_backtrace() {
    let dir = tempfile::tempdir().unwrap();
    supraman_ui::init_logger(Some(dir.path()));
    supraman_ui::log_panics();
    let line = line!() + 1;
    let caught = std::panic::catch_unwind(|| panic!("m5/68 probe"));
    assert!(caught.is_err());
    let log = std::fs::read_to_string(dir.path().join("foreman.log")).unwrap();
    assert!(log.contains("[ERROR foreman::panic] thread 'a_panic_is_in_the_log_with_its_location_and_backtrace' panicked at"), "{log}");
    assert!(log.contains(&format!("panic_log.rs:{line}:")), "{log}");
    assert!(log.contains("m5/68 probe"), "{log}");
    assert!(log.contains("backtrace:\n   0: ") && log.contains("core::panicking::panic_fmt"), "{log}");
    let frame = format!("panic_log.rs:{line}");
    assert!(log.lines().any(|l| l.trim_start().starts_with("at ") && l.ends_with(&frame)), "no frame at {frame}: {log}");
}
