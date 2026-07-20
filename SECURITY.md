# Security policy

## Supported versions

Security fixes are applied to the latest release. Users should upgrade before
reporting behavior that may already have been corrected.

## Report a vulnerability

Use GitHub’s private vulnerability reporting for this repository. Include:

- the affected version and macOS version;
- the exact command shape, with sensitive paths replaced by placeholders;
- the observed rewrite or bypass;
- the expected safe behavior; and
- a minimal reproduction when possible.

Do not include real prompts, transcripts, secrets, usernames, private paths, or
production data. Do not open a public issue for an unpatched vulnerability.

If private vulnerability reporting is unavailable, open a public issue that
contains no exploit details and asks the maintainer to provide a private
channel.

## Security model

`rm-to-trash` reduces accidental permanent deletion for supported local shell
calls. It is not a sandbox, authorization system, malware defense, backup, or
complete deletion interceptor. Unsupported and ambiguous forms remain subject
to Claude Code or Codex permission controls.
