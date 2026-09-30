#!/usr/bin/env python3
"""Repository acceptance tests for the HookKit Copier template.

The wrapper in run.sh supplies the pinned Copier and PyYAML dependencies.  The
test deliberately stages the working-tree template outside its Git checkout so
an uncommitted template change is what gets rendered.
"""

from __future__ import annotations

import argparse
import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

import copier
import yaml
from copier.errors import InteractiveSessionError


SUPPORTED_HARNESSES = ("claude-code", "codex", "antigravity")
EXPECTED_EVENT_COUNTS = {"claude-code": 33, "codex": 12, "antigravity": 5}
EXPECTED_UNIVERSAL_FAMILIES = 3
EXPECTED_CLAUDE_CODEX_ONLY_FAMILIES = 8
ALL_STATE_CAPABILITIES = (
    "session_metadata",
    "claim_once",
    "inspectable_set",
    "record_queue",
    "run_artifacts",
    "custom_aggregate",
    "file_activity",
)

REPO_ROOT = Path(__file__).resolve().parents[3]
# Answers a render receives when only the required identity and HookKit
# source are given; any drift from these is a questionnaire change.
DEFAULT_ANSWERS = {
    "output_mode": "project",
    "starter": "custom",
    "harness_mode": "cross",
    "harnesses": list(SUPPORTED_HARNESSES),
    "aligned_hooks": ["pre_tool"],
    "state_capabilities": [],
    "github_actions": True,
}
CATALOG_ROOT = REPO_ROOT / "templates" / "hook-project" / "catalog"

# The compatibility catalog is the single source for toolchain versions;
# `cargo xtask template-catalog check` keeps the remaining literal copies
# (run.sh, copier.yml, CI, and the release check) in agreement with it.
with (CATALOG_ROOT / "compatibility.yml").open(encoding="utf-8") as _stream:
    _COMPATIBILITY = yaml.safe_load(_stream)
COPIER_VERSION = str(_COMPATIBILITY["copier_version"])
RUST_MSRV = str(_COMPATIBILITY["rust_msrv"])


class AcceptanceFailure(RuntimeError):
    """A failure with enough context to diagnose one generated case."""


@dataclass(frozen=True)
class Case:
    name: str
    data: dict[str, Any]
    host: str = "empty"

    @property
    def crate_path(self) -> Path:
        return Path(self.data["crate_path"])


def load_yaml(path: Path) -> Any:
    with path.open(encoding="utf-8") as stream:
        return yaml.safe_load(stream)


def event_catalog() -> dict[str, list[str]]:
    result = {harness: [] for harness in SUPPORTED_HARNESSES}
    for event in load_yaml(CATALOG_ROOT / "event-scaffolds.yml")["events"]:
        harness = event["harness"]
        if harness not in result:
            raise AcceptanceFailure(f"unsupported event harness in catalog: {harness!r}")
        result[harness].append(event["stable_id"])
    for harness, expected in EXPECTED_EVENT_COUNTS.items():
        actual = len(result[harness])
        if actual != expected:
            raise AcceptanceFailure(
                f"{harness} catalog contains {actual} events; design requires {expected}"
            )
    return result


def alignment_catalog() -> list[dict[str, Any]]:
    alignments = load_yaml(CATALOG_ROOT / "alignment-families.yml")["alignments"]
    universal = [
        item
        for item in alignments
        if set(item["supported_harnesses"]) == set(SUPPORTED_HARNESSES)
    ]
    pair_only = [
        item
        for item in alignments
        if set(item["supported_harnesses"]) == {"claude-code", "codex"}
    ]
    if len(universal) != EXPECTED_UNIVERSAL_FAMILIES:
        raise AcceptanceFailure(
            f"alignment catalog contains {len(universal)} universal families; "
            f"design requires {EXPECTED_UNIVERSAL_FAMILIES}"
        )
    if len(pair_only) != EXPECTED_CLAUDE_CODEX_ONLY_FAMILIES:
        raise AcceptanceFailure(
            f"alignment catalog contains {len(pair_only)} Claude+Codex-only families; "
            f"design requires {EXPECTED_CLAUDE_CODEX_ONLY_FAMILIES}"
        )
    return alignments


def available_families(
    harnesses: Iterable[str], alignments: list[dict[str, Any]]
) -> list[str]:
    selected = set(harnesses)
    return [
        item["stable_id"]
        for item in alignments
        if selected <= set(item["supported_harnesses"])
    ]


def identity(name: str, *, output_mode: str = "project") -> dict[str, Any]:
    package = name.replace("_", "-")
    return {
        "output_mode": output_mode,
        "project_name": f"{name.replace('_', ' ').title()} hooks",
        "package_name": package,
        "binary_name": package,
        "crate_path": f"crates/{package}",
        "description": f"Acceptance fixture for {name}",
        "license": "MIT OR Apache-2.0",
    }


def cross_data(
    name: str,
    harnesses: list[str],
    hooks: list[str],
    *,
    output_mode: str = "project",
    starter: str = "custom",
    state: Iterable[str] = (),
    ci: bool = False,
) -> dict[str, Any]:
    return {
        **identity(name, output_mode=output_mode),
        "starter": starter,
        "harness_mode": "cross",
        "harnesses": harnesses,
        "state_capabilities": list(state),
        "aligned_hooks": hooks,
        "lowering_policy": "best-effort-with-warnings",
        "dependency_source": "path",
        "hookkit_path": str(REPO_ROOT),
        "github_actions": ci,
    }


def single_data(
    name: str,
    harness: str,
    hooks: list[str],
    *,
    output_mode: str = "project",
    starter: str = "custom",
    state: Iterable[str] = (),
    ci: bool = False,
) -> dict[str, Any]:
    return {
        **identity(name, output_mode=output_mode),
        "starter": starter,
        "harness_mode": "single",
        "harness": harness,
        "state_capabilities": list(state),
        "native_hooks": hooks,
        "lowering_policy": "best-effort-with-warnings",
        "dependency_source": "path",
        "hookkit_path": str(REPO_ROOT),
        "github_actions": ci,
    }


def with_quality(data: dict[str, Any], *, deferred: bool = False) -> dict[str, Any]:
    result = dict(data)
    result.update(
        quality_profile="rust",
        quality_tools=["cargoFmt", "cargoClippy"],
        quality_config_path=f"{result['crate_path']}/config/{result['package_name']}.pkl",
    )
    if deferred:
        result["reconciliation_posture"] = "best-effort"
    return result


def with_quality_profile(
    data: dict[str, Any], profile: str, tools: list[str]
) -> dict[str, Any]:
    result = dict(data)
    result.update(
        quality_profile=profile,
        quality_tools=tools,
        quality_config_path=f"{result['crate_path']}/config/{result['package_name']}.pkl",
    )
    return result


def build_cases() -> list[Case]:
    events = event_catalog()
    alignments = alignment_catalog()
    all_three = list(SUPPORTED_HARNESSES)
    universal = available_families(all_three, alignments)
    claude_codex = available_families(["claude-code", "codex"], alignments)
    cases = [
        Case(
            "project_topology",
            cross_data("project_topology", all_three, universal),
        ),
        Case(
            "crate_non_workspace",
            cross_data(
                "crate_non_workspace",
                ["codex", "antigravity"],
                available_families(["codex", "antigravity"], alignments),
                output_mode="crate",
            ),
            "non-workspace",
        ),
        Case(
            "crate_workspace_ci",
            cross_data(
                "crate_workspace_ci",
                ["claude-code", "antigravity"],
                available_families(["claude-code", "antigravity"], alignments),
                output_mode="crate",
                ci=True,
            ),
            "workspace",
        ),
    ]

    for harness in SUPPORTED_HARNESSES:
        cases.append(
            Case(
                f"single_{harness.replace('-', '_')}_maximal",
                single_data(
                    f"single_{harness.replace('-', '_')}_maximal",
                    harness,
                    events[harness],
                ),
            )
        )

    for suffix, harnesses in (
        ("claude_codex", ["claude-code", "codex"]),
        ("claude_antigravity", ["claude-code", "antigravity"]),
        ("codex_antigravity", ["codex", "antigravity"]),
        ("all", all_three),
    ):
        cases.append(
            Case(
                f"cross_{suffix}",
                cross_data(
                    f"cross_{suffix}",
                    harnesses,
                    available_families(harnesses, alignments),
                ),
            )
        )

    cases.extend(
        [
            Case(
                "archetype_policy_guard",
                cross_data(
                    "archetype_policy_guard",
                    all_three,
                    ["pre_tool"],
                    starter="policy_guard",
                ),
            ),
            Case(
                "archetype_scoped_context_once",
                single_data(
                    "archetype_scoped_context_once",
                    "claude-code",
                    ["pre_tool_use"],
                    starter="scoped_context_once",
                    state=("session_metadata", "claim_once"),
                ),
            ),
            Case(
                "archetype_scoped_context_once_codex",
                single_data(
                    "archetype_scoped_context_once_codex",
                    "codex",
                    ["pre_tool_use"],
                    starter="scoped_context_once",
                    state=("session_metadata", "claim_once"),
                ),
            ),
            Case(
                "archetype_session_bootstrap",
                cross_data(
                    "archetype_session_bootstrap",
                    ["claude-code", "codex"],
                    ["session_start"],
                    starter="session_bootstrap",
                    state=("session_metadata",),
                ),
            ),
            Case(
                "archetype_immediate_quality",
                with_quality(
                    cross_data(
                        "archetype_immediate_quality",
                        all_three,
                        ["post_tool"],
                        starter="immediate_quality",
                    )
                ),
            ),
            Case(
                "runner_profile_python",
                with_quality_profile(
                    cross_data(
                        "runner_profile_python",
                        all_three,
                        ["post_tool"],
                        starter="immediate_quality",
                    ),
                    "python",
                    ["ruffFormat", "ruff"],
                ),
            ),
            Case(
                "runner_profile_javascript_typescript",
                with_quality_profile(
                    cross_data(
                        "runner_profile_javascript_typescript",
                        all_three,
                        ["post_tool"],
                        starter="immediate_quality",
                    ),
                    "javascript-typescript",
                    ["prettier", "eslint"],
                ),
            ),
            Case(
                "runner_profile_go",
                with_quality_profile(
                    cross_data(
                        "runner_profile_go",
                        all_three,
                        ["post_tool"],
                        starter="immediate_quality",
                    ),
                    "go",
                    ["gofmt", "goVet"],
                ),
            ),
            Case(
                "runner_profile_custom",
                with_quality_profile(
                    cross_data(
                        "runner_profile_custom",
                        all_three,
                        ["post_tool"],
                        starter="immediate_quality",
                    ),
                    "custom",
                    ["custom"],
                ),
            ),
            Case(
                "archetype_deferred_quality",
                with_quality(
                    cross_data(
                        "archetype_deferred_quality",
                        all_three,
                        ["post_tool", "turn_completion"],
                        starter="deferred_quality",
                        state=("session_metadata", "file_activity", "run_artifacts"),
                        ci=True,
                    ),
                    deferred=True,
                ),
            ),
            Case(
                "archetype_pre_tool_rewrite",
                cross_data(
                    "archetype_pre_tool_rewrite",
                    ["claude-code", "codex"],
                    ["pre_tool"],
                    starter="pre_tool_rewrite",
                ),
            ),
            Case(
                "state_exact_all",
                single_data(
                    "state_exact_all",
                    "claude-code",
                    events["claude-code"],
                    state=ALL_STATE_CAPABILITIES,
                    ci=True,
                ),
            ),
            Case(
                "state_independent_custom",
                single_data(
                    "state_independent_custom",
                    "codex",
                    ["post_compact"],
                    state=("session_metadata", "record_queue"),
                ),
            ),
            Case(
                "state_cross_pre_tool",
                cross_data(
                    "state_cross_pre_tool",
                    all_three,
                    ["pre_tool"],
                    state=("session_metadata", "inspectable_set"),
                ),
            ),
            Case(
                "state_inferred_all",
                single_data(
                    "state_inferred_all",
                    "antigravity",
                    events["antigravity"],
                    state=ALL_STATE_CAPABILITIES,
                ),
            ),
        ]
    )
    # Guard against a typo silently replacing one coverage cell with another.
    names = [case.name for case in cases]
    if len(names) != len(set(names)):
        raise AcceptanceFailure("duplicate acceptance-case name")
    if set(claude_codex) != {
        item["stable_id"]
        for item in alignments
        if {"claude-code", "codex"} <= set(item["supported_harnesses"])
    }:
        raise AcceptanceFailure("Claude+Codex intersection calculation is inconsistent")
    return cases


def stage_source(parent: Path) -> Path:
    source = parent / "working-tree-template"
    source.mkdir()
    shutil.copy2(REPO_ROOT / "copier.yml", source / "copier.yml")
    shutil.copytree(REPO_ROOT / "templates", source / "templates")
    if (source / ".git").exists():
        raise AcceptanceFailure("working-tree template staging unexpectedly copied .git")
    return source


def prepare_host(destination: Path, kind: str) -> dict[Path, bytes]:
    destination.mkdir(parents=True)
    if kind == "empty":
        return {}
    files = {
        destination / "README.md": b"# Existing repository\n",
        destination / ".gitignore": b"/existing-target\n",
    }
    if kind == "workspace":
        files[destination / "Cargo.toml"] = (
            b"[workspace]\nresolver = \"2\"\nmembers = []\n"
        )
    elif kind == "non-workspace":
        # Deliberately no root Cargo.toml: this is an ordinary repository into
        # which an independently buildable CLI crate is inserted.
        pass
    else:
        raise AcceptanceFailure(f"unknown host kind: {kind}")
    for path, content in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    return files


def assert_host_unchanged(before: dict[Path, bytes]) -> None:
    for path, content in before.items():
        if path.read_bytes() != content:
            raise AcceptanceFailure(f"crate render modified pre-existing host file {path}")


def copy_case(source: Path, destination: Path, data: dict[str, Any]) -> None:
    try:
        copier.run_copy(
            str(source),
            destination,
            data=data,
            defaults=True,
            quiet=True,
            cleanup_on_error=True,
        )
    except Exception as error:
        raise AcceptanceFailure(f"Copier render failed: {error}") from error


def answers_path(destination: Path, data: dict[str, Any]) -> Path:
    if data["output_mode"] == "crate":
        return destination / f".copier-answers.{data['package_name']}.yml"
    return destination / ".copier-answers.yml"


def assert_render(case: Case, destination: Path, source: Path) -> None:
    data = case.data
    crate = destination / data["crate_path"]
    required = (
        crate / "Cargo.toml",
        crate / "README.md",
        crate / "hookkit-template.manifest.yml",
        crate / "src" / "main.rs",
        crate / "src" / "lib.rs",
        crate / "src" / "scaffold" / "cli.rs",
        crate / "src" / "hooks" / "pre_tool.rs",
        answers_path(destination, data),
    )
    for path in required:
        if not path.is_file():
            raise AcceptanceFailure(f"{case.name}: required generated file is absent: {path}")

    for path in destination.rglob("*"):
        if not path.is_file():
            continue
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        # GitHub Actions expressions intentionally use `${{ ... }}`; plain
        # Jinja delimiters must never survive rendering.
        if re.search(r"(?<!\$)\{\{|\{%|\{#", content):
            raise AcceptanceFailure(f"{case.name}: unrendered Jinja remains in {path}")
        try:
            path.resolve().relative_to(destination.resolve())
        except ValueError as error:
            raise AcceptanceFailure(f"{case.name}: generated path escaped destination") from error

    answers = load_yaml(answers_path(destination, data))
    if answers.get("_src_path") != str(source):
        raise AcceptanceFailure(f"{case.name}: answers do not record staged source")
    concrete = (
        "output_mode",
        "package_name",
        "binary_name",
        "crate_path",
        "starter",
        "harness_mode",
        "state_capabilities",
        "dependency_source",
        "github_actions",
    )
    for key in concrete:
        actual = answers.get(key)
        expected = data[key]
        if key == "state_capabilities":
            matches = set(actual or []) == set(expected)
        else:
            matches = actual == expected
        if not matches:
            raise AcceptanceFailure(
                f"{case.name}: answers[{key!r}]={actual!r}, expected {expected!r}"
            )

    manifest = load_yaml(crate / "hookkit-template.manifest.yml")
    expected_harnesses = (
        data["harnesses"] if data["harness_mode"] == "cross" else [data["harness"]]
    )
    expected_hooks = (
        data["aligned_hooks"]
        if data["harness_mode"] == "cross"
        else data["native_hooks"]
    )
    if manifest["harnesses"] != expected_harnesses:
        raise AcceptanceFailure(f"{case.name}: resolved harness manifest is inaccurate")
    if set(manifest["requested_hooks"]) != set(expected_hooks):
        raise AcceptanceFailure(f"{case.name}: requested hook manifest is inaccurate")
    expected_effective_hooks = set(expected_hooks)
    if data["harness_mode"] == "cross" and "file_activity" in data["state_capabilities"]:
        expected_effective_hooks.add("session_start_state")
    if set(manifest["effective_hooks"]) != expected_effective_hooks:
        raise AcceptanceFailure(f"{case.name}: resolved hook manifest is inaccurate")
    if set(manifest["state_capabilities"]) != set(data["state_capabilities"]):
        raise AcceptanceFailure(f"{case.name}: resolved state manifest is inaccurate")

    selected = set(expected_harnesses)
    expected_available = available_families(selected, alignment_catalog())
    if data["harness_mode"] == "cross":
        if manifest["available_aligned_families"] != expected_available:
            raise AcceptanceFailure(
                f"{case.name}: generated alignment availability is not the selected intersection"
            )
        if "antigravity" in selected and len(expected_available) != 3:
            raise AcceptanceFailure(
                f"{case.name}: richer pair-only families leaked into an Antigravity set"
            )

    cargo = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    dependencies = cargo.get("dependencies", {})
    hookkit_dependencies = {
        name: spec for name, spec in dependencies.items() if name.startswith("hookkit-")
    }
    if not hookkit_dependencies:
        raise AcceptanceFailure(f"{case.name}: no HookKit dependencies were generated")
    expected_hookkit_dependencies = {"hookkit-core", "hookkit-runtime"}
    if data["harness_mode"] == "cross":
        expected_hookkit_dependencies.add("hookkit-common")
    else:
        native_package = "claude" if data["harness"] == "claude-code" else data["harness"]
        expected_hookkit_dependencies.add(f"hookkit-{native_package}")
    if data["state_capabilities"]:
        expected_hookkit_dependencies.add("hookkit-session-state")
    if "file_activity" in data["state_capabilities"]:
        expected_hookkit_dependencies.update(
            {"hookkit-file-activity", "hookkit-tool-runner"}
        )
    if data["starter"] in {"immediate_quality", "deferred_quality"}:
        expected_hookkit_dependencies.add("hookkit-tool-runner")
    if data["starter"] in {"policy_guard", "scoped_context_once"}:
        expected_hookkit_dependencies.add("hookkit-tool-access")
    if data["harness_mode"] == "single" and data["starter"] == "scoped_context_once":
        expected_hookkit_dependencies.add("hookkit-common")
    if set(hookkit_dependencies) != expected_hookkit_dependencies:
        raise AcceptanceFailure(
            f"{case.name}: direct HookKit dependencies {sorted(hookkit_dependencies)} "
            f"do not match generated source intent {sorted(expected_hookkit_dependencies)}"
        )
    expected_optional_dependencies = set()
    if {"record_queue", "custom_aggregate"} & set(data["state_capabilities"]):
        expected_optional_dependencies.add("serde")
    if data["starter"] == "pre_tool_rewrite":
        expected_optional_dependencies.add("serde_json")
    actual_optional_dependencies = {"serde", "serde_json"} & set(dependencies)
    if actual_optional_dependencies != expected_optional_dependencies:
        raise AcceptanceFailure(
            f"{case.name}: optional direct dependencies "
            f"{sorted(actual_optional_dependencies)} do not match generated source intent "
            f"{sorted(expected_optional_dependencies)}"
        )
    dependency_source = data["dependency_source"]
    compatibility = load_yaml(CATALOG_ROOT / "compatibility.yml")["hookkit"]
    if dependency_source == "path":
        for name, spec in hookkit_dependencies.items():
            if not isinstance(spec, dict) or "path" not in spec:
                raise AcceptanceFailure(
                    f"{case.name}: {name} is not a path dependency: {spec!r}"
                )
            expected = Path(data["hookkit_path"]) / "crates" / name
            resolved = (crate / spec["path"]).resolve()
            if resolved != expected.resolve():
                raise AcceptanceFailure(
                    f"{case.name}: {name} resolves to {resolved}, expected {expected}"
                )
    elif dependency_source == "git":
        revisions: set[str] = set()
        repositories: set[str] = set()
        for name, spec in hookkit_dependencies.items():
            if not isinstance(spec, dict):
                raise AcceptanceFailure(f"{case.name}: {name} has shorthand Git source")
            forbidden = {"path", "branch", "tag", "version"} & set(spec)
            if forbidden:
                raise AcceptanceFailure(
                    f"{case.name}: {name} Git source contains forbidden keys: "
                    f"{', '.join(sorted(forbidden))}"
                )
            revision = spec.get("rev")
            repository = spec.get("git")
            if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", revision):
                raise AcceptanceFailure(
                    f"{case.name}: {name} is not pinned to a full Git revision"
                )
            revisions.add(revision)
            repositories.add(repository)
        if revisions != {data["hookkit_git_rev"]}:
            raise AcceptanceFailure(
                f"{case.name}: direct HookKit dependencies do not share the requested revision"
            )
        if repositories != {compatibility["repository"]}:
            raise AcceptanceFailure(
                f"{case.name}: direct HookKit dependencies do not share the compatibility repository"
            )
    elif dependency_source == "crates-io":
        versions: set[str] = set()
        for name, spec in hookkit_dependencies.items():
            if not isinstance(spec, dict):
                raise AcceptanceFailure(f"{case.name}: {name} has shorthand crates.io source")
            forbidden = {"git", "rev", "branch", "tag", "path"} & set(spec)
            if forbidden:
                raise AcceptanceFailure(
                    f"{case.name}: {name} crates.io source contains forbidden keys: "
                    f"{', '.join(sorted(forbidden))}"
                )
            version = spec.get("version")
            if not isinstance(version, str):
                raise AcceptanceFailure(
                    f"{case.name}: {name} lacks an explicit crates.io version"
                )
            versions.add(version)
        expected_version = str(compatibility["crates_io_version"])
        if versions != {expected_version}:
            raise AcceptanceFailure(
                f"{case.name}: direct HookKit dependencies do not share version {expected_version}"
            )
    else:
        raise AcceptanceFailure(f"{case.name}: unknown dependency source {dependency_source!r}")

    if "file_activity" in data["state_capabilities"]:
        required_state_dependencies = {"hookkit-file-activity", "hookkit-session-state"}
        missing_dependencies = required_state_dependencies - set(hookkit_dependencies)
        if missing_dependencies:
            raise AcceptanceFailure(
                f"{case.name}: standalone file activity lacks direct dependencies: "
                f"{', '.join(sorted(missing_dependencies))}"
            )
        runners = crate / "src" / "scaffold" / "runners.rs"
        if not runners.is_file():
            raise AcceptanceFailure(
                f"{case.name}: standalone file activity did not generate runner adapters"
            )
        runner_manifest = manifest.get("runner")
        if not isinstance(runner_manifest, dict) or not runner_manifest.get("config_path"):
            raise AcceptanceFailure(
                f"{case.name}: standalone file activity lacks resolved runner configuration"
            )
        config = destination / runner_manifest["config_path"]
        if not config.is_file() or not config.read_text(encoding="utf-8").strip():
            raise AcceptanceFailure(
                f"{case.name}: standalone file activity did not generate its Pkl configuration"
            )

    if data["output_mode"] == "project":
        workspace = tomllib.loads((destination / "Cargo.toml").read_text(encoding="utf-8"))
        if workspace.get("workspace", {}).get("resolver") != "3":
            raise AcceptanceFailure(
                f"{case.name}: the generated workspace must use the MSRV-aware resolver \"3\""
            )

    cli_source = (crate / "src" / "scaffold" / "cli.rs").read_text(encoding="utf-8")
    for hook in expected_hooks:
        command = hook.replace("_", "-")
        if command not in cli_source:
            raise AcceptanceFailure(f"{case.name}: selected command {command!r} is unreachable")
    # Clap's own exit path uses status 2, which Claude Code and Codex treat as
    # a blocking hook decision.
    if "try_parse()" not in cli_source or ".exit()" in cli_source:
        raise AcceptanceFailure(
            f"{case.name}: argument errors must not exit through Clap's blocking status 2"
        )

    if (
        data["harness_mode"] == "single"
        and data["harness"] == "claude-code"
        and "pre_tool_use" in data["native_hooks"]
    ):
        # An explicit "allow" skips Claude Code's permission prompt, so the
        # starters' no-objection path must be an empty pass-through, and
        # context is delivered without a permission decision.
        if data["starter"] == "custom":
            starter_path = crate / "src" / "hooks" / "native" / "claude_code_pre_tool_use.rs"
        else:
            starter_path = crate / "src" / "scaffold" / "dispatch.rs"
        starter_source = starter_path.read_text(encoding="utf-8")
        if "hookkit_claude::catalog::PreToolUseOutput::no_op()" not in starter_source or any(
            allow in starter_source
            for allow in ("PreToolPermissionDecision::Allow", "PreToolUseOutput::allow()")
        ):
            raise AcceptanceFailure(
                f"{case.name}: Claude PreToolUse starter auto-approves tool calls "
                f"instead of passing through in {starter_path.name}"
            )

    if (
        data["harness_mode"] == "single"
        and data["harness"] == "antigravity"
        and "pre_tool_use" in data["native_hooks"]
    ):
        # Antigravity requires a decision and documents no pass-through;
        # "allow" auto-approves and bypasses Ask presets, so the no-objection
        # path must answer the least-privilege "ask".
        if data["starter"] == "custom":
            starter_path = crate / "src" / "hooks" / "native" / "antigravity_pre_tool_use.rs"
        else:
            starter_path = crate / "src" / "scaffold" / "dispatch.rs"
        starter_source = starter_path.read_text(encoding="utf-8")
        if "hookkit_antigravity::PreToolUseOutput::ask()" not in starter_source or any(
            allow in starter_source
            for allow in ("ToolDecision::Allow", "PreToolUseOutput::allow()")
        ):
            raise AcceptanceFailure(
                f"{case.name}: Antigravity PreToolUse starter auto-approves tool calls "
                f"instead of answering ask in {starter_path.name}"
            )

    if data["harness_mode"] == "cross" and "pre_tool" in data["aligned_hooks"]:
        # The portable no-objection path is the aligned pass-through; an
        # aligned allow auto-approves on Claude Code and Antigravity, and a
        # rewrite starter must not approve the calls it rewrites.
        dispatch = (crate / "src" / "scaffold" / "dispatch.rs").read_text(encoding="utf-8")
        if (
            "hookkit_common::PreToolUseOutput::pass_through(" not in dispatch
            or "PreToolUseOutput::allow(" in dispatch
            or "RewriteApproval::AutoApprove" in dispatch
        ):
            raise AcceptanceFailure(
                f"{case.name}: aligned PreToolUse starter auto-approves tool calls "
                "instead of passing through in dispatch.rs"
            )

    claude_file_activity_post_tool = (
        "file_activity" in data["state_capabilities"]
        and data["starter"] != "immediate_quality"
        and (
            (
                data["harness_mode"] == "cross"
                and "post_tool" in data["aligned_hooks"]
                and "claude-code" in data["harnesses"]
            )
            or (
                data["harness_mode"] == "single"
                and data["harness"] == "claude-code"
                and "post_tool_use" in data["native_hooks"]
            )
        )
    )
    if claude_file_activity_post_tool:
        # The file-activity observer also records failed Claude tool calls,
        # which may have written files before failing.
        readme = (crate / "README.md").read_text(encoding="utf-8")
        if "`PostToolUseFailure`" not in readme:
            raise AcceptanceFailure(
                f"{case.name}: README does not bind the file-activity command to "
                "Claude Code PostToolUseFailure"
            )

    if data["harness_mode"] == "cross" and "permission_request" in data["aligned_hooks"]:
        # An aligned PermissionRequest allow answers the permission dialog on
        # the user's behalf; the starter must leave the dialog to the user.
        starter_path = crate / "src" / "hooks" / "aligned" / "permission_request.rs"
        starter_source = starter_path.read_text(encoding="utf-8")
        if (
            "hookkit_common::PermissionRequestOutput::no_op(" not in starter_source
            or "PermissionRequestOutput::allow(" in starter_source
        ):
            raise AcceptanceFailure(
                f"{case.name}: aligned PermissionRequest starter auto-approves "
                f"permission requests instead of passing through in {starter_path.name}"
            )

    if data["harness_mode"] == "cross":
        # The generated catalog test must check each starter's native output,
        # not only its exit status, so an approving or blocking starter
        # (such as a PermissionRequest `"behavior":"allow"`) fails it.
        catalog_test = (crate / "tests" / "aligned_catalog_protocol.rs").read_text(
            encoding="utf-8"
        )
        for test in catalog_test.split("#[test]")[1:]:
            if "assert_neutral(" not in test and "stdout().is_empty()" not in test:
                name = re.search(r"fn (\w+)\(", test)
                raise AcceptanceFailure(
                    f"{case.name}: aligned catalog test "
                    f"{name.group(1) if name else '?'} does not check the starter's output"
                )

    if (
        data["harness_mode"] == "single"
        and data["starter"] == "custom"
        and "pre_tool_use" in data["native_hooks"]
    ):
        prefix = data["harness"].replace("-", "_")
        exact_handler = crate / "src" / "hooks" / "native" / f"{prefix}_pre_tool_use.rs"
        if not exact_handler.is_file():
            raise AcceptanceFailure(
                f"{case.name}: custom native PreToolUse lacks an exact handler seam"
            )
        native_exports = (crate / "src" / "hooks" / "native.rs").read_text(
            encoding="utf-8"
        )
        if "handle as pre_tool_use;" not in native_exports:
            raise AcceptanceFailure(
                f"{case.name}: custom native PreToolUse seam is not exported"
            )
        dispatch = (crate / "src" / "scaffold" / "dispatch.rs").read_text(
            encoding="utf-8"
        )
        if "hooks::native::pre_tool_use" not in dispatch:
            raise AcceptanceFailure(
                f"{case.name}: custom native PreToolUse dispatch bypasses its exact seam"
            )
        catalog_test = (crate / "tests" / "native_catalog_protocol.rs").read_text(
            encoding="utf-8"
        )
        if "hooks::native::pre_tool_use" not in catalog_test:
            raise AcceptanceFailure(
                f"{case.name}: native catalog test does not execute the exact PreToolUse seam"
            )

    workflow = destination / ".github" / "workflows" / f"{data['package_name']}-hook-ci.yml"
    if data["github_actions"]:
        if not workflow.is_file():
            raise AcceptanceFailure(f"{case.name}: requested workflow was not generated")
        parsed_workflow = load_yaml(workflow)
        if not isinstance(parsed_workflow, dict) or "jobs" not in parsed_workflow:
            raise AcceptanceFailure(f"{case.name}: generated workflow is not valid YAML")
        workflow_text = workflow.read_text(encoding="utf-8")
        if data["package_name"] not in workflow_text:
            raise AcceptanceFailure(f"{case.name}: workflow is not package-namespaced")
        assert_workflow_cache_target(case, parsed_workflow)
    elif workflow.exists():
        raise AcceptanceFailure(f"{case.name}: workflow generated despite opt-out")


def assert_workflow_cache_target(case: Case, workflow: dict[str, Any]) -> None:
    """Check that rust-cache saves the directory Cargo actually builds into."""
    for job in workflow["jobs"].values():
        caches = [
            step
            for step in job.get("steps", [])
            if str(step.get("uses", "")).startswith("Swatinem/rust-cache@")
        ]
        if len(caches) != 1:
            raise AcceptanceFailure(f"{case.name}: workflow job lacks exactly one Rust cache")
        workspace_root, _, target = caches[0]["with"]["workspaces"].partition(" -> ")
        cached_target = os.path.normpath(os.path.join(workspace_root, target))
        if case.data["output_mode"] == "project":
            # Project mode builds the generated workspace at the repository root.
            expected = workspace_root == "." and cached_target == "target"
        else:
            # Crate mode pins CARGO_TARGET_DIR because the host may or may not
            # own a workspace; the cache must point at that same directory.
            target_dir = job.get("env", {}).get("CARGO_TARGET_DIR")
            expected = (
                workspace_root == case.data["crate_path"]
                and cached_target == "target"
                and target_dir == "${{ github.workspace }}/target"
            )
        if not expected:
            raise AcceptanceFailure(
                f"{case.name}: workflow caches {caches[0]['with']['workspaces']!r}, "
                "not the directory Cargo builds into"
            )


def register_workspace_member(destination: Path, case: Case) -> None:
    if case.host != "workspace":
        return
    manifest = destination / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    old = "members = []"
    if old not in text:
        raise AcceptanceFailure(f"{case.name}: workspace fixture has unexpected manifest")
    manifest.write_text(
        text.replace(old, f'members = ["{case.data["crate_path"]}"]'), encoding="utf-8"
    )


def run_command(command: list[str], *, cwd: Path, environment: dict[str, str]) -> None:
    process = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if process.returncode:
        output = process.stdout[-30_000:]
        raise AcceptanceFailure(
            f"command failed ({process.returncode}): {' '.join(command)}\n{output}"
        )


def validate_generated(
    destination: Path, case: Case, validation: str, target: Path
) -> None:
    if validation == "render":
        return
    manifest = destination / case.data["crate_path"] / "Cargo.toml"
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target)
    if validation == "check":
        commands = [["cargo", "check", "--manifest-path", str(manifest), "--all-targets"]]
    else:
        commands = [
            ["cargo", "fmt", "--manifest-path", str(manifest), "--all", "--", "--check"],
            [
                "cargo",
                f"+{RUST_MSRV}",
                "check",
                "--manifest-path",
                str(manifest),
                "--all-targets",
            ],
            ["cargo", "check", "--manifest-path", str(manifest), "--all-targets"],
            [
                "cargo",
                "clippy",
                "--manifest-path",
                str(manifest),
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
            ["cargo", "test", "--manifest-path", str(manifest), "--all-targets"],
        ]
    for command in commands:
        run_command(command, cwd=destination, environment=environment)


def run_render_case(
    case: Case, source: Path, root: Path, validation: str, target: Path
) -> None:
    started = time.monotonic()
    destination = root / "renders" / case.name
    before = prepare_host(destination, case.host)
    copy_case(source, destination, case.data)
    assert_host_unchanged(before)
    assert_render(case, destination, source)
    register_workspace_member(destination, case)
    validate_generated(destination, case, validation, target)
    print(f"PASS {case.name} ({time.monotonic() - started:.1f}s)", flush=True)


def default_answers_data(name: str, hookkit_path: Path) -> dict[str, Any]:
    """The fewest answers a render needs: every other question keeps its default."""
    return {
        "project_name": f"{name.replace('_', ' ').title()} hooks",
        "dependency_source": "path",
        "hookkit_path": str(hookkit_path),
    }


def copy_default_answers(
    source: Path, destination: Path, name: str, hookkit_path: Path
) -> Case:
    """Render the questionnaire's defaults and return them as a checked case."""
    copy_case(source, destination, default_answers_data(name, hookkit_path))
    answers = load_yaml(answers_path(destination, {"output_mode": "project"}))
    for key, expected in DEFAULT_ANSWERS.items():
        if answers.get(key) != expected:
            raise AcceptanceFailure(
                f"{name}: default answers[{key!r}]={answers.get(key)!r}, expected {expected!r}"
            )
    return Case(name, answers)


def run_default_answers(source: Path, root: Path, validation: str, target: Path) -> None:
    """Render and validate the project a user gets by accepting every default."""
    started = time.monotonic()
    destination = root / "renders" / "default_answers"
    case = copy_default_answers(source, destination, "default_answers", REPO_ROOT)
    assert_render(case, destination, source)
    validate_generated(destination, case, validation, target)
    print(f"PASS default_answers ({time.monotonic() - started:.1f}s)", flush=True)


def run_coexistence(source: Path, root: Path, validation: str, target: Path) -> None:
    started = time.monotonic()
    destination = root / "coexistence"
    before = prepare_host(destination, "workspace")
    alignments = alignment_catalog()
    alpha = Case(
        "coexistence_alpha",
        cross_data(
            "coexistence_alpha",
            ["claude-code", "codex"],
            available_families(["claude-code", "codex"], alignments),
            output_mode="crate",
            ci=True,
        ),
        "workspace",
    )
    beta = Case(
        "coexistence_beta",
        single_data(
            "coexistence_beta",
            "antigravity",
            ["pre_tool_use", "post_tool_use", "stop"],
            output_mode="crate",
            ci=True,
        ),
        "workspace",
    )
    copy_case(source, destination, alpha.data)
    assert_host_unchanged(before)
    assert_render(alpha, destination, source)
    alpha_hook = destination / alpha.data["crate_path"] / "src" / "hooks" / "pre_tool.rs"
    alpha_bytes = alpha_hook.read_bytes()
    copy_case(source, destination, beta.data)
    assert_host_unchanged(before)
    assert_render(beta, destination, source)
    if alpha_hook.read_bytes() != alpha_bytes:
        raise AcceptanceFailure("second crate render changed the first crate's user-owned hook")
    for case in (alpha, beta):
        if not answers_path(destination, case.data).is_file():
            raise AcceptanceFailure(f"missing namespaced answers for {case.name}")
        workflow = (
            destination
            / ".github"
            / "workflows"
            / f"{case.data['package_name']}-hook-ci.yml"
        )
        if not workflow.is_file():
            raise AcceptanceFailure(f"missing namespaced workflow for {case.name}")
    manifest = destination / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    members = f'members = ["{alpha.data["crate_path"]}", "{beta.data["crate_path"]}"]'
    manifest.write_text(text.replace("members = []", members), encoding="utf-8")
    for case in (alpha, beta):
        validate_generated(destination, case, validation, target)
    print(f"PASS coexistence ({time.monotonic() - started:.1f}s)", flush=True)


def git(source: Path, *arguments: str) -> None:
    run_command(["git", *arguments], cwd=source, environment=os.environ.copy())


def cargo_check_generated(destination: Path, data: dict[str, Any], target: Path) -> None:
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target)
    run_command(
        [
            "cargo",
            "check",
            "--manifest-path",
            str(destination / data["crate_path"] / "Cargo.toml"),
            "--all-targets",
        ],
        cwd=destination,
        environment=environment,
    )


def cargo_test_runner_config(
    destination: Path, data: dict[str, Any], target: Path
) -> None:
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target)
    run_command(
        [
            "cargo",
            "test",
            "--manifest-path",
            str(destination / data["crate_path"] / "Cargo.toml"),
            "--test",
            "runner_config",
        ],
        cwd=destination,
        environment=environment,
    )


def deferred_quality_data(name: str) -> dict[str, Any]:
    return with_quality(
        cross_data(
            name,
            list(SUPPORTED_HARNESSES),
            ["post_tool", "turn_completion"],
            output_mode="crate",
            starter="deferred_quality",
            state=("session_metadata", "file_activity", "run_artifacts"),
        ),
        deferred=True,
    )


def customize_quality_config(destination: Path, data: dict[str, Any]) -> bytes:
    config = destination / data["quality_config_path"]
    text = config.read_text(encoding="utf-8")
    replacements = (
        (
            '  ["cargoClippy"] = Builtins.cargoClippy\n',
            '  ["cargoClippy"] = Builtins.cargoClippy\n'
            '  ["ruff"] = Builtins.ruff\n',
        ),
        (
            '  "cargoClippy"\n}',
            '  "cargoClippy"\n  "ruff"\n}',
        ),
    )
    for original, customized in replacements:
        if text.count(original) != 1:
            raise AcceptanceFailure(
                "generated deferred-quality policy has an unexpected shape"
            )
        text = text.replace(original, customized, 1)
    config.write_text(text, encoding="utf-8")
    return text.encode()


def assert_quality_config_ownership(
    destination: Path,
    data: dict[str, Any],
    customized: bytes,
    target: Path,
) -> None:
    config = destination / data["quality_config_path"]
    if config.read_bytes() != customized:
        raise AcceptanceFailure("Copier discarded a customized quality policy")
    rejects = list(destination.rglob("*.rej"))
    if rejects:
        raise AcceptanceFailure(f"unexpected Copier conflict files: {rejects}")
    cargo_test_runner_config(destination, data, target)


def ownership_cases() -> list[tuple[str, dict[str, Any], dict[str, Any], Path, Path, str]]:
    cross_initial = cross_data(
        "ownership_cross",
        list(SUPPORTED_HARNESSES),
        ["post_tool"],
        output_mode="crate",
    )
    cross_updated = dict(cross_initial)
    cross_updated["aligned_hooks"] = ["post_tool", "turn_completion"]
    native_initial = single_data(
        "ownership_native",
        "codex",
        ["pre_tool_use"],
        output_mode="crate",
    )
    native_updated = dict(native_initial)
    native_updated["native_hooks"] = ["pre_tool_use", "stop"]
    return [
        (
            "cross",
            cross_initial,
            cross_updated,
            Path("src/hooks/aligned/post_tool.rs"),
            Path("src/hooks/aligned/turn_completion.rs"),
            "turn_completion",
        ),
        (
            "native",
            native_initial,
            native_updated,
            Path("src/hooks/native/codex_pre_tool_use.rs"),
            Path("src/hooks/native/codex_stop.rs"),
            "stop",
        ),
    ]


def assert_answer_change_ownership(
    destination: Path,
    data: dict[str, Any],
    old_handler: Path,
    new_handler: Path,
    new_hook: str,
    user_marker: str,
    target: Path,
) -> None:
    crate = destination / data["crate_path"]
    old_path = crate / old_handler
    if user_marker not in old_path.read_text(encoding="utf-8"):
        raise AcceptanceFailure("Copier discarded a customized per-hook handler")
    if not (crate / new_handler).is_file():
        raise AcceptanceFailure(f"Copier did not create newly selected handler {new_handler}")
    namespace = "aligned" if data["harness_mode"] == "cross" else "native"
    aggregator = (crate / "src" / "hooks" / f"{namespace}.rs").read_text(
        encoding="utf-8"
    )
    if f"handle as {new_hook};" not in aggregator:
        raise AcceptanceFailure(f"managed {namespace} exports omit {new_hook}")
    dispatch = (crate / "src" / "scaffold" / "dispatch.rs").read_text(encoding="utf-8")
    if f"hooks::{namespace}::{new_hook}" not in dispatch:
        raise AcceptanceFailure(f"managed dispatch omits {namespace}::{new_hook}")
    rejects = list(destination.rglob("*.rej"))
    if rejects:
        raise AcceptanceFailure(f"unexpected Copier conflict files: {rejects}")
    cargo_check_generated(destination, data, target)


def run_recopy_ownership(source: Path, root: Path) -> None:
    started = time.monotonic()
    target = root / "cargo-target-ownership-recopy"
    for kind, initial, updated, old_handler, new_handler, new_hook in ownership_cases():
        destination = root / f"recopy-{kind}"
        copy_case(source, destination, initial)
        old_path = destination / initial["crate_path"] / old_handler
        user_marker = f"\n// USER {kind.upper()} HANDLER EDIT MUST SURVIVE RECOPY\n"
        old_path.write_text(
            old_path.read_text(encoding="utf-8") + user_marker,
            encoding="utf-8",
        )
        try:
            copier.run_copy(
                str(source),
                destination,
                data=updated,
                defaults=True,
                quiet=True,
                overwrite=True,
            )
        except Exception as error:
            raise AcceptanceFailure(f"Copier recopy failed for {kind}: {error}") from error
        assert_answer_change_ownership(
            destination,
            updated,
            old_handler,
            new_handler,
            new_hook,
            user_marker,
            target,
        )

        deselected = dict(updated)
        hook_key = "aligned_hooks" if kind == "cross" else "native_hooks"
        old_hook = "post_tool" if kind == "cross" else "pre_tool_use"
        deselected[hook_key] = [new_hook]
        copier.run_copy(
            str(source),
            destination,
            data=deselected,
            defaults=True,
            quiet=True,
            overwrite=True,
        )
        if user_marker not in old_path.read_text(encoding="utf-8"):
            raise AcceptanceFailure(
                f"Copier removed orphaned {kind} handler after deselection"
            )
        namespace = "aligned" if kind == "cross" else "native"
        aggregator = (
            destination
            / updated["crate_path"]
            / "src"
            / "hooks"
            / f"{namespace}.rs"
        ).read_text(encoding="utf-8")
        if f"handle as {old_hook};" in aggregator:
            raise AcceptanceFailure(f"managed {namespace} exports retain deselected {old_hook}")

        copier.run_copy(
            str(source),
            destination,
            data=updated,
            defaults=True,
            quiet=True,
            overwrite=True,
        )
        assert_answer_change_ownership(
            destination,
            updated,
            old_handler,
            new_handler,
            new_hook,
            user_marker,
            target,
        )
    print(f"PASS recopy_ownership ({time.monotonic() - started:.1f}s)", flush=True)


def run_shape_change_ownership(source: Path, root: Path) -> None:
    started = time.monotonic()
    target = root / "cargo-target-ownership-shapes"

    state_addition_initial = single_data(
        "ownership_state_addition",
        "codex",
        ["post_compact"],
        output_mode="crate",
    )
    state_addition_updated = dict(state_addition_initial)
    state_addition_updated["state_capabilities"] = ["session_metadata"]
    state_addition_destination = root / "recopy-state-addition"
    copy_case(source, state_addition_destination, state_addition_initial)
    state_addition_handler = (
        state_addition_destination
        / state_addition_initial["crate_path"]
        / "src/hooks/native/codex_post_compact.rs"
    )
    state_addition_marker = "\n// HANDLER EDIT MUST SURVIVE STATE ADDITION\n"
    state_addition_handler.write_text(
        state_addition_handler.read_text(encoding="utf-8") + state_addition_marker,
        encoding="utf-8",
    )
    copier.run_copy(
        str(source),
        state_addition_destination,
        data=state_addition_updated,
        defaults=True,
        quiet=True,
        overwrite=True,
    )
    if state_addition_marker not in state_addition_handler.read_text(encoding="utf-8"):
        raise AcceptanceFailure("adding state discarded an existing event handler")
    state_addition_crate = (
        state_addition_destination / state_addition_updated["crate_path"]
    )
    if not (state_addition_crate / "src/scaffold/state.rs").is_file():
        raise AcceptanceFailure("adding session state did not create state helpers")
    state_addition_dispatch = (
        state_addition_crate / "src/scaffold/dispatch.rs"
    ).read_text(encoding="utf-8")
    if "state_dir.as_deref()" not in state_addition_dispatch:
        raise AcceptanceFailure("ordinary event dispatch did not forward --state-dir")
    cargo_check_generated(state_addition_destination, state_addition_updated, target)

    session_initial = cross_data(
        "ownership_session_starter",
        ["claude-code", "codex"],
        ["session_start"],
        output_mode="crate",
    )
    session_updated = dict(session_initial)
    session_updated.update(
        starter="session_bootstrap",
        state_capabilities=["session_metadata"],
    )
    session_destination = root / "recopy-session-starter"
    copy_case(source, session_destination, session_initial)
    session_handler = (
        session_destination
        / session_initial["crate_path"]
        / "src/hooks/aligned/session_start.rs"
    )
    session_marker = "\n// SESSION HANDLER EDIT MUST SURVIVE STARTER CHANGE\n"
    session_handler.write_text(
        session_handler.read_text(encoding="utf-8") + session_marker,
        encoding="utf-8",
    )
    copier.run_copy(
        str(source),
        session_destination,
        data=session_updated,
        defaults=True,
        quiet=True,
        overwrite=True,
    )
    if session_marker not in session_handler.read_text(encoding="utf-8"):
        raise AcceptanceFailure("starter change discarded the aligned session handler")
    session_dispatch = (
        session_destination
        / session_updated["crate_path"]
        / "src/scaffold/dispatch.rs"
    ).read_text(encoding="utf-8")
    if "session_bootstrap::build_context" not in session_dispatch:
        raise AcceptanceFailure("session-bootstrap lowering was not refreshed in dispatch")
    if "session_bootstrap" in session_handler.read_text(encoding="utf-8"):
        raise AcceptanceFailure("protected session handler still depends on selected starter")
    cargo_check_generated(session_destination, session_updated, target)

    policy_initial = cross_data(
        "ownership_policy_starter",
        list(SUPPORTED_HARNESSES),
        ["pre_tool"],
        output_mode="crate",
    )
    policy_updated = dict(policy_initial)
    policy_updated["starter"] = "policy_guard"
    # Copier evaluates a skipped multiselect's default while merging prior
    # answers; keep that hidden native answer valid during the mode-stable recopy.
    policy_updated["native_hooks"] = ["pre_tool_use"]
    policy_destination = root / "recopy-policy-starter"
    copy_case(source, policy_destination, policy_initial)
    policy = (
        policy_destination
        / policy_initial["crate_path"]
        / "src/hooks/pre_tool/policy.rs"
    )
    policy_marker = "\n// POLICY EDIT MUST SURVIVE STARTER CHANGE\n"
    policy.write_text(policy.read_text(encoding="utf-8") + policy_marker, encoding="utf-8")
    copier.run_copy(
        str(source),
        policy_destination,
        data=policy_updated,
        defaults=True,
        quiet=True,
        overwrite=True,
    )
    if policy_marker not in policy.read_text(encoding="utf-8"):
        raise AcceptanceFailure("starter change discarded the prior pre-tool policy")
    policy_crate = policy_destination / policy_updated["crate_path"]
    if not (policy_crate / "src/hooks/pre_tool/policy_guard.rs").is_file():
        raise AcceptanceFailure("starter change did not create the report-aware policy seam")
    policy_exports = (policy_crate / "src/hooks/pre_tool.rs").read_text(encoding="utf-8")
    if "policy_guard::evaluate" not in policy_exports:
        raise AcceptanceFailure("managed pre-tool exports did not select policy_guard")
    cargo_check_generated(policy_destination, policy_updated, target)

    state_initial = cross_data(
        "ownership_state_types",
        list(SUPPORTED_HARNESSES),
        ["post_tool"],
        output_mode="crate",
        state=["session_metadata", "record_queue"],
    )
    state_updated = dict(state_initial)
    state_updated["state_capabilities"] = [
        "session_metadata",
        "record_queue",
        "custom_aggregate",
    ]
    state_destination = root / "recopy-state-types"
    copy_case(source, state_destination, state_initial)
    record_type = (
        state_destination
        / state_initial["crate_path"]
        / "src/hooks/state_types/record_queue.rs"
    )
    record_marker = "\n// RECORD TYPE EDIT MUST SURVIVE CAPABILITY CHANGE\n"
    record_type.write_text(
        record_type.read_text(encoding="utf-8") + record_marker,
        encoding="utf-8",
    )
    copier.run_copy(
        str(source),
        state_destination,
        data=state_updated,
        defaults=True,
        quiet=True,
        overwrite=True,
    )
    if record_marker not in record_type.read_text(encoding="utf-8"):
        raise AcceptanceFailure("state capability change discarded the queue record type")
    state_crate = state_destination / state_updated["crate_path"]
    if not (state_crate / "src/hooks/state_types/custom_aggregate.rs").is_file():
        raise AcceptanceFailure("state capability change did not create aggregate types")
    state_exports = (state_crate / "src/hooks/state_types.rs").read_text(encoding="utf-8")
    if "CustomAggregateEvent" not in state_exports:
        raise AcceptanceFailure("managed state type exports omit custom aggregate types")
    cargo_check_generated(state_destination, state_updated, target)

    print(f"PASS shape_change_ownership ({time.monotonic() - started:.1f}s)", flush=True)


def run_update_ownership(staged_source: Path, root: Path) -> None:
    started = time.monotonic()
    source = root / "versioned-template"
    shutil.copytree(staged_source, source)
    git(source, "init", "--quiet")
    git(source, "config", "user.name", "HookKit template tests")
    git(source, "config", "user.email", "hookkit-template-tests@example.invalid")
    git(source, "add", ".")
    git(source, "commit", "--quiet", "-m", "template v1")
    git(source, "tag", "v1.0.0")

    destinations = []
    for kind, initial, updated, old_handler, new_handler, new_hook in ownership_cases():
        destination = root / f"update-{kind}"
        copier.run_copy(
            str(source),
            destination,
            data=initial,
            defaults=True,
            quiet=True,
            vcs_ref="v1.0.0",
        )
        git(destination, "init", "--quiet")
        git(destination, "config", "user.name", "HookKit template tests")
        git(destination, "config", "user.email", "hookkit-template-tests@example.invalid")
        git(destination, "add", ".")
        git(destination, "commit", "--quiet", "-m", "generated from template v1")
        old_path = destination / initial["crate_path"] / old_handler
        user_marker = f"\n// USER {kind.upper()} HANDLER EDIT MUST SURVIVE UPDATE\n"
        old_path.write_text(
            old_path.read_text(encoding="utf-8") + user_marker,
            encoding="utf-8",
        )
        git(destination, "add", ".")
        git(destination, "commit", "--quiet", "-m", "customize user-owned handler")
        destinations.append(
            (
                kind,
                destination,
                updated,
                old_handler,
                new_handler,
                new_hook,
                user_marker,
            )
        )

    readme_template = (
        source
        / "templates"
        / "hook-project"
        / "template"
        / "{{ crate_path }}"
        / "README.md.jinja"
    )
    managed_marker = "\nManaged update marker: v1.1.0\n"
    readme_template.write_text(
        readme_template.read_text(encoding="utf-8") + managed_marker,
        encoding="utf-8",
    )
    git(source, "add", ".")
    git(source, "commit", "--quiet", "-m", "template v1.1")
    git(source, "tag", "v1.1.0")
    target = root / "cargo-target-ownership-update"
    for (
        kind,
        destination,
        updated,
        old_handler,
        new_handler,
        new_hook,
        user_marker,
    ) in destinations:
        try:
            copier.run_update(
                destination,
                data=updated,
                answers_file=answers_path(destination, updated).name,
                defaults=True,
                quiet=True,
                conflict="rej",
                overwrite=True,
            )
        except Exception as error:
            raise AcceptanceFailure(f"Copier update failed for {kind}: {error}") from error
        crate = destination / updated["crate_path"]
        if managed_marker not in (crate / "README.md").read_text(encoding="utf-8"):
            raise AcceptanceFailure("Copier update did not refresh a managed file")
        assert_answer_change_ownership(
            destination,
            updated,
            old_handler,
            new_handler,
            new_hook,
            user_marker,
            target,
        )
        answers = load_yaml(answers_path(destination, updated))
        if answers.get("_commit") != "v1.1.0":
            raise AcceptanceFailure(
                f"updated answers record commit {answers.get('_commit')!r}, expected 'v1.1.0'"
            )
    print(f"PASS update_ownership ({time.monotonic() - started:.1f}s)", flush=True)


def run_quality_config_ownership(staged_source: Path, root: Path) -> None:
    started = time.monotonic()
    target = root / "cargo-target-quality-config-ownership"

    recopy_data = deferred_quality_data("ownership_quality_recopy")
    recopy_destination = root / "recopy-quality-config"
    copy_case(staged_source, recopy_destination, recopy_data)
    recopy_customized = customize_quality_config(recopy_destination, recopy_data)
    copier.run_copy(
        str(staged_source),
        recopy_destination,
        data=recopy_data,
        defaults=True,
        quiet=True,
        overwrite=True,
    )
    assert_quality_config_ownership(
        recopy_destination,
        recopy_data,
        recopy_customized,
        target,
    )

    versioned_source = root / "versioned-quality-config-template"
    shutil.copytree(staged_source, versioned_source)
    git(versioned_source, "init", "--quiet")
    git(versioned_source, "config", "user.name", "HookKit template tests")
    git(
        versioned_source,
        "config",
        "user.email",
        "hookkit-template-tests@example.invalid",
    )
    git(versioned_source, "add", ".")
    git(versioned_source, "commit", "--quiet", "-m", "template v1")
    git(versioned_source, "tag", "v1.0.0")

    update_data = deferred_quality_data("ownership_quality_update")
    update_destination = root / "update-quality-config"
    copier.run_copy(
        str(versioned_source),
        update_destination,
        data=update_data,
        defaults=True,
        quiet=True,
        vcs_ref="v1.0.0",
    )
    git(update_destination, "init", "--quiet")
    git(update_destination, "config", "user.name", "HookKit template tests")
    git(
        update_destination,
        "config",
        "user.email",
        "hookkit-template-tests@example.invalid",
    )
    git(update_destination, "add", ".")
    git(update_destination, "commit", "--quiet", "-m", "generated from template v1")
    update_customized = customize_quality_config(update_destination, update_data)
    git(update_destination, "add", ".")
    git(update_destination, "commit", "--quiet", "-m", "customize quality policy")

    readme_template = (
        versioned_source
        / "templates"
        / "hook-project"
        / "template"
        / "{{ crate_path }}"
        / "README.md.jinja"
    )
    managed_marker = "\nManaged quality-config update marker: v1.1.0\n"
    readme_template.write_text(
        readme_template.read_text(encoding="utf-8") + managed_marker,
        encoding="utf-8",
    )
    git(versioned_source, "add", ".")
    git(versioned_source, "commit", "--quiet", "-m", "template v1.1")
    git(versioned_source, "tag", "v1.1.0")

    try:
        copier.run_update(
            update_destination,
            data=update_data,
            answers_file=answers_path(update_destination, update_data).name,
            defaults=True,
            quiet=True,
            conflict="rej",
            overwrite=True,
        )
    except Exception as error:
        raise AcceptanceFailure(
            f"Copier update failed for customized quality policy: {error}"
        ) from error
    crate = update_destination / update_data["crate_path"]
    if managed_marker not in (crate / "README.md").read_text(encoding="utf-8"):
        raise AcceptanceFailure("Copier did not update a managed quality-project file")
    answers = load_yaml(answers_path(update_destination, update_data))
    if answers.get("_commit") != "v1.1.0":
        raise AcceptanceFailure(
            "quality-project answers did not advance to template v1.1.0"
        )
    assert_quality_config_ownership(
        update_destination,
        update_data,
        update_customized,
        target,
    )
    print(
        f"PASS quality_config_ownership ({time.monotonic() - started:.1f}s)",
        flush=True,
    )


def is_question_rejection(error: BaseException, question: str) -> bool:
    """Whether Copier refused an answer to ``question`` through its validator.

    Template syntax errors, raising validator expressions, and I/O failures
    also surface as exceptions; only Copier's answer-validation message proves
    that the intended questionnaire rule rejected the input.
    """
    message = str(error)
    return isinstance(error, ValueError) and (
        message.startswith(f"Validation error for question '{question}':")
        or message.startswith(f"Invalid choice for '{question}':")
    )


def assert_rejected(
    source: Path, root: Path, name: str, data: dict[str, Any], question: str
) -> None:
    destination = root / "negative" / name
    try:
        copier.run_copy(
            str(source),
            destination,
            data=data,
            defaults=True,
            quiet=True,
            cleanup_on_error=True,
        )
    except Exception as error:
        if not is_question_rejection(error, question):
            raise AcceptanceFailure(
                f"negative case {name} failed for an unexpected reason instead of "
                f"rejecting {question!r}: {type(error).__name__}: {error}"
            ) from error
        if destination.exists() and any(destination.rglob("*")):
            raise AcceptanceFailure(f"negative case {name} left a partial destination")
        return
    raise AcceptanceFailure(f"negative case {name} was unexpectedly accepted")


def run_negative_cases(source: Path, root: Path) -> None:
    universal = available_families(SUPPORTED_HARNESSES, alignment_catalog())
    one_harness = cross_data("invalid_one_harness", ["claude-code"], ["pre_tool"])
    invalid_family = cross_data(
        "invalid_family",
        list(SUPPORTED_HARNESSES),
        ["session_start"],
    )
    invalid_state = cross_data(
        "invalid_state",
        list(SUPPORTED_HARNESSES),
        universal,
        state=("claim_once",),
    )
    invalid_archetype = single_data(
        "invalid_archetype",
        "claude-code",
        ["pre_tool_use"],
        starter="policy_guard",
    )
    escaping = cross_data("invalid_escape", list(SUPPORTED_HARNESSES), universal)
    escaping["crate_path"] = "../outside"
    reserved_package = cross_data(
        "invalid_reserved_package",
        list(SUPPORTED_HARNESSES),
        universal,
    )
    reserved_package.update(package_name="type", binary_name="type")
    dependency_collision = cross_data(
        "invalid_dependency_collision",
        list(SUPPORTED_HARNESSES),
        universal,
    )
    dependency_collision.update(package_name="hookkit-core", binary_name="hookkit-core")
    short_sha = cross_data("invalid_sha", list(SUPPORTED_HARNESSES), universal)
    short_sha.update(dependency_source="git", hookkit_git_rev="abc123")
    strict_antigravity = with_quality(
        cross_data(
            "invalid_strict_antigravity",
            list(SUPPORTED_HARNESSES),
            ["post_tool"],
            starter="immediate_quality",
        )
    )
    strict_antigravity["lowering_policy"] = "strict"
    runner_state_without_seam = with_quality(
        cross_data(
            "invalid_runner_state_without_seam",
            list(SUPPORTED_HARNESSES),
            ["post_tool"],
            starter="immediate_quality",
            state=("session_metadata",),
        )
    )
    for name, data, question in (
        ("one_harness_cross", one_harness, "harnesses"),
        ("unavailable_aligned_family", invalid_family, "aligned_hooks"),
        ("state_without_metadata", invalid_state, "state_capabilities"),
        ("archetype_mode_mismatch", invalid_archetype, "harness_mode"),
        ("escaping_crate_path", escaping, "crate_path"),
        ("reserved_package_name", reserved_package, "package_name"),
        ("dependency_name_collision", dependency_collision, "package_name"),
        ("incomplete_git_sha", short_sha, "hookkit_git_rev"),
        ("strict_antigravity_output", strict_antigravity, "lowering_policy"),
        ("runner_state_without_custom_seam", runner_state_without_seam, "aligned_hooks"),
    ):
        assert_rejected(source, root, name, data, question)

    collision_root = root / "negative" / "duplicate-owned-path"
    first = cross_data(
        "duplicate_owner_alpha",
        list(SUPPORTED_HARNESSES),
        universal,
        output_mode="crate",
    )
    first["crate_path"] = "crates/shared-hook"
    second = cross_data(
        "duplicate_owner_beta",
        list(SUPPORTED_HARNESSES),
        universal,
        output_mode="crate",
    )
    second["crate_path"] = first["crate_path"]
    copy_case(source, collision_root, first)
    protected = collision_root / first["crate_path"] / "Cargo.toml"
    protected_bytes = protected.read_bytes()
    try:
        copier.run_copy(
            str(source),
            collision_root,
            data=second,
            defaults=True,
            quiet=True,
            cleanup_on_error=True,
        )
    except InteractiveSessionError:
        # Copier refuses to overwrite the first crate's existing files
        # without an explicit, interactive overwrite decision.
        if protected.read_bytes() != protected_bytes:
            raise AcceptanceFailure("duplicate owned-path rejection changed the first crate")
    except Exception as error:
        raise AcceptanceFailure(
            "duplicate owned crate path failed for an unexpected reason instead of "
            f"an overwrite conflict: {type(error).__name__}: {error}"
        ) from error
    else:
        raise AcceptanceFailure("duplicate owned crate path was unexpectedly accepted")
    print("PASS negative_validation", flush=True)


def git_source_data(name: str, revision: str) -> dict[str, Any]:
    """Deferred-quality answers that depend on HookKit at a public Git revision."""
    data = deferred_quality_data(name)
    data.pop("hookkit_path")
    data.update(dependency_source="git", hookkit_git_rev=revision)
    return data


def run_dependency_source_integrity(source: Path, root: Path) -> None:
    """Render the pinned Git and gated crates.io sources.

    Both are render-only here: a Git-mode project builds against the pinned
    revision, which lags any unmerged change to the APIs the template uses, so
    the ``pinned`` release gate compiles it instead.
    """
    started = time.monotonic()
    compatibility = load_yaml(CATALOG_ROOT / "compatibility.yml")["hookkit"]
    universal = available_families(SUPPORTED_HARNESSES, alignment_catalog())

    git_data = git_source_data("dependency_git", compatibility["git_revision"])
    git_case = Case("dependency_git", git_data)
    git_destination = root / "dependency-sources" / "git"
    copy_case(source, git_destination, git_data)
    assert_render(git_case, git_destination, source)

    crates_data = cross_data(
        "dependency_crates_io", list(SUPPORTED_HARNESSES), universal
    )
    crates_data.pop("hookkit_path")
    crates_data["dependency_source"] = "crates-io"
    crates_case = Case("dependency_crates_io", crates_data)
    crates_destination = root / "dependency-sources" / "crates-io"
    copy_case(source, crates_destination, crates_data)
    assert_render(crates_case, crates_destination, source)

    nonexistent_data = cross_data(
        "dependency_missing_path", list(SUPPORTED_HARNESSES), universal
    )
    nonexistent_data["hookkit_path"] = str(root / "does-not-exist" / "hookkit")
    nonexistent_destination = root / "dependency-sources" / "missing-path"
    try:
        copier.run_copy(
            str(source),
            nonexistent_destination,
            data=nonexistent_data,
            defaults=True,
            quiet=True,
            cleanup_on_error=True,
        )
    except Exception as error:
        if not is_question_rejection(error, "hookkit_path"):
            raise AcceptanceFailure(
                "nonexistent HookKit path failed for an unexpected reason instead of "
                f"rejecting 'hookkit_path': {type(error).__name__}: {error}"
            ) from error
        print("PASS dependency_nonexistent_path", flush=True)
    else:
        raise AcceptanceFailure(
            "nonexistent HookKit path was unexpectedly accepted"
        )
    print(f"PASS dependency_sources ({time.monotonic() - started:.1f}s)", flush=True)


def export_revision(revision: str, destination: Path) -> None:
    """Extract ``git archive <revision>`` of this repository into ``destination``."""
    archive = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "archive", "--format=tar", revision],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if archive.returncode:
        raise AcceptanceFailure(
            f"cannot export HookKit revision {revision}: "
            f"{archive.stderr.decode(errors='replace').strip()}"
        )
    destination.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
        tar.extractall(destination, filter="data")


def run_pinned_revision(source: Path, root: Path, revision: str | None) -> None:
    """Compile Git-mode renders against exactly the pinned HookKit revision.

    A Git-mode project builds against ``compatibility.hookkit.git_revision``,
    not this checkout, so every other lane can pass while the pin predates an
    API the template emits. This lane renders the default answers and one
    maximal single-mode project per harness with path dependencies on a
    ``git archive`` export of that revision and compiles every target, then
    compiles a Git-source project fetched from the public repository at the
    same revision. It is a release gate: while a change that adds
    template-facing APIs is unmerged, the pin cannot point at it yet, so it
    is not part of the default run.
    """
    started = time.monotonic()
    compatibility = load_yaml(CATALOG_ROOT / "compatibility.yml")["hookkit"]
    revision = revision or str(compatibility["git_revision"])
    export = root / "pinned-hookkit"
    export_revision(revision, export)
    events = event_catalog()
    renders = [("pinned_default", None)]
    for harness in SUPPORTED_HARNESSES:
        name = f"pinned_single_{harness.replace('-', '_')}_maximal"
        data = single_data(name, harness, events[harness])
        data["hookkit_path"] = str(export)
        renders.append((name, data))
    target = root / "cargo-target-pinned-revision"
    for name, data in renders:
        destination = root / "pinned" / name
        if data is None:
            data = copy_default_answers(source, destination, name, export).data
        else:
            copy_case(source, destination, data)
        try:
            cargo_check_generated(destination, data, target)
        except AcceptanceFailure as error:
            raise AcceptanceFailure(
                f"{name} does not compile against the pinned HookKit revision "
                f"{revision}; repin compatibility.hookkit.git_revision to a main "
                f"commit that has every API the template uses (see RELEASE.md)\n{error}"
            ) from error
        print(f"PASS {name} against {revision}", flush=True)

    git_data = git_source_data("pinned_public_git", revision)
    git_destination = root / "pinned" / "public-git"
    copy_case(source, git_destination, git_data)
    assert_render(Case("pinned_public_git", git_data), git_destination, source)
    cargo_check_generated(git_destination, git_data, root / "cargo-target-public-git-source")
    print(f"PASS pinned_public_git against {revision}", flush=True)
    print(f"PASS pinned_revision ({time.monotonic() - started:.1f}s)", flush=True)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--validation",
        choices=("render", "check", "full"),
        default="check",
        help="render only, cargo check each render, or run the complete Rust validation lane",
    )
    parser.add_argument(
        "--case",
        action="append",
        default=[],
        help=(
            "run one named render case (repeatable); special names: "
            "defaults, coexistence, update, negative, sources, and pinned "
            "(release gate; only runs when named)"
        ),
    )
    parser.add_argument(
        "--pinned-revision",
        help=(
            "HookKit revision for the pinned case instead of the compatibility "
            "catalog's git_revision, for example to try a candidate repin"
        ),
    )
    parser.add_argument("--list", action="store_true", help="list case names and exit")
    parser.add_argument(
        "--keep-workdir",
        action="store_true",
        help="retain generated fixtures for inspection",
    )
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    if copier.__version__ != COPIER_VERSION:
        raise AcceptanceFailure(
            f"Copier {COPIER_VERSION} is required; wrapper supplied {copier.__version__}"
        )
    cases = build_cases()
    special = ("defaults", "coexistence", "update", "negative", "sources", "pinned")
    if arguments.list:
        print("\n".join([case.name for case in cases] + list(special)))
        return 0
    requested = set(arguments.case)
    known = {case.name for case in cases} | set(special)
    unknown = requested - known
    if unknown:
        raise AcceptanceFailure(f"unknown case(s): {', '.join(sorted(unknown))}")
    selected = [case for case in cases if not requested or case.name in requested]
    run_special = lambda name: not requested or name in requested

    if arguments.keep_workdir:
        root = Path(tempfile.mkdtemp(prefix="hookkit-copier-tests-")).resolve()
        cleanup = None
        print(f"work directory: {root}", flush=True)
    else:
        cleanup = tempfile.TemporaryDirectory(prefix="hookkit-copier-tests-")
        root = Path(cleanup.name).resolve()
    try:
        source = stage_source(root)
        target = root / "cargo-target"
        for case in selected:
            run_render_case(case, source, root, arguments.validation, target)
        if run_special("defaults"):
            run_default_answers(source, root, arguments.validation, target)
        if run_special("coexistence"):
            run_coexistence(source, root, arguments.validation, target)
        if run_special("update"):
            run_recopy_ownership(source, root)
            run_shape_change_ownership(source, root)
            run_update_ownership(source, root)
            run_quality_config_ownership(source, root)
        if run_special("negative"):
            run_negative_cases(source, root)
        if run_special("sources"):
            run_dependency_source_integrity(source, root)
        if "pinned" in requested:
            run_pinned_revision(source, root, arguments.pinned_revision)
    finally:
        if cleanup is not None:
            cleanup.cleanup()
    print("all selected Copier acceptance tests passed", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AcceptanceFailure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
