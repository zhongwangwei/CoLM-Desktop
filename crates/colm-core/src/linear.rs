//! Linear-system kernels shared by initialization and time stepping.
//!
//! `solve_tridiagonal` is the direct flat-buffer port of
//! `MOD_Utils.F90:tridia`, which is used by soil, lake, snow, glacier,
//! plant-hydraulic, urban-temperature, ocean, and BGC transport paths.

/// Solves CoLM's tridiagonal system with the exact forward-elimination and
/// backward-substitution order of `MOD_Utils::tridia`.
///
/// All vectors use the same zero-based order.  As in the Fortran routine,
/// `subdiagonal[0]` and `superdiagonal[n - 1]` are unused boundary entries.
/// Singular pivots retain IEEE division semantics rather than silently
/// changing the model equation; callers validate their physical inputs.
pub fn solve_tridiagonal(
    subdiagonal: &[f64],
    diagonal: &[f64],
    superdiagonal: &[f64],
    rhs: &[f64],
) -> Result<Vec<f64>, &'static str> {
    let n = diagonal.len();
    if n == 0 || subdiagonal.len() != n || superdiagonal.len() != n || rhs.len() != n {
        return Err("tridiagonal system vectors must be nonempty and have equal lengths");
    }

    let mut solution = vec![0.0; n];
    let mut gamma = vec![0.0; n];
    let mut pivot = diagonal[0];
    solution[0] = rhs[0] / pivot;
    for index in 1..n {
        gamma[index] = superdiagonal[index - 1] / pivot;
        pivot = diagonal[index] - subdiagonal[index] * gamma[index];
        solution[index] = (rhs[index] - subdiagonal[index] * solution[index - 1]) / pivot;
    }
    for index in (0..n - 1).rev() {
        solution[index] -= gamma[index + 1] * solution[index + 1];
    }
    Ok(solution)
}

#[cfg(test)]
#[path = "linear_tests.rs"]
mod linear_tests;
