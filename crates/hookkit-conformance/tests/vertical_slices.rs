#[test]
fn every_registry_case_executes_against_the_native_event_spec() {
    let descriptors = hookkit_conformance::verified_descriptors().unwrap();
    // 34 Claude Code, 14 Codex, and 6 Antigravity events, with 138, 46, and 9
    // conformance cases.
    assert_eq!(descriptors.len(), 54);
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.conformance_cases().len())
            .sum::<usize>(),
        193
    );
}
