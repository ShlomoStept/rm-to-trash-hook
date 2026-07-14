use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, Read};
use std::path::Path;
use std::sync::LazyLock;
use tree_sitter::{Node, Parser, Tree};

const MAX_WALK_DEPTH: usize = 64;
const TRASH_PATH: &str = "/usr/bin/trash";

static ANSI_ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b[^\[\]]")
        .expect("ANSI escape regex must compile")
});

static HAS_RM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\brm\b").expect("rm detection regex must compile"));

static RM_REWRITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:command\s+)?(?:/(?:usr/)?bin/)?rm\b(?:\s+(?:--(?:recursive|force|interactive|verbose|dir|one-file-system|no-preserve-root|preserve-root)|-[a-zA-Z]+))*",
    )
    .expect("rm rewrite regex must compile")
});

static TRASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/usr/bin/trash\b").expect("trash regex must compile"));

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

fn parse_bash(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("").trim()
}

fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

fn command_name(node: Node, source: &str) -> String {
    let parsed_name = (0..node.named_child_count())
        .filter_map(|index| node.named_child(index))
        .find(|child| child.kind() == "command_name")
        .map(|child| node_text(child, source))
        .unwrap_or("");

    if command_basename(parsed_name) == "rm" {
        return parsed_name.to_string();
    }

    if !matches!(
        command_basename(parsed_name),
        "command" | "builtin" | "exec" | "env" | "noglob"
    ) {
        return parsed_name.to_string();
    }

    effective_prefixed_command(node_text(node, source))
}

fn effective_prefixed_command(command: &str) -> String {
    let mut tokens = command.split_whitespace().peekable();

    while let Some(token) = tokens.next() {
        let base = command_basename(token);
        if matches!(base, "command" | "builtin" | "exec" | "noglob") || token == "--" {
            continue;
        }
        if base == "env" {
            while tokens.peek().is_some_and(|next| next.starts_with('-')) {
                let option = tokens.next().unwrap_or_default();
                if matches!(
                    option,
                    "-u" | "-P" | "-S" | "-C" | "--chdir" | "--unset" | "--split-string"
                ) {
                    let _ = tokens.next();
                }
            }
            continue;
        }
        if token.contains('=') && !token.starts_with('=') {
            continue;
        }
        return token.trim_matches(['\'', '"']).to_string();
    }

    String::new()
}

fn is_total_parse_error(root: Node) -> bool {
    root.has_error()
        && root.named_child_count() > 0
        && (0..root.named_child_count())
            .filter_map(|index| root.named_child(index))
            .all(|child| child.kind() == "ERROR")
}

fn strip_ansi_escapes(value: &str) -> String {
    if !value.as_bytes().contains(&0x1b) {
        return value.to_string();
    }
    ANSI_ESCAPE_RE.replace_all(value, "").into_owned()
}

fn rewrite_rm_to_trash(command: &str) -> Option<String> {
    let cleaned = strip_ansi_escapes(command);
    if !HAS_RM_RE.is_match(&cleaned) {
        return None;
    }

    match rewrite_rm_to_trash_ast(&cleaned) {
        Ok(Some(rewritten)) => Some(rewritten),
        Ok(None) => None,
        Err(()) => rewrite_rm_to_trash_regex(&cleaned),
    }
}

fn rewrite_rm_to_trash_ast(command: &str) -> Result<Option<String>, ()> {
    let tree = parse_bash(command).ok_or(())?;
    let root = tree.root_node();
    if is_total_parse_error(root) {
        return Err(());
    }

    let mut replacements = Vec::new();
    collect_rm_replacements(root, command, &mut replacements, 0);
    if replacements.is_empty() {
        return Ok(None);
    }

    replacements.sort_by(|left, right| right.0.cmp(&left.0));
    let mut rewritten = command.to_string();
    for (start, end) in replacements {
        if start <= end && end <= rewritten.len() {
            rewritten.replace_range(start..end, TRASH_PATH);
        }
    }

    if rewritten == command || !verify_trash_has_args(&rewritten) {
        return Ok(None);
    }
    Ok(Some(rewritten))
}

fn collect_rm_replacements(
    node: Node,
    source: &str,
    replacements: &mut Vec<(usize, usize)>,
    depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }

    if node.kind() == "command" && command_basename(&command_name(node, source)) == "rm" {
        if let Some(range) = compute_rm_replacement_range(node, source) {
            replacements.push(range);
        }
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            collect_rm_replacements(child, source, replacements, depth + 1);
        }
    }
}

fn compute_rm_replacement_range(node: Node, source: &str) -> Option<(usize, usize)> {
    let raw_text = source.get(node.byte_range())?;
    let matched = RM_REWRITE_RE.find(raw_text)?;
    let start = node.start_byte() + matched.start();
    let end = node.start_byte() + matched.end();
    (end <= source.len()).then_some((start, end))
}

fn verify_trash_has_args(rewritten: &str) -> bool {
    let mut found_trash = false;

    for matched in TRASH_RE.find_iter(rewritten) {
        found_trash = true;
        let trimmed = rewritten[matched.end()..].trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with("&&")
            || trimmed.starts_with("||")
            || trimmed.starts_with(';')
            || trimmed.starts_with('|')
            || trimmed.starts_with('&')
            || trimmed.starts_with('\n')
        {
            return false;
        }

        let segment = trimmed
            .split("&&")
            .next()
            .unwrap_or("")
            .split("||")
            .next()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .split('|')
            .next()
            .unwrap_or("")
            .trim();
        if segment.is_empty() {
            return false;
        }

        let mut skip_next = false;
        let has_path_argument = segment.split_whitespace().any(|token| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if matches!(token, ">" | ">>" | "<" | "2>" | "2>>" | "&>") {
                skip_next = true;
                return false;
            }
            if token.starts_with('>') || token.starts_with('<') || token.starts_with('-') {
                return false;
            }
            token != "--"
        });
        if !has_path_argument {
            return false;
        }
    }

    found_trash
}

fn rewrite_rm_to_trash_regex(command: &str) -> Option<String> {
    let rewritten = RM_REWRITE_RE.replace_all(command, TRASH_PATH).into_owned();
    (rewritten != command && verify_trash_has_args(&rewritten)).then_some(rewritten)
}

fn run(input: HookInput) -> Result<Option<HookOutput>, String> {
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

    if !Path::new(TRASH_PATH).is_file() {
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
    fn rewrites_simple_rm() {
        assert_eq!(
            rewrite_rm_to_trash("rm file.txt"),
            Some("/usr/bin/trash file.txt".to_string())
        );
    }

    #[test]
    fn removes_rm_flags_and_preserves_paths() {
        assert_eq!(
            rewrite_rm_to_trash("rm -rf dir1 \"dir two\""),
            Some("/usr/bin/trash dir1 \"dir two\"".to_string())
        );
    }

    #[test]
    fn rewrites_absolute_rm_path() {
        assert_eq!(
            rewrite_rm_to_trash("/bin/rm -rf foo"),
            Some("/usr/bin/trash foo".to_string())
        );
    }

    #[test]
    fn rewrites_compound_command_without_changing_other_segments() {
        assert_eq!(
            rewrite_rm_to_trash("cd /tmp && rm -rf old"),
            Some("cd /tmp && /usr/bin/trash old".to_string())
        );
    }

    #[test]
    fn rewrites_each_direct_rm_command() {
        assert_eq!(
            rewrite_rm_to_trash("rm one; /usr/bin/rm -f two"),
            Some("/usr/bin/trash one; /usr/bin/trash two".to_string())
        );
    }

    #[test]
    fn rewrites_command_builtin_prefix() {
        assert_eq!(
            rewrite_rm_to_trash("command rm -f item"),
            Some("/usr/bin/trash item".to_string())
        );
    }

    #[test]
    fn ignores_rm_text_in_arguments() {
        assert_eq!(rewrite_rm_to_trash("echo \"rm -rf /tmp/test\""), None);
        assert_eq!(rewrite_rm_to_trash("printf '%s\\n' 'rm -rf foo'"), None);
    }

    #[test]
    fn ignores_wrapped_rm_commands() {
        assert_eq!(rewrite_rm_to_trash("sudo rm -rf foo"), None);
        assert_eq!(rewrite_rm_to_trash("xargs rm -f"), None);
        assert_eq!(rewrite_rm_to_trash("find . -exec rm {} \\;"), None);
    }

    #[test]
    fn ignores_rm_without_a_path() {
        assert_eq!(rewrite_rm_to_trash("rm"), None);
        assert_eq!(rewrite_rm_to_trash("rm -rf"), None);
        assert_eq!(rewrite_rm_to_trash("rm --recursive --force"), None);
        assert_eq!(rewrite_rm_to_trash("rm > out.txt"), None);
        assert_eq!(rewrite_rm_to_trash("rm >> out.txt"), None);
    }

    #[test]
    fn strips_ansi_sequences_before_rewriting() {
        assert_eq!(
            rewrite_rm_to_trash("\u{1b}[31mrm\u{1b}[0m file.txt"),
            Some("/usr/bin/trash file.txt".to_string())
        );
    }

    #[test]
    fn emits_complete_updated_input_for_bash_pre_tool_use() {
        let input: HookInput = serde_json::from_value(json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {
                "command": "rm -rf target",
                "description": "Clean generated output",
                "timeout": 30000
            }
        }))
        .expect("valid test input");

        let output = serde_json::to_value(run(input).expect("hook succeeds").expect("rewrite"))
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
}
