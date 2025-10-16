// Trybuild tests for compile-time validation
// These tests ensure that proc macros generate the expected code
// and that compile-time errors are caught appropriately

#[test]
fn ui_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass/*.rs");
    // Uncomment the following line when you have failing test cases
    // t.compile_fail("tests/ui/fail/*.rs");
}
