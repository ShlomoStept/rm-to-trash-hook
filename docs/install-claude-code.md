# Install for Claude Code

This guide installs `rm-to-trash` as a user-level Claude Code hook. The same
shape can be placed in a project-level `.claude/settings.json` when a repository
should carry the configuration for all contributors.

## 1. Install the executable

Download `rm-to-trash-macos-arm64` and `SHA256SUMS` from the latest
[GitHub release](https://github.com/ShlomoStept/rm-to-trash-hook/releases/latest),
then verify the download:

```sh
shasum -a 256 -c SHA256SUMS
```

Put the verified binary at a stable absolute path:

```sh
mkdir -p "$HOME/.claude/hooks/rm-to-trash"
install -m 755 ./rm-to-trash-macos-arm64 "$HOME/.claude/hooks/rm-to-trash/rm-to-trash"
```

Confirm the platform and executable bit:

```sh
file "$HOME/.claude/hooks/rm-to-trash/rm-to-trash"
test -x "$HOME/.claude/hooks/rm-to-trash/rm-to-trash"
```

The release binary is ad hoc signed and is not Apple-notarized. If your local
security policy blocks it, build from the tagged source instead of disabling
or bypassing that policy.

## 2. Register the hook

Merge this handler into `~/.claude/settings.json`. Preserve existing events,
matcher groups, and handlers:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/rm-to-trash",
            "timeout": 10,
            "statusMessage": "Redirecting rm to macOS Trash"
          }
        ]
      }
    ]
  }
}
```

Replace the example path with the exact absolute path from your machine.

Do not add an `if: "Bash(rm *)"` filter. Claude Code’s `matcher` selects the
tool name, while an `if` rule filters subcommands. A direct-only filter can
prevent the hook from seeing `sudo rm`, `xargs rm`, `find -exec rm`, and nested
shell forms. With the configuration above, the program starts on every Bash
tool call and exits quickly and silently when no `rm` token exists.

## 3. Review and test

Open `/hooks` in Claude Code and confirm that the handler appears under
`PreToolUse: Bash`.

Use a disposable directory for the first test:

```sh
fixture="$(mktemp -d)"
touch "$fixture/direct" "$fixture/wrapped"
rm "$fixture/direct"
sudo rm "$fixture/wrapped"
```

Both files should disappear from the fixture and appear in the current user’s
macOS Trash. The `sudo` wrapper is removed by the hook, so this test should not
ask for a password.

Also run a non-deletion command and confirm it behaves normally:

```sh
printf '%s\n' "hook smoke test"
```

## How Claude Code evaluates the configuration

Claude Code fires `PreToolUse` after it has created the Bash tool input and
before the command runs. The `Bash` matcher is an exact tool-name match. The
hook receives the complete tool input as JSON on standard input.

When a rewrite applies, `rm-to-trash` returns `permissionDecision: "allow"` and
the complete replacement input. Claude Code then executes the rewritten
command without a separate permission prompt. When no rewrite applies, the
hook exits with no output, which leaves the normal permission flow unchanged.

See the official [Claude Code hooks reference](https://code.claude.com/docs/en/hooks)
for configuration precedence, event schemas, and decision behavior.

## Uninstall

Remove only the `rm-to-trash` handler object from the relevant `PreToolUse`
matcher group. Keep any other handlers in the same array.

After verifying that no settings file references the executable, you may remove
the installed binary:

```sh
/usr/bin/trash "$HOME/.claude/hooks/rm-to-trash/rm-to-trash"
```
