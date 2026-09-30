#[test]
fn every_registry_case_executes_against_the_native_event_spec() {
    let descriptors = hookkit_conformance::verified_descriptors().unwrap();
    // 33 Claude Code, 12 Codex, and 5 Antigravity events.
    assert_eq!(descriptors.len(), 50);
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.conformance_cases().len())
            .sum::<usize>(),
        147
    );
}
