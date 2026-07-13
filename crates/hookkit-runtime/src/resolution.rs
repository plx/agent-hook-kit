use hookkit_core::{
    ContractId, EventId, HarnessId, HookkitError, IdentificationDescriptor, IdentificationStrength,
    RawInvocation, ResolutionProvenance, SnapshotId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEvent {
    pub event: EventId,
    pub contract: ContractId,
    pub snapshot: SnapshotId,
    pub provenance: ResolutionProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedCandidate {
    pub event: EventId,
    pub strength: IdentificationStrength,
    pub evidence: DetectionEvidence,
    pub overlaps: Vec<EventId>,
    pub native_parser: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionEvidence {
    Discriminator { pointer: String, value: String },
    NativeParserAccepted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectionConstraints {
    pub harness: Option<HarnessId>,
    pub event: Option<EventId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectionReport {
    pub candidates: Vec<DetectedCandidate>,
    pub ambiguous: bool,
    pub constraint_mismatches: Vec<String>,
    pub validation_failures: Vec<(EventId, String)>,
}

pub struct Detector {
    descriptors: Vec<IdentificationDescriptor>,
}

impl Detector {
    pub fn builtins() -> Self {
        Self {
            descriptors: builtin_descriptors(),
        }
    }

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

pub fn builtin_descriptors() -> Vec<IdentificationDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(hookkit_claude::protocol::identification_descriptors());
    descriptors.extend(hookkit_codex::protocol::identification_descriptors());
    descriptors.extend(hookkit_gemini::protocol::identification_descriptors());
    descriptors.extend(hookkit_antigravity::identification_descriptors());
    descriptors
}

pub fn resolve_builtin_event(
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
) -> hookkit_core::Result<ResolvedEvent> {
    resolve_event(&builtin_descriptors(), harness, invocation, hint)
}

pub fn resolve_event(
    registry: &[IdentificationDescriptor],
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
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
        descriptor.validate(invocation)?;
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

        if constraints
            .harness
            .as_ref()
            .is_some_and(|harness| descriptor.event().harness() != harness)
        {
            if matches!(evidence, DetectionEvidence::Discriminator { .. })
                || descriptor.strength() == IdentificationStrength::SoundShape
            {
                report.constraint_mismatches.push(format!(
                    "payload evidence identifies {}, outside harness constraint {}",
                    descriptor.event(),
                    constraints.harness.as_ref().expect("checked")
                ));
            }
            continue;
        }
        if constraints
            .event
            .as_ref()
            .is_some_and(|event| descriptor.event() != event)
        {
            if matches!(evidence, DetectionEvidence::Discriminator { .. })
                || descriptor.strength() == IdentificationStrength::SoundShape
            {
                report.constraint_mismatches.push(format!(
                    "payload evidence identifies {}, outside event constraint {}",
                    descriptor.event(),
                    constraints.event.as_ref().expect("checked")
                ));
            }
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
    report.ambiguous = report.candidates.len() > 1
        || report.candidates.iter().any(|candidate| {
            matches!(
                candidate.strength,
                IdentificationStrength::WeakShape | IdentificationStrength::Ambiguous
            ) || !candidate.overlaps.is_empty()
        });
    report
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
        ContractId, EventCategory, EventSpec, NativeContext, ProcessEmission, SnapshotId,
    };

    macro_rules! test_event {
        ($type:ident, $harness:literal, $event:literal, $required:literal) => {
            struct $type;
            impl EventSpec for $type {
                type Input = serde_json::Value;
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
    fn builtin_detector_covers_catalog_only_events_without_authorizing_them() {
        let descriptors = builtin_descriptors();
        assert_eq!(descriptors.len(), 56);
        assert_eq!(
            descriptors
                .iter()
                .filter(|descriptor| descriptor.has_native_parser())
                .count(),
            13
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
        assert!(!report.candidates[0].native_parser);
    }

    #[test]
    fn catalog_only_authoritative_event_contradicts_an_implemented_hint() {
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
