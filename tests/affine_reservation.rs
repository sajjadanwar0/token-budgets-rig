#[test]
fn reservation_is_affine() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/reservation_no_clone.rs");
    t.compile_fail("tests/compile_fail/reservation_no_reuse.rs");
}
