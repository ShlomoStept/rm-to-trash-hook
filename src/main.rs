mod install;
mod rewrite;
mod trash_backend;

use rewrite::{has_rm_candidate, rewrite_rm_to_trash};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
use std::ffi::OsString;
use std::io::{self, Read};

#[derive(Deserialize)]
struct HookInput {
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<Map<String, Value>>,
}

#[derive(Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: &'static str,
    #[serde(rename = "updatedInput")]
    updated_input: Map<String, Value>,
}

fn run(input: HookInput) -> Result<Option<HookOutput>, String> {
    run_with_trash_command(input, trash_backend::trash_command_for_current_exe)
}

fn run_with_trash_command<F>(
    input: HookInput,
    trash_command: F,
) -> Result<Option<HookOutput>, String>
where
    F: FnOnce() -> Result<String, String>,
{
    if input.hook_event_name.as_deref() != Some("PreToolUse")
        || input.tool_name.as_deref() != Some("Bash")
    {
        return Ok(None);
    }

    let mut tool_input = input.tool_input.unwrap_or_default();
    let Some(command) = tool_input.get("command").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !has_rm_candidate(command) {
        return Ok(None);
    }

    let trash_command = trash_command()?;
    let Some(rewritten) = rewrite_rm_to_trash(command, &trash_command) else {
        return Ok(None);
    };

    tool_input.insert("command".to_string(), Value::String(rewritten));
    Ok(Some(HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "allow",
            permission_decision_reason: "rm redirected to the operating system Trash",
            updated_input: tool_input,
        },
    }))
}

fn run_hook() -> Result<(), String> {
    let mut raw_input = String::new();
    io::stdin()
        .read_to_string(&mut raw_input)
        .map_err(|error| format!("failed to read hook input: {error}"))?;

    let input: HookInput =
        serde_json::from_str(&raw_input).map_err(|error| format!("invalid hook JSON: {error}"))?;

    if let Some(output) = run(input)? {
        let json = serde_json::to_string(&output)
            .map_err(|error| format!("failed to serialize hook output: {error}"))?;
        println!("{json}");
    }
    Ok(())
}

fn print_help() {
    println!(
        "rm-to-trash {}\n\n\
         USAGE:\n  \
           rm-to-trash install [--claude] [--codex] [--dry-run]\n  \
           rm-to-trash uninstall [--claude] [--codex] [--dry-run]\n  \
           rm-to-trash doctor [--claude] [--codex]\n  \
           rm-to-trash hook < hook-input.json\n  \
           rm-to-trash --trash -- <path>...\n\n\
         The --trash mode is an internal target used by rewritten hook commands.",
        env!("CARGO_PKG_VERSION")
    );
}

fn dispatch(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    match arguments.next().as_deref().and_then(|value| value.to_str()) {
        None => run_hook(),
        Some("hook") => {
            if arguments.next().is_some() {
                return Err("hook does not accept arguments".to_string());
            }
            run_hook()
        }
        Some("install") => install::install(arguments),
        Some("uninstall") => install::uninstall(arguments),
        Some("doctor") => install::doctor(arguments),
        Some("--trash") => trash_backend::trash_paths(arguments),
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("rm-to-trash {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(argument) => Err(format!(
            "unknown argument {argument:?}; run rm-to-trash --help"
        )),
    }
}

fn main() {
    if let Err(error) = dispatch(env::args_os().skip(1)) {
        eprintln!("rm-to-trash: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_TRASH_COMMAND: &str = "\"/tmp/rm to trash\" --trash --";

    fn run_for_test(input: HookInput) -> Result<Option<HookOutput>, String> {
        run_with_trash_command(input, || Ok(TEST_TRASH_COMMAND.to_string()))
    }

    #[test]
    fn emits_complete_updated_input_for_bash_pre_tool_use() {
        let input: HookInput = serde_json::from_value(json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {
                "command": "sudo rm -rf target",
                "description": "Clean generated output",
                "timeout": 30000
            }
        }))
        .expect("valid test input");

        let output = serde_json::to_value(
            run_for_test(input)
                .expect("hook succeeds")
                .expect("rewrite"),
        )
        .expect("serializable output");
        assert_eq!(
            output["hookSpecificOutput"]["updatedInput"],
            json!({
                "command": "\"/tmp/rm to trash\" --trash -- target",
                "description": "Clean generated output",
                "timeout": 30000
            })
        );
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn emits_nothing_for_unrelated_inputs_without_resolving_the_binary() {
        for value in [
            json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "rm file"}
            }),
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Read",
                "tool_input": {"command": "rm file"}
            }),
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "git status"}
            }),
        ] {
            let input = serde_json::from_value(value).expect("valid test input");
            let output = run_with_trash_command(input, || {
                Err("resolver must not run for unrelated input".to_string())
            })
            .expect("hook succeeds");
            assert!(output.is_none());
        }
    }

    #[test]
    fn fails_closed_when_the_hook_binary_cannot_be_resolved() {
        let input: HookInput = serde_json::from_value(json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "rm file"}
        }))
        .expect("valid test input");

        let result = run_with_trash_command(input, || {
            Err("current executable is unavailable".to_string())
        });
        assert_eq!(
            result.err().as_deref(),
            Some("current executable is unavailable")
        );
    }
}
