use super::*;

#[test]
fn tridia_matches_the_current_fortran_elimination_order() {
    let solution = solve_tridiagonal(
        &[0.0, 1.0, 2.0],
        &[4.0, 5.0, 6.0],
        &[1.0, 1.0, 0.0],
        &[6.0, 14.0, 22.0],
    )
    .unwrap();
    for (actual, expected) in solution.into_iter().zip([1.0, 2.0, 3.0]) {
        assert!(
            (actual - expected).abs() < 1.0e-14,
            "{actual:.17e} vs {expected:.17e}"
        );
    }
}

#[test]
fn tridia_handles_the_single_equation_and_rejects_bad_shapes() {
    assert_eq!(
        solve_tridiagonal(&[0.0], &[2.0], &[0.0], &[8.0]).unwrap(),
        [4.0]
    );
    assert!(solve_tridiagonal(&[], &[], &[], &[]).is_err());
    assert!(solve_tridiagonal(&[0.0], &[1.0, 2.0], &[0.0], &[1.0]).is_err());
}
