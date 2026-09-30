use hookkit_core::{
    ContractId, EventId, HarnessId, HookkitError, IdentificationDescriptor, IdentificationStrength,
    RawInvocation, ResolutionProvenance, SnapshotId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Executable event resolution result.
pub struct ResolvedEvent {
    /// Exact event selected for decoding.
    pub event: EventId,
    /// Event/binding contract that emission must preserve.
    pub contract: ContractId,
    /// Immutable catalog snapshot associated with the event.
    pub snapshot: SnapshotId,
    /// Evidence by which the event was selected.
    pub provenance: ResolutionProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One possible event reported by best-effort inspection.
pub struct DetectedCandidate {
    /// Exact candidate event.
    pub event: EventId,
    /// Catalog-declared strength of its identification shape.
    pub strength: IdentificationStrength,
    /// Evidence observed in this invocation.
    pub evidence: DetectionEvidence,
    /// Known event-shape overlaps from the descriptor.
    pub overlaps: Vec<EventId>,
    /// Whether an exact native parser is available for validation/execution.
    pub native_parser: bool,
}

/// Evidence that caused a candidate to appear in a detection report.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DetectionEvidence {
    /// A documented string discriminator matched.
    Discriminator {
        /// JSON pointer used by the descriptor.
        pointer: String,
        /// Expected discriminator value that was observed.
        value: String,
    },
    /// The candidate's exact native parser accepted the payload.
    NativeParserAccepted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Optional harness/event filters for best-effort detection.
pub struct DetectionConstraints {
    /// Restrict reported candidates to this harness.
    pub harness: Option<HarnessId>,
    /// Restrict reported candidates to this exact event.
    pub event: Option<EventId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Non-executable result of best-effort event inspection.
pub struct DetectionReport {
    /// Candidates consistent with the requested constraints.
    pub candidates: Vec<DetectedCandidate>,
    /// Whether candidate evidence is insufficient for unique safe selection.
    pub ambiguous: bool,
    /// Constraint contradictions: either the supplied `harness` and `event`
    /// constraints are mutually inconsistent, or authoritative (discriminator)
    /// or sound-shape payload evidence identifies an event outside the supplied
    /// constraints.
    ///
    /// Evidence outside the constraints is only a contradiction when nothing
    /// inside them explains the payload equally well. Claude Code and Codex
    /// share the `/hook_event_name` discriminator, so a Claude payload also
    /// matches the Codex descriptor for the same event name; with a
    /// `claude-code` constraint that match is not reported here.
    pub constraint_mismatches: Vec<String>,
    /// Exact parser failures collected while inspecting descriptors.
    pub validation_failures: Vec<(EventId, String)>,
}

/// Reusable best-effort detector backed by identification descriptors.
///
/// Detection reports evidence but never authorize execution; use
/// [`resolve_event`] for checked runtime selection.
#[derive(Debug, Clone)]
pub struct Detector {
    descriptors: Vec<IdentificationDescriptor>,
}

impl Detector {
    /// Creates a detector containing every built-in harness descriptor.
    pub fn builtins() -> Self {
        Self {
            descriptors: builtin_descriptors(),
        }
    }

    /// Creates a detector from an application-supplied descriptor registry.
    pub fn new(descriptors: Vec<IdentificationDescriptor>) -> Self {
        Self { descriptors }
    }

    /// Best-effort inspection only. Reports are deliberately not executable.
    pub fn inspect(
        &self,
        invocation: &RawInvocation,
        constraints: DetectionConstraints,
    ) -> DetectionReport {
        detect_candidates(&self.descriptors, invocation, constraints)
    }
}

/// Collects identification descriptors from all built-in harness adapters.
pub fn builtin_descriptors() -> Vec<IdentificationDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(hookkit_claude::protocol::identification_descriptors());
    descriptors.extend(hookkit_codex::protocol::identification_descriptors());
    descriptors.extend(hookkit_antigravity::identification_descriptors());
    descriptors
}

/// Resolves an executable event using the complete built-in registry.
pub fn resolve_builtin_event(
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
) -> hookkit_core::Result<ResolvedEvent> {
    resolve_event(&builtin_descriptors(), harness, invocation, hint)
}

/// Selects and validates one executable native event.
///
/// A hint must belong to `harness`, name a registered event with a native
/// parser, agree with any authoritative discriminator, and pass that parser.
/// Without a hint, authoritative discriminators take priority, followed by one
/// unique sound-shape parser. Other parser matches are reported as ambiguous;
/// weak or catalog-only shape evidence never authorizes execution.
///
/// When a discriminator selects an event whose parser rejects the payload,
/// the error is [`HookkitError::InvalidInputForEvent`], which names the event
/// and keeps the parser's error as its source.
pub fn resolve_event(
    registry: &[IdentificationDescriptor],
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
) -> hookkit_core::Result<ResolvedEvent> {
    resolve_event_with(registry, harness, invocation, hint, ParserCheck::Validate)
}

/// Whether event selection runs the selected event's native parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserCheck {
    /// Validate every selected event with its parser (the public contract).
    Validate,
    /// Skip the parser when a hint or an authoritative discriminator already
    /// determines the event. The caller must decode the payload with that
    /// event's parser and, if decoding fails, classify the failure with
    /// [`resolve_event`]. Shape-based selection still validates, because the
    /// parsers are what establish the shape.
    DeferToDecode,
}

pub(crate) fn resolve_event_with(
    registry: &[IdentificationDescriptor],
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
    check: ParserCheck,
) -> hookkit_core::Result<ResolvedEvent> {
    if let Some(hint) = hint.as_ref().filter(|hint| hint.harness() != &harness) {
        return Err(HookkitError::HintHarnessMismatch {
            selected: harness,
            hint: hint.clone(),
        });
    }

    let descriptors: Vec<_> = registry
        .iter()
        .filter(|descriptor| descriptor.event().harness() == &harness)
        .collect();

    if let Some(hint) = hint {
        if let Some(actual) = authoritative_event(&descriptors, invocation) {
            if actual.event() != &hint {
                return Err(HookkitError::EventHintMismatch {
                    hint,
                    actual: actual.event().clone(),
                });
            }
        }
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event() == &hint)
            .ok_or_else(|| HookkitError::InvalidInputForHint {
                event: hint.clone(),
                message: "event is not registered for selected harness".into(),
            })?;

        if !descriptor.has_native_parser() {
            return Err(HookkitError::InvalidInputForHint {
                event: hint,
                message: "catalog event has no implemented native parser".into(),
            });
        }
        if check == ParserCheck::DeferToDecode {
            return Ok(resolved(descriptor, ResolutionProvenance::HintValidated));
        }

        if let Err(error) = descriptor.validate(invocation) {
            let established = unique_sound_candidate(&descriptors, invocation, Some(&hint));
            if let Some(actual) = established {
                return Err(HookkitError::EventHintMismatch {
                    hint,
                    actual: actual.event().clone(),
                });
            }
            return Err(HookkitError::InvalidInputForHint {
                event: hint,
                message: error.to_string(),
            });
        }
        return Ok(resolved(descriptor, ResolutionProvenance::HintValidated));
    }

    if let Some(descriptor) = authoritative_event(&descriptors, invocation) {
        if !descriptor.has_native_parser() {
            return Err(HookkitError::UnrecognizedEvent {
                harness,
                message: format!(
                    "payload identifies catalog event {}, but its native parser is not implemented",
                    descriptor.event()
                ),
            });
        }
        if check == ParserCheck::Validate {
            descriptor.validate(invocation).map_err(|source| {
                HookkitError::InvalidInputForEvent {
                    event: descriptor.event().clone(),
                    source: Box::new(source),
                }
            })?;
        }
        return Ok(resolved(
            descriptor,
            ResolutionProvenance::DefinitiveDiscriminator,
        ));
    }

    let sound: Vec<_> = descriptors
        .iter()
        .filter(|descriptor| descriptor.has_native_parser())
        .filter(|descriptor| descriptor.strength() == IdentificationStrength::SoundShape)
        .filter(|descriptor| descriptor.validate(invocation).is_ok())
        .collect();
    if let [descriptor] = sound.as_slice() {
        return Ok(resolved(descriptor, ResolutionProvenance::SoundShape));
    }

    let compatible: Vec<_> = descriptors
        .iter()
        .filter(|descriptor| descriptor.has_native_parser())
        .filter(|descriptor| descriptor.validate(invocation).is_ok())
        .map(|descriptor| descriptor.event().clone())
        .collect();
    match compatible.len() {
        0 => Err(HookkitError::UnrecognizedEvent {
            harness,
            message: "no implemented native event parser accepted the invocation".into(),
        }),
        _ => Err(HookkitError::AmbiguousEvent {
            harness,
            candidates: compatible,
        }),
    }
}

/// Inspects an invocation for possible events without authorizing execution.
///
/// Parser failures and constraint contradictions are retained in the report.
/// `ambiguous` is true for multiple candidates, weak/ambiguous candidates, or
/// a candidate with declared overlaps—even if only one candidate is listed.
/// Evidence outside the constraints is reported as a contradiction only when
/// no candidate inside them is backed by the same discriminator (or, for
/// sound-shape evidence, when the constraints produced no candidate at all).
pub fn detect_candidates(
    registry: &[IdentificationDescriptor],
    invocation: &RawInvocation,
    constraints: DetectionConstraints,
) -> DetectionReport {
    let mut report = DetectionReport::default();
    if let (Some(harness), Some(event)) = (&constraints.harness, &constraints.event) {
        if event.harness() != harness {
            report.constraint_mismatches.push(format!(
                "event constraint {event} does not belong to harness {harness}"
            ));
        }
    }

    // Pass 1: collect evidence and split it by the constraints.
    let mut outside = Vec::new();
    for descriptor in registry {
        let discriminator_evidence = descriptor.discriminator().and_then(|(pointer, expected)| {
            (pointer_value(invocation.json(), pointer) == Some(expected)).then(|| {
                DetectionEvidence::Discriminator {
                    pointer: pointer.to_string(),
                    value: expected.to_string(),
                }
            })
        });
        let validation = descriptor
            .has_native_parser()
            .then(|| descriptor.validate(invocation));
        let evidence = discriminator_evidence.or_else(|| {
            validation
                .as_ref()
                .is_some_and(|result| result.is_ok())
                .then_some(DetectionEvidence::NativeParserAccepted)
        });

        if let Some(Err(error)) = &validation {
            report
                .validation_failures
                .push((descriptor.event().clone(), error.to_string()));
        }

        let Some(evidence) = evidence else {
            continue;
        };

        let outside_harness = constraints
            .harness
            .as_ref()
            .filter(|harness| descriptor.event().harness() != *harness);
        let outside_event = constraints
            .event
            .as_ref()
            .filter(|event| descriptor.event() != *event);
        if let Some(harness) = outside_harness {
            outside.push((
                descriptor,
                evidence,
                format!("harness constraint {harness}"),
            ));
            continue;
        }
        if let Some(event) = outside_event {
            outside.push((descriptor, evidence, format!("event constraint {event}")));
            continue;
        }
        report.candidates.push(DetectedCandidate {
            event: descriptor.event().clone(),
            strength: descriptor.strength(),
            evidence,
            overlaps: descriptor.overlap_events(),
            native_parser: descriptor.has_native_parser(),
        });
    }

    // Pass 2: out-of-constraint evidence contradicts the constraints only if
    // nothing inside them is explained by the same evidence.
    for (descriptor, evidence, constraint) in outside {
        let contradicts = match &evidence {
            DetectionEvidence::Discriminator { .. } => !report
                .candidates
                .iter()
                .any(|candidate| candidate.evidence == evidence),
            DetectionEvidence::NativeParserAccepted => {
                descriptor.strength() == IdentificationStrength::SoundShape
                    && report.candidates.is_empty()
            }
        };
        if contradicts {
            report.constraint_mismatches.push(format!(
                "payload evidence identifies {}, outside {constraint}",
                descriptor.event()
            ));
        }
    }

    report.ambiguous = report.candidates.len() > 1
        || report.candidates.iter().any(|candidate| {
            matches!(
                candidate.strength,
                IdentificationStrength::WeakShape | IdentificationStrength::Ambiguous
            ) || !candidate.overlaps.is_empty()
        });
    report
}

/// Returns the event named by an authoritative discriminator in `invocation`,
/// without running any parser. Used to classify failures after execution
/// could not produce a response.
///
/// When no registered descriptor matches but every discriminated descriptor
/// of `harness` reads the same JSON pointer (Claude Code's and Codex's
/// `/hook_event_name`), the string at that pointer still names the event, so
/// an event the adapter does not model yet is identified by its wire name.
pub(crate) fn discriminated_event(
    registry: &[IdentificationDescriptor],
    harness: &HarnessId,
    invocation: &RawInvocation,
) -> Option<EventId> {
    let descriptors: Vec<_> = registry
        .iter()
        .filter(|descriptor| descriptor.event().harness() == harness)
        .collect();
    if let Some(descriptor) = authoritative_event(&descriptors, invocation) {
        return Some(descriptor.event().clone());
    }
    let mut pointers = descriptors
        .iter()
        .filter_map(|descriptor| descriptor.discriminator().map(|(pointer, _)| pointer));
    let pointer = pointers.next()?;
    if pointers.any(|other| other != pointer) {
        return None;
    }
    let name = pointer_value(invocation.json(), pointer)?;
    EventId::new(harness.clone(), name).ok()
}

fn unique_sound_candidate<'a>(
    descriptors: &[&'a IdentificationDescriptor],
    invocation: &RawInvocation,
    excluded: Option<&EventId>,
) -> Option<&'a IdentificationDescriptor> {
    let matches: Vec<_> = descriptors
        .iter()
        .copied()
        .filter(|descriptor| Some(descriptor.event()) != excluded)
        .filter(|descriptor| descriptor.has_native_parser())
        .filter(|descriptor| {
            descriptor.strength() == IdentificationStrength::SoundShape
                && descriptor.validate(invocation).is_ok()
        })
        .collect();
    match matches.as_slice() {
        [descriptor] => Some(*descriptor),
        _ => None,
    }
}

fn resolved(
    descriptor: &IdentificationDescriptor,
    provenance: ResolutionProvenance,
) -> ResolvedEvent {
    ResolvedEvent {
        event: descriptor.event().clone(),
        contract: descriptor.contract(),
        snapshot: descriptor.snapshot(),
        provenance,
    }
}

fn authoritative_event<'a>(
    descriptors: &[&'a IdentificationDescriptor],
    invocation: &RawInvocation,
) -> Option<&'a IdentificationDescriptor> {
    descriptors.iter().copied().find(|descriptor| {
        descriptor
            .discriminator()
            .is_some_and(|(pointer, expected)| {
                pointer_value(invocation.json(), pointer) == Some(expected)
            })
    })
}

fn pointer_value<'a>(value: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{
        ContractId, EventCategory, EventSpec, NativeContext, NoCommandEnvironment, ProcessEmission,
        SnapshotId,
    };
    use proptest::prelude::*;

    macro_rules! test_event {
        ($type:ident, $harness:literal, $event:literal, $required:literal) => {
            struct $type;
            impl EventSpec for $type {
                type Input = serde_json::Value;
                type CommandEnvironment = NoCommandEnvironment;
                type CommandOutput = ();
                const HARNESS: HarnessId = HarnessId::builtin($harness);
                const SNAPSHOT: SnapshotId = SnapshotId::builtin("v1");
                const EVENT: EventId = EventId::builtin(Self::HARNESS, $event);
                const CATEGORY: EventCategory = EventCategory::Other;
                const CONTRACT: ContractId = ContractId::builtin(concat!($harness, "/v1/", $event));
                fn parse(raw: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                    raw.json()
                        .get($required)
                        .is_some()
                        .then(|| raw.json().clone())
                        .ok_or(HookkitError::MissingHookEventName)
                }
                fn emit(_: ()) -> hookkit_core::Result<ProcessEmission> {
                    Ok(ProcessEmission::command_empty(Self::CONTRACT))
                }
                fn context(_: &Self::Input) -> NativeContext {
                    NativeContext::default()
                }
            }
        };
    }
    test_event!(Alpha, "h", "Alpha", "alpha");
    test_event!(Beta, "h", "Beta", "shared");
    test_event!(Gamma, "h", "Gamma", "shared");
    test_event!(Sound, "sound", "Only", "sound");
    test_event!(Weak, "weak", "Only", "weak");

    fn registry() -> Vec<IdentificationDescriptor> {
        vec![
            IdentificationDescriptor::definitive::<Alpha>("/kind", "Alpha"),
            IdentificationDescriptor::ambiguous::<Beta>(&["Gamma"]),
            IdentificationDescriptor::ambiguous::<Gamma>(&["Beta"]),
            IdentificationDescriptor::sound_shape::<Sound>(&[]),
            IdentificationDescriptor::weak_shape::<Weak>(&[]),
        ]
    }

    proptest! {
        /// Property: unrelated payload fields cannot perturb a definitive
        /// discriminator. Resolution depends on the declared JSON pointer and
        /// still validates the selected native parser before succeeding.
        #[test]
        fn definitive_resolution_ignores_unrelated_fields(
            extras in prop::collection::btree_map("x_[a-z]{1,8}", any::<i64>(), 0..20),
            alpha in any::<i64>(),
        ) {
            let mut object = serde_json::Map::new();
            object.insert("kind".into(), serde_json::Value::String("Alpha".into()));
            object.insert("alpha".into(), serde_json::Value::Number(alpha.into()));
            object.extend(extras.into_iter().map(|(key, value)| (key, value.into())));
            let raw = RawInvocation::parse(serde_json::to_vec(&object).unwrap()).unwrap();

            let resolved = resolve_event(&registry(), HarnessId::builtin("h"), &raw, None).unwrap();
            prop_assert_eq!(resolved.event, Alpha::EVENT);
            prop_assert_eq!(resolved.provenance, ResolutionProvenance::DefinitiveDiscriminator);
        }

        /// Property: RFC 6901 escaping is honored for discriminator pointers.
        /// Harnesses may identify events beneath object keys containing `/` or
        /// `~`, and those keys must not be confused with pointer syntax.
        #[test]
        fn discriminator_pointer_uses_json_pointer_escaping(
            key in "[a-z]{1,6}[/~][a-z]{0,6}",
            value in any::<String>(),
        ) {
            let json = serde_json::json!({key.clone(): value.clone()});
            let pointer = format!("/{}", key.replace('~', "~0").replace('/', "~1"));

            prop_assert_eq!(pointer_value(&json, &pointer), Some(value.as_str()));
        }
    }

    #[test]
    fn cross_harness_hint_fails_before_payload_analysis() {
        let invalid = RawInvocation::parse(br#"{}"#.to_vec()).unwrap();
        let error = resolve_event(
            &registry(),
            HarnessId::builtin("h"),
            &invalid,
            Some(EventId::builtin(HarnessId::builtin("other"), "Alpha")),
        )
        .unwrap_err();
        assert!(matches!(error, HookkitError::HintHarnessMismatch { .. }));
    }

    #[test]
    fn definitive_discriminator_and_matching_hint_resolve() {
        let raw = RawInvocation::parse(br#"{"kind":"Alpha","alpha":1}"#.to_vec()).unwrap();
        let resolved = resolve_event(&registry(), HarnessId::builtin("h"), &raw, None).unwrap();
        assert_eq!(
            resolved.provenance,
            ResolutionProvenance::DefinitiveDiscriminator
        );
        let hinted = resolve_event(
            &registry(),
            HarnessId::builtin("h"),
            &raw,
            Some(EventId::builtin(HarnessId::builtin("h"), "Alpha")),
        )
        .unwrap();
        assert_eq!(hinted.provenance, ResolutionProvenance::HintValidated);
    }

    #[test]
    fn contradiction_invalid_hint_and_ambiguity_are_distinct() {
        let raw = RawInvocation::parse(br#"{"kind":"Alpha","alpha":1}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(
                &registry(),
                HarnessId::builtin("h"),
                &raw,
                Some(EventId::builtin(HarnessId::builtin("h"), "Beta"))
            ),
            Err(HookkitError::EventHintMismatch { .. })
        ));
        let shared = RawInvocation::parse(br#"{"shared":true}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(&registry(), HarnessId::builtin("h"), &shared, None),
            Err(HookkitError::AmbiguousEvent { .. })
        ));
    }

    #[test]
    fn catalog_declared_sound_shape_resolves_without_hint() {
        let raw = RawInvocation::parse(br#"{"sound":true}"#.to_vec()).unwrap();
        let resolved = resolve_event(&registry(), HarnessId::builtin("sound"), &raw, None).unwrap();
        assert_eq!(resolved.provenance, ResolutionProvenance::SoundShape);
    }

    #[test]
    fn unique_weak_shape_still_requires_a_hint() {
        let raw = RawInvocation::parse(br#"{"weak":true}"#.to_vec()).unwrap();
        let error = resolve_event(&registry(), HarnessId::builtin("weak"), &raw, None).unwrap_err();
        assert!(matches!(
            error,
            HookkitError::AmbiguousEvent { candidates, .. } if candidates.len() == 1
        ));
        let resolved = resolve_event(
            &registry(),
            HarnessId::builtin("weak"),
            &raw,
            Some(EventId::builtin(HarnessId::builtin("weak"), "Only")),
        )
        .unwrap();
        assert_eq!(resolved.provenance, ResolutionProvenance::HintValidated);
    }

    #[test]
    fn detector_reports_constraint_mismatch_without_executing() {
        let raw = RawInvocation::parse(br#"{"kind":"Alpha","alpha":1}"#.to_vec()).unwrap();
        let report = detect_candidates(
            &registry(),
            &raw,
            DetectionConstraints {
                harness: Some(HarnessId::builtin("sound")),
                event: None,
            },
        );
        assert!(report.candidates.is_empty());
        assert!(!report.constraint_mismatches.is_empty());

        let sound = RawInvocation::parse(br#"{"sound":true}"#.to_vec()).unwrap();
        let sound_report = detect_candidates(
            &registry(),
            &sound,
            DetectionConstraints {
                harness: Some(HarnessId::builtin("h")),
                event: None,
            },
        );
        assert!(sound_report.candidates.is_empty());
        assert!(!sound_report.constraint_mismatches.is_empty());
    }

    #[test]
    fn builtin_detector_covers_every_selected_event_with_a_native_parser() {
        let descriptors = builtin_descriptors();
        assert_eq!(descriptors.len(), 47);
        assert_eq!(
            descriptors
                .iter()
                .filter(|descriptor| descriptor.has_native_parser())
                .count(),
            47
        );

        let raw = RawInvocation::parse(
            br#"{"session_id":"s","hook_event_name":"SessionStart"}"#.to_vec(),
        )
        .unwrap();
        let report = Detector::builtins().inspect(
            &raw,
            DetectionConstraints {
                harness: Some(HarnessId::CODEX),
                event: None,
            },
        );
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(
            report.candidates[0].event,
            EventId::builtin(HarnessId::CODEX, "SessionStart")
        );
        assert!(report.candidates[0].native_parser);
        assert!(
            report
                .validation_failures
                .iter()
                .any(|(event, _)| event == &EventId::builtin(HarnessId::CODEX, "SessionStart"))
        );
    }

    #[test]
    fn authoritative_event_discriminator_contradicts_a_different_hint() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","hook_event_name":"SessionStart"}"#.to_vec(),
        )
        .unwrap();
        let error = resolve_builtin_event(
            HarnessId::CODEX,
            &raw,
            Some(EventId::builtin(HarnessId::CODEX, "PreToolUse")),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            HookkitError::EventHintMismatch { hint, actual }
                if hint == EventId::builtin(HarnessId::CODEX, "PreToolUse")
                    && actual == EventId::builtin(HarnessId::CODEX, "SessionStart")
        ));
    }

    #[test]
    fn shared_discriminators_do_not_report_a_matching_harness_constraint_as_a_mismatch() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"startup"}"#.to_vec(),
        )
        .unwrap();
        for harness in [HarnessId::CLAUDE_CODE, HarnessId::CODEX] {
            let report = Detector::builtins().inspect(
                &raw,
                DetectionConstraints {
                    harness: Some(harness.clone()),
                    event: None,
                },
            );
            assert_eq!(report.candidates.len(), 1, "{harness}");
            assert_eq!(
                report.candidates[0].event,
                EventId::builtin(harness.clone(), "SessionStart")
            );
            assert!(
                report.constraint_mismatches.is_empty(),
                "{harness}: {:?}",
                report.constraint_mismatches
            );
        }

        let event_constrained = Detector::builtins().inspect(
            &raw,
            DetectionConstraints {
                harness: None,
                event: Some(EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart")),
            },
        );
        assert!(event_constrained.constraint_mismatches.is_empty());
    }

    #[test]
    fn discriminator_for_a_different_event_is_still_a_constraint_mismatch() {
        let raw = RawInvocation::parse(br#"{"hook_event_name":"Stop"}"#.to_vec()).unwrap();
        let report = Detector::builtins().inspect(
            &raw,
            DetectionConstraints {
                harness: Some(HarnessId::CLAUDE_CODE),
                event: Some(EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart")),
            },
        );
        assert!(report.candidates.is_empty());
        assert!(
            report
                .constraint_mismatches
                .iter()
                .any(|mismatch| mismatch.contains("claude-code/Stop"))
        );
    }

    #[test]
    fn definitive_parser_rejection_names_the_event() {
        let raw = RawInvocation::parse(br#"{"kind":"Alpha"}"#.to_vec()).unwrap();
        let error = resolve_event(&registry(), HarnessId::builtin("h"), &raw, None).unwrap_err();
        assert!(matches!(
            &error,
            HookkitError::InvalidInputForEvent { event, .. } if event == &Alpha::EVENT
        ));
        assert!(error.to_string().contains("h/Alpha"), "{error}");
    }

    #[test]
    fn deferred_parser_checks_select_hinted_and_discriminated_events_without_parsing() {
        let raw = RawInvocation::parse(br#"{"kind":"Alpha"}"#.to_vec()).unwrap();
        let resolved = resolve_event_with(
            &registry(),
            HarnessId::builtin("h"),
            &raw,
            None,
            ParserCheck::DeferToDecode,
        )
        .unwrap();
        assert_eq!(resolved.event, Alpha::EVENT);
        let hinted = resolve_event_with(
            &registry(),
            HarnessId::builtin("h"),
            &RawInvocation::parse(br#"{}"#.to_vec()).unwrap(),
            Some(Beta::EVENT),
            ParserCheck::DeferToDecode,
        )
        .unwrap();
        assert_eq!(hinted.event, Beta::EVENT);
        assert_eq!(
            discriminated_event(&registry(), &HarnessId::builtin("h"), &raw),
            Some(Alpha::EVENT)
        );
    }

    #[test]
    fn unregistered_events_are_identified_by_the_shared_discriminator_pointer() {
        let raw =
            RawInvocation::parse(br#"{"hook_event_name":"PreFutureThing"}"#.to_vec()).unwrap();
        let descriptors = hookkit_claude::protocol::identification_descriptors();
        assert_eq!(
            discriminated_event(&descriptors, &HarnessId::CLAUDE_CODE, &raw),
            Some(EventId::new(HarnessId::CLAUDE_CODE, "PreFutureThing").unwrap())
        );
        let blank = RawInvocation::parse(br#"{"hook_event_name":""}"#.to_vec()).unwrap();
        assert_eq!(
            discriminated_event(&descriptors, &HarnessId::CLAUDE_CODE, &blank),
            None
        );
        let missing = RawInvocation::parse(br#"{"session_id":"s"}"#.to_vec()).unwrap();
        assert_eq!(
            discriminated_event(&descriptors, &HarnessId::CLAUDE_CODE, &missing),
            None
        );
        // Antigravity has no discriminator, so nothing names the event.
        let antigravity = hookkit_antigravity::identification_descriptors();
        assert_eq!(
            discriminated_event(&antigravity, &HarnessId::ANTIGRAVITY, &raw),
            None
        );
    }

    #[test]
    fn detector_retains_parser_failure_alongside_discriminator_evidence() {
        let raw = RawInvocation::parse(br#"{"hook_event_name":"PreToolUse"}"#.to_vec()).unwrap();
        let report = Detector::builtins().inspect(
            &raw,
            DetectionConstraints {
                harness: Some(HarnessId::CODEX),
                event: None,
            },
        );
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(
            report.candidates[0].event,
            EventId::builtin(HarnessId::CODEX, "PreToolUse")
        );
        assert!(
            report
                .validation_failures
                .iter()
                .any(|(event, _)| event == &EventId::builtin(HarnessId::CODEX, "PreToolUse"))
        );
    }
}
