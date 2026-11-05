//! Telling whoever's at the PC what the phone just did with it. Waits for notify-send to hand it over.

pub async fn tell(icon: &str, title: &str, body: &str) {
    let sent = tokio::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", &format!("--icon={icon}"), title])
        .arg(escape(body))
        .status()
        .await;
    if !sent.is_ok_and(|s| s.success()) {
        tracing::warn!("couldn't show the notification \"{title}\"");
    }
}

/// Notification bodies are read as markup, and names come from the phone or from any file.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_cant_be_markup() {
        assert_eq!(super::escape("a & <b>.txt"), "a &amp; &lt;b&gt;.txt");
    }
}
