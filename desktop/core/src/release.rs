//! Vyoma's releases on GitHub, for the update checks: the newest one, and whether a version is
//! later than another. Asked with curl, which every Linux desktop, macOS and Windows 10/11 has.

use anyhow::{Context, Result};
use serde_json::Value;

pub const REPO: &str = "Prajwaljb/VyomaConnect";

/// The newest release: its version ("0.9.0"), its page, and the names of the files it carries.
#[derive(Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub assets: Vec<String>,
}

pub async fn latest() -> Result<Release> {
    let mut curl = tokio::process::Command::new("curl");
    curl.args(["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-A", "Vyoma"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .kill_on_drop(true);
    // Windows 10 and 11 have curl too; started from Vyoma's agent (no console of its own) it would
    // flash a console window without this.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        curl.creation_flags(CREATE_NO_WINDOW);
    }
    let out = curl.output().await.context("curl isn't installed")?;
    anyhow::ensure!(out.status.success(), "GitHub didn't answer ({})", out.status);
    parse(&serde_json::from_slice(&out.stdout)?)
}

fn parse(v: &Value) -> Result<Release> {
    let tag = v["tag_name"].as_str().context("no release")?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_owned(),
        page: v["html_url"].as_str().unwrap_or("").to_owned(),
        assets: v["assets"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect()).unwrap_or_default(),
    })
}

/// [a] is a later version than [b] ("0.10.0" after "0.9.1"); anything unreadable isn't.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>();
    match (parts(a), parts(b)) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0", "0.99.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("nightly", "0.3.0"));
    }

    #[test]
    fn reads_a_release() {
        let v = serde_json::json!({ "tag_name": "v0.9.0", "html_url": "https://x/r", "assets": [{ "name": "vyoma-0.9.0.apk" }, { "name": "Vyoma-mac-0.9.0.zip" }] });
        assert_eq!(parse(&v).unwrap(), Release { version: "0.9.0".into(), page: "https://x/r".into(), assets: vec!["vyoma-0.9.0.apk".into(), "Vyoma-mac-0.9.0.zip".into()] });
        assert!(parse(&serde_json::json!({ "message": "Not Found" })).is_err());
    }
}
