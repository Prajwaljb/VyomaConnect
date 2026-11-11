//! "Send to Phone" in Finder: a Quick Action (right-click > Quick Actions, and the Services menu)
//! that runs `vyoma-mac --send-any` with the selected files and folders. A Share-menu extension
//! needs an app signed by Apple; a Quick Action is a small Automator workflow in
//! ~/Library/Services that Finder picks up by itself. Written (again) whenever Vyoma.app starts
//! from somewhere new, so it follows the app when it's moved; `--uninstall` removes it.
//!
//! The workflow's layout is the one Automator saves for a "Run Shell Script" Quick Action that
//! takes files in Finder.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const NAME: &str = "Send to Phone";

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn workflow_dir(home: &Path) -> PathBuf {
    home.join("Library/Services").join(format!("{NAME}.workflow"))
}

/// Puts the Quick Action in place for the helper at [exe] (inside Vyoma.app). True if it changed.
pub fn install(exe: &Path) -> Result<bool> {
    install_in(&home(), exe)
}

fn install_in(home: &Path, exe: &Path) -> Result<bool> {
    let contents = workflow_dir(home).join("Contents");
    let info = info_plist();
    let document = document(exe);
    let same = |name: &str, text: &str| std::fs::read_to_string(contents.join(name)).is_ok_and(|t| t == text);
    if same("Info.plist", &info) && same("document.wflow", &document) {
        return Ok(false);
    }
    std::fs::create_dir_all(&contents).with_context(|| format!("couldn't make {}", contents.display()))?;
    std::fs::write(contents.join("Info.plist"), info)?;
    std::fs::write(contents.join("document.wflow"), document)?;
    refresh();
    Ok(true)
}

pub fn remove() {
    remove_in(&home());
}

fn remove_in(home: &Path) {
    let dir = workflow_dir(home);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        refresh();
    }
}

/// Tells macOS's services list to look again (it also notices by itself, a little later).
fn refresh() {
    let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

/// Not where macOS runs an app straight from a download (App Translocation): that path changes on
/// every start, so a Quick Action pointing there would stop working.
pub fn lasting(app: &Path) -> bool {
    !app.to_string_lossy().contains("/AppTranslocation/")
}

/// The shell line: the helper's path quoted for the shell, then the files.
fn command(exe: &Path) -> String {
    let quoted = format!("'{}'", exe.to_string_lossy().replace('\'', r"'\''"));
    format!("exec {quoted} --send-any \"$@\"")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>dev.vyoma.Vyoma.send</string>
	<key>CFBundleName</key>
	<string>{NAME}</string>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{NAME}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.item</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#
    )
}

fn document(exe: &Path) -> String {
    let command = xml(&command(exe));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>509</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>1.1.1</string>
				<key>AMApplication</key>
				<array>
					<string>Finder</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{command}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/zsh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>6C1F7A2E-3B8D-4E51-9A0C-2F4D8B7E1A93</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
				</array>
				<key>OutputUUID</key>
				<string>A84E2D19-7C3F-4B6A-8E05-D1B9F3C27E64</string>
				<key>UUID</key>
				<string>3F9B6C81-2A4E-4D7F-B153-8E6A0C9D4F27</string>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceApplicationBundleID</key>
		<string>com.apple.finder</string>
		<key>serviceApplicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_survives_any_app_path() {
        let exe = Path::new("/Users/sam/Apps & Tools/Sam's \"Vyoma\".app/Contents/MacOS/vyoma-mac");
        assert_eq!(command(exe), r#"exec '/Users/sam/Apps & Tools/Sam'\''s "Vyoma".app/Contents/MacOS/vyoma-mac' --send-any "$@""#);
        let doc = document(exe);
        assert!(doc.contains(r#"<string>exec '/Users/sam/Apps &amp; Tools/Sam'\''s &quot;Vyoma&quot;.app/Contents/MacOS/vyoma-mac' --send-any &quot;$@&quot;</string>"#));
        assert!(lasting(Path::new("/Applications/Vyoma.app")));
        assert!(!lasting(Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Vyoma.app")));
    }

    #[test]
    fn writes_once_and_follows_the_app() {
        let home = std::env::temp_dir().join(format!("vyoma-finder-{}", std::process::id()));
        let at = |p: &str| PathBuf::from(p).join("Contents/MacOS/vyoma-mac");
        assert!(install_in(&home, &at("/Applications/Vyoma.app")).unwrap());
        assert!(!install_in(&home, &at("/Applications/Vyoma.app")).unwrap(), "unchanged");
        assert!(install_in(&home, &at("/Users/sam/Applications/Vyoma.app")).unwrap(), "moved");
        let doc = std::fs::read_to_string(workflow_dir(&home).join("Contents/document.wflow")).unwrap();
        assert!(doc.contains("/Users/sam/Applications/Vyoma.app/Contents/MacOS/vyoma-mac"));
        remove_in(&home);
        assert!(!workflow_dir(&home).exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
