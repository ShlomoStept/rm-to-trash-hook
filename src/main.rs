mod rewrite;

use rewrite::{rewrite_rm_to_trash, TRASH_PATH};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, Read};
use std::path::Path;

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
    run_with_trash_availability(input, Path::new(TRASH_PATH).is_file())
}

fn run_with_trash_availability(
    input: HookInput,
    trash_available: bool,
) -> Result<Option<HookOutput>, String> {
    if input.hook_event_name.as_deref() != Some("PreToolUse")
        || input.tool_name.as_deref() != Some("Bash")
    {
        return Ok(None);
    }

    let mut tool_input = input.tool_input.unwrap_or_default();
    let Some(command) = tool_input.get("command").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(rewritten) = rewrite_rm_to_trash(command) else {
        return Ok(None);
    };

    if !trash_available {
        return Err(format!(
            "required Trash command is unavailable at {TRASH_PATH}"
        ));
    }

    tool_input.insert("command".to_string(), Value::String(rewritten));
    Ok(Some(HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "allow",
            permission_decision_reason: "rm redirected to macOS Trash",
            updated_input: tool_input,
        },
    }))
}

fn main() {
    let mut raw_input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut raw_input) {
        eprintln!("rm-to-trash: failed to read hook input: {error}");
        std::process::exit(2);
    }

    let input: HookInput = match serde_json::from_str(&raw_input) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("rm-to-trash: invalid hook JSON: {error}");
            std::process::exit(2);
        }
    };

    match run(input) {
        Ok(Some(output)) => match serde_json::to_string(&output) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("rm-to-trash: failed to serialize hook output: {error}");
                std::process::exit(2);
            }
        },
        Ok(None) => {}
        Err(error) => {
            eprintln!("rm-to-trash: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            run_with_trash_availability(input, true)
                .expect("hook succeeds")
                .expect("rewrite"),
        )
        .expect("serializable output");
        assert_eq!(
            output["hookSpecificOutput"]["updatedInput"],
            json!({
                "command": "/usr/bin/trash target",
                "description": "Clean generated output",
                "timeout": 30000
            })
        );
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn emits_nothing_for_unrelated_inputs() {
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
            assert!(run(input).expect("hook succeeds").is_none());
        }
    }

    #[test]
    fn rejects_a_rewrite_when_trash_is_unavailable() {
        let input: HookInput = serde_json::from_value(json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "rm file"}
        }))
        .expect("valid test input");

        let result = run_with_trash_availability(input, false);
        assert_eq!(
            result.err().as_deref(),
            Some("required Trash command is unavailable at /usr/bin/trash")
        );
    }
}
