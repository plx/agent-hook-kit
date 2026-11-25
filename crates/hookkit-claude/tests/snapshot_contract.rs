//! Checks the native crate against every fixture and output schema of the
//! snapshot it implements (draft or frozen).
//!
//! The conformance crate exercises only the registry-selected snapshot and
//! only one representative case per outcome. This suite follows
//! [`SNAPSHOT_ID`] instead and covers every positive and negative input
//! fixture, every command output fixture, every command process case, and
//! every typed output constructor and builder.
#![allow(deprecated)]

use base64::Engine as _;
use hookkit_claude::catalog::{CatalogInput, PermissionRequestBehavior, PreToolPermissionDecision};
use hookkit_claude::events::*;
use hookkit_claude::protocol::{ClaudeCode, SNAPSHOT_ID};
use hookkit_core::{
    EventId, EventSpec, HarnessId, HarnessSpec, ProcessEmission, RawInvocation, Utf8PathBuf,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Snapshot event directory and native wire name for every event.
const EVENTS: [(&str, &str); 34] = [
    ("config-change", "ConfigChange"),
    ("cwd-changed", "CwdChanged"),
    ("directory-added", "DirectoryAdded"),
    ("elicitation", "Elicitation"),
    ("elicitation-result", "ElicitationResult"),
    ("file-changed", "FileChanged"),
    ("instructions-loaded", "InstructionsLoaded"),
    ("message-display", "MessageDisplay"),
    ("notification", "Notification"),
    ("permission-denied", "PermissionDenied"),
    ("permission-request", "PermissionRequest"),
    ("post-compact", "PostCompact"),
    ("post-model-switch", "PostModelSwitch"),
    ("post-tool-batch", "PostToolBatch"),
    ("post-tool-use", "PostToolUse"),
    ("post-tool-use-failure", "PostToolUseFailure"),
    ("pre-compact", "PreCompact"),
    ("pre-model-switch", "PreModelSwitch"),
    ("pre-skill-shell-command", "PreSkillShellCommand"),
    ("pre-tool-use", "PreToolUse"),
    ("session-end", "SessionEnd"),
    ("session-start", "SessionStart"),
    ("setup", "Setup"),
    ("stop", "Stop"),
    ("stop-failure", "StopFailure"),
    ("subagent-start", "SubagentStart"),
    ("subagent-stop", "SubagentStop"),
    ("task-completed", "TaskCompleted"),
    ("task-created", "TaskCreated"),
    ("teammate-idle", "TeammateIdle"),
    ("user-prompt-expansion", "UserPromptExpansion"),
    ("user-prompt-submit", "UserPromptSubmit"),
    ("worktree-create", "WorktreeCreate"),
    ("worktree-remove", "WorktreeRemove"),
];

fn snapshot_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/harnesses/claude-code/snapshots")
        .join(SNAPSHOT_ID.as_str())
}

fn read_yaml(path: PathBuf) -> Value {
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_yaml_ng::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn fixtures(event_dir: &str) -> Value {
    read_yaml(
        snapshot_root()
            .join("events")
            .join(event_dir)
            .join("fixtures.yaml"),
    )
}

fn output_validator(event_dir: &str) -> Option<jsonschema::Validator> {
    let path = snapshot_root()
        .join("events")
        .join(event_dir)
        .join("output.command.schema.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let schema: Value = serde_json::from_str(&text).unwrap();
    Some(
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
}

fn fixture_list<'a>(fixtures: &'a Value, pointer: &str) -> &'a [Value] {
    fixtures
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn fixture_id(fixture: &Value) -> &str {
    fixture["id"].as_str().expect("fixture id")
}

fn invocation(value: &Value) -> RawInvocation {
    RawInvocation::parse(serde_json::to_vec(value).unwrap()).unwrap()
}

fn wire_name(event_dir: &str) -> &'static str {
    EVENTS
        .iter()
        .find(|(dir, _)| *dir == event_dir)
        .map(|(_, name)| *name)
        .unwrap_or_else(|| panic!("unknown event directory {event_dir}"))
}

#[test]
fn the_crate_implements_exactly_the_snapshot_events() {
    let snapshot = read_yaml(snapshot_root().join("snapshot.yaml"));
    assert_eq!(snapshot["id"], SNAPSHOT_ID.as_str());
    // The crate may implement a draft before the integration step freezes
    // and selects it; a snapshot in any other state is not implementable.
    assert!(
        matches!(snapshot["state"].as_str(), Some("draft" | "frozen")),
        "snapshot state {}",
        snapshot["state"]
    );
    let documented: BTreeSet<(String, String)> = snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let path = event["path"].as_str().unwrap();
            (
                path.trim_start_matches("events/").to_owned(),
                event["wire_name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: BTreeSet<(String, String)> = EVENTS
        .iter()
        .map(|(dir, name)| ((*dir).to_owned(), (*name).to_owned()))
        .collect();
    assert_eq!(documented, expected);

    let descriptors = hookkit_claude::protocol::events();
    let implemented: BTreeSet<&str> = descriptors
        .iter()
        .map(|descriptor| descriptor.event().name())
        .collect();
    let wire_names: BTreeSet<&str> = EVENTS.iter().map(|(_, name)| *name).collect();
    assert_eq!(implemented, wire_names);
    for descriptor in &descriptors {
        assert_eq!(descriptor.snapshot(), SNAPSHOT_ID);
        assert_eq!(
            descriptor.contract().as_str(),
            format!(
                "claude-code/{}/{}",
                SNAPSHOT_ID.as_str(),
                descriptor.event().name()
            )
        );
    }

    let identification = ClaudeCode::identification_descriptors();
    let identified: BTreeSet<&str> = identification
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.snapshot(), SNAPSHOT_ID);
            assert_eq!(
                descriptor.discriminator(),
                Some(("/hook_event_name", descriptor.event().name()))
            );
            descriptor.event().name()
        })
        .collect();
    assert_eq!(identified, wire_names);
    assert_eq!(identification.len(), wire_names.len());
}

/// Every positive fixture parses through the exact parser and the dynamic
/// harness decoder, re-serializes to the same JSON value, and yields the
/// documented native context. Every negative fixture is rejected.
fn check_inputs<E>(event_dir: &str)
where
    E: EventSpec,
    E::Input: Serialize,
{
    let fixtures = fixtures(event_dir);
    let positives = fixture_list(&fixtures, "/input/positive");
    assert!(
        positives.len() >= 2,
        "{event_dir}: expected minimal and representative positives"
    );
    for fixture in positives {
        let id = fixture_id(fixture);
        let value = &fixture["value"];
        let raw = invocation(value);
        let input =
            E::parse(&raw).unwrap_or_else(|error| panic!("{event_dir}/{id} must parse: {error}"));
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            *value,
            "{event_dir}/{id} must round-trip"
        );

        let context = E::context(&input);
        assert_eq!(
            context.workspace_roots,
            vec![Utf8PathBuf::from(value["cwd"].as_str().unwrap())],
            "{event_dir}/{id}"
        );
        assert_eq!(
            context.session_id.as_ref().map(|id| id.as_str()),
            value["session_id"].as_str(),
            "{event_dir}/{id}"
        );
        // A PreSkillShellCommand `tool_use_id` names no tool call.
        let tool_call_id = if E::EVENT.name() == "PreSkillShellCommand" {
            None
        } else {
            value["tool_use_id"].as_str()
        };
        assert_eq!(
            context.tool_call_id.as_ref().map(|id| id.as_str()),
            tool_call_id,
            "{event_dir}/{id}"
        );

        let decoded = ClaudeCode::decode(&E::EVENT, &raw)
            .unwrap_or_else(|error| panic!("{event_dir}/{id} must decode: {error}"));
        assert_eq!(
            ClaudeCode::input_event(&decoded),
            E::EVENT,
            "{event_dir}/{id}"
        );
    }

    let negatives = fixture_list(&fixtures, "/input/negative");
    assert!(!negatives.is_empty(), "{event_dir}: expected negatives");
    for fixture in negatives {
        let id = fixture_id(fixture);
        let raw = invocation(&fixture["value"]);
        assert!(
            E::parse(&raw).is_err(),
            "{event_dir}/{id} must be rejected by the native parser"
        );
        assert!(
            ClaudeCode::decode(&E::EVENT, &raw).is_err(),
            "{event_dir}/{id} must be rejected by the harness decoder"
        );
    }
}

#[test]
fn every_positive_input_fixture_parses_and_every_negative_is_rejected() {
    check_inputs::<ConfigChange>("config-change");
    check_inputs::<CwdChanged>("cwd-changed");
    check_inputs::<DirectoryAdded>("directory-added");
    check_inputs::<Elicitation>("elicitation");
    check_inputs::<ElicitationResult>("elicitation-result");
    check_inputs::<FileChanged>("file-changed");
    check_inputs::<InstructionsLoaded>("instructions-loaded");
    check_inputs::<MessageDisplay>("message-display");
    check_inputs::<Notification>("notification");
    check_inputs::<PermissionDenied>("permission-denied");
    check_inputs::<PermissionRequest>("permission-request");
    check_inputs::<PostCompact>("post-compact");
    check_inputs::<PostModelSwitch>("post-model-switch");
    check_inputs::<PostToolBatch>("post-tool-batch");
    check_inputs::<PostToolUse>("post-tool-use");
    check_inputs::<PostToolUseFailure>("post-tool-use-failure");
    check_inputs::<PreCompact>("pre-compact");
    check_inputs::<PreModelSwitch>("pre-model-switch");
    check_inputs::<PreSkillShellCommand>("pre-skill-shell-command");
    check_inputs::<PreToolUse>("pre-tool-use");
    check_inputs::<SessionEnd>("session-end");
    check_inputs::<SessionStart>("session-start");
    check_inputs::<Setup>("setup");
    check_inputs::<Stop>("stop");
    check_inputs::<StopFailure>("stop-failure");
    check_inputs::<SubagentStart>("subagent-start");
    check_inputs::<SubagentStop>("subagent-stop");
    check_inputs::<TaskCompleted>("task-completed");
    check_inputs::<TaskCreated>("task-created");
    check_inputs::<TeammateIdle>("teammate-idle");
    check_inputs::<UserPromptExpansion>("user-prompt-expansion");
    check_inputs::<UserPromptSubmit>("user-prompt-submit");
    check_inputs::<WorktreeCreate>("worktree-create");
    check_inputs::<WorktreeRemove>("worktree-remove");
}

/// Input fields the snapshot schema may require but the native parser
/// accepts when missing, for forward and backward compatibility: a parse
/// failure exits 1, which fails a gating hook open. As `(event directory,
/// field, reason)`.
const TOLERATED_MISSING_FIELDS: &[(&str, &str, &str)] = &[
    (
        "user-prompt-expansion",
        "command_source",
        "the Agent SDK (0.3.223, 0.3.285) types it optional",
    ),
    (
        "teammate-idle",
        "team_name",
        "deprecated; Claude Code announced its removal",
    ),
];

#[test]
fn tolerated_missing_fields_are_accepted_for_compatibility() {
    for (event_dir, field, reason) in TOLERATED_MISSING_FIELDS {
        let mut value = positive(event_dir, "representative");
        assert!(
            value.as_object_mut().unwrap().remove(*field).is_some(),
            "{event_dir}: the representative fixture no longer sends {field}; drop the entry"
        );
        // `check_inputs` still requires every negative fixture to be
        // rejected, so none of them may be a payload missing only this field.
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, wire_name(event_dir));
        let decoded = ClaudeCode::decode(&event, &invocation(&value))
            .unwrap_or_else(|error| panic!("{event_dir} without {field} ({reason}): {error}"));
        let hookkit_claude::protocol::AnyInput::Catalog(input) = decoded else {
            panic!("{event_dir} is a catalog event");
        };
        assert_eq!(serde_json::to_value(&input).unwrap(), value, "{event_dir}");
        assert_eq!(input.field(field), None);
    }
}

#[test]
fn known_event_schema_violations_are_invalid_input_for_the_hinted_event() {
    // Discriminator matches, but a required envelope or event field is
    // missing: every parser reports the hinted event, whichever check fails.
    for (event_dir, field) in [
        ("session-start", "source"),
        ("post-tool-use", "tool_use_id"),
        ("worktree-create", "name"),
        ("pre-model-switch", "to_model"),
        ("pre-tool-use", "cwd"),
        ("pre-tool-use", "tool_name"),
        ("pre-skill-shell-command", "command_name"),
        ("pre-skill-shell-command", "invoked_by"),
        ("pre-skill-shell-command", "tool_input"),
    ] {
        let mut value = positive(event_dir, "minimal");
        value.as_object_mut().unwrap().remove(field);
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, wire_name(event_dir));
        let error = ClaudeCode::decode(&event, &invocation(&value)).unwrap_err();
        assert!(
            matches!(
                &error,
                hookkit_core::HookkitError::InvalidInputForHint { event: hinted, message }
                    if *hinted == event && message.contains(field)
            ),
            "{event_dir} without {field}: {error:?}"
        );
    }
}

fn positive(event_dir: &str, id: &str) -> Value {
    let fixtures = fixtures(event_dir);
    fixture_list(&fixtures, "/input/positive")
        .iter()
        .find(|fixture| fixture_id(fixture) == id)
        .unwrap_or_else(|| panic!("{event_dir}/{id} is missing"))["value"]
        .clone()
}

#[test]
fn typed_accessors_read_the_snapshot_fixtures() {
    let raw = invocation(&positive("pre-tool-use", "mcp-tool"));
    let input = PreToolUse::parse(&raw).unwrap();
    assert!(input.tool_name().unwrap().starts_with("mcp__"));
    assert!(input.tool_input().is_some());
    assert!(input.tool_use_id().is_some());
    let server = input.mcp_server().unwrap();
    assert!(!server.name.is_empty());

    let raw = invocation(&positive("post-tool-use", "mcp-tool"));
    let input = PostToolUse::parse(&raw).unwrap();
    assert!(input.mcp_server.is_some());

    let raw = invocation(&positive("user-prompt-submit", "representative"));
    let input = UserPromptSubmit::parse(&raw).unwrap();
    assert_eq!(input.prompt(), raw.json()["prompt"].as_str());
    assert_eq!(input.stop_hook_active(), None);

    let raw = invocation(&positive("stop", "minimal"));
    let input = Stop::parse(&raw).unwrap();
    assert_eq!(
        input.stop_hook_active(),
        raw.json()["stop_hook_active"].as_bool()
    );
    assert_eq!(input.last_assistant_message(), None);

    let raw = invocation(&positive("subagent-stop", "internal-agent"));
    let input = SubagentStop::parse(&raw).unwrap();
    assert_eq!(input.agent_type(), Some(""));
    assert!(input.agent_transcript_path().is_some());

    let raw = invocation(&positive(
        "session-end",
        "legacy-bypass-permissions-disabled",
    ));
    let input = SessionEnd::parse(&raw).unwrap();
    assert_eq!(
        input.session_end_reason().unwrap().as_str(),
        "bypass_permissions_disabled"
    );

    let raw = invocation(&positive("notification", "quota-auto-resume"));
    let input = Notification::parse(&raw).unwrap();
    assert!(input.notification_type().unwrap().is_documented());
    assert!(input.notification_message().is_some());

    let raw = invocation(&positive("stop-failure", "cloud-credential-error"));
    let input = StopFailure::parse(&raw).unwrap();
    assert_eq!(
        input.stop_failure_error().unwrap().as_str(),
        "cloud_credential_error"
    );

    let raw = invocation(&positive("pre-compact", "minimal"));
    let input = PreCompact::parse(&raw).unwrap();
    assert_eq!(input.custom_instructions(), None);
    assert_eq!(input.compact_trigger().unwrap().as_str(), "auto");

    let raw = invocation(&positive("post-model-switch", "automatic-fallback"));
    let input = PostModelSwitch::parse(&raw).unwrap();
    assert_eq!(input.source.as_str(), "auto");
    assert_eq!(input.requested_model, None);

    let raw = invocation(&positive("session-start", "representative"));
    let input = SessionStart::parse(&raw).unwrap();
    assert_eq!(input.context_tokens, Some(182_340));
    assert!(input.scratchpad_dir.is_some());
}

#[test]
fn typed_accessors_read_the_docs_2026_11_25_fixtures() {
    use hookkit_claude::protocol::{CompactTrigger, NotificationType, SkillInvoker};

    // PreSkillShellCommand: every documented field has a scoped accessor.
    let raw = invocation(&positive("pre-skill-shell-command", "representative"));
    let input = PreSkillShellCommand::parse(&raw).unwrap();
    assert_eq!(input.tool_name(), Some("PowerShell"));
    assert_eq!(input.injected_command(), Some("git log -5 --oneline"));
    assert_eq!(input.tool_use_id(), Some("skillsh_1"));
    assert_eq!(input.command_name(), Some("release-notes"));
    assert_eq!(input.command_source(), Some("plugin"));
    assert_eq!(input.invoked_by(), Some(SkillInvoker::Claude));
    assert_eq!(input.prompt(), None);
    let raw = invocation(&positive("pre-skill-shell-command", "subagent-preload"));
    let input = PreSkillShellCommand::parse(&raw).unwrap();
    assert!(input.invoked_by().unwrap().is_other());
    assert_eq!(input.command_source(), None);
    assert_eq!(input.agent_id(), Some("agent-1"));
    let raw = invocation(&positive(
        "pre-skill-shell-command",
        "unrecognized-invoked-by",
    ));
    let input = PreSkillShellCommand::parse(&raw).unwrap();
    let invoker = input.invoked_by().unwrap();
    assert_eq!(invoker, SkillInvoker::Unknown("scheduler".into()));
    assert!(invoker.is_other() && !invoker.is_documented());
    let raw = invocation(&positive("pre-skill-shell-command", "plugin-skill"));
    assert_eq!(
        PreSkillShellCommand::parse(&raw).unwrap().command_name(),
        Some("my-plugin:deploy")
    );

    // The same names on UserPromptExpansion; not on PreToolUse.
    let raw = invocation(&positive("user-prompt-expansion", "mcp-skill"));
    let input = UserPromptExpansion::parse(&raw).unwrap();
    assert_eq!(input.command_name(), Some("docs:pdf"));
    assert_eq!(input.injected_command(), None);
    assert_eq!(input.invoked_by(), None);
    let raw = invocation(&positive("pre-tool-use", "monitor-call"));
    let input = PreToolUse::parse(&raw).unwrap();
    assert_eq!(input.tool_name(), Some("Monitor"));
    assert_eq!(input.injected_command(), None);
    assert_eq!(input.command_name(), None);

    // UserPromptSubmit session_title and unexpanded slash-command prompts.
    let raw = invocation(&positive("user-prompt-submit", "representative"));
    let input = UserPromptSubmit::parse(&raw).unwrap();
    assert_eq!(input.session_title(), Some("Review"));
    let raw = invocation(&positive("user-prompt-submit", "slash-command-line"));
    let input = UserPromptSubmit::parse(&raw).unwrap();
    assert_eq!(input.prompt(), Some("/deploy staging"));
    assert_eq!(input.session_title(), None);
    let raw = invocation(&positive("user-prompt-submit", "sdk-source"));
    let input = UserPromptSubmit::parse(&raw).unwrap();
    assert_eq!(input.field("source"), Some(&json!("loop_wakeup")));
    let mut expansion = positive("user-prompt-expansion", "representative");
    expansion["session_title"] = "Review".into();
    let input = UserPromptExpansion::parse(&invocation(&expansion)).unwrap();
    assert_eq!(input.session_title(), None);

    // New notification types are documented; an unrelated one stays open.
    for (id, kind) in [
        ("auth-storage-failure", NotificationType::AuthStorageFailure),
        ("plugin-notification", NotificationType::PluginNotification),
    ] {
        let raw = invocation(&positive("notification", id));
        let input = Notification::parse(&raw).unwrap();
        assert_eq!(input.notification_type(), Some(kind), "{id}");
    }
    let raw = invocation(&positive("notification", "plugin-notification"));
    assert_eq!(
        Notification::parse(&raw).unwrap().notification_title(),
        Some("ci-watch")
    );
    assert_eq!(
        NotificationType::from("plugin_notification"),
        NotificationType::PluginNotification
    );
    assert!(!NotificationType::from("plugin_notifications").is_documented());

    // MessageDisplay's single non-interactive call carries the whole message.
    let raw = invocation(&positive("message-display", "non-interactive-single-call"));
    let input = MessageDisplay::parse(&raw).unwrap();
    assert_eq!(input.message_index(), Some(0));
    assert_eq!(input.message_final(), Some(true));
    assert_eq!(
        input.message_delta(),
        Some("Here is the plan:\n1. Build\n2. Test")
    );
    let raw = invocation(&positive("message-display", "interactive-empty-final"));
    let input = MessageDisplay::parse(&raw).unwrap();
    assert_eq!(
        (
            input.message_index(),
            input.message_final(),
            input.message_delta()
        ),
        (Some(2), Some(true), Some(""))
    );

    // Plugin-scoped agent types and user-declined elicitation results.
    let raw = invocation(&positive("subagent-start", "plugin-agent"));
    let input = SubagentStart::parse(&raw).unwrap();
    assert_eq!(input.agent_type(), Some("my-plugin:reviewer"));
    let raw = invocation(&positive("elicitation-result", "user-declined"));
    let input = ElicitationResult::parse(&raw).unwrap();
    assert_eq!(input.field("action"), Some(&json!("decline")));

    // A managed PostToolUse hook sees a failed call's error as a string.
    let raw = invocation(&positive("post-tool-use", "managed-failed-call"));
    let input = PostToolUse::parse(&raw).unwrap();
    assert!(input.tool_response.is_string());

    // Subagent compaction carries the agent fields on SessionStart and
    // PreCompact.
    let raw = invocation(&positive("session-start", "subagent-compact"));
    let input = SessionStart::parse(&raw).unwrap();
    assert_eq!(input.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(input.source.as_str(), "compact");
    let raw = invocation(&positive("pre-compact", "subagent-compaction"));
    let input = PreCompact::parse(&raw).unwrap();
    assert_eq!(input.agent_type(), Some("Explore"));
    assert_eq!(input.compact_trigger(), Some(CompactTrigger::Auto));

    // PostToolBatch items include a refused call with no marker.
    let raw = invocation(&positive("post-tool-batch", "refused-call-in-batch"));
    let input = PostToolBatch::parse(&raw).unwrap();
    assert_eq!(input.tool_calls().unwrap().len(), 2);
}

fn check_schema(event_dir: &str, value: &Value) {
    let Some(validator) = output_validator(event_dir) else {
        panic!("{event_dir} has no command output schema");
    };
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path().as_str()))
        .collect();
    assert!(
        errors.is_empty(),
        "{event_dir}: emitted {value} violates the output schema: {errors:?}"
    );
}

/// Validates JSON stdout against the event's command output schema and
/// returns it. Text, empty, and stderr-only emissions return `None`.
fn emitted_json(event_dir: &str, emission: &ProcessEmission) -> Option<Value> {
    let stdout = emission.stdout();
    let trimmed = String::from_utf8_lossy(stdout);
    let trimmed = trimmed.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return None;
    }
    let value: Value = serde_json::from_slice(stdout)
        .unwrap_or_else(|error| panic!("{event_dir}: stdout is not JSON: {error}"));
    check_schema(event_dir, &value);
    assert!(
        matches!(emission.exit_code(), 0 | 2),
        "{event_dir}: JSON is emitted only with exit 0 or 2"
    );
    Some(value)
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn catalog<E: EventSpec>(output: E::CommandOutput) -> ProcessEmission {
    E::emit(output).unwrap_or_else(|error| panic!("{}: {error}", E::EVENT.name()))
}

/// Typed outputs reproducing each command output fixture and each declared
/// process case, by fixture id.
fn fixture_output(event_dir: &str, id: &str) -> Option<ProcessEmission> {
    let process = id.starts_with("command-");
    let id = id.strip_prefix("command-").unwrap_or(id);
    const MISSING_FILE: &str = "/bin/sh: /repo/.claude/hooks/check.sh: No such file or directory\n";
    let emission = match (event_dir, id) {
        ("config-change", "structured" | "exit-2-structured") => {
            let output = ConfigChangeOutput::block("Configuration change rejected.");
            if id == "structured" {
                catalog::<ConfigChange>(output)
            } else {
                catalog::<ConfigChange>(output.into_blocking_error("blocked by hook").unwrap())
            }
        }
        ("config-change", "exit-2") => {
            catalog::<ConfigChange>(ConfigChangeOutput::blocking_error("blocked by hook"))
        }
        ("config-change", "nonzero-unstructured") => {
            catalog::<ConfigChange>(ConfigChangeOutput::nonblocking_error("hook failed"))
        }
        ("cwd-changed", "structured") => catalog::<CwdChanged>(
            CwdChangedOutput::system_message("Working directory changed.")
                .with_watch_paths(vec!["/repo/crate/.env".into()])
                .unwrap(),
        ),
        ("cwd-changed", "nonzero-unstructured") => {
            catalog::<CwdChanged>(CwdChangedOutput::nonblocking_error("hook failed"))
        }
        ("directory-added", "structured") => catalog::<DirectoryAdded>(
            DirectoryAddedOutput::system_message("Working directory added."),
        ),
        ("elicitation", "structured") => {
            catalog::<Elicitation>(ElicitationOutput::accept(map(json!({"name": "Ada"}))))
        }
        ("elicitation", "decision-block") => {
            catalog::<Elicitation>(ElicitationOutput::block("Declined by policy."))
        }
        ("elicitation", "decision-block-overrides-action") => catalog::<Elicitation>(
            ElicitationOutput::accept_without_content()
                .with_block("r")
                .unwrap(),
        ),
        ("elicitation", "exit-2") => {
            catalog::<Elicitation>(ElicitationOutput::blocking_error("blocked by hook"))
        }
        ("elicitation", "nonzero-unstructured") => {
            catalog::<Elicitation>(ElicitationOutput::nonblocking_error("hook failed"))
        }
        ("elicitation-result", "structured") => {
            catalog::<ElicitationResult>(ElicitationResultOutput::decline())
        }
        ("elicitation-result", "decision-block") => {
            catalog::<ElicitationResult>(ElicitationResultOutput::block("Declined by policy."))
        }
        ("elicitation-result", "decision-block-overrides-action") => catalog::<ElicitationResult>(
            ElicitationResultOutput::accept(map(json!({"name": "Ada"})))
                .with_block("r")
                .unwrap(),
        ),
        ("elicitation-result", "cancel") => {
            catalog::<ElicitationResult>(ElicitationResultOutput::cancel())
        }
        ("elicitation-result", "official-change-values") => catalog::<ElicitationResult>(
            ElicitationResultOutput::accept(map(json!({"username": "alice@example.com"}))),
        ),
        ("elicitation-result", "exit-2") => {
            catalog::<ElicitationResult>(ElicitationResultOutput::blocking_error("blocked by hook"))
        }
        ("elicitation-result", "nonzero-unstructured") => {
            catalog::<ElicitationResult>(ElicitationResultOutput::nonblocking_error("hook failed"))
        }
        ("file-changed", "structured") => catalog::<FileChanged>(
            FileChangedOutput::system_message("Watched file changed.")
                .with_watch_paths(vec!["/repo/.env".into(), "/repo/.env.local".into()])
                .unwrap(),
        ),
        ("file-changed", "nonzero-unstructured") => {
            catalog::<FileChanged>(FileChangedOutput::nonblocking_error("hook failed"))
        }
        ("instructions-loaded", "structured") => {
            catalog::<InstructionsLoaded>(InstructionsLoadedOutput::no_op())
        }
        ("instructions-loaded", "discarded-universal-fields") => catalog::<InstructionsLoaded>(
            InstructionsLoadedOutput::with_system_message("Instructions audited.")
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Ignored.")
                .unwrap(),
        ),
        ("message-display", "structured") => {
            catalog::<MessageDisplay>(MessageDisplayOutput::display("Here is the plan:"))
        }
        ("notification", "structured") => {
            catalog::<Notification>(NotificationOutput::terminal_sequence("\u{7}"))
        }
        ("notification", "discarded-universal-fields") => catalog::<Notification>(
            NotificationOutput::with_system_message("Permission notification emitted.")
                .with_continue(true)
                .unwrap(),
        ),
        ("permission-denied", "structured") => {
            catalog::<PermissionDenied>(PermissionDeniedOutput::retry(true))
        }
        ("permission-request", "structured") => catalog::<PermissionRequest>(
            PermissionRequestOutput::deny("Blocked by policy.")
                .with_interrupt(false)
                .unwrap(),
        ),
        ("permission-request", "set-mode-manual") => catalog::<PermissionRequest>(
            PermissionRequestOutput::allow()
                .with_updated_permissions(vec![json!({
                    "type": "setMode",
                    "mode": "manual",
                    "destination": "session",
                })])
                .unwrap(),
        ),
        ("permission-request", "add-rules-and-directories") => catalog::<PermissionRequest>(
            PermissionRequestOutput::allow()
                .with_updated_permissions(vec![
                    json!({
                        "type": "addRules",
                        "rules": [
                            {"toolName": "Bash", "ruleContent": "npm test"},
                            {"toolName": "mcp__github__get_issue"},
                        ],
                        "behavior": "allow",
                        "destination": "localSettings",
                    }),
                    json!({
                        "type": "addDirectories",
                        "directories": ["/repo/related"],
                        "destination": "session",
                    }),
                ])
                .unwrap(),
        ),
        ("permission-request", "exit-2-structured") => {
            catalog::<PermissionRequest>(PermissionRequestOutput::deny("Blocked by policy."))
        }
        ("permission-request", "nonzero-unstructured") => {
            catalog::<PermissionRequest>(PermissionRequestOutput::nonblocking_error("hook failed"))
        }
        ("post-compact", "structured") => catalog::<PostCompact>(PostCompactOutput::no_op()),
        ("post-compact", "discarded-universal-fields") => catalog::<PostCompact>(
            PostCompactOutput::with_system_message("Compaction complete.")
                .with_continue(true)
                .unwrap(),
        ),
        ("post-compact", "nonzero-unstructured") => {
            catalog::<PostCompact>(PostCompactOutput::nonblocking_error("hook failed"))
        }
        ("post-model-switch", "structured") => {
            catalog::<PostModelSwitch>(PostModelSwitchOutput::with_context(
                "On Opus, delegate implementation work to subagents.",
            ))
        }
        ("post-model-switch", "text") => catalog::<PostModelSwitch>(
            PostModelSwitchOutput::text_context("Hook-provided context."),
        ),
        ("post-model-switch", "text-open-brace") => catalog::<PostModelSwitch>(
            PostModelSwitchOutput::text_context("{ context without a closing brace"),
        ),
        ("post-model-switch", "text-json-array") => {
            catalog::<PostModelSwitch>(PostModelSwitchOutput::text_context(r#"["a","b"]"#))
        }
        ("post-model-switch", "text-json-string") => {
            catalog::<PostModelSwitch>(PostModelSwitchOutput::text_context(r#""quoted""#))
        }
        ("post-model-switch", "nonzero-unstructured") => {
            catalog::<PostModelSwitch>(PostModelSwitchOutput::nonblocking_error("hook failed"))
        }
        ("post-tool-batch", "structured") => {
            catalog::<PostToolBatch>(PostToolBatchOutput::with_context("Hook-provided context."))
        }
        ("post-tool-batch", "exit-2-structured") => catalog::<PostToolBatch>(
            PostToolBatchOutput::block("Stop before the next model call.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("post-tool-batch", "exit-2") => {
            catalog::<PostToolBatch>(PostToolBatchOutput::blocking_error("blocked by hook"))
        }
        ("post-tool-batch", "nonzero-unstructured") => {
            catalog::<PostToolBatch>(PostToolBatchOutput::nonblocking_error("hook failed"))
        }
        ("post-tool-use", "structured") => catalog::<PostToolUse>(
            PostToolUseOutput::with_context("Generated files changed.")
                .with_updated_tool_output(json!({"status": "redacted"}))
                .unwrap()
                .with_block("Review result.")
                .unwrap(),
        ),
        ("post-tool-use", "exit-2-structured") => catalog::<PostToolUse>(
            PostToolUseOutput::with_context("Generated files changed.")
                .into_feedback_error("blocked by hook")
                .unwrap(),
        ),
        ("post-tool-use", "classifier-context") => catalog::<PostToolUse>(
            PostToolUseOutput::no_op()
                .with_classifier_context(
                    "This query ran against the staging database, not production.",
                )
                .unwrap(),
        ),
        ("post-tool-use", "exit-2") => {
            catalog::<PostToolUse>(PostToolUseOutput::feedback_error("blocked by hook"))
        }
        ("post-tool-use", "nonzero-unstructured") => {
            catalog::<PostToolUse>(PostToolUseOutput::nonblocking_error("hook failed"))
        }
        ("post-tool-use-failure", "structured") => catalog::<PostToolUseFailure>(
            PostToolUseFailureOutput::with_context("Hook-provided context."),
        ),
        ("post-tool-use-failure", "exit-2-structured") => catalog::<PostToolUseFailure>(
            PostToolUseFailureOutput::with_context("Retry with --offline.")
                .into_feedback_error("blocked by hook")
                .unwrap(),
        ),
        ("post-tool-use-failure", "exit-2") => catalog::<PostToolUseFailure>(
            PostToolUseFailureOutput::feedback_error("blocked by hook"),
        ),
        ("post-tool-use-failure", "nonzero-unstructured") => catalog::<PostToolUseFailure>(
            PostToolUseFailureOutput::nonblocking_error("hook failed"),
        ),
        ("pre-compact", "structured") => {
            catalog::<PreCompact>(PreCompactOutput::block("Save state first."))
        }
        ("pre-compact", "exit-2-structured") => catalog::<PreCompact>(
            PreCompactOutput::block("Save state first.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("pre-compact", "exit-2") => {
            catalog::<PreCompact>(PreCompactOutput::blocking_error("blocked by hook"))
        }
        ("pre-compact", "nonzero-unstructured") => {
            catalog::<PreCompact>(PreCompactOutput::nonblocking_error("hook failed"))
        }
        ("pre-compact", "no-op") => catalog::<PreCompact>(PreCompactOutput::no_op()),
        ("pre-compact", "compaction-instructions") => catalog::<PreCompact>(
            PreCompactOutput::compaction_instructions("Preserve the failing test names.\n")
                .unwrap(),
        ),
        ("pre-compact", "text-json-array") => catalog::<PreCompact>(
            PreCompactOutput::compaction_instructions(r#"["a","b"]"#).unwrap(),
        ),
        ("pre-compact", "text-over-cap") => catalog::<PreCompact>(
            PreCompactOutput::compaction_instructions("a".repeat(10_001)).unwrap(),
        ),
        ("pre-model-switch", "structured") => catalog::<PreModelSwitch>(PreModelSwitchOutput::ask(
            "Switching now re-sends about 180k tokens to the new model. Continue?",
        )),
        ("pre-model-switch", "exit-2-structured") => catalog::<PreModelSwitch>(
            PreModelSwitchOutput::allow()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("pre-model-switch", "block") => catalog::<PreModelSwitch>(PreModelSwitchOutput::block(
            "Opus 4.6 is retired for this project.",
        )),
        ("pre-model-switch", "cost-notice") => catalog::<PreModelSwitch>(
            PreModelSwitchOutput::no_op()
                .with_system_message("Switching re-caches about 182k tokens (about $1.14).")
                .unwrap(),
        ),
        ("pre-model-switch", "exit-2") => {
            catalog::<PreModelSwitch>(PreModelSwitchOutput::blocking_error("blocked by hook"))
        }
        ("pre-model-switch", "nonzero-unstructured") => {
            catalog::<PreModelSwitch>(PreModelSwitchOutput::nonblocking_error("hook failed"))
        }
        ("pre-tool-use", "structured") => catalog::<PreToolUse>(
            PreToolUseOutput::ask("Review command.")
                .with_updated_input(map(json!({"command": "cargo test"})))
                .unwrap()
                .with_additional_context("Production environment.")
                .unwrap(),
        ),
        ("pre-tool-use", "exit-2-structured") => catalog::<PreToolUse>(
            PreToolUseOutput::allow()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("pre-tool-use", "exit-2") => {
            catalog::<PreToolUse>(PreToolUseOutput::blocking_error("blocked by hook"))
        }
        ("pre-tool-use", "nonzero-unstructured") => {
            catalog::<PreToolUse>(PreToolUseOutput::nonblocking_error("hook failed"))
        }
        ("pre-tool-use", "official-decision-control") => catalog::<PreToolUse>(
            PreToolUseOutput::allow()
                .with_decision_reason("My reason here")
                .unwrap()
                .with_updated_input(map(json!({"field_to_modify": "new value"})))
                .unwrap()
                .with_additional_context("Current environment: production. Proceed with caution.")
                .unwrap(),
        ),
        ("pre-tool-use", "updated-input-without-decision") => catalog::<PreToolUse>(
            PreToolUseOutput::no_op()
                .with_updated_input(map(json!({"command": "cargo test --offline"})))
                .unwrap(),
        ),
        ("pre-tool-use", "updated-input-unknown-key") => catalog::<PreToolUse>(
            PreToolUseOutput::allow()
                .with_updated_input(map(json!({"command": "ls", "bogus": true})))
                .unwrap(),
        ),
        ("pre-tool-use", "deny-and-stop") => catalog::<PreToolUse>(
            PreToolUseOutput::deny("Blocked.")
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Policy stop.")
                .unwrap(),
        ),
        ("pre-tool-use", "continue-false-without-deny") => catalog::<PreToolUse>(
            PreToolUseOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Stop after this call.")
                .unwrap(),
        ),
        ("pre-tool-use", "exit-2-continue-false") => catalog::<PreToolUse>(
            PreToolUseOutput::no_op()
                .with_continue(false)
                .unwrap()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("pre-tool-use", "exit-2-missing-file") => {
            catalog::<PreToolUse>(PreToolUseOutput::blocking_error(MISSING_FILE))
        }
        ("pre-tool-use", "defer") => catalog::<PreToolUse>(PreToolUseOutput::defer()),
        ("pre-skill-shell-command", "structured") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::deny("curl is not allowed in skills."),
        ),
        ("pre-skill-shell-command", "ask-refuses") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::ask("Confirm network access."),
        ),
        ("pre-skill-shell-command", "decision-block") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::block("Destructive command blocked by hook"),
        ),
        ("pre-skill-shell-command", "continue-false") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Skill disabled by policy.")
                .unwrap(),
        ),
        ("pre-skill-shell-command", "exit-2") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::blocking_error("blocked by hook"),
        ),
        ("pre-skill-shell-command", "exit-2-skill-policy") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::blocking_error("curl is not allowed in skills"),
        ),
        ("pre-skill-shell-command", "nonzero-unstructured") => catalog::<PreSkillShellCommand>(
            PreSkillShellCommandOutput::nonblocking_error("hook failed"),
        ),
        ("session-end", "structured") => catalog::<SessionEnd>(SessionEndOutput::no_op()),
        ("session-end", "discarded-universal-fields") => {
            catalog::<SessionEnd>(SessionEndOutput::with_system_message("Session ended."))
        }
        ("session-end", "nonzero-unstructured") => {
            catalog::<SessionEnd>(SessionEndOutput::nonblocking_error("hook failed"))
        }
        ("session-start", "structured") => catalog::<SessionStart>(
            SessionStartOutput::with_context("Read conventions.")
                .with_reload_skills(true)
                .unwrap()
                .with_session_title("Review")
                .unwrap()
                .with_watch_paths(vec!["/repo/.env".into()])
                .unwrap(),
        ),
        ("session-start", "text") => {
            catalog::<SessionStart>(SessionStartOutput::text_context("Hook-provided context."))
        }
        ("session-start", "text-open-brace") => catalog::<SessionStart>(
            SessionStartOutput::text_context("{ context without a closing brace"),
        ),
        ("session-start", "text-json-array") => {
            catalog::<SessionStart>(SessionStartOutput::text_context(r#"["a","b"]"#))
        }
        ("session-start", "text-json-string") => {
            catalog::<SessionStart>(SessionStartOutput::text_context(r#""quoted""#))
        }
        ("session-start", "nonzero-unstructured") => {
            catalog::<SessionStart>(SessionStartOutput::nonblocking_error("hook failed"))
        }
        ("setup", "structured") => catalog::<Setup>(SetupOutput::no_op()),
        ("setup", "discarded-universal-fields") => catalog::<Setup>(
            SetupOutput::no_op()
                .with_continue(true)
                .unwrap()
                .with_system_message("Dependencies installed.")
                .unwrap(),
        ),
        ("stop", "structured") => catalog::<Stop>(StopOutput::block_with_context(
            "Run tests again.",
            "Focus on failures.",
        )),
        ("stop", "exit-2-structured") => catalog::<Stop>(
            StopOutput::block("Run tests again.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("stop", "exit-2") => catalog::<Stop>(StopOutput::blocking_error("blocked by hook")),
        ("stop", "exit-2-other-stderr") => {
            catalog::<Stop>(StopOutput::blocking_error("tests failed"))
        }
        ("stop", "exit-2-missing-file-with-stdout") => catalog::<Stop>(
            StopOutput::block("Run tests again.")
                .into_blocking_error(MISSING_FILE)
                .unwrap(),
        ),
        ("stop", "nonzero-unstructured") => {
            catalog::<Stop>(StopOutput::nonblocking_error("hook failed"))
        }
        ("stop-failure", "structured") => catalog::<StopFailure>(StopFailureOutput::no_op()),
        ("stop-failure", "terminal-sequence") => {
            catalog::<StopFailure>(StopFailureOutput::terminal_sequence("\u{7}"))
        }
        ("subagent-start", "structured") => {
            catalog::<SubagentStart>(SubagentStartOutput::with_context("Hook-provided context."))
        }
        ("subagent-start", "nonzero-unstructured") => {
            catalog::<SubagentStart>(SubagentStartOutput::nonblocking_error("hook failed"))
        }
        ("subagent-stop", "structured") => catalog::<SubagentStop>(
            SubagentStopOutput::with_context("Check edge cases.")
                .with_block("Run another pass.")
                .unwrap(),
        ),
        ("subagent-stop", "exit-2-structured") => catalog::<SubagentStop>(
            SubagentStopOutput::block("Run another pass.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("subagent-stop", "exit-2") => {
            catalog::<SubagentStop>(SubagentStopOutput::blocking_error("blocked by hook"))
        }
        ("subagent-stop", "exit-2-other-stderr") => {
            catalog::<SubagentStop>(SubagentStopOutput::blocking_error("tests failed"))
        }
        ("subagent-stop", "exit-2-missing-file-with-stdout") => catalog::<SubagentStop>(
            SubagentStopOutput::block("Run another pass.")
                .into_blocking_error(MISSING_FILE)
                .unwrap(),
        ),
        ("subagent-stop", "nonzero-unstructured") => {
            catalog::<SubagentStop>(SubagentStopOutput::nonblocking_error("hook failed"))
        }
        ("task-completed", "structured") => catalog::<TaskCompleted>(
            TaskCompletedOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Verification is incomplete.")
                .unwrap(),
        ),
        ("task-completed", "exit-2-structured") => catalog::<TaskCompleted>(
            TaskCompletedOutput::no_op()
                .with_system_message("Tests are still failing.")
                .unwrap()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("task-completed", "exit-2") => {
            catalog::<TaskCompleted>(TaskCompletedOutput::blocking_error("blocked by hook"))
        }
        ("task-completed", "exit-2-other-stderr") => {
            catalog::<TaskCompleted>(TaskCompletedOutput::blocking_error("tests failed"))
        }
        ("task-completed", "exit-2-missing-file-with-stdout") => catalog::<TaskCompleted>(
            TaskCompletedOutput::no_op()
                .with_system_message("Tests are still failing.")
                .unwrap()
                .into_blocking_error(MISSING_FILE)
                .unwrap(),
        ),
        // The output example and the process cases use different reasons.
        ("task-completed", "block") if !process => catalog::<TaskCompleted>(
            TaskCompletedOutput::block("Tests not passing. Fix failing tests before completing."),
        ),
        ("task-completed", "block" | "exit-2-block") => {
            let output = TaskCompletedOutput::block("Tests not passing.");
            if id == "block" {
                catalog::<TaskCompleted>(output)
            } else {
                catalog::<TaskCompleted>(output.into_blocking_error("blocked by hook").unwrap())
            }
        }
        // The task stays open whatever triggered the event, so the exit-2
        // form of continue: false is not rejected here.
        ("task-completed", "exit-2-continue-false") => catalog::<TaskCompleted>(
            TaskCompletedOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Done.")
                .unwrap()
                .into_blocking_error("keep going")
                .unwrap(),
        ),
        ("task-completed", "nonzero-unstructured") => {
            catalog::<TaskCompleted>(TaskCompletedOutput::nonblocking_error("hook failed"))
        }
        ("task-created", "structured" | "exit-2-structured") => {
            let output = TaskCreatedOutput::block("Task needs an owner.");
            if id == "structured" {
                catalog::<TaskCreated>(output)
            } else {
                catalog::<TaskCreated>(output.into_blocking_error("blocked by hook").unwrap())
            }
        }
        ("task-created", "continue-ignored") => catalog::<TaskCreated>(
            TaskCreatedOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Ignored for TaskCreated.")
                .unwrap(),
        ),
        ("task-created", "exit-2") => {
            catalog::<TaskCreated>(TaskCreatedOutput::blocking_error("blocked by hook"))
        }
        ("task-created", "nonzero-unstructured") => {
            catalog::<TaskCreated>(TaskCreatedOutput::nonblocking_error("hook failed"))
        }
        ("teammate-idle", "structured") => catalog::<TeammateIdle>(
            TeammateIdleOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("Continue reviewing.")
                .unwrap(),
        ),
        ("teammate-idle", "exit-2-structured") => catalog::<TeammateIdle>(
            TeammateIdleOutput::no_op()
                .with_system_message("Teammate kept working.")
                .unwrap()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("teammate-idle", "exit-2") => {
            catalog::<TeammateIdle>(TeammateIdleOutput::blocking_error("blocked by hook"))
        }
        ("teammate-idle", "block" | "exit-2-block") => {
            let output = TeammateIdleOutput::block("Review the remaining files first.");
            if id == "block" {
                catalog::<TeammateIdle>(output)
            } else {
                catalog::<TeammateIdle>(output.into_blocking_error("blocked by hook").unwrap())
            }
        }
        ("teammate-idle", "nonzero-unstructured") => {
            catalog::<TeammateIdle>(TeammateIdleOutput::nonblocking_error("hook failed"))
        }
        ("user-prompt-expansion", "structured") => {
            catalog::<UserPromptExpansion>(UserPromptExpansionOutput::block_with_context(
                "Unavailable.",
                "Use the team checklist.",
            ))
        }
        ("user-prompt-expansion", "suppress-original-prompt") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::block("Unavailable.")
                .with_suppress_original_prompt(true)
                .unwrap(),
        ),
        ("user-prompt-expansion", "exit-2-structured") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::block("Blocked by JSON reason.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("user-prompt-expansion", "text") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::text_context("Hook-provided context."),
        ),
        ("user-prompt-expansion", "text-open-brace") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::text_context("{ context without a closing brace"),
        ),
        ("user-prompt-expansion", "text-json-array") => {
            catalog::<UserPromptExpansion>(UserPromptExpansionOutput::text_context(r#"["a","b"]"#))
        }
        ("user-prompt-expansion", "text-json-string") => {
            catalog::<UserPromptExpansion>(UserPromptExpansionOutput::text_context(r#""quoted""#))
        }
        ("user-prompt-expansion", "exit-2") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::blocking_error("blocked by hook"),
        ),
        ("user-prompt-expansion", "nonzero-unstructured") => catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::nonblocking_error("hook failed"),
        ),
        ("user-prompt-submit", "structured") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::block("Confirmation required.")
                .with_additional_context("Clarify scope.")
                .unwrap()
                .with_session_title("Clarify")
                .unwrap()
                .with_suppress_original_prompt(true)
                .unwrap(),
        ),
        ("user-prompt-submit", "exit-2-structured") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::block("Blocked by JSON reason.")
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("user-prompt-submit", "exit-2-suppress-original-prompt") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::no_op()
                .with_suppress_original_prompt(true)
                .unwrap()
                .into_blocking_error("blocked by hook")
                .unwrap(),
        ),
        ("user-prompt-submit", "session-title-only") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::no_op()
                .with_session_title("Test run")
                .unwrap(),
        ),
        ("user-prompt-submit", "text") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::text_context("Hook-provided context."),
        ),
        ("user-prompt-submit", "text-open-brace") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::text_context("{ context without a closing brace"),
        ),
        ("user-prompt-submit", "text-json-array") => {
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::text_context(r#"["a","b"]"#))
        }
        ("user-prompt-submit", "text-json-string") => {
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::text_context(r#""quoted""#))
        }
        ("user-prompt-submit", "official-decision-control") => catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::block("Explanation for decision")
                .with_additional_context("My additional context here")
                .unwrap()
                .with_session_title("My session title")
                .unwrap()
                .with_suppress_original_prompt(true)
                .unwrap(),
        ),
        ("user-prompt-submit", "exit-2") => {
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::blocking_error("blocked by hook"))
        }
        ("user-prompt-submit", "nonzero-unstructured") => {
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::nonblocking_error("hook failed"))
        }
        ("worktree-create", "created") => catalog::<WorktreeCreate>(
            WorktreeCreateOutput::path_with_newline("/tmp/hookkit-worktree".into()).unwrap(),
        ),
        ("worktree-create", "failed") => {
            catalog::<WorktreeCreate>(WorktreeCreateOutput::failed("", 1).unwrap())
        }
        ("worktree-remove", "removed") => {
            catalog::<WorktreeRemove>(WorktreeRemoveOutput::removed())
        }
        ("worktree-remove", "failed") => catalog::<WorktreeRemove>(
            WorktreeRemoveOutput::failed("worktree is still in use", 1).unwrap(),
        ),
        ("worktree-remove", "exit-2") => catalog::<WorktreeRemove>(
            WorktreeRemoveOutput::failed("worktree is still in use", 2).unwrap(),
        ),
        _ => return None,
    };
    Some(emission)
}

#[test]
fn every_command_output_fixture_is_reproduced_by_typed_constructors() {
    for (event_dir, _) in EVENTS {
        let fixtures = fixtures(event_dir);
        for fixture in fixture_list(&fixtures, "/output") {
            let id = fixture_id(fixture);
            if fixture["schema"] != "command-response" {
                // WorktreeCreate's only JSON output is the HTTP response; a
                // command hook prints the path instead.
                assert_eq!(
                    (event_dir, fixture["schema"].as_str()),
                    ("worktree-create", Some("http-response"))
                );
                continue;
            }
            check_schema(event_dir, &fixture["value"]);
            if let Some(reason) = unreproduced_output_reason(event_dir, id) {
                assert!(
                    fixture_output(event_dir, id).is_none(),
                    "{event_dir}/{id} is reproduced, so drop its classification ({reason})"
                );
                continue;
            }
            let emission = fixture_output(event_dir, id)
                .unwrap_or_else(|| panic!("{event_dir}/{id}: add a typed constructor"));
            assert_eq!(
                emitted_json(event_dir, &emission).as_ref(),
                Some(&fixture["value"]),
                "{event_dir}/{id}"
            );
        }
    }
}

/// Events whose plain-text stdout becomes context, which `text_context`
/// reproduces byte for byte unless it is brace-delimited.
const TEXT_CONTEXT_EVENTS: &[&str] = &[
    "post-model-switch",
    "session-start",
    "user-prompt-expansion",
    "user-prompt-submit",
];

/// Why a documented command output example is not reproduced by a typed
/// constructor.
fn unreproduced_output_reason(event_dir: &str, id: &str) -> Option<&'static str> {
    Some(match (event_dir, id) {
        ("elicitation-result", "content-without-action") => {
            "Claude Code ignores hookSpecificOutput without an action; no_op() has the same effect"
        }
        ("setup", "discarded-hook-specific-output") => {
            "Claude Code validates and then discards Setup additionalContext; no_op() has the same effect"
        }
        ("pre-tool-use", "deprecated-decision-block") => {
            "the deprecated top-level decision, which HookKit never emits; deny(reason) is the current form"
        }
        ("pre-skill-shell-command", "allow-no-op" | "exit-2-structured") => {
            "allow adds nothing on PreSkillShellCommand, so there is no allow constructor; no_op() lets the command run"
        }
        ("pre-skill-shell-command", "updated-input-refuses") => {
            "updatedInput is never applied there and only blocks the command; deny(reason) blocks with a reason"
        }
        ("pre-skill-shell-command", "pretooluse-format-deny") => {
            "typed outputs use the event's own hookEventName; deny(reason) is the same answer"
        }
        _ => return None,
    })
}

/// Why a documented process case is not reproduced by a typed constructor.
fn undeclared_reason(event_dir: &str, id: &str) -> Option<&'static str> {
    Some(match (event_dir, id) {
        (_, "command-invalid-json" | "command-schema-invalid-json") => {
            "malformed JSON that typed constructors never emit"
        }
        (_, "command-json-lines-with-field") => {
            "several JSON lines, one setting a field, which Claude Code fails to parse; typed constructors emit one JSON object"
        }
        (
            "pre-compact",
            "command-plain-text-json-lines" | "command-compaction-instructions-open-brace",
        ) => {
            "brace-delimited text, which compaction_instructions rejects because Claude Code may parse it as JSON"
        }
        (_, "command-plain-text-json-lines") => {
            "plain text that Claude Code only logs for this event; text_context exists only where plain text becomes context"
        }
        (_, "command-text-json-array")
            if !TEXT_CONTEXT_EVENTS.contains(&event_dir) && event_dir != "pre-compact" =>
        {
            "plain text that Claude Code only logs for this event; text_context exists only where plain text becomes context"
        }
        (
            _,
            "command-nonzero-decision-block"
            | "command-nonzero-allow-json"
            | "command-nonzero-terminal-sequence",
        ) => {
            "JSON with a failing exit other than 2; typed constructors emit JSON only with exit 0 or 2"
        }
        (
            "stop" | "subagent-stop" | "task-completed",
            "command-exit-2-missing-file" | "command-exit-2-missing-file-python",
        )
        | ("user-prompt-submit", "command-exit-2-missing-file") => {
            "blocking_error emits these bytes when its message quotes a missing file, which Claude Code downgrades to a non-blocking error (on UserPromptSubmit for plugin hooks); blocking_error documents the hazard, and a JSON block through into_blocking_error avoids it"
        }
        (
            "stop" | "subagent-stop" | "task-completed",
            "command-exit-2-missing-file-blank-stdout",
        ) => "whitespace-only stdout with exit 2, which typed constructors never emit",
        ("stop" | "subagent-stop", "command-exit-2-continue-false") => {
            "into_blocking_error rejects continue: false here, because it ends the turn instead of continuing it"
        }
        ("teammate-idle", "command-exit-2-continue-false") => {
            "into_blocking_error rejects continue: false here, because it sends the teammate idle instead of keeping it working"
        }
        ("permission-request", "command-malformed-permission-update") => {
            "an updatedPermissions entry without a destination, which with_updated_permissions rejects"
        }
        ("elicitation-result", "command-content-without-action") => {
            "Claude Code ignores hookSpecificOutput without an action; no constructor emits one"
        }
        ("pre-skill-shell-command", "command-allow" | "command-exit-2-structured") => {
            "allow adds nothing on PreSkillShellCommand, so there is no allow constructor; no_op() lets the command run"
        }
        ("pre-skill-shell-command", "command-updated-input-refuses") => {
            "updatedInput is never applied there and only blocks the command; deny(reason) blocks with a reason"
        }
        ("pre-skill-shell-command", "command-pretooluse-format-deny") => {
            "typed outputs use the event's own hookEventName; deny(reason) is the same answer"
        }
        (
            "pre-tool-use",
            "command-deprecated-decision-block" | "command-deprecated-decision-approve",
        ) => {
            "the deprecated top-level decision, which HookKit never emits; deny(reason) and allow() are the current forms"
        }
        (
            _,
            "command-exit-2-invalid-stdout"
            | "command-exit-2-schema-invalid-json"
            | "command-failed-invalid-stdout",
        ) => "invalid stdout that typed constructors never emit",
        (_, "command-nonzero-structured") => {
            "JSON with a failing exit other than 2; typed constructors emit JSON only with exit 0 or 2"
        }
        (_, "command-text-json-lines") => {
            "text_context re-routes brace-delimited text to structured additionalContext"
        }
        (_, "command-no-op") if event_dir != "pre-compact" => {
            "no_op() prints `{}`, the structured outcome with no fields, which has the same effect as empty stdout; typed constructors print empty stdout only for WorktreeRemove and PreCompact"
        }
        (_, "command-plain-text") => {
            "plain text that Claude Code only logs for this event; text_context exists only where plain text becomes context"
        }
        (
            "cwd-changed" | "file-changed" | "post-model-switch" | "session-start"
            | "subagent-start",
            "command-exit-2",
        ) => {
            "exit 2 cannot block here and only shows stderr to the user, the notice nonblocking_error gives with exit 1"
        }
        (
            "cwd-changed" | "file-changed" | "post-model-switch" | "session-start"
            | "subagent-start",
            "command-exit-2-structured",
        ) => {
            "exit 2 cannot block here, so typed constructors never pair JSON with it; the same JSON applies on exit 0"
        }
        ("message-display", "command-exit-2" | "command-exit-2-json-ignored") => {
            "a MessageDisplay hook that exits 2 only logs stderr, ignores any displayContent, and displays the original text, which no_op() also leaves unchanged"
        }
        ("message-display", "command-nonzero-unstructured") => {
            "a failing MessageDisplay hook only logs stderr and displays the original text, which no_op() also leaves unchanged"
        }
        (
            "directory-added"
            | "instructions-loaded"
            | "notification"
            | "permission-denied"
            | "stop-failure"
            | "setup"
            | "session-end"
            | "post-compact",
            "command-exit-2",
        ) => "exit 2 is an ordinary failure here, like any other nonzero exit",
        (
            "directory-added"
            | "instructions-loaded"
            | "notification"
            | "permission-denied"
            | "stop-failure",
            "command-nonzero-unstructured",
        )
        | ("setup", "command-nonzero") => {
            "Claude Code ignores or only logs this failure, so no error constructor is offered"
        }
        ("permission-request", "command-exit-2-ignored") => {
            "Claude Code ignores exit 2 on PermissionRequest"
        }
        ("permission-request", "command-exit-2-structured") => {
            "exit 2 adds nothing to a PermissionRequest decision; deny() applies on every exit code"
        }
        ("worktree-create", "command-created-after-banner") => {
            "the path constructor rejects line breaks and escape sequences"
        }
        ("worktree-create", "command-missing-path") => {
            "a missing path fails creation; use failed(..) instead"
        }
        ("worktree-create", "command-refused-path") => {
            "the path constructor rejects absolute paths with . or .. segments"
        }
        ("worktree-remove", "command-removed-json-ignored") => {
            "Claude Code ignores WorktreeRemove stdout; removed() prints nothing"
        }
        _ => return None,
    })
}

#[test]
fn every_process_case_is_declared_and_exact_or_classified() {
    let engine = base64::engine::general_purpose::STANDARD;
    let descriptors = hookkit_claude::protocol::events();
    for (event_dir, name) in EVENTS {
        let fixtures = fixtures(event_dir);
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event().name() == name)
            .unwrap();
        let declared: BTreeSet<&str> = descriptor.conformance_cases().iter().copied().collect();
        let cases: Vec<&Value> = fixture_list(&fixtures, "/process")
            .iter()
            .filter(|case| case["binding"] == "command")
            .collect();
        let documented: BTreeSet<&str> = cases.iter().map(|case| fixture_id(case)).collect();
        assert!(
            declared.is_subset(&documented),
            "{event_dir}: declares unknown cases {:?}",
            declared.difference(&documented).collect::<Vec<_>>()
        );

        for case in cases {
            let id = fixture_id(case);
            if !declared.contains(id) {
                assert!(
                    undeclared_reason(event_dir, id).is_some(),
                    "{event_dir}/{id}: declare this case or classify why it is not reproduced"
                );
                continue;
            }
            assert!(
                undeclared_reason(event_dir, id).is_none(),
                "{event_dir}/{id}: declared case is also classified as undeclared"
            );
            let emission = fixture_output(event_dir, id)
                .unwrap_or_else(|| panic!("{event_dir}/{id}: add a typed constructor"));
            let stdout = engine
                .decode(case["stdout_base64"].as_str().unwrap())
                .unwrap();
            let stderr = engine
                .decode(case["stderr_base64"].as_str().unwrap())
                .unwrap();
            // As in the conformance crate, JSON member order is not
            // protocol-significant: JSON is compared structurally, with its
            // framing, and every other stdout byte for byte.
            match (
                serde_json::from_slice::<Value>(emission.stdout()),
                serde_json::from_slice::<Value>(&stdout),
            ) {
                (Ok(actual), Ok(expected)) => {
                    assert_eq!(actual, expected, "{event_dir}/{id} stdout");
                    assert_eq!(
                        emission.stdout().ends_with(b"\n"),
                        stdout.ends_with(b"\n"),
                        "{event_dir}/{id} stdout framing"
                    );
                }
                _ => assert_eq!(
                    emission.stdout(),
                    stdout.as_slice(),
                    "{event_dir}/{id} stdout: {}",
                    String::from_utf8_lossy(emission.stdout())
                ),
            }
            assert_eq!(
                emission.stderr(),
                stderr.as_slice(),
                "{event_dir}/{id} stderr"
            );
            assert_eq!(
                u64::from(emission.exit_code()),
                case["exit_code"].as_u64().unwrap(),
                "{event_dir}/{id} exit code"
            );
            assert_eq!(
                emission.contract(),
                descriptor.contract(),
                "{event_dir}/{id}"
            );
            if output_validator(event_dir).is_some() {
                emitted_json(event_dir, &emission);
            }
        }
    }
}

#[test]
fn documented_output_negatives_are_unrepresentable() {
    for (event_dir, _) in EVENTS {
        let fixtures = fixtures(event_dir);
        for fixture in fixture_list(&fixtures, "/output_negative") {
            let id = fixture_id(fixture);
            match (event_dir, id) {
                // PreModelSwitchOutput has no defer decision and no context
                // builder.
                ("pre-model-switch", "defer-rejected" | "additional-context-rejected") => {}
                // SetupOutput has no hook-specific builder; the deprecated
                // `with_context` emits `{}`.
                ("setup", "additional-context-not-string") => {
                    let value = emitted_json(
                        event_dir,
                        &catalog::<Setup>(SetupOutput::with_context("Dependencies installed.")),
                    );
                    assert_eq!(value, Some(json!({})));
                }
                // `with_updated_permissions` rejects an entry that does not
                // match the documented entry shape.
                (
                    "permission-request",
                    "updated-permission-unknown-type"
                    | "updated-permission-without-destination"
                    | "updated-permission-not-object"
                    | "updated-permission-rule-behavior",
                ) => {
                    let entries =
                        fixture["value"]["hookSpecificOutput"]["decision"]["updatedPermissions"]
                            .as_array()
                            .unwrap()
                            .clone();
                    assert!(
                        PermissionRequestOutput::allow()
                            .with_updated_permissions(entries)
                            .is_err(),
                        "{event_dir}/{id}"
                    );
                }
                // A command WorktreeCreate hook prints a path, never JSON.
                ("worktree-create", "missing-worktree-path") => {}
                // Every block constructor and builder sets `reason`.
                ("stop" | "subagent-stop", "block-without-reason") => {}
                // `with_updated_input` takes a JSON object (`Map`).
                ("permission-request" | "pre-tool-use", "updated-input-not-object") => {}
                // PreToolUseOutput has no top-level decision builder.
                ("pre-tool-use", "deprecated-decision-deny") => {}
                // PreSkillShellCommandOutput has no context builder, no approve
                // decision, and always names its own event.
                (
                    "pre-skill-shell-command",
                    "additional-context-rejected"
                    | "approve-rejected"
                    | "pretooluse-format-approve-rejected",
                ) => {}
                _ => panic!("{event_dir}/{id}: classify this output negative"),
            }
            if let Some(validator) = output_validator(event_dir) {
                assert!(
                    !validator.is_valid(&fixture["value"]),
                    "{event_dir}/{id} must violate the schema"
                );
            }
        }
    }
}

fn all_json(event_dir: &str, emissions: Vec<ProcessEmission>) {
    let name = wire_name(event_dir);
    for emission in emissions {
        assert!(
            emission.contract().as_str().ends_with(&format!("/{name}")),
            "{event_dir}: wrong contract {}",
            emission.contract()
        );
        let value = emitted_json(event_dir, &emission);
        if let Some(value) = value {
            if let Some(specific) = value.get("hookSpecificOutput") {
                assert_eq!(specific["hookEventName"], name, "{event_dir}");
            }
        }
    }
}

#[test]
fn every_typed_output_constructor_satisfies_the_output_schema() {
    let paths = || vec![Utf8PathBuf::from("/repo/.env")];
    let input = || map(json!({"command": "cargo test", "timeout": 10}));

    all_json(
        "config-change",
        vec![
            catalog::<ConfigChange>(ConfigChangeOutput::no_op()),
            catalog::<ConfigChange>(ConfigChangeOutput::block("no")),
            catalog::<ConfigChange>(
                ConfigChangeOutput::no_op()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_block("no")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<ConfigChange>(
                ConfigChangeOutput::block("no")
                    .into_blocking_error("stderr")
                    .unwrap(),
            ),
        ],
    );

    for (event_dir, outputs) in [
        (
            "cwd-changed",
            vec![
                catalog::<CwdChanged>(CwdChangedOutput::no_op()),
                catalog::<CwdChanged>(
                    CwdChangedOutput::system_message("m")
                        .with_watch_paths(paths())
                        .unwrap()
                        .with_watch_paths(Vec::new())
                        .unwrap()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_continue(true)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_suppress_output(false)
                        .unwrap(),
                ),
                catalog::<CwdChanged>(CwdChangedOutput::with_system_message("m")),
            ],
        ),
        (
            "file-changed",
            vec![
                catalog::<FileChanged>(FileChangedOutput::no_op()),
                catalog::<FileChanged>(
                    FileChangedOutput::system_message("m")
                        .with_watch_paths(paths())
                        .unwrap()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_continue(true)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_suppress_output(false)
                        .unwrap(),
                ),
                catalog::<FileChanged>(FileChangedOutput::with_system_message("m")),
            ],
        ),
        (
            "directory-added",
            vec![
                catalog::<DirectoryAdded>(DirectoryAddedOutput::no_op()),
                catalog::<DirectoryAdded>(
                    DirectoryAddedOutput::system_message("m")
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_continue(true)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_suppress_output(false)
                        .unwrap(),
                ),
                catalog::<DirectoryAdded>(DirectoryAddedOutput::with_system_message("m")),
            ],
        ),
    ] {
        all_json(event_dir, outputs);
    }

    for (event_dir, outputs) in [
        (
            "elicitation",
            vec![
                catalog::<Elicitation>(ElicitationOutput::no_op()),
                catalog::<Elicitation>(ElicitationOutput::accept(map(json!({"a": 1})))),
                catalog::<Elicitation>(ElicitationOutput::accept_without_content()),
                catalog::<Elicitation>(ElicitationOutput::decline()),
                catalog::<Elicitation>(
                    ElicitationOutput::cancel()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_continue(true)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_system_message("m")
                        .unwrap()
                        .with_suppress_output(true)
                        .unwrap(),
                ),
                catalog::<Elicitation>(ElicitationOutput::block("no")),
                catalog::<Elicitation>(ElicitationOutput::no_op().with_block("no").unwrap()),
            ],
        ),
        (
            "elicitation-result",
            vec![
                catalog::<ElicitationResult>(ElicitationResultOutput::no_op()),
                catalog::<ElicitationResult>(ElicitationResultOutput::accept(map(json!({})))),
                catalog::<ElicitationResult>(ElicitationResultOutput::accept_without_content()),
                catalog::<ElicitationResult>(ElicitationResultOutput::decline()),
                catalog::<ElicitationResult>(
                    ElicitationResultOutput::cancel()
                        .with_terminal_sequence("\u{7}")
                        .unwrap(),
                ),
                catalog::<ElicitationResult>(ElicitationResultOutput::block("no")),
            ],
        ),
    ] {
        all_json(event_dir, outputs);
    }

    macro_rules! terminal_only {
        ($dir:literal, $event:ident, $output:ident) => {
            all_json(
                $dir,
                vec![
                    catalog::<$event>($output::no_op()),
                    catalog::<$event>($output::terminal_sequence("\u{1b}]9;done\u{7}")),
                    catalog::<$event>(
                        $output::no_op()
                            .with_terminal_sequence("\u{7}")
                            .unwrap()
                            .with_continue(true)
                            .unwrap()
                            .with_stop_reason("r")
                            .unwrap()
                            .with_suppress_output(false)
                            .unwrap(),
                    ),
                    catalog::<$event>($output::with_system_message("m")),
                ],
            );
        };
    }
    terminal_only!(
        "instructions-loaded",
        InstructionsLoaded,
        InstructionsLoadedOutput
    );
    terminal_only!("notification", Notification, NotificationOutput);
    terminal_only!("stop-failure", StopFailure, StopFailureOutput);
    terminal_only!("session-end", SessionEnd, SessionEndOutput);
    terminal_only!("post-compact", PostCompact, PostCompactOutput);

    all_json(
        "setup",
        vec![
            catalog::<Setup>(SetupOutput::no_op()),
            catalog::<Setup>(SetupOutput::with_context("ignored")),
            catalog::<Setup>(
                SetupOutput::no_op()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
        ],
    );

    all_json(
        "message-display",
        vec![
            catalog::<MessageDisplay>(MessageDisplayOutput::no_op()),
            catalog::<MessageDisplay>(
                MessageDisplayOutput::display("shown")
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
        ],
    );

    all_json(
        "permission-denied",
        vec![
            catalog::<PermissionDenied>(PermissionDeniedOutput::no_op()),
            catalog::<PermissionDenied>(
                PermissionDeniedOutput::retry(false)
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
        ],
    );

    all_json(
        "subagent-start",
        vec![
            catalog::<SubagentStart>(SubagentStartOutput::no_op()),
            catalog::<SubagentStart>(
                SubagentStartOutput::with_context("c")
                    .with_additional_context("d")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<SubagentStart>(
                SubagentStartOutput::no_op()
                    .with_additional_context("c")
                    .unwrap(),
            ),
        ],
    );

    macro_rules! decision_with_context {
        ($dir:literal, $event:ident, $output:ident) => {
            all_json(
                $dir,
                vec![
                    catalog::<$event>($output::no_op()),
                    catalog::<$event>($output::with_context("c")),
                    catalog::<$event>($output::block("no")),
                    catalog::<$event>($output::block_with_context("no", "c")),
                    catalog::<$event>(
                        $output::block("no")
                            .with_additional_context("c")
                            .unwrap()
                            .with_block("again")
                            .unwrap()
                            .with_continue(false)
                            .unwrap()
                            .with_stop_reason("r")
                            .unwrap()
                            .with_system_message("m")
                            .unwrap()
                            .with_terminal_sequence("\u{7}")
                            .unwrap()
                            .with_suppress_output(true)
                            .unwrap(),
                    ),
                ],
            );
        };
    }
    decision_with_context!("post-tool-batch", PostToolBatch, PostToolBatchOutput);
    decision_with_context!(
        "post-tool-use-failure",
        PostToolUseFailure,
        PostToolUseFailureOutput
    );
    decision_with_context!("stop", Stop, StopOutput);
    decision_with_context!("subagent-stop", SubagentStop, SubagentStopOutput);
    decision_with_context!(
        "user-prompt-expansion",
        UserPromptExpansion,
        UserPromptExpansionOutput
    );

    all_json(
        "post-tool-batch",
        vec![catalog::<PostToolBatch>(
            PostToolBatchOutput::with_context("c")
                .into_blocking_error("stderr")
                .unwrap(),
        )],
    );
    all_json(
        "post-tool-use-failure",
        vec![catalog::<PostToolUseFailure>(
            PostToolUseFailureOutput::block("no")
                .into_feedback_error("stderr")
                .unwrap(),
        )],
    );

    all_json(
        "pre-compact",
        vec![
            catalog::<PreCompact>(PreCompactOutput::no_op()),
            catalog::<PreCompact>(PreCompactOutput::compaction_instructions("Keep it.").unwrap()),
            catalog::<PreCompact>(PreCompactOutput::no_op().with_block("no").unwrap()),
            // A terminal sequence must come with a block; see
            // `pre_compact_json_at_exit_0_must_block`.
            catalog::<PreCompact>(
                PreCompactOutput::no_op()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_block("no")
                    .unwrap(),
            ),
            catalog::<PreCompact>(
                PreCompactOutput::no_op()
                    .into_blocking_error("stderr")
                    .unwrap(),
            ),
            catalog::<PreCompact>(
                PreCompactOutput::block("no")
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
        ],
    );

    for (event_dir, outputs) in [
        (
            "task-created",
            vec![
                catalog::<TaskCreated>(TaskCreatedOutput::no_op()),
                catalog::<TaskCreated>(
                    TaskCreatedOutput::block("no")
                        .with_system_message("m")
                        .unwrap()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_continue(false)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_suppress_output(true)
                        .unwrap(),
                ),
            ],
        ),
        (
            "task-completed",
            vec![
                catalog::<TaskCompleted>(
                    TaskCompletedOutput::no_op()
                        .with_continue(false)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_system_message("m")
                        .unwrap()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_suppress_output(true)
                        .unwrap(),
                ),
                catalog::<TaskCompleted>(TaskCompletedOutput::block("no")),
                catalog::<TaskCompleted>(
                    TaskCompletedOutput::no_op()
                        .with_system_message("m")
                        .unwrap()
                        .with_block("no")
                        .unwrap()
                        .into_blocking_error("")
                        .unwrap(),
                ),
            ],
        ),
        (
            "teammate-idle",
            vec![
                catalog::<TeammateIdle>(
                    TeammateIdleOutput::no_op()
                        .with_continue(false)
                        .unwrap()
                        .with_stop_reason("r")
                        .unwrap()
                        .with_system_message("m")
                        .unwrap()
                        .with_terminal_sequence("\u{7}")
                        .unwrap()
                        .with_suppress_output(true)
                        .unwrap(),
                ),
                catalog::<TeammateIdle>(TeammateIdleOutput::block("no")),
                catalog::<TeammateIdle>(
                    TeammateIdleOutput::block("no")
                        .into_blocking_error("stderr")
                        .unwrap(),
                ),
            ],
        ),
    ] {
        all_json(event_dir, outputs);
    }

    all_json(
        "permission-request",
        vec![
            catalog::<PermissionRequest>(PermissionRequestOutput::no_op()),
            catalog::<PermissionRequest>(PermissionRequestOutput::allow()),
            catalog::<PermissionRequest>(
                PermissionRequestOutput::allow()
                    .with_updated_input(input())
                    .unwrap()
                    .with_updated_permissions(vec![json!({
                        "type": "setMode",
                        "mode": "acceptEdits",
                        "destination": "session"
                    })])
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<PermissionRequest>(
                PermissionRequestOutput::deny("no")
                    .with_interrupt(true)
                    .unwrap(),
            ),
            catalog::<PermissionRequest>(PermissionRequestOutput::decide(
                PermissionRequestBehavior::Deny,
                Some("no".into()),
                Some(false),
            )),
            catalog::<PermissionRequest>(PermissionRequestOutput::blocking_error("no")),
        ],
    );

    let mut pre_tool = vec![
        catalog::<PreToolUse>(PreToolUseOutput::no_op()),
        catalog::<PreToolUse>(PreToolUseOutput::with_context("c")),
        catalog::<PreToolUse>(
            PreToolUseOutput::with_context("c")
                .with_updated_input(input())
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::no_op()
                .with_updated_input(input())
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::allow()
                .with_updated_input(input())
                .unwrap()
                .with_decision_reason("r")
                .unwrap()
                .with_additional_context("c")
                .unwrap()
                .with_continue(true)
                .unwrap()
                .with_stop_reason("r")
                .unwrap()
                .with_system_message("m")
                .unwrap()
                .with_terminal_sequence("\u{7}")
                .unwrap()
                .with_suppress_output(true)
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::deny("no")
                .with_additional_context("c")
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::ask("sure?")
                .with_updated_input(input())
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::defer()
                .with_decision_reason("later")
                .unwrap(),
        ),
        catalog::<PreToolUse>(
            PreToolUseOutput::deny("no")
                .into_blocking_error("stderr")
                .unwrap(),
        ),
    ];
    for decision in [
        PreToolPermissionDecision::Allow,
        PreToolPermissionDecision::Deny,
        PreToolPermissionDecision::Ask,
        PreToolPermissionDecision::Defer,
    ] {
        pre_tool.push(catalog::<PreToolUse>(PreToolUseOutput::decide(
            decision,
            Some("r".into()),
            Some(input()),
            Some("c".into()),
        )));
    }
    all_json("pre-tool-use", pre_tool);

    all_json(
        "pre-skill-shell-command",
        vec![
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::no_op()),
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::deny("no")),
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::ask("sure?")),
            catalog::<PreSkillShellCommand>(
                PreSkillShellCommandOutput::defer()
                    .with_decision_reason("later")
                    .unwrap(),
            ),
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::block("no")),
            catalog::<PreSkillShellCommand>(
                PreSkillShellCommandOutput::deny("no")
                    .with_decision_reason("again")
                    .unwrap()
                    .with_block("no")
                    .unwrap()
                    .with_continue(false)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<PreSkillShellCommand>(
                PreSkillShellCommandOutput::deny("no")
                    .into_blocking_error("")
                    .unwrap(),
            ),
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::blocking_error("no")),
            catalog::<PreSkillShellCommand>(PreSkillShellCommandOutput::nonblocking_error("x")),
        ],
    );

    all_json(
        "user-prompt-submit",
        vec![
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::no_op()),
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::with_context("c")),
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::text_context("{\"a\":1}")),
            catalog::<UserPromptSubmit>(
                UserPromptSubmitOutput::with_context("c")
                    .with_session_title("t")
                    .unwrap()
                    .with_block("no")
                    .unwrap()
                    .with_suppress_original_prompt(true)
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<UserPromptSubmit>(UserPromptSubmitOutput::block_with_context(
                "no",
                "c",
                Some("t".into()),
                Some(true),
            )),
        ],
    );
    all_json(
        "user-prompt-expansion",
        vec![catalog::<UserPromptExpansion>(
            UserPromptExpansionOutput::text_context(" {\"a\":1} \n"),
        )],
    );

    all_json(
        "pre-model-switch",
        vec![
            catalog::<PreModelSwitch>(PreModelSwitchOutput::no_op()),
            catalog::<PreModelSwitch>(PreModelSwitchOutput::allow()),
            catalog::<PreModelSwitch>(
                PreModelSwitchOutput::deny("no")
                    .with_decision_reason("again")
                    .unwrap(),
            ),
            catalog::<PreModelSwitch>(
                PreModelSwitchOutput::ask("sure?")
                    .with_block("no")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap(),
            ),
            catalog::<PreModelSwitch>(PreModelSwitchOutput::block("no")),
        ],
    );
    all_json(
        "post-model-switch",
        vec![
            catalog::<PostModelSwitch>(PostModelSwitchOutput::no_op()),
            catalog::<PostModelSwitch>(PostModelSwitchOutput::text_context("{}")),
            catalog::<PostModelSwitch>(
                PostModelSwitchOutput::with_context("c")
                    .with_additional_context("d")
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap(),
            ),
        ],
    );

    all_json(
        "session-start",
        vec![
            catalog::<SessionStart>(SessionStartOutput::no_op()),
            catalog::<SessionStart>(SessionStartOutput::with_context("c")),
            catalog::<SessionStart>(SessionStartOutput::with_context_and_system_message(
                "c", "m",
            )),
            catalog::<SessionStart>(SessionStartOutput::text_context("\n{\"a\": 1}\n")),
            catalog::<SessionStart>(
                SessionStartOutput::no_op()
                    .with_initial_user_message("first")
                    .unwrap()
                    .with_session_title("t")
                    .unwrap()
                    .with_additional_context("c")
                    .unwrap()
                    .with_reload_skills(true)
                    .unwrap()
                    .with_watch_paths(paths())
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<SessionStart>(
                SessionStartOutput::structured(Some("c".into()), Some(false), None, paths())
                    .unwrap(),
            ),
        ],
    );

    all_json(
        "post-tool-use",
        vec![
            catalog::<PostToolUse>(PostToolUseOutput::no_op()),
            catalog::<PostToolUse>(PostToolUseOutput::block("no")),
            catalog::<PostToolUse>(
                PostToolUseOutput::block("no")
                    .with_additional_context("c")
                    .unwrap()
                    .with_classifier_context("k")
                    .unwrap()
                    .with_updated_tool_output(json!({"stdout": ""}))
                    .unwrap()
                    .with_updated_mcp_tool_output(json!({"content": []}))
                    .unwrap()
                    .with_continue(true)
                    .unwrap()
                    .with_stop_reason("r")
                    .unwrap()
                    .with_system_message("m")
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap()
                    .with_suppress_output(true)
                    .unwrap(),
            ),
            catalog::<PostToolUse>(
                PostToolUseOutput::with_context("c")
                    .with_protocol_stderr("debug")
                    .unwrap(),
            ),
            catalog::<PostToolUse>(
                PostToolUseOutput::no_op()
                    .into_feedback_error("stderr")
                    .unwrap(),
            ),
        ],
    );
}

#[test]
fn stderr_and_text_outcomes_use_their_documented_channels() {
    let text = SessionStart::emit(SessionStartOutput::text_context("plain")).unwrap();
    assert_eq!(text.stdout(), b"plain");
    assert_eq!(text.exit_code(), 0);

    let blocked = PreToolUse::emit(PreToolUseOutput::blocking_error("stop")).unwrap();
    assert!(blocked.stdout().is_empty());
    assert_eq!(blocked.stderr(), b"stop");
    assert_eq!(blocked.exit_code(), 2);
    assert!(PreToolUse::emit(PreToolUseOutput::blocking_error("")).is_err());
    // Stderr is the only message without a blocking decision, so an empty one
    // is rejected when the response is built.
    assert!(PreToolUseOutput::allow().into_blocking_error("").is_err());
    // The exit-2-structured outcome makes stderr optional, and Claude Code
    // takes the message from the JSON decision's reason.
    let denied = PreToolUse::emit(
        PreToolUseOutput::deny("no rm")
            .into_blocking_error("")
            .unwrap(),
    )
    .unwrap();
    assert_eq!((denied.exit_code(), denied.stderr()), (2, &b""[..]));
    assert_eq!(
        emitted_json("pre-tool-use", &denied).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    let notice = SessionEnd::emit(SessionEndOutput::nonblocking_error("oops")).unwrap();
    assert_eq!(notice.exit_code(), 1);
    assert_eq!(notice.stderr(), b"oops");

    let removed = WorktreeRemove::emit(WorktreeRemoveOutput::removed()).unwrap();
    assert!(removed.stdout().is_empty() && removed.stderr().is_empty());
    assert_eq!(removed.exit_code(), 0);
    assert!(WorktreeRemoveOutput::failed("x", 0).is_err());
    let failed = WorktreeRemove::emit(WorktreeRemoveOutput::failed("", 3).unwrap()).unwrap();
    assert_eq!(failed.exit_code(), 3);
}

#[test]
fn catalog_inputs_built_outside_the_parsers_never_panic() {
    let mut value = positive("pre-tool-use", "minimal");
    let mut input = PreToolUse::parse(&invocation(&value)).unwrap();
    // The parser fixes the identity even if the public field is edited.
    input.hook_event_name = "SessionStart".into();
    let any = hookkit_claude::protocol::AnyInput::Catalog(input);
    assert_eq!(ClaudeCode::input_event(&any), PreToolUse::EVENT);

    // Direct deserialization derives the identity from the wire name, and a
    // name outside the catalog yields a dynamic id instead of a panic.
    value["hook_event_name"] = "FutureEvent".into();
    let direct: CatalogInput = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        direct.event_id(),
        EventId::new(HarnessId::CLAUDE_CODE, "FutureEvent").unwrap()
    );
    let any = hookkit_claude::protocol::AnyInput::Catalog(direct);
    assert_eq!(ClaudeCode::input_event(&any).name(), "FutureEvent");

    value["hook_event_name"] = "Stop".into();
    let direct: CatalogInput = serde_json::from_value(value).unwrap();
    assert_eq!(direct.event_id(), Stop::EVENT);
}

/// Events whose snapshot catalogs the `command-on-failure-block` binding, the
/// `"onFailure": "block"` configuration (Claude Code v2.1.295 or later) on an
/// event where it changes what a failure does.
const ON_FAILURE_BLOCK_EVENTS: &[&str] = &[
    "config-change",
    "elicitation",
    "elicitation-result",
    "permission-request",
    "post-tool-batch",
    "post-tool-use",
    "post-tool-use-failure",
    "pre-compact",
    "pre-model-switch",
    "pre-skill-shell-command",
    "pre-tool-use",
    "task-created",
    "user-prompt-expansion",
    "user-prompt-submit",
];

fn read_contract(event_dir: &str) -> Value {
    read_yaml(
        snapshot_root()
            .join("events")
            .join(event_dir)
            .join("contract.yaml"),
    )
}

fn outcome_effect<'a>(binding: &'a Value, id: &str) -> &'a str {
    binding["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outcome| outcome["id"] == id)
        .and_then(|outcome| outcome["effect"].as_str())
        .unwrap_or_else(|| panic!("no {id} outcome"))
}

fn case_bytes(case: &Value) -> (Vec<u8>, Vec<u8>, u64) {
    let engine = base64::engine::general_purpose::STANDARD;
    (
        engine
            .decode(case["stdout_base64"].as_str().unwrap())
            .unwrap(),
        engine
            .decode(case["stderr_base64"].as_str().unwrap())
            .unwrap(),
        case["exit_code"].as_u64().unwrap(),
    )
}

fn same_bytes(emission: &ProcessEmission, case: &Value) -> bool {
    let (stdout, stderr, exit_code) = case_bytes(case);
    let stdout_equal = match (
        serde_json::from_slice::<Value>(emission.stdout()),
        serde_json::from_slice::<Value>(&stdout),
    ) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => emission.stdout() == stdout.as_slice(),
    };
    stdout_equal
        && emission.stderr() == stderr.as_slice()
        && u64::from(emission.exit_code()) == exit_code
}

/// Under `"onFailure": "block"`, `nonblocking_error` takes the event's
/// exit-2 effect (a deny on PermissionRequest), as its rustdoc says, while
/// every declared typed response keeps its outcome.
#[test]
fn on_failure_block_turns_nonblocking_errors_into_the_exit_2_effect() {
    let descriptors = hookkit_claude::protocol::events();
    for (event_dir, name) in EVENTS {
        let contract = read_contract(event_dir);
        let binding = &contract["bindings"]["command-on-failure-block"];
        assert_eq!(
            !binding.is_null(),
            ON_FAILURE_BLOCK_EVENTS.contains(&event_dir),
            "{event_dir}: command-on-failure-block binding"
        );
        if binding.is_null() {
            continue;
        }
        assert_eq!(binding["kind"], "process", "{event_dir}");
        let failure = outcome_effect(binding, "failure-blocked");
        if event_dir == "permission-request" {
            assert_eq!(failure, "deny");
        } else {
            assert_eq!(
                failure,
                outcome_effect(&contract["bindings"]["command"], "exit-2"),
                "{event_dir}: failure-blocked takes the exit-2 effect"
            );
        }

        let fixtures = fixtures(event_dir);
        let cases = fixture_list(&fixtures, "/process");
        let counterpart = |suffix: &str| {
            cases.iter().find(|case| {
                case["binding"] == "command-on-failure-block"
                    && fixture_id(case) == format!("command-on-failure-block-{suffix}")
            })
        };
        let failed = counterpart("nonzero-unstructured").unwrap_or_else(|| {
            panic!("{event_dir}: no on-failure-block nonzero-unstructured case")
        });
        assert_eq!(failed["outcome"], "failure-blocked", "{event_dir}");
        let emission = fixture_output(event_dir, "command-nonzero-unstructured").unwrap();
        assert!(
            same_bytes(&emission, failed),
            "{event_dir}: nonblocking_error reproduces the failure-blocked case"
        );

        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event().name() == name)
            .unwrap();
        for id in descriptor.conformance_cases() {
            let suffix = id.strip_prefix("command-").unwrap();
            if suffix == "nonzero-unstructured" {
                continue;
            }
            let Some(case) = counterpart(suffix) else {
                continue;
            };
            let emission = fixture_output(event_dir, id).unwrap();
            if same_bytes(&emission, case) {
                assert_ne!(
                    case["outcome"], "failure-blocked",
                    "{event_dir}/{id}: a typed response fails under onFailure: block"
                );
            }
        }
    }
}

/// Claude Code caps hook strings at 10,000 characters and escapes
/// `<system-reminder>` tags itself; typed builders pass text through
/// unchanged and the schemas set no length limit.
#[test]
fn capped_and_escaped_strings_are_emitted_unchanged() {
    for text in [
        "a".repeat(10_001),
        "<system-reminder>x</system-reminder>".to_owned(),
    ] {
        let emission = catalog::<PostToolUse>(PostToolUseOutput::with_context(text.clone()));
        let value = emitted_json("post-tool-use", &emission).unwrap();
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], text);

        let emission = catalog::<UserPromptSubmit>(
            UserPromptSubmitOutput::no_op()
                .with_system_message(text.clone())
                .unwrap(),
        );
        let value = emitted_json("user-prompt-submit", &emission).unwrap();
        assert_eq!(value["systemMessage"], text);

        let emission = catalog::<SessionStart>(SessionStartOutput::text_context(text.clone()));
        assert_eq!(emission.stdout(), text.as_bytes());
    }
}

/// The input fixtures added for docs-2026-11-25-r1 that exercise shapes the
/// typed inputs keep as raw JSON: Monitor, Code Mode, and PowerShell calls on
/// PreToolUse, Monitor calls on PermissionRequest and PostToolUse, and an
/// MCP-skill PermissionRequest, each re-serialize exactly.
#[test]
fn new_tool_shapes_round_trip_through_the_raw_tool_input() {
    for (event_dir, id, tool) in [
        ("pre-tool-use", "monitor-call", "Monitor"),
        ("pre-tool-use", "monitor-ws", "Monitor"),
        ("pre-tool-use", "monitor-command-via-bash-matcher", "Bash"),
        ("pre-tool-use", "code-mode-repl", "REPL"),
        ("pre-tool-use", "code-mode-cd-prefix", "Bash"),
        ("pre-tool-use", "powershell-tool", "PowerShell"),
        ("permission-request", "mcp-skill-ask", "Skill"),
        ("permission-request", "monitor-call", "Monitor"),
        (
            "permission-request",
            "monitor-command-via-bash-matcher",
            "Bash",
        ),
    ] {
        let value = positive(event_dir, id);
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, wire_name(event_dir));
        let decoded = ClaudeCode::decode(&event, &invocation(&value)).unwrap();
        let hookkit_claude::protocol::AnyInput::Catalog(input) = decoded else {
            panic!("{event_dir} is a catalog event");
        };
        assert_eq!(input.tool_name(), Some(tool), "{event_dir}/{id}");
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            value,
            "{event_dir}/{id}"
        );
    }
    let value = positive("post-tool-use", "monitor-call");
    let input = PostToolUse::parse(&invocation(&value)).unwrap();
    assert_eq!(input.tool_name, "Monitor");
    assert_eq!(input.tool_input, value["tool_input"]);
    assert_eq!(input.tool_response, value["tool_response"]);
    assert_eq!(serde_json::to_value(&input).unwrap(), value);
    let raw = invocation(&positive("permission-request", "mcp-skill-ask"));
    let input = PermissionRequest::parse(&raw).unwrap();
    assert_eq!(input.mcp_server().unwrap().name, "docs");
    assert_eq!(input.field("tool_kind"), Some(&json!("mcp_skill")));
}

/// Claude Code uses the stdout of a PreCompact hook that exits 0 without
/// blocking as compaction instructions, JSON included, so a typed JSON
/// response at exit 0 must block; the empty `no_op`, plain instructions, a
/// block, and exit 2 still emit.
#[test]
fn pre_compact_json_at_exit_0_must_block() {
    #[allow(deprecated)]
    let discarded = PreCompactOutput::no_op().with_continue(true).unwrap();
    for (label, output) in [
        (
            "terminal sequence",
            PreCompactOutput::no_op()
                .with_terminal_sequence("\u{7}")
                .unwrap(),
        ),
        ("deprecated discarded field", discarded),
    ] {
        let error = PreCompact::emit(output).expect_err(label).to_string();
        assert!(error.contains("must block"), "{label}: {error}");
    }

    for output in [
        PreCompactOutput::no_op(),
        PreCompactOutput::compaction_instructions("Keep the test names.").unwrap(),
        PreCompactOutput::block("Save state first.")
            .with_terminal_sequence("\u{7}")
            .unwrap(),
        PreCompactOutput::no_op()
            .with_terminal_sequence("\u{7}")
            .unwrap()
            .into_blocking_error("blocked")
            .unwrap(),
    ] {
        let emission = catalog::<PreCompact>(output);
        if emission.exit_code() == 0 {
            if let Some(value) = emitted_json("pre-compact", &emission) {
                assert_eq!(value["decision"], "block", "{value}");
            }
        }
    }
}

/// `with_updated_permissions` accepts every documented entry shape,
/// including the input's suggestions echoed back, and its output always
/// satisfies the typed entry schema; it rejects what the schema rejects.
#[test]
fn permission_updates_follow_the_documented_entry_shape() {
    let official = positive("permission-request", "official-example");
    let suggestions = official["permission_suggestions"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!suggestions.is_empty());
    let valid = [
        json!({"type": "setMode", "mode": "bypassPermissions", "destination": "cliArg"}),
        json!({"type": "setMode", "mode": "futureMode", "destination": "session", "extra": 1}),
        json!({"type": "removeRules", "rules": [{"toolName": "Bash"}], "behavior": "deny", "destination": "userSettings"}),
        json!({"type": "replaceRules", "rules": [], "behavior": "ask", "destination": "projectSettings"}),
        json!({"type": "removeDirectories", "directories": ["/tmp/a"], "destination": "localSettings"}),
    ];
    for entries in [suggestions, valid.to_vec()] {
        let emission = catalog::<PermissionRequest>(
            PermissionRequestOutput::allow()
                .with_updated_permissions(entries)
                .unwrap(),
        );
        emitted_json("permission-request", &emission).expect("JSON");
    }
    for invalid in [
        json!("addRules"),
        json!({"type": "setMode", "mode": "plan"}),
        json!({"type": "setMode", "destination": "session"}),
        json!({"type": "setMode", "mode": 1, "destination": "session"}),
        json!({"type": "addRule", "destination": "session"}),
        json!({"type": "addRules", "behavior": "allow", "destination": "session"}),
        json!({"type": "addRules", "rules": [{"ruleContent": "x"}], "behavior": "allow", "destination": "session"}),
        json!({"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": 1}], "behavior": "allow", "destination": "session"}),
        json!({"type": "addRules", "rules": [], "behavior": "approve", "destination": "session"}),
        json!({"type": "addDirectories", "directories": "/tmp", "destination": "session"}),
        json!({"type": "addDirectories", "directories": [1], "destination": "session"}),
        json!({"type": "addDirectories", "directories": [], "destination": "global"}),
    ] {
        assert!(
            PermissionRequestOutput::allow()
                .with_updated_permissions(vec![invalid.clone()])
                .is_err(),
            "{invalid}"
        );
        let value = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow", "updatedPermissions": [invalid]}}});
        assert!(
            !output_validator("permission-request")
                .unwrap()
                .is_valid(&value),
            "{value}"
        );
    }
}

/// `continue: false` printed with exit 2 sends a teammate idle, so the
/// TeammateIdle exit-2 form rejects it, while on TaskCompleted, where the
/// task stays open either way, it is the documented exit-2-structured case.
#[test]
fn teammate_continue_false_and_exit_2() {
    let error = TeammateIdleOutput::no_op()
        .with_continue(false)
        .unwrap()
        .with_stop_reason("Done.")
        .unwrap()
        .into_blocking_error("keep going")
        .expect_err("continue: false sends the teammate idle")
        .to_string();
    assert!(error.contains("teammate idle"), "{error}");
    assert!(
        TeammateIdleOutput::block("again")
            .with_continue(false)
            .unwrap()
            .into_blocking_error("")
            .is_err()
    );
    // At exit 0 it is the documented way to let the teammate go idle.
    let emission = catalog::<TeammateIdle>(
        TeammateIdleOutput::no_op()
            .with_continue(false)
            .unwrap()
            .with_stop_reason("Done.")
            .unwrap(),
    );
    assert_eq!(emission.exit_code(), 0);

    let contract = read_contract("teammate-idle");
    assert_eq!(
        outcome_effect(&contract["bindings"]["command"], "exit-2-continue-false"),
        "stop"
    );
    let fixtures = fixtures("task-completed");
    let case = fixture_list(&fixtures, "/process")
        .iter()
        .find(|case| fixture_id(case) == "command-exit-2-continue-false")
        .unwrap();
    assert_eq!(case["outcome"], "exit-2-structured");
    let emission = fixture_output("task-completed", "command-exit-2-continue-false").unwrap();
    assert!(same_bytes(&emission, case));
}
