//! Compile-time non-bypassability: the cases in tests/compile_fail/ must NOT
//! compile. Generate the expected output once, then commit the .stderr files:
//!   TRYBUILD=overwrite cargo test --test affine_reservation
#[test]
fn reservation_is_affine() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/reservation_no_clone.rs");
    t.compile_fail("tests/compile_fail/reservation_no_reuse.rs");
}
