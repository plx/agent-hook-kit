# hookkit-shell

Opt-in shell tool-call extraction and bounded Bash syntax analysis for
HookKit. The crate is useful when a native `PreToolUse`, `BeforeTool`, or
corresponding post-tool event identifies only a shell tool plus an opaque
command string.

It provides four deliberately separate layers:

1. exact native adapters for recognized shell tools;
2. structural Bash facts, nesting context, redirections, and conservative
   literal argv recovery;
3. small best-effort summaries for common file reads, listings, and searches;
4. semantic file-access candidates with read/modify roles, target scope,
   provenance, certainty, and explicit unresolved gaps.

None of these layers executes the command or decides that it is safe.
Applications that need unified structured-field, patch, and shell evidence
should normally consume these facts through `hookkit-tool-access` rather than
rebuilding phase-specific extraction on top of this crate.

## Features

The parser has no harness dependency by default. Enable only the adapters a
hook uses:

```toml
[dependencies]
hookkit-shell = { version = "0.1", features = ["codex"] }
```

Available adapter features are `claude`, `codex`, and `antigravity`;
`all-harnesses` enables all three. The optional `serde` feature
adds serialization derives to owned analysis, summary, and file-access values.
Borrowed native tool-call JSON is never serialized implicitly.

## From native event to syntax facts

```rust
use hookkit_codex::protocol::PreToolUseInput;
use hookkit_shell::{
    BashAnalysisOutcome, BashAnalyzer, ShellToolCallExt, ShellToolCallMatch,
};

fn inspect(input: &PreToolUseInput) {
    let call = match input.shell_tool_call() {
        ShellToolCallMatch::Matched(call) => call,
        ShellToolCallMatch::NotShell => return,
        ShellToolCallMatch::Malformed(error) => {
            // The exact Bash tool matched, but its command field did not.
            eprintln!("{error}");
            return;
        }
        // The enum is non-exhaustive so callers remain forward-compatible.
        _ => return,
    };

    match BashAnalyzer::default().analyze(call.command) {
        BashAnalysisOutcome::Complete(analysis) => {
            for argv in analysis.literal_commands() {
                eprintln!("literal command: {argv:?}");
            }
            if analysis.contains_dynamic_syntax() {
                eprintln!("the syntax also contains runtime-dependent words");
            }
        }
        BashAnalysisOutcome::Partial { analysis, reason } => {
            // Facts remain useful for diagnostics, but a policy must account
            // for the missing or erroneous portion.
            eprintln!("partial Bash analysis ({reason:?}): {analysis:?}");
        }
        BashAnalysisOutcome::Unavailable(reason) => {
            eprintln!("Bash analysis unavailable: {reason:?}");
        }
        // Also non-exhaustive: treat an outcome this version does not know
        // as unavailable.
        _ => eprintln!("Bash analysis unavailable"),
    }
}
```

`Complete` means that Tree-sitter parsed and HookKit traversed the supplied
syntax within configured limits. It does not mean that runtime argv or effects
are completely known. Individual commands report either `ArgvStatus::Literal`
or `ArgvStatus::Dynamic`, with reasons such as parameter expansion, command
substitution, globbing, or unsupported escaping.

Where the grammar's tree shape differs from what Bash executes, the analysis
follows Bash:

- words the grammar attaches to a redirection after its target
  (`rm 2>/dev/null .env`) or after a here-document start (`rm <<EOF .env`)
  are command arguments, and redirections written after a here-document start
  (`cat <<EOF > file`) belong to the command;
- redirections the grammar hangs on a whole `&&`/`||` list, pipeline, or
  negation (`cd dir && apply_patch <<'EOF'`) belong to its last simple command;
- redirections on a compound statement, subshell, `if`/loop/`case`, test,
  declaration, or function definition are reported in
  `BashAnalysis::statement_redirections` with the range of enclosed commands,
  so a consumer can find, for example, the here-document read by
  `(cd dir && apply_patch) <<'EOF'`;
- a redirection with no command (`> file`, `< file`, `$(< file)`) is a
  nameless `CommandOccurrence` whose argv is empty;
- a backslash-newline inside a word joins its pieces, and `0<file`/`{fd}>file`
  descriptors are not arguments;
- comma and sequence brace expansion (`.e{n,}v`, `{1..3}`) is expanded
  statically into separate argv words sharing the source word's span, or the
  word is marked `DynamicReason::BraceExpansion` when it cannot be;
- backtick substitutions in an unquoted here-document body are re-parsed and
  their commands reported; one that cannot be re-parsed exactly makes the
  outcome `Partial` with `IncompleteReason::UnparsedCommandSubstitution`, and
  backslash escapes Bash would process leave `literal_body` unset.

Glob and `~` words carry `ShellWord::pattern`, their quote-removed text with
quoted glob metacharacters bracket-escaped, instead of a literal value.

## File-access inference

`FileAccessAnalyzer` turns the structural report into a practical, explicitly
incomplete view of files that a command may read or modify:

```rust
use hookkit_shell::{
    BashAnalyzer, FileAccessAnalyzer, FileInferenceContext, ShellToolCallRef,
};

fn inspect_files(call: &ShellToolCallRef<'_>) {
    let bash = BashAnalyzer::default().analyze(call.command);
    let access = FileAccessAnalyzer::default().infer(
        &bash,
        FileInferenceContext::for_call(call),
    );

    for candidate in access.may_read() {
        eprintln!("may read: {:?}", candidate.target);
    }
    for candidate in access.may_modify() {
        eprintln!("may modify: {:?}", candidate.target);
    }
    for gap in &access.unresolved {
        eprintln!("could not infer: {:?}", gap.reason);
    }
}
```

Each `FileAccessCandidate` carries:

- a `FileAccessKind`, including read, modify, read-modify, enumerate, delete,
  and distinct move-source/move-destination roles;
- a `FileTargetScope` distinguishing an exact path, descendants, an ambiguous
  exact-or-descendants operand, and an unexpanded glob;
- the raw path expression, an optional lexically resolved path, and its
  basis: absolute, the invocation cwd, unknown after a directory change, or
  the home directory for `~` paths (resolved only when
  `FileInferenceContext::with_home` supplies it);
- its argument, command or statement redirection, working-directory-default,
  or rule origin;
- `Direct`, `Conditional`, or `Heuristic` certainty and the rule identifier
  that inferred it.

The default analyzer understands file redirections (on commands and on
statements) and a deliberately bounded table of common readers, listings,
searches, `sed`, direct mutators, removals, copy/move/link commands, and
indirect evaluation:

- `sed` scripts are scanned for `r`/`R`/`w`/`W`/`e` commands and `s///w`/`e`
  flags unless `--sandbox` is given; script files and unparseable scripts are
  unresolved;
- `grep` without a path reads stdin unless it is recursive, and `rg` searches
  the working directory only when stdin is neither piped nor redirected; a
  dynamic search word that could be an option or split into several words is
  unresolved;
- `mv` sources, and every copy/move/link destination, cover a directory's
  descendants, and a destination also yields heuristic
  `destination/basename(source)` candidates;
- shells, language runtimes (`python`, `node`, `perl`, `awk`, ...), `xargs`,
  `find -exec`, and interactive pagers (`less`, `more`) are
  `IndirectEvaluation`;
- wrappers such as `sudo`, `env`, `nice`, `nohup`, `time`, `timeout`,
  `command`, `exec`, `builtin`, and `stdbuf` are skipped so the wrapped
  command's semantics apply (custom rules see the wrapped command too);
- names match exactly or as the basename of a path in a standard system binary
  directory such as `/usr/bin`.

Unknown commands, ambiguous option layouts, dynamic paths, indirect
evaluation, partial parsing, and unavailable analysis are retained in
`FileAccessReport::unresolved` instead of being silently treated as file-free.

`apply_patch` is recognized without candidates, because its targets are in
the patch body, which `hookkit-tool-access` parses. A standalone consumer must
not read a fully resolved report for `apply_patch` as proof that no file
changes.

Unknown, ambiguous, or indirectly evaluating commands can optionally use
`UnknownCommandFallback::LiteralPathOperands`. Its documented path-likeness
rule recognizes dot paths, absolute/explicit-relative/home prefixes,
separators, and filename-style dots (including the value side of
`name=value`) in literal words and in glob or `~` patterns; other dynamic
words are skipped. These candidates are `Heuristic` read-modify candidates
with argument provenance; the original unresolved semantics record is always
retained. The default is `Disabled`.

The command table is extensible without replacing the parser. Implement
`CommandFileSemantics`, emit candidates through `FileAccessSink`, and register
the rule with `FileAccessAnalyzer::with_semantics` or `register`. Custom rules
run before bundled rules, so a hook suite can model its own linters, formatters,
build tools, or wrappers. `emit_workspace` represents tools whose implicit
scope is the whole project; `emit_argument` retains the exact argv provenance.

Path resolution is lexical and does not touch the filesystem. It does not
canonicalize symlinks or expand globs. Relative paths resolve against the
native tool call's absolute cwd when available; after an earlier `cd`,
`pushd`, or `popd` (including `builtin cd` and a directory change anywhere in
an enclosing loop or function), later relative paths remain candidates but
their resolved path is withheld and an unresolved cwd gap is reported.

Native adapters recognize the contract-backed names and fields only:

| Harness event | Exact tool | Command location | Cwd |
| --- | --- | --- | --- |
| Claude `PreToolUse`, `PermissionRequest`, `PermissionDenied`, `PostToolUse`, `PostToolUseFailure` | `Bash` | `/command` | hook `cwd` |
| Codex `PreToolUse` / `PostToolUse` | `Bash` | `/command` | `/workdir`, else hook `cwd` (unverified) |
| Antigravity `PreToolUse` / `PostToolUse` | `run_command` | `/CommandLine` | `/Cwd`, else first workspace path |

The Claude catalog adapter returns `ShellToolCallMatch::UnsupportedEvent` for
other events rather than claiming they are not shell calls.

Antigravity's current `PostToolUse` input carries the originating `toolCall`, so
the same exact adapter is available in both tool phases. Its post-tool input
does not carry a tool result, so extraction has no response value.

A tool-input cwd must be absolute to become `ShellToolCallRef::cwd`. A
relative value is joined onto an absolute fallback and exposed only through
`ShellToolCallRef::effective_cwd`; an empty or `null` value falls back.
`ShellToolCallRef::cwd_origin` records where the cwd came from. Codex's
`exec_command` accepts a `workdir` argument that it joins onto the step's
environment cwd, but its hook payload carries only `command`, so the hook
`cwd` is only the default and the adapter reports
`ShellCwdOrigin::UnverifiedFallback`: relative paths resolved against it are
best effort.

Codex `exec_command` sessions with `tty: true` accept later input through
`write_stdin`, which emits no `PreToolUse` hook. An interactive program started
that way can open files or run commands the hook never sees, which is why
pagers and shells are reported as indirect evaluation.

Use `ShellToolProfile` to describe an exact custom tool name and JSON Pointer;
the bundled adapters do not guess aliases such as `shell` or `exec_command`.
With the matching harness features enabled, the exact reusable values are
`CLAUDE_BASH_PROFILE`, `CODEX_BASH_PROFILE`, and
`ANTIGRAVITY_RUN_COMMAND_PROFILE`.

## Analysis boundary

The report can expose commands nested in substitutions, pipelines, logical
lists, conditionals, loops, functions, and subshells. It also reports source
spans, assignments, expansions, redirections, heredocs, and here-strings.
Here-document facts include source-backed delimiters, body spans, and literal
bodies only when static quote/expansion analysis justifies recovery.

Static syntax analysis cannot resolve environment values, glob results, shell
functions or aliases, `eval`, sourced/generated scripts, executable behavior,
filesystem state, symlinks, subprocess effects, or which conditional branch
will execute. It is not a sandbox, and a parse failure must not be interpreted
as permission to run a command.

The analyzer accepts Bash syntax, but harness shell tools run the user's
shell: Claude Code and Codex use zsh when it is the user's shell (the macOS
default), and Codex uses PowerShell on Windows. The native payloads do not
identify the shell, so the bundled adapters report `ShellDialect::Unknown`.
Pass it to `FileInferenceContext::with_dialect`: unless the dialect is
`Bash`, zsh-only forms whose effect differs (`>! file`, `>>!file`) are
reported as unresolved alongside heuristic zsh-target candidates, and
`PowerShell` marks the whole analysis unresolved. `n>&word` with a filename
is treated as a write, as zsh performs it. A caller should not silently feed
the analyzer PowerShell, `cmd.exe`, or fish input.

Default resource limits are 256 KiB of source, 100 ms of parser time, 50,000
visited AST nodes, and 128 levels of traversal. `BashAnalyzerLimits` makes all
four bounds explicit and configurable through `with_*` builders. A subtree
deeper than the depth limit is skipped while its shallower siblings are still
analyzed, and `&&`/`||` chains are walked iteratively so their length does not
count toward the depth.

## Adversarial corpus

`tests/adversarial.rs` holds a table of guard-relevant shell forms, each with
the files it touches or the gap it must report. Set
`HOOKKIT_SHELL_DIFFERENTIAL=1` to also run its executable cases under `bash`
in a scratch directory of sentinel files and check every observed read, write,
creation, and removal against the analysis.

## Attribution

Literal-word recovery is adapted from OpenAI Codex's Apache-2.0-licensed
`shell-command` implementation. See [`NOTICE`](NOTICE) for the pinned upstream
revision and modification statement. Tree-sitter and tree-sitter-bash are
consumed as normal upstream dependencies under their respective licenses.
