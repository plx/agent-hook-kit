#[test]
fn every_registry_case_executes_against_the_native_event_spec() {
    let descriptors = hookkit_conformance::verified_descriptors().unwrap();
    assert_eq!(descriptors.len(), 56);
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.conformance_cases().len())
            .sum::<usize>(),
        92
    );
}
