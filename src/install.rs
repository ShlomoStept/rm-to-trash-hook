use serde_json::{json, Map, Value};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
enum Client {
    Claude,
    Codex,
}

impl Client {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    fn matcher(self) -> &'static str {
        match self {
            Self::Claude => "Bash",
            Self::Codex => "^Bash$",
        }
    }

    fn config_path(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude/settings.json"),
            Self::Codex => home.join(".codex/hooks.json"),
        }
    }
}

#[derive(Default)]
struct ClientSelection {
    claude: bool,
    codex: bool,
    dry_run: bool,
}

impl ClientSelection {
    fn clients(&self) -> impl Iterator<Item = Client> {
        [
            self.claude.then_some(Client::Claude),
            self.codex.then_some(Client::Codex),
        ]
        .into_iter()
        .flatten()
    }
}

pub(crate) fn install(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let selection = parse_selection(arguments)?;
    let home = home_directory()?;
    let source = env::current_exe()
        .map_err(|error| format!("cannot locate the downloaded binary: {error}"))?;
    let destination = installed_binary_path(&home);

    if selection.dry_run {
        println!(
            "Would install {} to {}",
            source.display(),
            destination.display()
        );
    } else {
        install_binary(&source, &destination)?;
        println!("Installed binary at {}", destination.display());
    }

    for client in selection.clients() {
        let path = client.config_path(&home);
        let outcome = update_config_file(
            &path,
            client,
            &destination,
            ConfigAction::Install,
            selection.dry_run,
        )?;
        println!("{}: {outcome}", client.name());
    }

    if !selection.dry_run {
        println!("Restart the client. In Codex, open /hooks and trust the new hook definition.");
    }
    Ok(())
}

pub(crate) fn uninstall(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let selection = parse_selection(arguments)?;
    let home = home_directory()?;
    let destination = installed_binary_path(&home);

    for client in selection.clients() {
        let path = client.config_path(&home);
        let outcome = update_config_file(
            &path,
            client,
            &destination,
            ConfigAction::Uninstall,
            selection.dry_run,
        )?;
        println!("{}: {outcome}", client.name());
    }

    if !selection.dry_run {
        println!(
            "Configuration removed. The binary remains at {} so uninstall works consistently on Windows; delete it after this process exits if desired.",
            destination.display()
        );
    }
    Ok(())
}

pub(crate) fn doctor(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let selection = parse_selection(arguments)?;
    if selection.dry_run {
        return Err("doctor does not accept --dry-run".to_string());
    }
    let home = home_directory()?;
    let destination = installed_binary_path(&home);
    let mut healthy = destination.is_file();

    println!(
        "Binary: {} ({})",
        destination.display(),
        if destination.is_file() {
            "present"
        } else {
            "missing"
        }
    );

    for client in selection.clients() {
        let path = client.config_path(&home);
        let installed = config_contains_handler(&path)?;
        healthy &= installed;
        println!(
            "{}: {} ({})",
            client.name(),
            path.display(),
            if installed {
                "handler present"
            } else {
                "handler missing"
            }
        );
    }

    if healthy {
        Ok(())
    } else {
        Err("installation is incomplete; run rm-to-trash install".to_string())
    }
}

fn parse_selection(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<ClientSelection, String> {
    let mut selection = ClientSelection::default();
    for argument in arguments {
        match argument.to_str() {
            Some("--claude") => selection.claude = true,
            Some("--codex") => selection.codex = true,
            Some("--dry-run") => selection.dry_run = true,
            Some(value) => {
                return Err(format!(
                    "unknown installer argument {value:?}; expected --claude, --codex, or --dry-run"
                ));
            }
            None => return Err("installer arguments must be valid Unicode".to_string()),
        }
    }
    if !selection.claude && !selection.codex {
        selection.claude = true;
        selection.codex = true;
    }
    Ok(selection)
}

fn home_directory() -> Result<PathBuf, String> {
    #[cfg(windows)]
    const HOME_VARIABLE: &str = "USERPROFILE";
    #[cfg(not(windows))]
    const HOME_VARIABLE: &str = "HOME";

    env::var_os(HOME_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{HOME_VARIABLE} is not set"))
}

fn installed_binary_path(home: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "rm-to-trash.exe"
    } else {
        "rm-to-trash"
    };
    home.join(".local/share/rm-to-trash/bin").join(name)
}

fn install_binary(source: &Path, destination: &Path) -> Result<(), String> {
    if source == destination {
        return Ok(());
    }
    if files_equal(source, destination)? {
        return Ok(());
    }

    let parent = destination
        .parent()
        .ok_or_else(|| "installed binary has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;

    let temporary = temporary_path(destination, "new");
    fs::copy(source, &temporary).map_err(|error| {
        format!(
            "cannot copy {} to {}: {error}",
            source.display(),
            temporary.display()
        )
    })?;
    replace_file(&temporary, destination)?;

    if !files_equal(source, destination)? {
        return Err("installed binary does not match the downloaded binary".to_string());
    }
    Ok(())
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, String> {
    if !right.exists() {
        return Ok(false);
    }
    let left_metadata = fs::metadata(left)
        .map_err(|error| format!("cannot inspect {}: {error}", left.display()))?;
    let right_metadata = fs::metadata(right)
        .map_err(|error| format!("cannot inspect {}: {error}", right.display()))?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file =
        File::open(left).map_err(|error| format!("cannot read {}: {error}", left.display()))?;
    let mut right_file =
        File::open(right).map_err(|error| format!("cannot read {}: {error}", right.display()))?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_count = left_file
            .read(&mut left_buffer)
            .map_err(|error| format!("cannot read {}: {error}", left.display()))?;
        let right_count = right_file
            .read(&mut right_buffer)
            .map_err(|error| format!("cannot read {}: {error}", right.display()))?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

#[derive(Clone, Copy)]
enum ConfigAction {
    Install,
    Uninstall,
}

fn update_config_file(
    path: &Path,
    client: Client,
    binary: &Path,
    action: ConfigAction,
    dry_run: bool,
) -> Result<String, String> {
    if matches!(action, ConfigAction::Uninstall) && !path.exists() {
        return Ok("configuration was already absent".to_string());
    }

    let original = if path.exists() {
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?
    } else {
        b"{}\n".to_vec()
    };
    let mut document: Value = serde_json::from_slice(&original)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    let before = document.clone();
    update_config(&mut document, client, binary, action)?;

    if document == before {
        return Ok(match action {
            ConfigAction::Install => "handler already current".to_string(),
            ConfigAction::Uninstall => "handler was already absent".to_string(),
        });
    }
    if dry_run {
        return Ok(match action {
            ConfigAction::Install => format!("would update {}", path.display()),
            ConfigAction::Uninstall => format!("would remove handler from {}", path.display()),
        });
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;

    let backup = path.exists().then(|| backup_path(path));
    if let Some(backup) = &backup {
        fs::copy(path, backup).map_err(|error| {
            format!(
                "cannot back up {} to {}: {error}",
                path.display(),
                backup.display()
            )
        })?;
    }

    let mut updated = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("cannot serialize {}: {error}", path.display()))?;
    updated.push(b'\n');
    let temporary = temporary_path(path, "new");
    write_private_file(&temporary, &updated, path.exists().then_some(path))?;
    replace_file(&temporary, path)?;

    Ok(match backup {
        Some(backup) => format!("updated {}; backup: {}", path.display(), backup.display()),
        None => format!("created {}", path.display()),
    })
}

fn update_config(
    document: &mut Value,
    client: Client,
    binary: &Path,
    action: ConfigAction,
) -> Result<(), String> {
    let root = document
        .as_object_mut()
        .ok_or_else(|| "configuration root must be a JSON object".to_string())?;

    if matches!(action, ConfigAction::Uninstall) {
        let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        let Some(groups) = hooks.get_mut("PreToolUse").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        remove_handlers(groups)?;
        return Ok(());
    }

    let hooks = object_entry(root, "hooks")?;
    let groups = array_entry(hooks, "PreToolUse")?;
    remove_handlers(groups)?;

    let group_index = groups
        .iter()
        .position(|group| group.get("matcher").and_then(Value::as_str) == Some(client.matcher()));
    let handler = desired_handler(client, binary);

    if let Some(index) = group_index {
        let group = groups[index]
            .as_object_mut()
            .ok_or_else(|| "PreToolUse matcher group must be a JSON object".to_string())?;
        array_entry(group, "hooks")?.push(handler);
    } else {
        groups.push(json!({
            "matcher": client.matcher(),
            "hooks": [handler]
        }));
    }
    Ok(())
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| format!("{key} must be a JSON object"))
}

fn array_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Vec<Value>, String> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    value
        .as_array_mut()
        .ok_or_else(|| format!("{key} must be a JSON array"))
}

fn remove_handlers(groups: &mut Vec<Value>) -> Result<(), String> {
    for group in groups {
        let group = group
            .as_object_mut()
            .ok_or_else(|| "PreToolUse matcher group must be a JSON object".to_string())?;
        let Some(handlers) = group.get_mut("hooks") else {
            continue;
        };
        let handlers = handlers
            .as_array_mut()
            .ok_or_else(|| "matcher-group hooks must be a JSON array".to_string())?;
        handlers.retain(|handler| !is_rm_to_trash_handler(handler));
    }
    Ok(())
}

fn is_rm_to_trash_handler(handler: &Value) -> bool {
    let Some(handler) = handler.as_object() else {
        return false;
    };
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains("rm-to-trash"))
}

fn desired_handler(client: Client, binary: &Path) -> Value {
    let binary = binary.to_string_lossy();
    match client {
        Client::Claude => json!({
            "type": "command",
            "command": binary,
            "args": ["hook"],
            "timeout": 10,
            "statusMessage": "Redirecting rm to the operating system Trash"
        }),
        Client::Codex => {
            let command = quote_command_path(&binary);
            let mut handler = json!({
                "type": "command",
                "command": format!("{command} hook"),
                "timeout": 10,
                "statusMessage": "Redirecting rm to the operating system Trash"
            });
            if cfg!(windows) {
                handler["commandWindows"] =
                    Value::String(format!("& {} hook", quote_powershell_path(&binary)));
            }
            handler
        }
    }
}

fn quote_command_path(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for character in path.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

fn quote_powershell_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "''"))
}

fn config_contains_handler(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let document: Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    Ok(document
        .get("hooks")
        .and_then(|hooks| hooks.get("PreToolUse"))
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| handlers.iter().any(is_rm_to_trash_handler))
            })
        }))
}

fn write_private_file(
    path: &Path,
    contents: &[u8],
    permissions_from: Option<&Path>,
) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    if let Some(original) = permissions_from {
        let permissions = fs::metadata(original)
            .map_err(|error| format!("cannot inspect {}: {error}", original.display()))?
            .permissions();
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("cannot set permissions on {}: {error}", path.display()))?;
    }
    Ok(())
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination).map_err(|error| {
            format!(
                "cannot replace {} with {}: {error}",
                destination.display(),
                temporary.display()
            )
        })
    }

    #[cfg(windows)]
    {
        let old = temporary_path(destination, "old");
        if destination.exists() {
            fs::rename(destination, &old).map_err(|error| {
                format!(
                    "cannot prepare {} for replacement: {error}",
                    destination.display()
                )
            })?;
        }
        if let Err(error) = fs::rename(temporary, destination) {
            if old.exists() {
                let _ = fs::rename(&old, destination);
            }
            return Err(format!("cannot replace {}: {error}", destination.display()));
        }
        if old.exists() {
            fs::remove_file(&old)
                .map_err(|error| format!("cannot remove {}: {error}", old.display()))?;
        }
        Ok(())
    }
}

fn temporary_path(path: &Path, suffix: &str) -> PathBuf {
    let pid = std::process::id();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rm-to-trash");
    path.with_file_name(format!(".{name}.{pid}.{stamp}.{suffix}"))
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    path.with_file_name(format!("{name}.rm-to-trash.backup.{stamp}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\Test User\.local\share\rm-to-trash\bin\rm-to-trash.exe")
        } else {
            PathBuf::from("/home/test user/.local/share/rm-to-trash/bin/rm-to-trash")
        }
    }

    #[test]
    fn installer_preserves_unrelated_settings_and_is_idempotent() {
        let mut document = json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{
                        "type": "command",
                        "command": "/opt/check-policy",
                        "timeout": 30
                    }]
                }]
            }
        });
        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Install,
        )
        .expect("first install");
        let once = document.clone();
        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Install,
        )
        .expect("second install");

        assert_eq!(document, once);
        assert_eq!(document["theme"], "dark");
        let handlers = document["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .expect("handlers");
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0]["command"], "/opt/check-policy");
    }

    #[test]
    fn installer_replaces_a_legacy_handler_and_uninstalls_only_its_own() {
        let mut document = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "~/.claude/hooks/rm-to-trash/rm-to-trash"
                        },
                        {
                            "type": "command",
                            "command": "/opt/keep-me"
                        }
                    ]
                }]
            }
        });
        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Install,
        )
        .expect("install");
        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Uninstall,
        )
        .expect("uninstall");

        let handlers = document["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .expect("handlers");
        assert_eq!(
            handlers,
            &[json!({"type": "command", "command": "/opt/keep-me"})]
        );
    }

    #[test]
    fn installer_preserves_preexisting_empty_matcher_groups() {
        let mut document = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Read",
                        "hooks": []
                    },
                    {
                        "matcher": "Bash",
                        "hooks": []
                    }
                ]
            }
        });

        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Install,
        )
        .expect("install");
        update_config(
            &mut document,
            Client::Claude,
            &binary(),
            ConfigAction::Uninstall,
        )
        .expect("uninstall");

        assert_eq!(
            document["hooks"]["PreToolUse"],
            json!([
                {
                    "matcher": "Read",
                    "hooks": []
                },
                {
                    "matcher": "Bash",
                    "hooks": []
                }
            ])
        );
    }

    #[cfg(windows)]
    #[test]
    fn codex_windows_command_uses_the_powershell_call_operator() {
        let handler = desired_handler(Client::Codex, &binary());
        assert_eq!(
            handler["commandWindows"],
            "& 'C:\\Users\\Test User\\.local\\share\\rm-to-trash\\bin\\rm-to-trash.exe' hook"
        );
    }

    #[test]
    fn installer_rejects_malformed_configuration_shapes() {
        let mut document = json!({"hooks": []});
        let result = update_config(
            &mut document,
            Client::Codex,
            &binary(),
            ConfigAction::Install,
        );
        assert_eq!(result.unwrap_err(), "hooks must be a JSON object");
    }

    #[test]
    fn client_selection_defaults_to_both_clients() {
        let selection = parse_selection(Vec::<OsString>::new()).expect("valid defaults");
        assert!(selection.claude);
        assert!(selection.codex);
        assert!(!selection.dry_run);
    }
}
