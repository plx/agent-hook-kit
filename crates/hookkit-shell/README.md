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

## Features

The parser has no harness dependency by default. Enable only the adapters a
hook uses:

```toml
[dependencies]
hookkit-shell = { version = "0.1", features = ["codex"] }
```

Available adapter features are `claude`, `codex`, `gemini`, and
`antigravity`; `all-harnesses` enables all four. The optional `serde` feature
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
    }
}
```

`Complete` means that Tree-sitter parsed and HookKit traversed the supplied
syntax within configured limits. It does not mean that runtime argv or effects
are completely known. Individual commands report either `ArgvStatus::Literal`
or `ArgvStatus::Dynamic`, with reasons such as parameter expansion, command
substitution, globbing, or unsupported escaping.

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
        FileInferenceContext::new(call.cwd),
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
- the raw path expression, an optional lexically resolved path, and its cwd
  basis;
- its argument, redirection, working-directory-default, or rule origin;
- `Direct`, `Conditional`, or `Heuristic` certainty and the rule identifier
  that inferred it.

The default analyzer understands file redirections and a deliberately bounded
table of common readers, listings, searches, direct mutators, removals,
copy/move/link commands, and indirect shell evaluation. Unknown commands,
ambiguous option layouts, dynamic paths, indirect evaluation, partial parsing,
and unavailable analysis are retained in `FileAccessReport::unresolved`
instead of being silently treated as file-free.

The command table is extensible without replacing the parser. Implement
`CommandFileSemantics`, emit candidates through `FileAccessSink`, and register
the rule with `FileAccessAnalyzer::with_semantics` or `register`. Custom rules
run before bundled rules, so a hook suite can model its own linters, formatters,
build tools, or wrappers. `emit_workspace` represents tools whose implicit
scope is the whole project; `emit_argument` retains the exact argv provenance.

Path resolution is lexical and does not touch the filesystem. It does not
canonicalize symlinks or expand globs. Relative paths resolve against the
native tool call's cwd when available; after an earlier `cd`, `pushd`, or
`popd`, later relative paths remain candidates but their resolved path is
withheld and an unresolved cwd gap is reported.

Native adapters recognize the contract-backed names and fields only:

| Harness event | Exact tool | Command location |
| --- | --- | --- |
| Claude/Codex tool events | `Bash` | `/command` |
| Gemini `BeforeTool`/`AfterTool` | `run_shell_command` | `/command` |
| Antigravity `PreToolUse` | `run_command` | `/CommandLine` |

Antigravity's current `PostToolUse` input does not contain the originating tool
call, so no post-tool adapter is provided. Correlating it with prior events is
session-state work outside this crate.

Use `ShellToolProfile` to describe an exact custom tool name and JSON Pointer;
the bundled adapters do not guess aliases such as `shell` or `exec_command`.

## Analysis boundary

The report can expose commands nested in substitutions, pipelines, logical
lists, conditionals, loops, functions, and subshells. It also reports source
spans, assignments, expansions, redirections, heredocs, and here-strings.

Static syntax analysis cannot resolve environment values, glob results, shell
functions or aliases, `eval`, sourced/generated scripts, executable behavior,
filesystem state, symlinks, subprocess effects, or which conditional branch
will execute. It is not a sandbox, and a parse failure must not be interpreted
as permission to run a command.

The analyzer accepts Bash syntax. A caller should not silently feed it
PowerShell, `cmd.exe`, fish, or zsh-specific input.

Default resource limits are 256 KiB of source, 100 ms of parser time, 50,000
visited AST nodes, and 128 levels of traversal. `BashAnalyzerLimits` makes all
four bounds explicit and configurable.

## Attribution

Literal-word recovery is adapted from OpenAI Codex's Apache-2.0-licensed
`shell-command` implementation. See [`NOTICE`](NOTICE) for the pinned upstream
revision and modification statement. Tree-sitter and tree-sitter-bash are
consumed as normal upstream dependencies under their respective licenses.
