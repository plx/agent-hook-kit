# Release process

MSRV is Rust 1.85. The libraries publish in dependency order:

1. `hookkit-core`
2. `hookkit-claude`, `hookkit-codex`, and `hookkit-antigravity`
3. `hookkit-common` and `hookkit-shell`
4. `hookkit-runtime`

Install the Rust 1.85 toolchain, then run `scripts/release-check.sh` before
tagging. The script and CI compile all supported libraries and runner crates on
the declared MSRV. CI checks every package file set
with `cargo package --list` and fully packages `hookkit-core`. Cargo cannot fully
assemble downstream archives until their versioned internal dependencies exist in
the registry; during an actual staged publish, run `cargo package` and `cargo
publish` at each step after the preceding package becomes visible.

`hookkit-pkl-config` and `hookkit-tool-runner` have a separate release lifecycle.
The non-published conformance tool and examples are never published.
