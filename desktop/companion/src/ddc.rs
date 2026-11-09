//! External monitors' brightness over DDC/CI (`ddcutil`), for desktops that don't offer it
//! themselves: GNOME and the rest. Plasma does it in its own service, and two programs talking DDC
//! to one monitor at once confuse it, so this is never used there.
//!
//! Finding the monitors takes a second or two, so the list is kept for a minute; each value is a
//! quick read. The ids are `ddc_<i2c bus>`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// One monitor ddcutil can talk to.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub bus: u32,
    /// Its model, as it names itself.
    pub label: String,
    /// Its output as the desktop names it ("DP-2"), when ddcutil knows it.
    pub connector: Option<String>,
}

const KEEP: Duration = Duration::from_secs(60);
static FOUND: Mutex<Option<(Instant, Vec<Monitor>)>> = Mutex::new(None);

/// The monitors on this PC that take brightness over DDC.
pub async fn monitors() -> Vec<Monitor> {
    if let Some((at, list)) = FOUND.lock().unwrap().as_ref() {
        if at.elapsed() < KEEP {
            return list.clone();
        }
    }
    let list = match run(&["detect", "--brief"], Duration::from_secs(8)).await {
        Ok(out) => parse_detect(&out),
        Err(_) => Vec::new(),
    };
    *FOUND.lock().unwrap() = Some((Instant::now(), list.clone()));
    list
}

/// Its brightness as a fraction.
pub async fn brightness(bus: u32) -> Result<f64> {
    let out = run(&["--bus", &bus.to_string(), "getvcp", "10", "--terse"], Duration::from_secs(3)).await?;
    parse_vcp(&out).context("the monitor didn't say its brightness")
}

pub async fn set_brightness(bus: u32, level: f64) -> Result<()> {
    // Monitors count 0..100 for brightness. 0 is still lit (their dimmest), unlike a laptop panel.
    let value = (level.clamp(0.0, 1.0) * 100.0).round() as u32;
    run(&["--bus", &bus.to_string(), "--noverify", "setvcp", "10", &value.to_string()], Duration::from_secs(4)).await?;
    Ok(())
}

/// The output the monitor on [bus] is plugged into, from the last look.
pub fn connector(bus: u32) -> Option<String> {
    FOUND.lock().unwrap().as_ref()?.1.iter().find(|m| m.bus == bus)?.connector.clone()
}

pub fn id(bus: u32) -> String {
    format!("ddc_{bus}")
}

pub fn bus_of(id: &str) -> Option<u32> {
    id.strip_prefix("ddc_")?.parse().ok()
}

async fn run(args: &[&str], limit: Duration) -> Result<String> {
    let out = tokio::time::timeout(limit, tokio::process::Command::new("ddcutil").args(args).kill_on_drop(true).output())
        .await
        .context("ddcutil took too long")?
        .context("ddcutil isn't installed")?;
    if !out.status.success() {
        bail!("ddcutil: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `ddcutil detect --brief`: a "Display N" block per monitor that answers, with its bus and its
/// "Monitor: maker:model:serial" line; "Invalid display" blocks (a laptop's own panel) are skipped.
fn parse_detect(out: &str) -> Vec<Monitor> {
    let mut found = Vec::new();
    for block in out.split("\n\n") {
        let block = block.trim();
        if !block.starts_with("Display ") {
            continue;
        }
        let field = |name: &str| block.lines().find_map(|l| l.trim().strip_prefix(name).map(|v| v.trim().to_owned()));
        let Some(bus) = field("I2C bus:").and_then(|b| b.rsplit('-').next()?.parse().ok()) else { continue };
        let label = field("Monitor:").and_then(|m| m.split(':').nth(1).map(|s| s.trim().to_owned())).filter(|s| !s.is_empty()).unwrap_or_else(|| "Monitor".into());
        // "card1-DP-2": the card's part is the kernel's, the rest is what the desktop calls it.
        let connector = field("DRM connector:").and_then(|c| c.split_once('-').map(|(_, rest)| rest.to_owned()));
        found.push(Monitor { bus, label, connector });
    }
    found
}

/// `getvcp 10 --terse`: "VCP 10 C <current> <max>".
fn parse_vcp(out: &str) -> Option<f64> {
    let parts: Vec<&str> = out.split_whitespace().collect();
    match parts.as_slice() {
        ["VCP", "10", "C", current, max, ..] => {
            let (current, max) = (current.parse::<f64>().ok()?, max.parse::<f64>().ok()?);
            (max > 0.0).then(|| current / max)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_ddcutil_finds() {
        let out = "Invalid display\n   I2C bus:          /dev/i2c-10\n   DRM connector:    card1-eDP-1\n   Monitor:          AAA::\n\nDisplay 1\n   I2C bus:          /dev/i2c-12\n   DRM connector:    card1-DP-2\n   Monitor:          ACME:Big Screen 27:123\n\nDisplay 2\n   I2C bus:          /dev/i2c-13\n   Monitor:          ::\n";
        assert_eq!(parse_detect(out), [
                Monitor { bus: 12, label: "Big Screen 27".into(), connector: Some("DP-2".into()) },
                Monitor { bus: 13, label: "Monitor".into(), connector: None }
            ]);
        assert_eq!(parse_vcp("VCP 10 C 50 100\n"), Some(0.5));
        assert_eq!(parse_vcp("VCP 10 ERR\n"), None);
        assert_eq!((bus_of(&id(12)), bus_of("ddc_x"), bus_of("mutter_eDP_1")), (Some(12), None, None));
    }
}
