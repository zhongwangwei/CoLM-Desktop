//! Linear-system kernels shared by initialization and time stepping.
//!
//! `solve_tridiagonal` is the direct flat-buffer port of
//! `MOD_Utils.F90:tridia`, which is used by soil, lake, snow, glacier,
//! plant-hydraulic, urban-temperature, ocean, and BGC transport paths.

/// Solves CoLM's tridiagonal system with the exact forward-elimination and
/// backward-substitution order of `MOD_Utils::tridia`.
///
/// **三处 `mul_add` 是照抄 gfortran 的收缩，不是优化。** 上游是
/// `bet = b(j) - a(j)*gam(j)`、`u(j) = (r(j) - a(j)*u(j-1))/bet`、
/// `u(j) = u(j) - gam(j+1)*u(j+1)`（`MOD_Utils.F90:2556-2565`），内核 `-O2` 下
/// `-ffp-contract=fast` 是默认，三处的乘积都被吸收（`X - p` 形状每层都收）。
/// 逐位量过：把 `tridia` 原样编出来跑 2000 组对角占优的三对角系统（n=15），
/// 不收缩的解向量只有 **183/2000** 逐位相同，三处都写 `mul_add` 之后 **2000/2000**。
///
/// 这条不是"又一个热路径"：`tridia` 是**整个模式共用的求解器** —— 上游有
/// `MOD_GroundTemperature`/`MOD_SoilSnowHydrology`（土壤水）/`MOD_PlantHydraulic`
/// （两次）/`MOD_Lake`/`MOD_Glacier`/`MOD_SimpleOcean`/`MOD_Urban_*`/BGC 输运
/// 共十余处调用，本仓库对应 `ground_temperature`、`soil_water`、
/// `plant_hydraulics`（三次）、`urban_temperature`（两次）、`urban_impervious`。
/// 解向量每步都喂回状态，所以这 1 ULP 会**每步、每条柱**累积。
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
        pivot = (-subdiagonal[index]).mul_add(gamma[index], diagonal[index]);
        solution[index] = (-subdiagonal[index]).mul_add(solution[index - 1], rhs[index]) / pivot;
    }
    for index in (0..n - 1).rev() {
        solution[index] = (-gamma[index + 1]).mul_add(solution[index + 1], solution[index]);
    }
    Ok(solution)
}

#[cfg(test)]
#[path = "linear_tests.rs"]
mod linear_tests;
