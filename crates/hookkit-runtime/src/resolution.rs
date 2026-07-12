use hookkit_core::{EventId, HarnessId, HookkitError, RawInvocation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentificationStrength {
    Definitive,
    SoundShape,
    WeakShape,
    Ambiguous,
    Impossible,
}

#[derive(Clone, Copy)]
pub struct EventDescriptor {
    pub harness: &'static str,
    pub event: &'static str,
    pub contract_id: &'static str,
    pub strength: IdentificationStrength,
    pub discriminator: Option<(&'static str, &'static str)>,
    pub validate: fn(&RawInvocation) -> hookkit_core::Result<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionProvenance {
    DefinitiveDiscriminator,
    SoundShape,
    HintValidated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEvent {
    pub harness: HarnessId,
    pub event: EventId,
    pub contract_id: &'static str,
    pub provenance: ResolutionProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedCandidate {
    pub harness: HarnessId,
    pub event: EventId,
    pub strength: IdentificationStrength,
}

pub fn resolve_event(
    registry: &[EventDescriptor],
    harness: HarnessId,
    invocation: &RawInvocation,
    hint: Option<EventId>,
) -> hookkit_core::Result<ResolvedEvent> {
    let descriptors: Vec<_> = registry
        .iter()
        .filter(|descriptor| descriptor.harness == harness.as_str())
        .collect();
    if let Some(hint) = hint {
        if let Some(actual) = authoritative_event(&descriptors, invocation)
            && actual != hint.as_str()
        {
            return Err(HookkitError::HintContradiction {
                harness,
                hint,
                actual: EventId::new(actual)?,
            });
        }
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event == hint.as_str())
            .ok_or_else(|| HookkitError::InvalidForHint {
                harness: harness.clone(),
                event: hint.clone(),
                message: "event is not registered".into(),
            })?;
        (descriptor.validate)(invocation).map_err(|error| HookkitError::InvalidForHint {
            harness: harness.clone(),
            event: hint.clone(),
            message: error.to_string(),
        })?;
        return Ok(resolved(
            descriptor,
            harness,
            ResolutionProvenance::HintValidated,
        ));
    }

    if let Some(actual) = authoritative_event(&descriptors, invocation) {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event == actual)
            .expect("authoritative event came from descriptors");
        (descriptor.validate)(invocation)?;
        return Ok(resolved(
            descriptor,
            harness,
            ResolutionProvenance::DefinitiveDiscriminator,
        ));
    }

    let sound: Vec<_> = descriptors
        .iter()
        .filter(|descriptor| descriptor.strength == IdentificationStrength::SoundShape)
        .filter(|descriptor| (descriptor.validate)(invocation).is_ok())
        .collect();
    if let [descriptor] = sound.as_slice() {
        return Ok(resolved(
            descriptor,
            harness,
            ResolutionProvenance::SoundShape,
        ));
    }

    let compatible: Vec<_> = descriptors
        .iter()
        .filter(|descriptor| (descriptor.validate)(invocation).is_ok())
        .map(|descriptor| EventId::builtin(descriptor.event))
        .collect();
    match compatible.len() {
        0 => Err(HookkitError::NoEventCandidate { harness }),
        _ => Err(HookkitError::AmbiguousEvent {
            harness,
            candidates: compatible,
        }),
    }
}

/// Best-effort inspection only. This result is deliberately not executable.
pub fn detect_candidates(
    registry: &[EventDescriptor],
    invocation: &RawInvocation,
) -> Vec<DetectedCandidate> {
    registry
        .iter()
        .filter(|descriptor| {
            descriptor.discriminator.is_some_and(|(pointer, expected)| {
                pointer_value(invocation.json(), pointer) == Some(expected)
            }) || (descriptor.validate)(invocation).is_ok()
        })
        .map(|descriptor| DetectedCandidate {
            harness: HarnessId::builtin(descriptor.harness),
            event: EventId::builtin(descriptor.event),
            strength: descriptor.strength,
        })
        .collect()
}

fn resolved(
    descriptor: &EventDescriptor,
    harness: HarnessId,
    provenance: ResolutionProvenance,
) -> ResolvedEvent {
    ResolvedEvent {
        harness,
        event: EventId::builtin(descriptor.event),
        contract_id: descriptor.contract_id,
        provenance,
    }
}

fn authoritative_event<'a>(
    descriptors: &[&'a EventDescriptor],
    invocation: &RawInvocation,
) -> Option<&'a str> {
    descriptors.iter().find_map(|descriptor| {
        let (pointer, expected) = descriptor.discriminator?;
        (pointer_value(invocation.json(), pointer) == Some(expected)).then_some(descriptor.event)
    })
}

fn pointer_value<'a>(value: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::EventSpec;

    fn require(value: &'static str) -> fn(&RawInvocation) -> hookkit_core::Result<()> {
        match value {
            "alpha" => |raw| {
                raw.json()
                    .get("alpha")
                    .is_some()
                    .then_some(())
                    .ok_or(HookkitError::MissingHookEventName)
            },
            _ => |raw| {
                raw.json()
                    .get("shared")
                    .is_some()
                    .then_some(())
                    .ok_or(HookkitError::MissingHookEventName)
            },
        }
    }

    fn registry() -> Vec<EventDescriptor> {
        vec![
            EventDescriptor {
                harness: "h",
                event: "Alpha",
                contract_id: "h/v/Alpha",
                strength: IdentificationStrength::Definitive,
                discriminator: Some(("/kind", "Alpha")),
                validate: require("alpha"),
            },
            EventDescriptor {
                harness: "h",
                event: "Beta",
                contract_id: "h/v/Beta",
                strength: IdentificationStrength::Ambiguous,
                discriminator: None,
                validate: require("shared"),
            },
            EventDescriptor {
                harness: "h",
                event: "Gamma",
                contract_id: "h/v/Gamma",
                strength: IdentificationStrength::Ambiguous,
                discriminator: None,
                validate: require("shared"),
            },
            EventDescriptor {
                harness: "other",
                event: "Beta",
                contract_id: "other/v/Beta",
                strength: IdentificationStrength::WeakShape,
                discriminator: None,
                validate: require("shared"),
            },
            EventDescriptor {
                harness: "sound",
                event: "Only",
                contract_id: "sound/v/Only",
                strength: IdentificationStrength::SoundShape,
                discriminator: None,
                validate: require("shared"),
            },
            EventDescriptor {
                harness: "weak",
                event: "Only",
                contract_id: "weak/v/Only",
                strength: IdentificationStrength::WeakShape,
                discriminator: None,
                validate: require("shared"),
            },
        ]
    }

    fn antigravity_invocation(raw: &RawInvocation) -> hookkit_core::Result<()> {
        hookkit_antigravity::PreInvocation::parse(raw).map(|_| ())
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
            Some(EventId::builtin("Alpha")),
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
                Some(EventId::builtin("Beta"))
            ),
            Err(HookkitError::HintContradiction { .. })
        ));
        let shared = RawInvocation::parse(br#"{"shared":true}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(&registry(), HarnessId::builtin("h"), &shared, None),
            Err(HookkitError::AmbiguousEvent { .. })
        ));
        let invalid = RawInvocation::parse(br#"{}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(
                &registry(),
                HarnessId::builtin("h"),
                &invalid,
                Some(EventId::builtin("Beta"))
            ),
            Err(HookkitError::InvalidForHint { .. })
        ));
    }

    #[test]
    fn detector_can_report_multiple_harnesses_but_cannot_execute() {
        let raw = RawInvocation::parse(br#"{"shared":true}"#.to_vec()).unwrap();
        let candidates = detect_candidates(&registry(), &raw);
        assert_eq!(candidates.len(), 5);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.harness.as_str() == "other")
        );
    }

    #[test]
    fn only_catalog_declared_sound_shape_resolves_without_a_hint() {
        let shared = RawInvocation::parse(br#"{"shared":true}"#.to_vec()).unwrap();
        let resolved =
            resolve_event(&registry(), HarnessId::builtin("sound"), &shared, None).unwrap();
        assert_eq!(resolved.provenance, ResolutionProvenance::SoundShape);
        assert!(matches!(
            resolve_event(&registry(), HarnessId::builtin("weak"), &shared, None),
            Err(HookkitError::AmbiguousEvent { .. })
        ));
        let empty = RawInvocation::parse(br#"{}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(&registry(), HarnessId::builtin("sound"), &empty, None),
            Err(HookkitError::NoEventCandidate { .. })
        ));
    }

    #[test]
    fn antigravity_identical_shapes_require_and_accept_a_hint() {
        let descriptors = [
            EventDescriptor {
                harness: "antigravity",
                event: "PreInvocation",
                contract_id: "antigravity/docs-2026-07-12-r1/PreInvocation",
                strength: IdentificationStrength::Impossible,
                discriminator: None,
                validate: antigravity_invocation,
            },
            EventDescriptor {
                harness: "antigravity",
                event: "PostInvocation",
                contract_id: "antigravity/docs-2026-07-12-r1/PostInvocation",
                strength: IdentificationStrength::Impossible,
                discriminator: None,
                validate: antigravity_invocation,
            },
        ];
        let raw = RawInvocation::parse(br#"{"conversationId":"c","workspacePaths":["/repo"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0}"#.to_vec()).unwrap();
        assert!(matches!(
            resolve_event(&descriptors, HarnessId::ANTIGRAVITY, &raw, None),
            Err(HookkitError::AmbiguousEvent { .. })
        ));
        let resolved = resolve_event(
            &descriptors,
            HarnessId::ANTIGRAVITY,
            &raw,
            Some(EventId::builtin("PreInvocation")),
        )
        .unwrap();
        assert_eq!(resolved.provenance, ResolutionProvenance::HintValidated);
    }
}
