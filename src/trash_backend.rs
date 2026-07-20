use std::env;
use std::ffi::{OsStr, OsString};
use std::path::Path;

pub(crate) fn trash_command_for_current_exe() -> Result<String, String> {
    let executable =
        env::current_exe().map_err(|error| format!("cannot locate the hook binary: {error}"))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "the hook binary path is not valid Unicode".to_string())?;

    #[cfg(windows)]
    let executable = executable.replace('\\', "/");

    Ok(format!(
        "{} --trash --",
        quote_for_bash(OsStr::new(&executable))?
    ))
}

fn quote_for_bash(value: &OsStr) -> Result<String, String> {
    let value = value
        .to_str()
        .ok_or_else(|| "a shell path is not valid Unicode".to_string())?;
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    Ok(quoted)
}

pub(crate) fn trash_paths(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut paths: Vec<OsString> = arguments.into_iter().collect();
    if paths.first().is_some_and(|argument| argument == "--") {
        paths.remove(0);
    }
    if paths.is_empty() {
        return Err("--trash requires at least one path".to_string());
    }

    trash::delete_all(paths.iter().map(Path::new))
        .map_err(|error| format!("failed to move paths to the operating system Trash: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_metacharacters_without_exposing_them() {
        assert_eq!(
            quote_for_bash(OsStr::new("/tmp/a b/$c`d\"e\\f")).expect("valid path"),
            "\"/tmp/a b/\\$c\\`d\\\"e\\\\f\""
        );
    }

    #[test]
    fn rejects_an_empty_internal_trash_request() {
        assert_eq!(
            trash_paths(Vec::<OsString>::new()).unwrap_err(),
            "--trash requires at least one path"
        );
    }
}
