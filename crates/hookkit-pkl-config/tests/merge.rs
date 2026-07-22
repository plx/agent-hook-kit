//! End-to-end Pkl merge tests: evaluate small Pkl snippets and verify the
//! merged result.

use hookkit_pkl_config::merge::merge_chain;
use hookkit_pkl_config::{
    evaluate_pkl_source,
    schema::{
        CheckScope, FileActivityVcsFallback, InvocationGranularity, MissingToolPolicy,
        WriteBehavior,
    },
};

fn pkl_available() -> bool {
    std::process::Command::new("pkl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

macro_rules! require_pkl {
    () => {
        if !pkl_available() {
            eprintln!("skipping test: pkl binary not on PATH");
            return;
        }
    };
}

#[test]
fn file_activity_fallback_settings_round_trip() {
    require_pkl!();
    let config = evaluate_pkl_source(
        r#"
amends "Config.pkl"

settings {
  fileActivity = new FileActivity {
    filesystemMtime = false
    vcs = "git-dirty"
    timestampToleranceMillis = 750
    maxEntries = 1234
    ignoredDirectoryNames = new Listing<String> { ".git"; "vendor" }
  }
}
"#,
    )
    .expect("file activity settings");

    let activity = config.settings.file_activity.expect("file activity");
    assert!(!activity.filesystem_mtime);
    assert_eq!(activity.vcs, FileActivityVcsFallback::GitDirty);
    assert_eq!(activity.timestamp_tolerance_millis, 750);
    assert_eq!(activity.max_entries, 1234);
    assert_eq!(activity.ignored_directory_names, vec![".git", "vendor"]);
}

#[test]
fn deferred_workflow_schema_round_trips_structured_commands() {
    require_pkl!();
    let config = evaluate_pkl_source(
        r#"
amends "Config.pkl"

tools {
  ["example"] = new ToolSpec {
    id = "example"
    displayName = "Example"
    executable = "example"
    files { include = new Listing { "**/*.rs" } }
    workflows {
      ["lint"] = new Workflow {
        check = new WorkflowCommand {
          argv = new Listing { "check"; new Files {} }
          exitCodes { issues = new Listing { 1 } }
        }
        remedy = new WorkflowCommand {
          argv = new Listing { "fix"; new Files {} }
          writes = "target-files"
        }
        checkScope = "workspace"
        invocation = "per-file"
      }
    }
    workflowOrder = new Listing { "lint" }
  }
}
run = new Listing { "example" }
"#,
    )
    .expect("workflow config");

    let tool = config.tools.get("example").expect("tool");
    assert_eq!(tool.workflow_order, vec!["lint"]);
    let workflow = tool.workflows.get("lint").expect("workflow");
    assert_eq!(workflow.check_scope, CheckScope::Workspace);
    assert_eq!(workflow.invocation, InvocationGranularity::PerFile);
    assert_eq!(
        workflow.remedy.as_ref().expect("remedy").writes,
        WriteBehavior::TargetFiles
    );
    assert_eq!(
        workflow.check.as_ref().expect("check").exit_codes.issues,
        vec![1]
    );
}

#[test]
fn project_config_merges_over_user_config_default_behavior() {
    require_pkl!();
    let user = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    )
    .expect("user pkl");

    let project = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["prettier"] = Builtins.prettier
}
run = new Listing<String> { "ruff"; "prettier" }
"#,
    )
    .expect("project pkl");

    let merged = merge_chain([user, project].into_iter());

    assert!(merged.tools.contains_key("ruff"));
    assert!(merged.tools.contains_key("prettier"));
    assert_eq!(merged.run, vec!["ruff", "prettier"]);
}

#[test]
fn project_reset_tools_drops_user_tools() {
    require_pkl!();
    let user = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
  ["prettier"] = Builtins.prettier
}
run = new Listing<String> { "ruff"; "prettier" }
"#,
    )
    .expect("user pkl");

    let project = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

merge {
  reset = new Listing { "tools"; "run" }
}

tools {
  ["biome"] = Builtins.biome
}
run = new Listing<String> { "biome" }
"#,
    )
    .expect("project pkl");

    let merged = merge_chain([user, project].into_iter());

    assert!(!merged.tools.contains_key("ruff"));
    assert!(!merged.tools.contains_key("prettier"));
    assert!(merged.tools.contains_key("biome"));
    assert_eq!(merged.run, vec!["biome"]);
}

#[test]
fn reset_all_overrides_everything() {
    require_pkl!();
    let user = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

settings {
  missingToolPolicy = "user-notice"
  jobs = 4
}

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    )
    .expect("user pkl");

    let project = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

merge { resetAll = true }

settings {
  missingToolPolicy = "hard-failure"
}

tools {
  ["cargoFmt"] = Builtins.cargoFmt
}
run = new Listing<String> { "cargoFmt" }
"#,
    )
    .expect("project pkl");

    let merged = merge_chain([user, project].into_iter());

    assert_eq!(merged.tools.len(), 1);
    assert!(merged.tools.contains_key("cargoFmt"));
    assert_eq!(merged.run, vec!["cargoFmt"]);
    assert_eq!(
        merged.settings.missing_tool_policy,
        MissingToolPolicy::HardFailure
    );
    // jobs went back to default since resetAll cleared user settings
    assert_eq!(merged.settings.jobs, 0);
}

#[test]
fn reset_tools_drops_specific_tools_then_overlays() {
    require_pkl!();
    let user = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
  ["prettier"] = Builtins.prettier
}
run = new Listing<String> { "ruff"; "prettier" }
"#,
    )
    .expect("user pkl");

    let project = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

merge {
  resetTools = new Listing { "ruff" }
}

tools {
  ["eslint"] = Builtins.eslint
}
run = new Listing<String> { "prettier"; "eslint" }
"#,
    )
    .expect("project pkl");

    let merged = merge_chain([user, project].into_iter());

    assert!(!merged.tools.contains_key("ruff"));
    assert!(merged.tools.contains_key("prettier"));
    assert!(merged.tools.contains_key("eslint"));
    assert_eq!(merged.run, vec!["prettier", "eslint"]);
}

#[test]
fn extra_args_and_phase_overrides_apply() {
    require_pkl!();
    let project = evaluate_pkl_source(
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = (Builtins.ruff) {
    phases {
      ["fix"] {
        extraArgs = new Listing { "--unfixable"; "F401" }
      }
      ["verify"] {
        enabled = false
      }
    }
  }
}
run = new Listing<String> { "ruff" }
"#,
    )
    .expect("project pkl");

    let ruff = project.tools.get("ruff").expect("ruff tool");
    let fix = ruff.phases.get("fix").expect("fix phase");
    assert_eq!(fix.extra_args, vec!["--unfixable", "F401"]);
    let verify = ruff.phases.get("verify").expect("verify phase");
    assert!(!verify.enabled, "verify should be disabled by override");
}
