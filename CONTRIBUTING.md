# Contributing

Contributions are welcome when they preserve the hook’s conservative rewrite
contract.

## Development setup

Requirements:

- Rust 1.85 or newer;
- `cargo-audit` and `cargo-deny` for the complete security check.

Development is supported on macOS, Linux, and Windows. Native end-to-end Trash
tests use only disposable paths. Windows Bash-rewrite tests require Git Bash.

Create a focused branch, make the smallest change that handles the proposed
command shape, and add both positive and negative tests.

## Required verification

Run:

```sh
cargo fmt --check
RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo test --locked --release
RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo clippy --locked --all-targets --all-features --release -- -D warnings
cargo audit
cargo deny check
```

For rewrite changes, also send representative hook JSON to the release binary
and inspect the complete `updatedInput`. Use only disposable test files for
end-to-end Trash checks. Platform changes must pass the matching native GitHub
runner, not only cross-compilation.

## Rewrite changes

Every new supported form should demonstrate:

1. a valid deletion that is rewritten;
2. options that consume following arguments;
3. nearby `rm` text that is data rather than an executable;
4. no-operand behavior;
5. compound-command behavior where applicable; and
6. an explicit documentation update to
   [docs/rewrite-contract.md](docs/rewrite-contract.md).

Prefer a false negative over a false positive when the shell command position
cannot be proven from syntax.

## Pull requests

Keep changes focused. Describe the practical behavior change, commands used for
verification, and any unsupported edge that remains.

Do not commit:

- `target/` or other build intermediates;
- logs, transcripts, prompts, or hook debug output;
- local client settings;
- usernames, home-directory paths, secrets, or credentials; or
- binaries containing local build paths.

Release binaries are produced only by the tagged release workflow. Every asset
must be path-remapped, stripped, scanned, checksummed, and smoke-tested on its
matching native runner.

Before creating a release tag, manually dispatch the Release workflow from the
intended commit. A manual run builds and verifies all five assets but skips the
publication job. Create and push `vX.Y.Z` only after that rehearsal passes; the
tagged run repeats the builds and publishes the checksummed release.
