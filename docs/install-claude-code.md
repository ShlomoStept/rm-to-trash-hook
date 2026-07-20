# Install for Claude Code

The recommended user-level installation uses the downloaded native binary to
copy itself and merge one `PreToolUse` handler into
`~/.claude/settings.json`.

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

The macOS assets are ad hoc signed and are not Apple-notarized. If local policy
rejects them, build the tagged source instead of bypassing that policy.

## 2. Preview and install

On macOS or Linux:

```sh
./"$asset" install --claude --dry-run
./"$asset" install --claude
./"$asset" doctor --claude
```

On Windows PowerShell:

```powershell
.\rm-to-trash-windows-x86_64.exe install --claude --dry-run
.\rm-to-trash-windows-x86_64.exe install --claude
.\rm-to-trash-windows-x86_64.exe doctor --claude
```

`doctor` checks both clients by default. The `--claude` flag limits this check
to the selected client.

The installer:

1. copies itself to
   `~/.local/share/rm-to-trash/bin/rm-to-trash` (`.exe` on Windows);
2. validates the existing settings as JSON;
3. creates a timestamped backup next to the settings file;
4. removes an older `rm-to-trash` handler; and
5. adds the current handler without replacing unrelated settings.

Repeating the same installation does not add a duplicate.

## 3. Inspect the registered hook

Restart Claude Code and open `/hooks`. The effective handler has this shape:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/installed/path/rm-to-trash",
            "args": ["hook"],
            "timeout": 10,
            "statusMessage": "Redirecting rm to the operating system Trash"
          }
        ]
      }
    ]
  }
}
```

The installer merges this object. It does not replace the complete
`settings.json`.

Do not add an `if: "Bash(rm *)"` filter. The `Bash` matcher selects the tool
name. A direct-command filter would prevent the hook from seeing `sudo rm`,
`xargs rm`, `find -exec rm`, and literal nested shell forms. The binary starts
for every Bash tool call and performs a cheap token check before parsing.

## 4. Test with disposable files

Ask Claude Code to run these commands, rather than running them in an ordinary
terminal where the hook is absent:

```sh
fixture="$(mktemp -d)"
touch "$fixture/direct" "$fixture/wrapped" "$fixture/xargs"
rm "$fixture/direct"
sudo rm "$fixture/wrapped"
printf '%s\0' "$fixture/xargs" | xargs -0 rm -f
```

On Windows, use Git Bash and omit the `sudo` line. The affected files should
leave the fixture and appear in the operating system Recycle Bin.

Also run an unrelated command:

```sh
printf '%s\n' "hook smoke test"
```

It should behave normally with no hook output.

## Windows boundary

Native Windows `rm` coverage requires Git Bash. Claude Code can also expose a
separate PowerShell tool whose deletion command is `Remove-Item`. That tool has
a different syntax and does not match this hook's `Bash` matcher.

WSL uses the Linux asset and FreeDesktop Trash inside the WSL distribution, not
the Windows Recycle Bin.

## Manual configuration fallback

If policy forbids the installer:

1. copy the verified binary to a stable absolute path;
2. back up `~/.claude/settings.json`;
3. merge the handler shown above; and
4. restart Claude Code and inspect `/hooks`.

Use exec form (`command` plus `args`) so paths containing spaces are passed as
one executable path on every platform.

## Uninstall

Preview and remove only this handler:

```sh
rm-to-trash uninstall --claude --dry-run
rm-to-trash uninstall --claude
```

The installed binary remains so the same command behaves consistently on
Windows, where a running executable cannot remove itself. After the command
exits and no client configuration references it, delete:

```text
~/.local/share/rm-to-trash/bin/rm-to-trash
```

Use the `.exe` filename on Windows.

See the official [Claude Code hooks reference](https://code.claude.com/docs/en/hooks)
for current event, matcher, exec-form, and Windows-shell behavior.
