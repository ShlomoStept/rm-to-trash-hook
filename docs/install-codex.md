# Install for Codex CLI and app

Codex CLI and the Codex desktop app use the same user hook configuration and
can share one `rm-to-trash` executable.

## 1. Install the executable

Download `rm-to-trash-macos-arm64` and `SHA256SUMS` from the latest
[GitHub release](https://github.com/ShlomoStept/rm-to-trash-hook/releases/latest),
then verify the download:

```sh
shasum -a 256 -c SHA256SUMS
```

Put the verified binary at a stable absolute path:

```sh
mkdir -p "$HOME/.codex/hooks/rm-to-trash"
install -m 755 ./rm-to-trash-macos-arm64 "$HOME/.codex/hooks/rm-to-trash/rm-to-trash"
```

The binary can instead live under `~/.claude/hooks` when both clients share the
same installation. Only the configured absolute path matters.

The release binary is ad hoc signed and is not Apple-notarized. If your local
security policy blocks it, build from the tagged source instead of disabling
or bypassing that policy.

## 2. Register the hook

Merge this handler into `~/.codex/hooks.json`. Preserve every existing event,
matcher group, and handler:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^Bash$",
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

Replace the example path with the exact absolute path on your machine.

Codex treats `matcher` as a regular expression over the canonical tool name.
`^Bash$` therefore matches only Bash tool calls. It does not inspect command
text. The hook starts for every supported Bash or unified-exec call and exits
quickly when no `rm` token exists.

Codex can also load inline hook tables from `~/.codex/config.toml`, but use only
one representation in the same config layer. The official documentation says
that defining both `hooks.json` and inline `[hooks]` in one layer causes them to
be merged with a warning.

Codex launches multiple matching command hooks concurrently. If another
`PreToolUse` handler also rewrites Bash input, test the combined configuration
carefully; neither hook can rely on running after the other.

## 3. Review trust

Open `/hooks` in Codex CLI after adding or changing the handler. Review the
source, command, and matcher, then trust the exact definition.

Codex records trust against a hash of the hook definition. Editing the command,
matcher, or other registration fields changes that hash, so Codex skips the
updated hook until it is reviewed again. Project-local hooks additionally
require the project’s `.codex` configuration layer to be trusted.

## 4. Test both runtimes

Use a disposable fixture:

```sh
fixture="$(mktemp -d)"
touch "$fixture/direct" "$fixture/xargs" "$fixture/find"
rm "$fixture/direct"
printf '%s\0' "$fixture/xargs" | xargs -0 rm -f
find "$fixture" -name find -exec rm -f {} +
```

The three files should disappear from the fixture and appear in macOS Trash.
Run the same representative request once in the CLI and once in the desktop
app if you use both, because they can ship different Codex runtime versions.

## Codex coverage and limits

Current Codex documentation says:

- shell commands and unified exec are presented to hooks as `Bash`;
- `PreToolUse` can return `permissionDecision: "allow"` plus `updatedInput` to
  rewrite a supported call;
- non-managed hooks must be reviewed and trusted;
- hosted tools do not use the local tool-hook path; and
- specialized tool paths may opt out of the default hook path.

This makes `rm-to-trash` useful for ordinary local shell execution in both the
CLI and app, but not a complete enforcement boundary.

See the official [Codex hooks documentation](https://developers.openai.com/codex/hooks)
for current configuration locations, trust behavior, and tool coverage.

## Uninstall

Remove only the `rm-to-trash` handler from the relevant `PreToolUse` matcher
group and preserve other configured hooks.

After confirming that no Codex or Claude configuration references the binary,
you may move it to Trash:

```sh
/usr/bin/trash "$HOME/.codex/hooks/rm-to-trash/rm-to-trash"
```
