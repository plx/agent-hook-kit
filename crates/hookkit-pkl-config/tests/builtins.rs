//! Verify that the embedded Pkl builtins evaluate to specs matching the
//! previous hand-written Rust `specs::ruff()`, `specs::prettier()`, etc.
//!
//! These tests run only when `pkl` is on PATH; otherwise they short-circuit
//! with an explanatory skip message so the suite doesn't fail in environments
//! without Pkl installed.

use hookkit_pkl_config::schema::{
    ArgToken, ArgvElement, CheckScope, ExitCodes, FileSelection, InvocationGranularity, Phase,
    PhaseMode, ToolSpec, UnexpectedExitPolicy, Workflow, WorkflowCommand, WriteBehavior,
};
use std::collections::BTreeMap;

fn pkl_available() -> bool {
    std::process::Command::new("pkl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Whether Pkl-dependent tests must run instead of skipping. CI installs Pkl
/// and sets `HOOKKIT_REQUIRE_PKL=1`, so a missing binary fails the test.
fn pkl_required() -> bool {
    std::env::var_os("HOOKKIT_REQUIRE_PKL").is_some_and(|value| !value.is_empty() && value != "0")
}

macro_rules! require_pkl {
    () => {
        if !pkl_available() {
            assert!(
                !pkl_required(),
                "HOOKKIT_REQUIRE_PKL is set, but the pkl binary is not on PATH"
            );
            eprintln!("skipping test: pkl binary not on PATH");
            return;
        }
    };
}

fn literal(s: &str) -> ArgvElement {
    ArgvElement::Literal(s.into())
}

fn token(t: ArgToken) -> ArgvElement {
    ArgvElement::Token(t)
}

fn argv_eq(actual: &[ArgvElement], expected: &[ArgvElement]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual.iter().zip(expected).all(|(a, e)| match (a, e) {
        (ArgvElement::Literal(a), ArgvElement::Literal(e)) => a == e,
        (ArgvElement::Token(a), ArgvElement::Token(e)) => a == e,
        _ => false,
    })
}

fn assert_argv(phase: &Phase, expected: Vec<ArgvElement>) {
    assert!(
        argv_eq(&phase.argv, &expected),
        "argv mismatch:\nactual:   {:?}\nexpected: {:?}",
        phase.argv,
        expected,
    );
}

fn assert_workflow_argv(command: &WorkflowCommand, expected: Vec<ArgvElement>) {
    assert!(
        argv_eq(&command.argv, &expected),
        "argv mismatch:\nactual:   {:?}\nexpected: {:?}",
        command.argv,
        expected,
    );
}

fn assert_exit_codes(actual: &ExitCodes, clean: &[i32], issues: &[i32], failure: &[i32]) {
    assert_eq!(actual.clean, clean, "clean codes");
    assert_eq!(actual.issues, issues, "issue codes");
    assert_eq!(actual.failure, failure, "failure codes");
}

fn spec(specs: &std::collections::BTreeMap<String, ToolSpec>, key: &str) -> ToolSpec {
    specs
        .get(key)
        .unwrap_or_else(|| panic!("missing builtin: {key}"))
        .clone()
}

#[test]
fn ruff_builtin_matches_rust_spec() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let ruff = spec(&specs, "ruff");

    assert_eq!(ruff.id, "ruff");
    assert_eq!(ruff.display_name, "Ruff");
    assert_eq!(ruff.executable, "ruff");
    assert_eq!(
        ruff.install_hint.as_deref(),
        Some("install ruff with `brew install ruff` or add it to the project")
    );
    assert_eq!(
        ruff.files,
        FileSelection {
            include: vec![
                "*.py".into(),
                "**/*.py".into(),
                "*.pyi".into(),
                "**/*.pyi".into(),
            ],
            exclude: vec![],
        },
        "ruff include globs",
    );
    assert!(ruff.workspace_indicator.is_none());

    let format = ruff.phases.get("format").expect("format phase");
    assert_eq!(format.mode, PhaseMode::Format);
    assert_argv(
        format,
        vec![
            literal("format"),
            literal("--quiet"),
            literal("--force-exclude"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&format.exit_codes, &[0], &[], &[2]);
    assert_eq!(format.writes, WriteBehavior::TargetFiles);

    let fix = ruff.phases.get("fix").expect("fix phase");
    assert_eq!(fix.mode, PhaseMode::Fix);
    assert_argv(
        fix,
        vec![
            literal("check"),
            literal("--force-exclude"),
            literal("--fix"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&fix.exit_codes, &[0], &[1], &[2]);
    assert_eq!(fix.writes, WriteBehavior::TargetFiles);

    let verify = ruff.phases.get("verify").expect("verify phase");
    assert_eq!(verify.mode, PhaseMode::Verify);
    assert_argv(
        verify,
        vec![
            literal("check"),
            literal("--force-exclude"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&verify.exit_codes, &[0], &[1], &[2]);
    assert_eq!(verify.writes, WriteBehavior::None);

    assert_eq!(ruff.phase_order, vec!["format", "fix", "verify"]);
}

#[test]
fn prettier_builtin_has_expected_extensions() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let prettier = spec(&specs, "prettier");

    assert_eq!(prettier.id, "prettier");
    assert_eq!(prettier.display_name, "Prettier");
    assert_eq!(prettier.executable, "prettier");
    let include = &prettier.files.include;
    assert!(include.contains(&"*.ts".to_string()));
    assert!(include.contains(&"**/*.tsx".to_string()));
    assert!(include.contains(&"*.vue".to_string()));
    assert!(include.contains(&"*.json".to_string()));
    assert!(include.contains(&"*.md".to_string()));

    let format = prettier.phases.get("format").expect("format");
    assert_argv(
        format,
        vec![
            literal("--write"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&format.exit_codes, &[0], &[], &[]);
    assert_eq!(format.writes, WriteBehavior::TargetFiles);

    let verify = prettier.phases.get("verify").expect("verify");
    assert_argv(
        verify,
        vec![
            literal("--check"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&verify.exit_codes, &[0], &[1], &[]);
}

#[test]
fn eslint_builtin_matches_rust_spec() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let eslint = spec(&specs, "eslint");

    assert_eq!(eslint.id, "eslint");
    assert_eq!(eslint.display_name, "ESLint");
    assert_eq!(eslint.executable, "eslint");
    assert_eq!(
        eslint.files.include,
        vec![
            "*.js".to_string(),
            "**/*.js".into(),
            "*.jsx".into(),
            "**/*.jsx".into(),
            "*.ts".into(),
            "**/*.ts".into(),
            "*.tsx".into(),
            "**/*.tsx".into(),
        ],
    );

    let fix = eslint.phases.get("fix").expect("fix");
    assert_argv(
        fix,
        vec![
            literal("--fix"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&fix.exit_codes, &[0], &[1], &[]);
    assert_eq!(fix.writes, WriteBehavior::TargetFiles);

    let verify = eslint.phases.get("verify").expect("verify");
    assert_argv(
        verify,
        vec![token(ArgToken::ExtraArgs), token(ArgToken::Files)],
    );
    assert_exit_codes(&verify.exit_codes, &[0], &[1], &[]);
}

#[test]
fn biome_builtin_matches_rust_spec() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let biome = spec(&specs, "biome");

    assert_eq!(biome.id, "biome");
    assert_eq!(biome.display_name, "Biome");
    assert_eq!(biome.executable, "biome");

    let fix = biome.phases.get("fix").expect("fix");
    assert_argv(
        fix,
        vec![
            literal("check"),
            literal("--write"),
            literal("--no-errors-on-unmatched"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&fix.exit_codes, &[0], &[1], &[]);
    assert_eq!(fix.writes, WriteBehavior::TargetFiles);

    let verify = biome.phases.get("verify").expect("verify");
    assert_argv(
        verify,
        vec![
            literal("check"),
            literal("--no-errors-on-unmatched"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
}

#[test]
fn cargo_fmt_builtin_uses_workspace_indicator() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let cargo_fmt = spec(&specs, "cargoFmt");

    assert_eq!(cargo_fmt.id, "cargo-fmt");
    assert_eq!(cargo_fmt.display_name, "cargo fmt");
    assert_eq!(cargo_fmt.executable, "cargo");
    assert_eq!(cargo_fmt.workspace_indicator.as_deref(), Some("Cargo.toml"));

    let format = cargo_fmt.phases.get("format").expect("format");
    assert_argv(
        format,
        vec![
            literal("fmt"),
            literal("--manifest-path"),
            token(ArgToken::WorkspaceIndicator),
            token(ArgToken::ExtraArgs),
        ],
    );
    assert_exit_codes(&format.exit_codes, &[0], &[], &[]);
    assert_eq!(format.writes, WriteBehavior::MatchingGlobs);

    let verify = cargo_fmt.phases.get("verify").expect("verify");
    assert_argv(
        verify,
        vec![
            literal("fmt"),
            literal("--check"),
            literal("--manifest-path"),
            token(ArgToken::WorkspaceIndicator),
            token(ArgToken::ExtraArgs),
        ],
    );
}

#[test]
fn cargo_clippy_builtin_carries_custom_messages_and_unexpected_policy() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let clippy = spec(&specs, "cargoClippy");

    assert_eq!(clippy.id, "cargo-clippy");
    assert_eq!(clippy.display_name, "cargo clippy");
    assert_eq!(clippy.executable, "cargo");
    assert_eq!(clippy.workspace_indicator.as_deref(), Some("Cargo.toml"));

    let fix = clippy.phases.get("fix").expect("fix");
    assert_argv(
        fix,
        vec![
            literal("clippy"),
            literal("--manifest-path"),
            token(ArgToken::WorkspaceIndicator),
            literal("--fix"),
            literal("--allow-dirty"),
            literal("--allow-staged"),
            literal("--quiet"),
            token(ArgToken::ExtraArgs),
        ],
    );
    assert_exit_codes(&fix.exit_codes, &[0], &[101], &[]);
    assert_eq!(fix.exit_codes.unexpected, UnexpectedExitPolicy::Failure);
    assert_eq!(fix.writes, WriteBehavior::MatchingGlobs);

    let verify = clippy.phases.get("verify").expect("verify");
    assert_argv(
        verify,
        vec![
            literal("clippy"),
            literal("--manifest-path"),
            token(ArgToken::WorkspaceIndicator),
            literal("--quiet"),
            token(ArgToken::ExtraArgs),
        ],
    );
    assert_exit_codes(&verify.exit_codes, &[0], &[101], &[]);

    assert_eq!(
        clippy.messages.issues_agent,
        "cargo clippy reports issues; inspect diagnostics at {{ diagnostics_path }}."
    );
    assert_eq!(
        clippy.messages.issues_changed_agent,
        "cargo clippy changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}."
    );
}

#[test]
fn catalog_validator_rejects_unchecked_remedies_unless_fallback_is_explicit() {
    let mut legacy = ToolSpec {
        id: "legacy".into(),
        display_name: "Legacy".into(),
        executable: "legacy".into(),
        ..ToolSpec::default()
    };
    legacy.phases.insert(
        "format".into(),
        Phase {
            mode: PhaseMode::Format,
            writes: WriteBehavior::TargetFiles,
            ..Phase::default()
        },
    );
    let mut specs = BTreeMap::from([("legacy".into(), legacy.clone())]);
    let error = hookkit_pkl_config::validate_builtin_catalog(&specs)
        .expect_err("unchecked remedy must fail");
    assert!(error.to_string().contains("no authoritative final check"));

    legacy.unverified_remedy_fallback = Some("upstream has no read-only mode".into());
    specs.insert("legacy".into(), legacy);
    hookkit_pkl_config::validate_builtin_catalog(&specs).expect("explicit fallback rationale");

    let mut explicit = ToolSpec {
        id: "explicit".into(),
        display_name: "Explicit".into(),
        executable: "explicit".into(),
        ..ToolSpec::default()
    };
    explicit.workflows.insert(
        "format".into(),
        Workflow {
            remedy: Some(WorkflowCommand {
                writes: WriteBehavior::TargetFiles,
                ..WorkflowCommand::default()
            }),
            ..Workflow::default()
        },
    );
    let error = hookkit_pkl_config::validate_builtin_catalog(&BTreeMap::from([(
        "explicit".into(),
        explicit,
    )]))
    .expect_err("explicit remedy without a check must fail");
    assert!(error.to_string().contains("has no authoritative check"));
}

#[test]
fn formerly_mutating_only_tools_and_ruff_have_authoritative_workflows() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");

    for (key, check_prefix) in [
        ("goFmt", "-l"),
        ("goFumpt", "-l"),
        ("goImports", "-l"),
        ("goLines", "--dry-run"),
    ] {
        let tool = spec(&specs, key);
        let workflow = tool.workflows.get("format").expect("format workflow");
        let check = workflow.check.as_ref().expect("format check");
        let remedy = workflow.remedy.as_ref().expect("format remedy");
        assert!(check.issues_on_stdout, "{key} stdout issue adapter");
        assert_eq!(check.writes, WriteBehavior::None);
        assert_eq!(remedy.writes, WriteBehavior::TargetFiles);
        assert!(
            matches!(check.argv.first(), Some(ArgvElement::Literal(value)) if value == check_prefix)
        );
        assert_eq!(workflow.check_scope, CheckScope::TargetFiles);
        assert_eq!(workflow.invocation, InvocationGranularity::Batch);
    }

    let tidy = spec(&specs, "gomodTidy");
    assert_eq!(
        tidy.files.include,
        vec![
            "*.go",
            "**/*.go",
            "go.mod",
            "**/go.mod",
            "go.sum",
            "**/go.sum"
        ]
    );
    let tidy_workflow = tidy.workflows.get("tidy").expect("tidy workflow");
    assert_eq!(tidy_workflow.check_scope, CheckScope::Workspace);
    assert_eq!(tidy_workflow.invocation, InvocationGranularity::Workspace);
    assert_workflow_argv(
        tidy_workflow.check.as_ref().expect("tidy check"),
        vec![
            literal("mod"),
            literal("tidy"),
            literal("-diff"),
            token(ArgToken::ExtraArgs),
        ],
    );
    assert_eq!(
        tidy_workflow.remedy.as_ref().expect("tidy remedy").writes,
        WriteBehavior::Workspace
    );

    let yq = spec(&specs, "yq");
    let yq = yq.workflows.get("format").expect("yq workflow");
    let yq_check = yq.check.as_ref().expect("yq check");
    assert_eq!(yq_check.program.as_deref(), Some("sh"));
    assert!(
        yq_check
            .argv
            .iter()
            .any(|arg| matches!(arg, ArgvElement::Token(ArgToken::ToolExecutable)))
    );
    assert_eq!(yq.invocation, InvocationGranularity::PerFile);

    let ruff = spec(&specs, "ruff");
    assert_eq!(ruff.workflow_order, vec!["lint", "format"]);
    let lint = ruff.workflows.get("lint").expect("lint workflow");
    let format = ruff.workflows.get("format").expect("format workflow");
    assert!(matches!(
        lint.check.as_ref().and_then(|check| check.argv.first()),
        Some(ArgvElement::Literal(value)) if value == "check"
    ));
    assert!(matches!(
        format.check.as_ref().and_then(|check| check.argv.first()),
        Some(ArgvElement::Literal(value)) if value == "format"
    ));
    assert!(
        format
            .check
            .as_ref()
            .expect("format check")
            .argv
            .iter()
            .any(|arg| matches!(arg, ArgvElement::Literal(value) if value == "--check"))
    );
}

/// Run one builtin verify phase's literal argv against `file` and return the
/// exit code, or `None` when the executable is unavailable.
fn run_verify_phase(spec: &ToolSpec, file: &std::path::Path) -> Option<i32> {
    let phase = spec.phases.get("verify").expect("verify phase");
    let mut command = std::process::Command::new(&spec.executable);
    for arg in &phase.argv {
        match arg {
            ArgvElement::Literal(value) => {
                command.arg(value);
            }
            ArgvElement::Token(ArgToken::Files) => {
                command.arg(file);
            }
            ArgvElement::Token(ArgToken::ExtraArgs) => {}
            ArgvElement::Token(other) => panic!("unexpected token {other:?}"),
        }
    }
    let output = command.output().ok()?;
    output.status.code()
}

fn classify(codes: &ExitCodes, code: i32) -> &'static str {
    if codes.clean.contains(&code) {
        "clean"
    } else if codes.issues.contains(&code) {
        "issues"
    } else {
        "failure"
    }
}

fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hookkit-builtin-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn jq_builtin_parses_without_exit_status_mode() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let jq = spec(&specs, "jq");
    let verify = jq.phases.get("verify").expect("verify phase");
    // `jq -e empty` exits 4 for every valid file because `empty` produces no
    // output; only parse and read errors may be issues.
    assert_argv(
        verify,
        vec![
            literal("empty"),
            token(ArgToken::ExtraArgs),
            token(ArgToken::Files),
        ],
    );
    assert_exit_codes(&verify.exit_codes, &[0], &[2, 5], &[]);
    // jq parses all of its file arguments as one concatenated stream, so a
    // batch would validate files together instead of independently.
    assert_eq!(verify.invocation, InvocationGranularity::PerFile);

    let valid = temp_file("valid.json", "{\"name\": \"hookkit\"}\n");
    let null = temp_file("null.json", "null\n");
    let invalid = temp_file("invalid.json", "{not valid json}\n");
    let Some(valid_code) = run_verify_phase(&jq, &valid) else {
        eprintln!("skipping real-jq probe: jq not on PATH");
        return;
    };
    assert_eq!(classify(&verify.exit_codes, valid_code), "clean");
    let null_code = run_verify_phase(&jq, &null).unwrap();
    assert_eq!(classify(&verify.exit_codes, null_code), "clean");
    let invalid_code = run_verify_phase(&jq, &invalid).unwrap();
    assert_eq!(classify(&verify.exit_codes, invalid_code), "issues");

    // Two files that are only valid (or only invalid) when concatenated: a
    // split object passes a batched `jq empty`, and two scalar documents
    // without trailing newlines fail it. Each file is judged on its own.
    let head = temp_file("head.json", "{\"a\":");
    let tail = temp_file("tail.json", "1}");
    let first_scalar = temp_file("first.json", "true");
    let second_scalar = temp_file("second.json", "true");
    let batched = std::process::Command::new(&jq.executable)
        .args(["empty"])
        .arg(&head)
        .arg(&tail)
        .output()
        .unwrap();
    assert_eq!(
        classify(&verify.exit_codes, batched.status.code().unwrap()),
        "clean",
        "a batch of the two halves hides both parse errors"
    );
    for half in [&head, &tail] {
        let code = run_verify_phase(&jq, half).unwrap();
        assert_eq!(classify(&verify.exit_codes, code), "issues", "{half:?}");
    }
    for scalar in [&first_scalar, &second_scalar] {
        let code = run_verify_phase(&jq, scalar).unwrap();
        assert_eq!(classify(&verify.exit_codes, code), "clean", "{scalar:?}");
    }
    for path in [
        valid,
        null,
        invalid,
        head,
        tail,
        first_scalar,
        second_scalar,
    ] {
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

#[test]
fn check_merge_conflict_ignores_heading_underlines_but_finds_conflicts() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    let checker = spec(&specs, "checkMergeConflict");
    let verify = checker.phases.get("verify").expect("verify phase");
    let heading = temp_file("CHANGES.rst", "License\n=======\n\nSummary\n=======\n");
    let conflict = temp_file(
        "conflict.py",
        "a = 1\n<<<<<<< HEAD\nb = 2\n=======\nb = 3\n>>>>>>> branch\n",
    );
    let Some(heading_code) = run_verify_phase(&checker, &heading) else {
        eprintln!("skipping merge-conflict probe: grep not on PATH");
        return;
    };
    assert_eq!(classify(&verify.exit_codes, heading_code), "clean");
    let conflict_code = run_verify_phase(&checker, &conflict).unwrap();
    assert_eq!(classify(&verify.exit_codes, conflict_code), "issues");
    for path in [heading, conflict] {
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

#[test]
fn builtin_catalog_audit_is_current() {
    require_pkl!();
    let specs = hookkit_pkl_config::builtin_specs().expect("evaluate builtins");
    hookkit_pkl_config::validate_builtin_catalog(&specs).expect("valid builtin catalog");
    let generated = hookkit_pkl_config::render_builtin_catalog_markdown(&specs);
    if std::env::var_os("HOOKKIT_PRINT_BUILTIN_AUDIT").is_some() {
        eprintln!("HOOKKIT_AUDIT_BEGIN\n{generated}HOOKKIT_AUDIT_END");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../planning/builtin-deferred-workflow-audit.md");
    let checked_in = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert_eq!(checked_in, generated, "regenerate {}", path.display());
}
