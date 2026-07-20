# Changelog

All notable changes to this project are documented here.

## [1.1.0] - 2026-07-20

### Added

- Syntax-aware handling for `sudo`, `env`, `command`, `exec`, `nice`, `nohup`,
  `time`, and `noglob` wrappers.
- Indirect deletion handling for explicit `xargs rm`, `find -exec rm`, and
  `find -execdir rm` commands.
- Recursive rewriting for single-quoted literal command strings passed to
  common shells with `-c` and to single-argument `eval`.
- Negative tests for option arguments, quoted data, computed commands, remote
  commands, and operand-free deletion forms.
- Client-specific installation guides, an explicit rewrite contract, security
  reporting guidance, contribution guidance, and continuous integration.

### Changed

- `sudo` is removed from recognized deletion paths so recovery remains in the
  current user’s Trash.
- Parsing and hook protocol responsibilities are separated into focused source
  modules.

## [1.0.0] - 2026-07-14

### Added

- Initial standalone Rust hook for direct `rm` commands in Claude Code and
  Codex.
- Bash syntax parsing, ANSI escape cleanup, conservative fallback handling,
  and a stripped Apple Silicon release binary.

[1.1.0]: https://github.com/ShlomoStept/rm-to-trash-hook/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/ShlomoStept/rm-to-trash-hook/tree/v1.0.0
