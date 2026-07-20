use regex::Regex;
use std::sync::LazyLock;
use tree_sitter::{Node, Parser, Tree};

const MAX_NESTED_SCRIPT_DEPTH: usize = 8;
const MAX_WALK_DEPTH: usize = 64;
pub(crate) const TRASH_PATH: &str = "/usr/bin/trash";

static ANSI_ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b[^\[\]]")
        .expect("ANSI escape regex must compile")
});

static HAS_RM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\brm\b").expect("rm detection regex must compile"));

static RM_REWRITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:command\s+)?(?:/(?:usr/)?bin/)?rm\b(?:\s+(?:--|--(?:recursive|force|interactive|verbose|dir|one-file-system|no-preserve-root|preserve-root)(?:=[^\s]+)?|-[a-zA-Z]+))*",
    )
    .expect("rm rewrite regex must compile")
});

static TRASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/usr/bin/trash\b").expect("trash regex must compile"));

#[derive(Clone, Debug, Eq, PartialEq)]
struct Replacement {
    start: usize,
    end: usize,
    value: String,
}

impl Replacement {
    fn trash(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            value: TRASH_PATH.to_string(),
        }
    }

    fn remove(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            value: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EffectiveCommand {
    index: usize,
    removable_prefix_start: Option<usize>,
}

pub(crate) fn rewrite_rm_to_trash(command: &str) -> Option<String> {
    let cleaned = strip_ansi_escapes(command);
    if !HAS_RM_RE.is_match(&cleaned) {
        return None;
    }

    match rewrite_rm_to_trash_ast(&cleaned, 0) {
        Ok(Some(rewritten)) => Some(rewritten),
        Ok(None) => None,
        Err(()) => rewrite_rm_to_trash_regex(&cleaned),
    }
}

fn parse_bash(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn strip_ansi_escapes(value: &str) -> String {
    if !value.as_bytes().contains(&0x1b) {
        return value.to_string();
    }
    ANSI_ESCAPE_RE.replace_all(value, "").into_owned()
}

fn rewrite_rm_to_trash_ast(command: &str, script_depth: usize) -> Result<Option<String>, ()> {
    let tree = parse_bash(command).ok_or(())?;
    let root = tree.root_node();
    if is_total_parse_error(root) {
        return Err(());
    }

    let mut replacements = Vec::new();
    collect_rm_replacements(root, command, &mut replacements, 0, script_depth);
    apply_replacements(command, replacements)
}

fn is_total_parse_error(root: Node<'_>) -> bool {
    root.has_error()
        && root.named_child_count() > 0
        && (0..root.named_child_count())
            .filter_map(|index| root.named_child(index))
            .all(|child| child.kind() == "ERROR")
}

fn collect_rm_replacements(
    node: Node<'_>,
    source: &str,
    replacements: &mut Vec<Replacement>,
    walk_depth: usize,
    script_depth: usize,
) {
    if walk_depth >= MAX_WALK_DEPTH {
        return;
    }

    if node.kind() == "command" {
        collect_command_replacements(node, source, replacements, script_depth);
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            collect_rm_replacements(child, source, replacements, walk_depth + 1, script_depth);
        }
    }
}

fn collect_command_replacements(
    command: Node<'_>,
    source: &str,
    replacements: &mut Vec<Replacement>,
    script_depth: usize,
) {
    let parts = named_children(command);
    let Some(effective) = locate_effective_command(&parts, source, 0, parts.len()) else {
        return;
    };
    let Some(name) = token_basename(parts[effective.index], source) else {
        return;
    };

    if name == "rm" {
        let start = effective
            .removable_prefix_start
            .unwrap_or_else(|| parts[effective.index].start_byte());
        if let Some(command_replacements) =
            rm_replacements(&parts, source, effective.index, parts.len(), start, false)
        {
            replacements.extend(command_replacements);
        }
        return;
    }

    let before = replacements.len();
    match name {
        "xargs" => {
            collect_xargs_replacement(&parts, source, effective.index, replacements, script_depth)
        }
        "find" => {
            collect_find_replacements(&parts, source, effective.index, replacements, script_depth)
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" => collect_shell_script_replacement(
            &parts,
            source,
            effective.index,
            parts.len(),
            replacements,
            script_depth,
        ),
        "eval" => {
            collect_eval_replacement(&parts, source, effective.index, replacements, script_depth)
        }
        _ => {}
    }

    if replacements.len() > before {
        if let Some(start) = effective.removable_prefix_start {
            replacements.push(Replacement::remove(
                start,
                parts[effective.index].start_byte(),
            ));
        }
    }
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    (0..node.named_child_count())
        .filter_map(|index| node.named_child(index))
        .collect()
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.byte_range()).map(str::trim)
}

fn simple_token<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    matches!(node.kind(), "command_name" | "word" | "number")
        .then(|| node_text(node, source))
        .flatten()
}

fn token_basename<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    simple_token(node, source).map(command_basename)
}

fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

fn locate_effective_command(
    parts: &[Node<'_>],
    source: &str,
    start: usize,
    limit: usize,
) -> Option<EffectiveCommand> {
    let mut index = start;
    let mut removable_prefix_start = None;
    let mut process_wrapper_seen = false;

    while index < limit {
        let name = token_basename(parts[index], source)?;
        let next = match name {
            "command" if !process_wrapper_seen => {
                removable_prefix_start.get_or_insert_with(|| parts[index].start_byte());
                parse_command_wrapper(parts, source, index, limit)?
            }
            "exec" if !process_wrapper_seen => parse_exec_wrapper(parts, source, index, limit)?,
            "noglob" if !process_wrapper_seen => index.checked_add(1)?,
            "env" => {
                process_wrapper_seen = true;
                parse_env_wrapper(parts, source, index, limit)?
            }
            "nice" => {
                process_wrapper_seen = true;
                parse_nice_wrapper(parts, source, index, limit)?
            }
            "nohup" => {
                process_wrapper_seen = true;
                parse_nohup_wrapper(parts, source, index, limit)?
            }
            "sudo" => {
                process_wrapper_seen = true;
                removable_prefix_start.get_or_insert_with(|| parts[index].start_byte());
                parse_sudo_wrapper(parts, source, index, limit)?
            }
            "time" => {
                process_wrapper_seen = true;
                parse_time_wrapper(parts, source, index, limit)?
            }
            _ => {
                return Some(EffectiveCommand {
                    index,
                    removable_prefix_start,
                });
            }
        };

        if next <= index || next >= limit {
            return None;
        }
        index = next;
    }

    None
}

fn parse_command_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        match token {
            "--" => return (cursor + 1 < limit).then_some(cursor + 1),
            "-p" => cursor += 1,
            "-v" | "-V" => return None,
            value if value.starts_with('-') => return None,
            _ => return Some(cursor),
        }
    }
    None
}

fn parse_exec_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        match token {
            "--" => return (cursor + 1 < limit).then_some(cursor + 1),
            "-a" => cursor = cursor.checked_add(2)?,
            "-c" | "-l" | "-cl" | "-lc" => cursor += 1,
            value if value.starts_with("-a") && value.len() > 2 => cursor += 1,
            value if value.starts_with('-') => return None,
            _ => return Some(cursor),
        }
    }
    None
}

fn parse_env_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = index + 1;
    let mut options_finished = false;

    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        if !options_finished {
            match token {
                "--" => {
                    options_finished = true;
                    cursor += 1;
                    continue;
                }
                "-i" | "--ignore-environment" | "-0" | "--null" | "-v" => {
                    cursor += 1;
                    continue;
                }
                "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string" => {
                    cursor = cursor.checked_add(2)?;
                    continue;
                }
                value
                    if value.starts_with("--unset=")
                        || value.starts_with("--chdir=")
                        || value.starts_with("--split-string=")
                        || (value.starts_with("-u") && value.len() > 2)
                        || (value.starts_with("-C") && value.len() > 2)
                        || (value.starts_with("-S") && value.len() > 2) =>
                {
                    cursor += 1;
                    continue;
                }
                value if value.starts_with('-') => return None,
                _ => {}
            }
        }

        if is_environment_assignment(token) {
            cursor += 1;
            continue;
        }
        return Some(cursor);
    }

    None
}

fn is_environment_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'))
}

fn parse_nice_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        match token {
            "--" => return (cursor + 1 < limit).then_some(cursor + 1),
            "-n" | "--adjustment" => cursor = cursor.checked_add(2)?,
            "--help" | "--version" => return None,
            value if value.starts_with("--adjustment=") || is_legacy_nice_adjustment(value) => {
                cursor += 1;
            }
            value if value.starts_with('-') => return None,
            _ => return Some(cursor),
        }
    }
    None
}

fn is_legacy_nice_adjustment(token: &str) -> bool {
    token.strip_prefix('-').is_some_and(|value| {
        !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
    })
}

fn parse_nohup_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let cursor = index + 1;
    let token = simple_token(*parts.get(cursor)?, source)?;
    match token {
        "--" => (cursor + 1 < limit).then_some(cursor + 1),
        "--help" | "--version" => None,
        value if value.starts_with('-') => None,
        _ => Some(cursor),
    }
}

fn parse_time_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        match token {
            "--" => return (cursor + 1 < limit).then_some(cursor + 1),
            "-p" => cursor += 1,
            value if value.starts_with('-') => return None,
            _ => return Some(cursor),
        }
    }
    None
}

fn parse_sudo_wrapper(
    parts: &[Node<'_>],
    source: &str,
    index: usize,
    limit: usize,
) -> Option<usize> {
    const OPTIONS_WITH_ARGUMENTS: &[&str] = &[
        "-C",
        "-D",
        "-g",
        "-h",
        "-p",
        "-R",
        "-r",
        "-T",
        "-t",
        "-U",
        "-u",
        "--chdir",
        "--close-from",
        "--group",
        "--host",
        "--other-user",
        "--prompt",
        "--role",
        "--timeout",
        "--type",
        "--user",
    ];
    const LONG_OPTIONS_WITH_ATTACHED_ARGUMENTS: &[&str] = &[
        "--chdir=",
        "--close-from=",
        "--group=",
        "--host=",
        "--other-user=",
        "--prompt=",
        "--role=",
        "--timeout=",
        "--type=",
        "--user=",
    ];
    const ACTION_OPTIONS: &[&str] = &[
        "-e",
        "-l",
        "-V",
        "-v",
        "--edit",
        "--list",
        "--validate",
        "--version",
    ];

    let mut cursor = index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        if token == "--" {
            return (cursor + 1 < limit).then_some(cursor + 1);
        }
        if is_environment_assignment(token) {
            cursor += 1;
            continue;
        }
        if ACTION_OPTIONS.contains(&token) {
            return None;
        }
        if OPTIONS_WITH_ARGUMENTS.contains(&token) {
            cursor = cursor.checked_add(2)?;
            continue;
        }
        if LONG_OPTIONS_WITH_ATTACHED_ARGUMENTS
            .iter()
            .any(|prefix| token.starts_with(prefix))
            || is_attached_sudo_short_option(token)
        {
            cursor += 1;
            continue;
        }
        if is_supported_sudo_flag_group(token) {
            cursor += 1;
            continue;
        }
        if token.starts_with('-') {
            return None;
        }
        return Some(cursor);
    }
    None
}

fn is_attached_sudo_short_option(token: &str) -> bool {
    token.len() > 2
        && token
            .as_bytes()
            .get(1)
            .is_some_and(|option| b"CDghpRrTtUu".contains(option))
}

fn is_supported_sudo_flag_group(token: &str) -> bool {
    token.strip_prefix('-').is_some_and(|flags| {
        !flags.is_empty()
            && flags
                .chars()
                .all(|flag| matches!(flag, 'A' | 'b' | 'E' | 'H' | 'K' | 'k' | 'n' | 'P' | 'S'))
    })
}

fn rm_replacements(
    parts: &[Node<'_>],
    source: &str,
    rm_index: usize,
    limit: usize,
    replacement_start: usize,
    allow_implicit_operands: bool,
) -> Option<Vec<Replacement>> {
    if token_basename(*parts.get(rm_index)?, source)? != "rm" {
        return None;
    }

    let mut cursor = rm_index + 1;
    let mut end = parts[rm_index].end_byte();
    let mut options_finished = false;

    while cursor < limit {
        let Some(token) = simple_token(parts[cursor], source) else {
            break;
        };
        if !options_finished && token == "--" {
            options_finished = true;
            end = parts[cursor].end_byte();
            cursor += 1;
            continue;
        }
        if !options_finished && token.starts_with('-') && token != "-" {
            end = parts[cursor].end_byte();
            cursor += 1;
            continue;
        }
        break;
    }

    let has_operand = parts[cursor..limit]
        .iter()
        .any(|part| is_rm_operand(*part, source))
        || has_operand_after_redirect(parts[rm_index]);
    if !allow_implicit_operands && !has_operand {
        return None;
    }

    let mut replacements = vec![Replacement::trash(replacement_start, end)];
    if options_finished {
        for operand in &parts[cursor..limit] {
            if literal_token_value(*operand, source).is_some_and(|value| value.starts_with('-')) {
                replacements.push(Replacement {
                    start: operand.start_byte(),
                    end: operand.start_byte(),
                    value: "./".to_string(),
                });
            }
        }
    }
    Some(replacements)
}

fn is_rm_operand(node: Node<'_>, source: &str) -> bool {
    if is_redirect(node) {
        return false;
    }
    !matches!(node_text(node, source), Some(";" | "\\;" | "+"))
}

fn is_redirect(node: Node<'_>) -> bool {
    node.kind().contains("redirect")
}

fn has_operand_after_redirect(command_name: Node<'_>) -> bool {
    let Some(command) = command_name.parent() else {
        return false;
    };
    let Some(parent) = command.parent() else {
        return false;
    };
    if parent.kind() != "redirected_statement" {
        return false;
    }

    (0..parent.named_child_count())
        .filter_map(|index| parent.named_child(index))
        .filter(|child| child.kind() == "file_redirect")
        .any(|redirect| {
            (0..redirect.named_child_count())
                .filter_map(|index| redirect.named_child(index))
                .filter(|child| child.kind() != "file_descriptor")
                .count()
                > 1
        })
}

fn literal_token_value<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let raw = node_text(node, source)?;
    match node.kind() {
        "word" | "number" | "command_name" => Some(raw),
        "raw_string" if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 => {
            raw.get(1..raw.len() - 1)
        }
        "string"
            if raw.starts_with('"')
                && raw.ends_with('"')
                && raw.len() >= 2
                && node.named_child_count() == 1
                && node
                    .named_child(0)
                    .is_some_and(|child| child.kind() == "string_content") =>
        {
            raw.get(1..raw.len() - 1)
        }
        _ => None,
    }
}

fn collect_xargs_replacement(
    parts: &[Node<'_>],
    source: &str,
    xargs_index: usize,
    replacements: &mut Vec<Replacement>,
    script_depth: usize,
) {
    let Some(command_index) = locate_xargs_command(parts, source, xargs_index) else {
        return;
    };
    let Some(effective) = locate_effective_command(parts, source, command_index, parts.len())
    else {
        return;
    };
    let mut nested_script_rewritten = false;
    match token_basename(parts[effective.index], source) {
        Some("rm") => {
            let start = effective
                .removable_prefix_start
                .unwrap_or_else(|| parts[effective.index].start_byte());
            if let Some(command_replacements) =
                rm_replacements(parts, source, effective.index, parts.len(), start, true)
            {
                replacements.extend(command_replacements);
            }
        }
        Some("sh" | "bash" | "zsh" | "dash" | "ksh") => {
            let before = replacements.len();
            collect_shell_script_replacement(
                parts,
                source,
                effective.index,
                parts.len(),
                replacements,
                script_depth,
            );
            nested_script_rewritten = replacements.len() > before;
        }
        _ => {}
    }
    if nested_script_rewritten {
        if let Some(start) = effective.removable_prefix_start {
            replacements.push(Replacement::remove(
                start,
                parts[effective.index].start_byte(),
            ));
        }
    }
}

fn locate_xargs_command(parts: &[Node<'_>], source: &str, xargs_index: usize) -> Option<usize> {
    const OPTIONS_WITH_ARGUMENTS: &[&str] = &["-E", "-I", "-J", "-L", "-n", "-P", "-R", "-S", "-s"];
    const LONG_OPTIONS_WITH_ARGUMENTS: &[&str] = &[
        "--arg-file",
        "--delimiter",
        "--eof",
        "--max-args",
        "--max-chars",
        "--max-lines",
        "--max-procs",
        "--process-slot-var",
        "--replace",
    ];
    const FLAG_OPTIONS: &[&str] = &[
        "-0",
        "-o",
        "-p",
        "-r",
        "-t",
        "-x",
        "--exit",
        "--interactive",
        "--no-run-if-empty",
        "--null",
        "--open-tty",
        "--show-limits",
        "--verbose",
    ];

    let mut cursor = xargs_index + 1;
    while cursor < parts.len() {
        let token = simple_token(parts[cursor], source)?;
        if token == "--" {
            return (cursor + 1 < parts.len()).then_some(cursor + 1);
        }
        if OPTIONS_WITH_ARGUMENTS.contains(&token) || LONG_OPTIONS_WITH_ARGUMENTS.contains(&token) {
            cursor = cursor.checked_add(2)?;
            continue;
        }
        if FLAG_OPTIONS.contains(&token)
            || has_attached_xargs_option_argument(token)
            || LONG_OPTIONS_WITH_ARGUMENTS
                .iter()
                .any(|option| token.starts_with(&format!("{option}=")))
        {
            cursor += 1;
            continue;
        }
        if token.starts_with('-') {
            return None;
        }
        return Some(cursor);
    }
    None
}

fn has_attached_xargs_option_argument(token: &str) -> bool {
    token.len() > 2
        && token
            .as_bytes()
            .get(1)
            .is_some_and(|option| b"EIJLnPRSs".contains(option))
}

fn collect_find_replacements(
    parts: &[Node<'_>],
    source: &str,
    find_index: usize,
    replacements: &mut Vec<Replacement>,
    script_depth: usize,
) {
    let mut cursor = find_index + 1;
    while cursor < parts.len() {
        let token = node_text(parts[cursor], source);
        if !matches!(token, Some("-exec" | "-execdir")) {
            cursor += 1;
            continue;
        }

        let command_start = cursor + 1;
        let Some(terminator) = (command_start..parts.len())
            .find(|index| matches!(node_text(parts[*index], source), Some(";" | "\\;" | "+")))
        else {
            return;
        };
        let Some(effective) = locate_effective_command(parts, source, command_start, terminator)
        else {
            cursor = terminator + 1;
            continue;
        };
        let mut nested_script_rewritten = false;
        match token_basename(parts[effective.index], source) {
            Some("rm") => {
                let start = effective
                    .removable_prefix_start
                    .unwrap_or_else(|| parts[effective.index].start_byte());
                if let Some(command_replacements) =
                    rm_replacements(parts, source, effective.index, terminator, start, false)
                {
                    replacements.extend(command_replacements);
                }
            }
            Some("sh" | "bash" | "zsh" | "dash" | "ksh") => {
                let before = replacements.len();
                collect_shell_script_replacement(
                    parts,
                    source,
                    effective.index,
                    terminator,
                    replacements,
                    script_depth,
                );
                nested_script_rewritten = replacements.len() > before;
            }
            _ => {}
        }
        if nested_script_rewritten {
            if let Some(start) = effective.removable_prefix_start {
                replacements.push(Replacement::remove(
                    start,
                    parts[effective.index].start_byte(),
                ));
            }
        }
        cursor = terminator + 1;
    }
}

fn collect_shell_script_replacement(
    parts: &[Node<'_>],
    source: &str,
    shell_index: usize,
    limit: usize,
    replacements: &mut Vec<Replacement>,
    script_depth: usize,
) {
    if script_depth >= MAX_NESTED_SCRIPT_DEPTH {
        return;
    }

    let Some(script_index) = locate_shell_script(parts, source, shell_index, limit) else {
        return;
    };
    let Some((start, end, script)) = single_quoted_content(parts[script_index], source) else {
        return;
    };
    let Ok(Some(rewritten)) = rewrite_rm_to_trash_ast(script, script_depth + 1) else {
        return;
    };
    replacements.push(Replacement {
        start,
        end,
        value: rewritten,
    });
}

fn locate_shell_script(
    parts: &[Node<'_>],
    source: &str,
    shell_index: usize,
    limit: usize,
) -> Option<usize> {
    let mut cursor = shell_index + 1;
    while cursor < limit {
        let token = simple_token(parts[cursor], source)?;
        if is_shell_command_string_option(token) {
            return (cursor + 1 < limit).then_some(cursor + 1);
        }
        if token == "--" || !token.starts_with('-') {
            return None;
        }
        cursor += 1;
    }
    None
}

fn is_shell_command_string_option(token: &str) -> bool {
    token == "-c"
        || token.strip_prefix('-').is_some_and(|options| {
            !options.is_empty()
                && options.chars().all(|option| option.is_ascii_alphabetic())
                && options.contains('c')
        })
}

fn collect_eval_replacement(
    parts: &[Node<'_>],
    source: &str,
    eval_index: usize,
    replacements: &mut Vec<Replacement>,
    script_depth: usize,
) {
    if script_depth >= MAX_NESTED_SCRIPT_DEPTH || eval_index + 2 != parts.len() {
        return;
    }
    let Some((start, end, script)) = single_quoted_content(parts[eval_index + 1], source) else {
        return;
    };
    let Ok(Some(rewritten)) = rewrite_rm_to_trash_ast(script, script_depth + 1) else {
        return;
    };
    replacements.push(Replacement {
        start,
        end,
        value: rewritten,
    });
}

fn single_quoted_content<'a>(node: Node<'_>, source: &'a str) -> Option<(usize, usize, &'a str)> {
    if node.kind() != "raw_string" {
        return None;
    }
    let raw = source.get(node.byte_range())?;
    if !raw.starts_with('\'') || !raw.ends_with('\'') || raw.len() < 2 {
        return None;
    }
    let start = node.start_byte() + 1;
    let end = node.end_byte() - 1;
    Some((start, end, source.get(start..end)?))
}

fn apply_replacements(
    command: &str,
    mut replacements: Vec<Replacement>,
) -> Result<Option<String>, ()> {
    if replacements.is_empty() {
        return Ok(None);
    }

    replacements.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then(left.end.cmp(&right.end))
            .then(left.value.cmp(&right.value))
    });
    replacements.dedup();

    let mut previous_end = 0;
    for replacement in &replacements {
        if replacement.start > replacement.end
            || replacement.end > command.len()
            || replacement.start < previous_end
        {
            return Err(());
        }
        previous_end = replacement.end;
    }

    let mut rewritten = command.to_string();
    for replacement in replacements.into_iter().rev() {
        rewritten.replace_range(replacement.start..replacement.end, &replacement.value);
    }

    if rewritten == command || !TRASH_RE.is_match(&rewritten) {
        return Ok(None);
    }
    Ok(Some(rewritten))
}

fn rewrite_rm_to_trash_regex(command: &str) -> Option<String> {
    let rewritten = RM_REWRITE_RE.replace_all(command, TRASH_PATH).into_owned();
    (rewritten != command && verify_regex_fallback(&rewritten)).then_some(rewritten)
}

fn verify_regex_fallback(rewritten: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_direct_rm_commands() {
        assert_eq!(
            rewrite_rm_to_trash("rm file.txt"),
            Some("/usr/bin/trash file.txt".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("rm -rf dir1 \"dir two\""),
            Some("/usr/bin/trash dir1 \"dir two\"".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("/bin/rm -rf foo"),
            Some("/usr/bin/trash foo".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("command rm -f item"),
            Some("/usr/bin/trash item".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("rm >out.log file"),
            Some("/usr/bin/trash >out.log file".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("rm -rf -- -leading '-second'"),
            Some("/usr/bin/trash ./-leading ./'-second'".to_string())
        );
    }

    #[test]
    fn rewrites_compound_and_nested_direct_commands() {
        assert_eq!(
            rewrite_rm_to_trash("cd /tmp && rm -rf old"),
            Some("cd /tmp && /usr/bin/trash old".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("rm one; /usr/bin/rm -f two"),
            Some("/usr/bin/trash one; /usr/bin/trash two".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("printf '%s' \"$(rm -f nested)\""),
            Some("printf '%s' \"$(/usr/bin/trash nested)\"".to_string())
        );
    }

    #[test]
    fn rewrites_supported_execution_wrappers() {
        for (proposed, expected) in [
            ("sudo rm -rf foo", "/usr/bin/trash foo"),
            ("sudo -n -- /bin/rm -f foo", "/usr/bin/trash foo"),
            ("sudo -u root rm -rf foo", "/usr/bin/trash foo"),
            ("env FOO=bar rm -f foo", "env FOO=bar /usr/bin/trash foo"),
            ("exec rm -rf foo", "exec /usr/bin/trash foo"),
            ("nice -n 5 rm -rf foo", "nice -n 5 /usr/bin/trash foo"),
            ("nohup rm -rf foo", "nohup /usr/bin/trash foo"),
            ("time -p rm -rf foo", "time -p /usr/bin/trash foo"),
            ("noglob rm -rf foo", "noglob /usr/bin/trash foo"),
        ] {
            assert_eq!(
                rewrite_rm_to_trash(proposed),
                Some(expected.to_string()),
                "{proposed}"
            );
        }
    }

    #[test]
    fn rewrites_xargs_dispatch() {
        assert_eq!(
            rewrite_rm_to_trash("printf '%s\\0' one | xargs -0 rm -rf"),
            Some("printf '%s\\0' one | xargs -0 /usr/bin/trash".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("xargs -n 2 -- /bin/rm -f fixed"),
            Some("xargs -n 2 -- /usr/bin/trash fixed".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("xargs sudo -n rm -rf"),
            Some("xargs /usr/bin/trash".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("xargs sh -c 'rm -f \"$1\"' _"),
            Some("xargs sh -c '/usr/bin/trash \"$1\"' _".to_string())
        );
    }

    #[test]
    fn rewrites_find_exec_and_execdir_dispatch() {
        assert_eq!(
            rewrite_rm_to_trash("find . -exec rm -rf {} +"),
            Some("find . -exec /usr/bin/trash {} +".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("find . -execdir /bin/rm -f {} \\;"),
            Some("find . -execdir /usr/bin/trash {} \\;".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("find . -exec rm -f {} \\; -o -exec sudo rm {} +"),
            Some("find . -exec /usr/bin/trash {} \\; -o -exec /usr/bin/trash {} +".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("find . -exec sh -c 'rm -f \"$1\"' _ {} \\;"),
            Some("find . -exec sh -c '/usr/bin/trash \"$1\"' _ {} \\;".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("sudo find . -exec rm -f {} +"),
            Some("find . -exec /usr/bin/trash {} +".to_string())
        );
    }

    #[test]
    fn rewrites_literal_nested_shell_scripts() {
        assert_eq!(
            rewrite_rm_to_trash("sh -c 'rm -rf \"$1\"' _ foo"),
            Some("sh -c '/usr/bin/trash \"$1\"' _ foo".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("bash -lc 'cd /tmp && rm -f old'"),
            Some("bash -lc 'cd /tmp && /usr/bin/trash old'".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("eval 'rm -rf foo'"),
            Some("eval '/usr/bin/trash foo'".to_string())
        );
        assert_eq!(
            rewrite_rm_to_trash("sudo sh -c 'rm -rf foo'"),
            Some("sh -c '/usr/bin/trash foo'".to_string())
        );
    }

    #[test]
    fn ignores_rm_text_that_is_not_executed() {
        assert_eq!(rewrite_rm_to_trash("echo \"rm -rf /tmp/test\""), None);
        assert_eq!(rewrite_rm_to_trash("printf '%s\\n' 'rm -rf foo'"), None);
        assert_eq!(rewrite_rm_to_trash("command -v rm"), None);
        assert_eq!(rewrite_rm_to_trash("xargs -I rm echo rm"), None);
        assert_eq!(rewrite_rm_to_trash("env -u rm echo foo"), None);
        assert_eq!(rewrite_rm_to_trash("sudo -p rm echo foo"), None);
        assert_eq!(rewrite_rm_to_trash("builtin rm -f foo"), None);
    }

    #[test]
    fn leaves_dynamic_or_ambiguous_indirection_unchanged() {
        assert_eq!(rewrite_rm_to_trash("$command -rf foo"), None);
        assert_eq!(rewrite_rm_to_trash("bash -c \"$command -rf foo\""), None);
        assert_eq!(rewrite_rm_to_trash("bash cleanup.sh"), None);
        assert_eq!(rewrite_rm_to_trash("eval \"$command -rf foo\""), None);
        assert_eq!(rewrite_rm_to_trash("ssh host rm -rf foo"), None);
    }

    #[test]
    fn ignores_rm_without_required_operands() {
        for command in [
            "rm",
            "rm -rf",
            "rm --recursive --force",
            "rm > out.txt",
            "rm >> out.txt",
            "rm 2>&1",
            "rm 2>err.log",
            "sudo rm -rf",
            "find . -exec rm -rf \\;",
        ] {
            assert_eq!(rewrite_rm_to_trash(command), None, "{command}");
        }
    }

    #[test]
    fn strips_ansi_sequences_before_rewriting() {
        assert_eq!(
            rewrite_rm_to_trash("\u{1b}[31mrm\u{1b}[0m file.txt"),
            Some("/usr/bin/trash file.txt".to_string())
        );
    }
}
