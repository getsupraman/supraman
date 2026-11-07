use std::sync::{Arc, Mutex};
use std::time::Duration;

use supraman_core::askpass::{Asked, Bridge, Prompt, Reply, VAR};
use supraman_core::process::{Kill, command, run};

fn sm_askpass(spec: Option<&str>, prompt: &str) -> (Option<i32>, String) {
    let mut cmd = command(env!("CARGO_BIN_EXE_sm"));
    cmd.arg("askpass").arg(prompt).env_remove(VAR);
    if let Some(spec) = spec {
        cmd.env(VAR, spec);
    }
    let out = run(&mut cmd, Duration::from_secs(60), Kill::Tree).unwrap();
    (out.code, out.stdout)
}

#[test]
fn sm_askpass_prints_the_bridges_answer_to_an_ssh_prompt_and_refuses_gits_own() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let log = heard.clone();
    let bridge = Bridge::start(None, move |asked: Asked| {
        log.lock().unwrap().push(asked.prompt.clone());
        asked.answer(Some(Reply { text: "yes".into(), remember: false }));
    })
    .unwrap();
    let host = "The authenticity of host 'h' can't be established.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
    assert_eq!(sm_askpass(Some(&bridge.spec()), host), (Some(0), "yes\n".into()));
    assert_eq!(sm_askpass(Some(&bridge.spec()), "Username for 'https://example.invalid': ").0, Some(1));
    assert_eq!(sm_askpass(None, "Enter passphrase: ").0, Some(1), "no bridge, no answer");
    assert!(matches!(heard.lock().unwrap().as_slice(), [Prompt::HostKey { .. }]));
}
