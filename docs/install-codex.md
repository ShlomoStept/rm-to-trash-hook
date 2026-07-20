# Install for Codex CLI and app

Codex CLI and the Codex desktop app share the user hook configuration at
`~/.codex/hooks.json`. The recommended installation uses the native binary to
copy itself and merge one handler safely.

## 1. Download and verify

Download `SHA256SUMS` and the matching asset from the
[latest GitHub release](https://github.com/ShlomoStept/rm-to-trash-hook/releases/latest):

| Host | Asset |
| --- | --- |
| Apple silicon macOS | `rm-to-trash-macos-arm64` |
| Intel macOS | `rm-to-trash-macos-x86_64` |
| ARM64 Linux | `rm-to-trash-linux-arm64` |
| x86-64 Linux | `rm-to-trash-linux-x86_64` |
| x86-64 Windows | `rm-to-trash-windows-x86_64.exe` |

On macOS:

```sh
asset=rm-to-trash-macos-arm64 # use macos-x86_64 on Intel
grep " ${asset}$" SHA256SUMS | shasum -a 256 -c -
chmod +x "$asset"
```

On Linux:

```sh
asset=rm-to-trash-linux-x86_64 # use linux-arm64 on ARM64
grep " ${asset}$" SHA256SUMS | sha256sum -c -
chmod +x "$asset"
```

On Windows PowerShell:

```powershell
$asset = "rm-to-trash-windows-x86_64.exe"
$expected = ((Select-String -Path SHA256SUMS -Pattern " $asset$").Line -split "\s+")[0]
$actual = (Get-FileHash -Algorithm SHA256 $asset).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "SHA-256 mismatch" }
```

The macOS assets are ad hoc signed and are not Apple-notarized. Build from the
tagged source if local policy rejects them.

## 2. Preview and install

On macOS or Linux:

```sh
./"$asset" install --codex --dry-run
./"$asset" install --codex
./"$asset" doctor --codex
```

On Windows PowerShell:

```powershell
.\rm-to-trash-windows-x86_64.exe install --codex --dry-run
.\rm-to-trash-windows-x86_64.exe install --codex
.\rm-to-trash-windows-x86_64.exe doctor --codex
```

`doctor` checks both clients by default. The `--codex` flag limits this check
to the selected client.

The installer copies itself to the neutral per-user path, validates the
existing JSON, creates a timestamped backup, replaces an older matching
handler, and preserves unrelated Codex hooks. Repeating the operation is
idempotent.

## 3. Review and trust

Restart Codex and open `/hooks`. Codex requires review before any non-managed
command hook can run. Inspect the source, matcher, and command, then trust the
exact definition.

Codex records trust against the definition's hash. A path, matcher, handler, or
plugin update changes that hash, so Codex skips the changed hook until it is
reviewed again.

The effective handler has this shape on macOS and Linux:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^Bash$",
        "hooks": [
          {
            "type": "command",
            "command": "\"/absolute/installed/path/rm-to-trash\" hook",
            "timeout": 10,
            "statusMessage": "Redirecting rm to the operating system Trash"
          }
        ]
      }
    ]
  }
}
```

On Windows, the installer also writes Codex's `commandWindows` override.

`^Bash$` is a regular expression over the canonical tool name. It matches Bash
and unified-exec calls presented as `Bash`, not command text. The binary then
performs its own conservative syntax inspection.

Use either `hooks.json` or inline `[hooks]` tables in a single configuration
layer. Current Codex documentation says defining both in the same layer merges
them and produces a warning.

## 4. Test CLI and desktop runtime

Ask Codex to execute a disposable fixture:

```sh
fixture="$(mktemp -d)"
touch "$fixture/direct" "$fixture/xargs" "$fixture/find"
rm "$fixture/direct"
printf '%s\0' "$fixture/xargs" | xargs -0 rm -f
find "$fixture" -name find -exec rm -f {} +
```

The files should leave the fixture and appear in the native Trash. Run the
representative request once in the CLI and once in the desktop app if you use
both because they can ship different Codex runtime versions.

On native Windows, run through Git Bash. WSL uses the Linux build and its Linux
FreeDesktop Trash.

## Coverage and limits

Current Codex behavior relevant to this project:

- shell and unified-exec calls use the `Bash` hook name;
- `PreToolUse` can return `allow` plus complete `updatedInput`;
- multiple matching command hooks launch concurrently;
- non-managed and plugin hooks require hash-based trust;
- hosted tools do not use the local tool-hook path; and
- specialized paths may opt out of the default hook path.

This makes the hook a useful recovery guardrail, not a complete enforcement
boundary.

## Manual configuration fallback

If policy forbids the installer:

1. copy the verified native binary to a stable absolute path;
2. back up `~/.codex/hooks.json`;
3. merge the handler shown above;
4. add `commandWindows` for a Windows installation; and
5. restart Codex, open `/hooks`, and trust the definition.

## Uninstall

Preview and remove only this handler:

```sh
rm-to-trash uninstall --codex --dry-run
rm-to-trash uninstall --codex
```

The binary remains at:

```text
~/.local/share/rm-to-trash/bin/rm-to-trash
```

Use the `.exe` filename on Windows. Delete it only after the uninstall process
exits and no Claude Code configuration references the shared binary.

See the official [Codex hooks documentation](https://learn.chatgpt.com/docs/hooks)
for current configuration, plugin, trust, matcher, and tool-coverage behavior.
