//! Other programs run for an answer (gsettings, hyprctl, swww, xfconf-query...), with a deadline:
//! a helper stuck on a hung daemon (swww's, hyprpaper's, dconf's) must not hold up whoever asked,
//! often a watch that asks again in a few seconds.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// What a quick question to another program gets.
pub const QUICK: Duration = Duration::from_secs(3);

/// Runs [cmd] (no stdin, stderr dropped) for its output; None if it can't start or isn't done
/// within [limit] (then it's killed). Blocking: call it off the async runtime.
pub fn output(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read meanwhile: a full pipe would stall the program until the deadline.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                tracing::debug!("{:?} took too long; stopped", cmd.get_program());
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some(Output { status, stdout: reader.join().unwrap_or_default(), stderr: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_gives_up() {
        let out = output(Command::new("echo").arg("hi"), QUICK).expect("echo runs");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
        let began = Instant::now();
        assert!(output(Command::new("sleep").arg("5"), Duration::from_millis(200)).is_none());
        assert!(began.elapsed() < Duration::from_secs(2));
        assert!(output(&mut Command::new("vyoma-no-such-program"), QUICK).is_none());
    }
}
