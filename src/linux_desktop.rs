use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutInstallResult {
    Installed,
    AlreadyInstalled {
        binding: Option<String>,
    },
    BindingConflict {
        binding: String,
        shortcut_name: Option<String>,
    },
    UnsupportedEnvironment,
    GsettingsUnavailable,
}

#[cfg(target_os = "linux")]
const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
#[cfg(target_os = "linux")]
const CUSTOM_KEYBINDING_SCHEMA: &str =
    "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
#[cfg(target_os = "linux")]
const CUSTOM_KEYBINDING_BASE: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
#[cfg(target_os = "linux")]
const ECHOPUP_SHORTCUT_NAME: &str = "EchoPup Toggle Recording";

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct ShortcutEntry {
    path: String,
    name: Option<String>,
    command: Option<String>,
    binding: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutConflict {
    pub name: Option<String>,
    pub command: Option<String>,
    pub binding: String,
}

#[cfg(target_os = "linux")]
pub fn maybe_install_gnome_wayland_shortcut(
    command_line: &str,
    binding: &str,
) -> Result<ShortcutInstallResult> {
    if !is_gnome_wayland_session() {
        return Ok(ShortcutInstallResult::UnsupportedEnvironment);
    }
    if !gsettings_available()? {
        return Ok(ShortcutInstallResult::GsettingsUnavailable);
    }

    let entries = load_shortcut_entries()?;
    if let Some(entry) = entries.iter().find(|entry| {
        entry.command.as_deref() == Some(command_line)
            || entry.name.as_deref() == Some(ECHOPUP_SHORTCUT_NAME)
    }) {
        return Ok(ShortcutInstallResult::AlreadyInstalled {
            binding: entry.binding.clone(),
        });
    }

    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.binding.as_deref() == Some(binding))
    {
        return Ok(ShortcutInstallResult::BindingConflict {
            binding: binding.to_string(),
            shortcut_name: entry.name.clone(),
        });
    }

    let mut paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    let new_path = next_custom_binding_path(&paths);
    paths.push(new_path.clone());
    set_custom_binding_paths(&paths)?;
    set_shortcut_value(&new_path, "name", ECHOPUP_SHORTCUT_NAME)?;
    set_shortcut_value(&new_path, "command", command_line)?;
    set_shortcut_value(&new_path, "binding", binding)?;

    Ok(ShortcutInstallResult::Installed)
}

#[cfg(not(target_os = "linux"))]
pub fn maybe_install_gnome_wayland_shortcut(
    _command_line: &str,
    _binding: &str,
) -> Result<ShortcutInstallResult> {
    Ok(ShortcutInstallResult::UnsupportedEnvironment)
}

#[cfg(target_os = "linux")]
pub fn find_echopup_shortcut_conflict(binding: &str) -> Result<Option<ShortcutConflict>> {
    if !gsettings_available()? {
        return Ok(None);
    }

    let entries = load_shortcut_entries()?;
    Ok(
        find_matching_echopup_shortcut(&entries, binding).map(|entry| ShortcutConflict {
            name: entry.name.clone(),
            command: entry.command.clone(),
            binding: binding.to_string(),
        }),
    )
}

#[cfg(not(target_os = "linux"))]
pub fn find_echopup_shortcut_conflict(_binding: &str) -> Result<Option<ShortcutConflict>> {
    Ok(None)
}

#[cfg(target_os = "linux")]
fn is_gnome_wayland_session() -> bool {
    let is_wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);
    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is_gnome = current_desktop
        .split(':')
        .any(|part| part.eq_ignore_ascii_case("gnome") || part.eq_ignore_ascii_case("ubuntu"));
    is_wayland && is_gnome
}

#[cfg(target_os = "linux")]
fn gsettings_available() -> Result<bool> {
    let output = std::process::Command::new("gsettings")
        .arg("writable")
        .arg(MEDIA_KEYS_SCHEMA)
        .arg("custom-keybindings")
        .output()
        .context("执行 gsettings writable 失败")?;

    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}

#[cfg(target_os = "linux")]
fn load_shortcut_entries() -> Result<Vec<ShortcutEntry>> {
    let paths = get_custom_binding_paths()?;
    paths
        .into_iter()
        .map(|path| {
            let name = get_shortcut_value(&path, "name")?;
            let command = get_shortcut_value(&path, "command")?;
            let binding = get_shortcut_value(&path, "binding")?;
            Ok(ShortcutEntry {
                path,
                name: empty_to_none(name),
                command: empty_to_none(command),
                binding: empty_to_none(binding),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn find_matching_echopup_shortcut<'a>(
    entries: &'a [ShortcutEntry],
    binding: &str,
) -> Option<&'a ShortcutEntry> {
    entries.iter().find(|entry| {
        entry.binding.as_deref() == Some(binding)
            && (entry.name.as_deref() == Some(ECHOPUP_SHORTCUT_NAME)
                || looks_like_echopup_trigger_command(entry.command.as_deref()))
    })
}

#[cfg(target_os = "linux")]
fn looks_like_echopup_trigger_command(command: Option<&str>) -> bool {
    let Some(command) = command else {
        return false;
    };
    let command = command.to_ascii_lowercase();
    command.contains("echopup") && command.contains("trigger") && command.contains("toggle")
}

#[cfg(target_os = "linux")]
fn get_custom_binding_paths() -> Result<Vec<String>> {
    let raw = run_gsettings(&["get", MEDIA_KEYS_SCHEMA, "custom-keybindings"])?;
    Ok(parse_gsettings_string_array(&raw))
}

#[cfg(target_os = "linux")]
fn set_custom_binding_paths(paths: &[String]) -> Result<()> {
    let value = format!(
        "[{}]",
        paths
            .iter()
            .map(|path| format!("'{}'", path))
            .collect::<Vec<_>>()
            .join(", ")
    );
    run_gsettings(&["set", MEDIA_KEYS_SCHEMA, "custom-keybindings", &value]).map(|_| ())
}

#[cfg(target_os = "linux")]
fn get_shortcut_value(path: &str, key: &str) -> Result<String> {
    run_gsettings(&["get", &schema_with_path(path), key]).map(|value| trim_gsettings_value(&value))
}

#[cfg(target_os = "linux")]
fn set_shortcut_value(path: &str, key: &str, value: &str) -> Result<()> {
    run_gsettings(&["set", &schema_with_path(path), key, value]).map(|_| ())
}

#[cfg(target_os = "linux")]
fn schema_with_path(path: &str) -> String {
    format!("{}:{}", CUSTOM_KEYBINDING_SCHEMA, path)
}

#[cfg(target_os = "linux")]
fn run_gsettings(args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .with_context(|| format!("执行 gsettings 失败: {:?}", args))?;
    if !output.status.success() {
        anyhow::bail!(
            "gsettings 执行失败: {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn parse_gsettings_string_array(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_string = false;

    for ch in raw.chars() {
        match ch {
            '\'' if in_string => {
                values.push(current.clone());
                current.clear();
                in_string = false;
            }
            '\'' => {
                in_string = true;
            }
            _ if in_string => current.push(ch),
            _ => {}
        }
    }

    values
}

#[cfg(target_os = "linux")]
fn trim_gsettings_value(raw: &str) -> String {
    raw.trim().trim_matches('\'').to_string()
}

#[cfg(target_os = "linux")]
fn empty_to_none(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("disabled") {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(target_os = "linux")]
fn next_custom_binding_path(existing_paths: &[String]) -> String {
    let next_index = existing_paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix(CUSTOM_KEYBINDING_BASE)?
                .strip_suffix('/')?
                .strip_prefix("custom")?
                .parse::<usize>()
                .ok()
        })
        .max()
        .map(|index| index + 1)
        .unwrap_or(0);

    format!("{}custom{}/", CUSTOM_KEYBINDING_BASE, next_index)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::{
        find_matching_echopup_shortcut, looks_like_echopup_trigger_command,
        next_custom_binding_path, parse_gsettings_string_array, ShortcutEntry,
    };

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_gsettings_string_array() {
        assert_eq!(
            parse_gsettings_string_array(
                "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']"
            ),
            vec!["/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/"]
        );
        assert!(parse_gsettings_string_array("@as []").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_next_custom_binding_path() {
        let next = next_custom_binding_path(&[
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/".to_string(),
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom2/".to_string(),
        ]);
        assert_eq!(
            next,
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom3/"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_looks_like_echopup_trigger_command() {
        assert!(looks_like_echopup_trigger_command(Some(
            "/tmp/echopup --config ~/.echopup/config.toml trigger toggle"
        )));
        assert!(!looks_like_echopup_trigger_command(Some("/usr/bin/true")));
        assert!(!looks_like_echopup_trigger_command(None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_find_matching_echopup_shortcut() {
        let entries = vec![
            ShortcutEntry {
                path: "/org/example/custom0/".to_string(),
                name: Some("Other Shortcut".to_string()),
                command: Some("/usr/bin/true".to_string()),
                binding: Some("F5".to_string()),
            },
            ShortcutEntry {
                path: "/org/example/custom1/".to_string(),
                name: Some("EchoPup Toggle Recording".to_string()),
                command: Some(
                    "/tmp/echopup --config ~/.echopup/config.toml trigger toggle".to_string(),
                ),
                binding: Some("F6".to_string()),
            },
        ];

        let matched = find_matching_echopup_shortcut(&entries, "F6");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name.as_deref(),
            Some("EchoPup Toggle Recording")
        );
        assert!(find_matching_echopup_shortcut(&entries, "F7").is_none());
    }
}
