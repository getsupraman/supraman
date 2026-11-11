use std::path::PathBuf;
use std::time::Duration;

use supraman_core::install::{self, Apply, Failed};

use crate::{Failure, value};

const WAIT: Duration = Duration::from_secs(60);

pub(crate) fn update(args: &[&str]) -> Result<(), Failure> {
    match args {
        ["apply", rest @ ..] => {
            let job = parse_apply(rest)?;
            let outcome = install::apply(&job);
            match outcome.result {
                Ok(()) => {
                    println!("applied {} over {}", job.from.display(), job.to.display());
                    Ok(())
                }
                Err(Failed::Stopped) => Err(Failure::Refused("error: stopped before the next move".into())),
                Err(Failed::Refused(why)) => Err(Failure::Refused(format!("error: {why}"))),
            }
        }
        _ => Err(Failure::Usage(format!("unrecognised arguments: update {}", args.join(" ")))),
    }
}

fn parse_apply(args: &[&str]) -> Result<Apply, Failure> {
    let (mut from, mut to, mut log, mut restart) = (None, None, None, None);
    let (mut wait_pid, mut wait, mut restart_args) = (None, WAIT, Vec::new());
    let mut it = args.iter();
    let number = |raw: &str, flag: &str| raw.parse::<u64>().map_err(|_| Failure::Usage(format!("{flag} must be a whole number, got '{raw}'")));
    while let Some(arg) = it.next() {
        match *arg {
            "--from" => from = Some(PathBuf::from(value(&mut it, arg)?)),
            "--to" => to = Some(PathBuf::from(value(&mut it, arg)?)),
            "--log" => log = Some(PathBuf::from(value(&mut it, arg)?)),
            "--restart" => restart = Some(PathBuf::from(value(&mut it, arg)?)),
            "--restart-arg" => restart_args.push(value(&mut it, arg)?.to_string()),
            "--wait-pid" => wait_pid = Some(number(value(&mut it, arg)?, arg)? as u32),
            "--wait-secs" => wait = Duration::from_secs(number(value(&mut it, arg)?, arg)?),
            other => return Err(Failure::Usage(format!("unexpected argument '{other}'"))),
        }
    }
    let need = |v: Option<PathBuf>, flag: &str| v.ok_or_else(|| Failure::Usage(format!("update apply needs {flag}")));
    Ok(Apply { from: need(from, "--from")?, to: need(to, "--to")?, wait_pid, wait, restart, restart_args, log: need(log, "--log")?, stop_after: None, one_rename: true })
}
