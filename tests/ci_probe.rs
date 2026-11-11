#[test]
fn ci_rejects_a_failing_test() {
    panic!("Intentional failure to verify the CI test step");
}
