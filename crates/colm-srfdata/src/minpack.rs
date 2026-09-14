//! Minimal, dependency-free port of the MINPACK `lmder` path CoLM uses.
//!
//! This is deliberately limited to the analytic-Jacobian solver used by the
//! soil hydraulic upscaling routines.  It retains the original QR pivoting,
//! damping, tolerances, and iteration bounds rather than changing scientific
//! fit results through normal equations or finite-difference derivatives.

/// An analytic least-squares problem in MINPACK's `lmder` form.
pub(crate) trait LeastSquaresProblem {
    /// Write `m` residuals for `x`. Return false for an invalid trial point.
    fn residual(&self, x: &[f64], output: &mut [f64]) -> bool;

    /// Write an `m × n` row-major Jacobian for `x`. Return false when invalid.
    fn jacobian(&self, x: &[f64], output: &mut [f64]) -> bool;
}

/// CoLM's `MOD_Utils::lmder` defaults for the soil curve fits.
pub(crate) fn lmder(problem: &impl LeastSquaresProblem, x: &mut [f64], m: usize) -> bool {
    let n = x.len();
    if n == 0 || m < n {
        return false;
    }
    let (ftol, xtol, gtol, factor) = (1.0e-5, 1.0e-4, 0.0, 0.1);
    let maxfev = 100 * (n + 1);
    let mut fvec = vec![0.0; m];
    let mut fjac = vec![0.0; m * n];
    if !problem.residual(x, &mut fvec) || !fvec.iter().all(|value| value.is_finite()) {
        return false;
    }
    let mut nfev = 1;
    let mut diag = vec![0.0; n];
    let mut qtf = vec![0.0; n];
    let mut wa1 = vec![0.0; n];
    let mut wa2 = vec![0.0; n];
    let mut wa3 = vec![0.0; n];
    let mut wa4 = vec![0.0; m];
    let mut fnorm = enorm(&fvec);
    let mut par = 0.0;
    let mut delta = 0.0;
    let mut xnorm = 0.0;
    let mut iteration = 1;

    loop {
        if !problem.jacobian(x, &mut fjac) || !fjac.iter().all(|value| value.is_finite()) {
            return false;
        }
        let (ipvt, rdiag, acnorm) = qrfac(&mut fjac, m, n);
        wa1.copy_from_slice(&rdiag);
        wa2.copy_from_slice(&acnorm);

        if iteration == 1 {
            for j in 0..n {
                diag[j] = if wa2[j] == 0.0 { 1.0 } else { wa2[j] };
                wa3[j] = diag[j] * x[j];
            }
            xnorm = enorm(&wa3);
            delta = if xnorm == 0.0 { factor } else { factor * xnorm };
        }

        wa4.copy_from_slice(&fvec);
        for j in 0..n {
            let diagonal = fjac[j * n + j];
            if diagonal != 0.0 {
                let sum = (j..m).fold(0.0, |sum, row| wa4[row].mul_add(fjac[row * n + j], sum));
                let scale = -sum / diagonal;
                for row in j..m {
                    wa4[row] = fjac[row * n + j].mul_add(scale, wa4[row]);
                }
            }
            fjac[j * n + j] = wa1[j];
            qtf[j] = wa4[j];
        }

        let mut gnorm = 0.0_f64;
        if fnorm != 0.0 {
            for j in 0..n {
                let column = ipvt[j];
                if wa2[column] != 0.0 {
                    let sum =
                        (0..=j).fold(0.0, |sum, row| qtf[row].mul_add(fjac[row * n + j], sum));
                    gnorm = gnorm.max((sum / fnorm / wa2[column]).abs());
                }
            }
        }
        if gnorm <= gtol {
            return true;
        }
        for j in 0..n {
            diag[j] = diag[j].max(wa2[j]);
        }

        loop {
            lmpar(
                &mut fjac, n, &ipvt, &diag, &qtf, delta, &mut par, &mut wa1, &mut wa2,
            );
            for j in 0..n {
                wa1[j] = -wa1[j];
                wa2[j] = x[j] + wa1[j];
                wa3[j] = diag[j] * wa1[j];
            }
            let pnorm = enorm(&wa3);
            if iteration == 1 {
                delta = delta.min(pnorm);
            }
            if !problem.residual(&wa2, &mut wa4) || !wa4.iter().all(|value| value.is_finite()) {
                return false;
            }
            nfev += 1;
            let fnorm1 = enorm(&wa4);
            let actred = if 0.1 * fnorm1 < fnorm {
                // Original MOD_Utils contracts the square and subtraction.
                let ratio = fnorm1 / fnorm;
                (-ratio).mul_add(ratio, 1.0)
            } else {
                -1.0
            };
            for j in 0..n {
                wa3[j] = 0.0;
                let column = ipvt[j];
                let step = wa1[column];
                for row in 0..=j {
                    wa3[row] = fjac[row * n + j].mul_add(step, wa3[row]);
                }
            }
            let temp1 = enorm(&wa3) / fnorm;
            let temp2 = par.sqrt() * pnorm / fnorm;
            let prered = temp1.powi(2) + temp2.powi(2) / 0.5;
            let dirder = -(temp1.powi(2) + temp2.powi(2));
            let ratio = if prered != 0.0 { actred / prered } else { 0.0 };

            if ratio <= 0.25 {
                let mut temp = if actred >= 0.0 {
                    0.5
                } else {
                    lm_step_shrink(dirder, actred)
                };
                if 0.1 * fnorm1 >= fnorm || temp < 0.1 {
                    temp = 0.1;
                }
                delta = temp * delta.min(pnorm / 0.1);
                par /= temp;
            } else if par == 0.0 || ratio >= 0.75 {
                delta = 2.0 * pnorm;
                par *= 0.5;
            }

            if ratio >= 0.0001 {
                x.copy_from_slice(&wa2);
                for j in 0..n {
                    wa2[j] = diag[j] * x[j];
                }
                fvec.copy_from_slice(&wa4);
                xnorm = enorm(&wa2);
                fnorm = fnorm1;
                iteration += 1;
            }

            let converged = (actred.abs() <= ftol && prered <= ftol && 0.5 * ratio <= 1.0)
                || delta <= xtol * xnorm;
            if converged || nfev >= maxfev {
                return true;
            }
            if (actred.abs() <= f64::EPSILON && prered <= f64::EPSILON && 0.5 * ratio <= 1.0)
                || delta <= f64::EPSILON * xnorm
                || gnorm <= f64::EPSILON
            {
                return true;
            }
            if ratio >= 0.0001 {
                break;
            }
        }
    }
}

fn lm_step_shrink(mut dirder: f64, mut actred: f64) -> f64 {
    // For a rejected step, dirder <= 0 and actred < 0. The original D
    // literals promote .5*d/(d+.5*a) to REAL16. Use d/(2*d+a), retaining
    // the denominator's low part and the quotient residual in f64. Tiny
    // ratios are subsequently clamped to 0.1 by lmder, as in the original.
    // Exact power-of-two scaling keeps the compensated residual normal.
    const SCALE: f64 = f64::from_bits((1023 + 512) << 52);
    let largest = dirder.abs().max(actred.abs());
    if largest > SCALE {
        dirder /= SCALE;
        actred /= SCALE;
    } else if largest < 1.0 / SCALE {
        dirder *= SCALE;
        actred *= SCALE;
    }
    let twice = dirder * 2.0;
    let denominator = twice + actred;
    let virtual_a = denominator - twice;
    let low = (twice - (denominator - virtual_a)) + (actred - virtual_a);
    let q = dirder / denominator;
    let residual = (-q).mul_add(denominator, dirder) - q * low;
    q + residual / denominator
}

#[allow(clippy::too_many_arguments)]
fn lmpar(
    r: &mut [f64],
    n: usize,
    ipvt: &[usize],
    diag: &[f64],
    qtb: &[f64],
    delta: f64,
    par: &mut f64,
    x: &mut [f64],
    sdiag: &mut [f64],
) {
    let mut wa1 = vec![0.0; n];
    let mut wa2 = vec![0.0; n];
    let mut nsing = n;
    for j in 0..n {
        wa1[j] = qtb[j];
        if r[j * n + j] == 0.0 && nsing == n {
            nsing = j;
        }
        if nsing < n {
            wa1[j] = 0.0;
        }
    }
    for offset in 0..nsing {
        let j = nsing - 1 - offset;
        wa1[j] /= r[j * n + j];
        let value = wa1[j];
        for row in 0..j {
            wa1[row] = (-r[row * n + j]).mul_add(value, wa1[row]);
        }
    }
    for j in 0..n {
        x[ipvt[j]] = wa1[j];
    }
    for j in 0..n {
        wa2[j] = diag[j] * x[j];
    }
    let dxnorm = enorm(&wa2);
    let mut fp = dxnorm - delta;
    if fp <= 0.1 * delta {
        *par = 0.0;
        return;
    }

    let mut parl = 0.0;
    if nsing == n {
        for j in 0..n {
            let column = ipvt[j];
            wa1[j] = diag[column] * (wa2[column] / dxnorm);
        }
        for j in 0..n {
            let sum = (0..j).fold(0.0, |sum, row| wa1[row].mul_add(r[row * n + j], sum));
            wa1[j] = (wa1[j] - sum) / r[j * n + j];
        }
        let norm = enorm(&wa1);
        parl = (fp / delta / norm) / norm;
    }
    for j in 0..n {
        let sum = (0..=j).fold(0.0, |sum, row| qtb[row].mul_add(r[row * n + j], sum));
        wa1[j] = sum / diag[ipvt[j]];
    }
    let gnorm = enorm(&wa1);
    let mut paru = gnorm / delta;
    if paru == 0.0 {
        paru = f64::MIN_POSITIVE / delta.min(0.1);
    }
    *par = (*par).max(parl).min(paru);
    if *par == 0.0 {
        *par = gnorm / dxnorm;
    }

    for iteration in 1..=10 {
        if *par == 0.0 {
            *par = f64::MIN_POSITIVE.max(0.001 * paru);
        }
        for j in 0..n {
            wa1[j] = par.sqrt() * diag[j];
        }
        qrsolv(r, n, ipvt, &wa1, qtb, x, sdiag);
        for j in 0..n {
            wa2[j] = diag[j] * x[j];
        }
        let dxnorm = enorm(&wa2);
        let previous = fp;
        fp = dxnorm - delta;
        if fp.abs() <= 0.1 * delta
            || (parl == 0.0 && fp <= previous && previous < 0.0)
            || iteration == 10
        {
            return;
        }
        for j in 0..n {
            let column = ipvt[j];
            wa1[j] = diag[column] * (wa2[column] / dxnorm);
        }
        for j in 0..n {
            wa1[j] /= sdiag[j];
            let value = wa1[j];
            for row in j + 1..n {
                wa1[row] = (-r[row * n + j]).mul_add(value, wa1[row]);
            }
        }
        let norm = enorm(&wa1);
        let parc = (fp / delta / norm) / norm;
        if fp > 0.0 {
            parl = parl.max(*par);
        } else if fp < 0.0 {
            paru = paru.min(*par);
        }
        *par = parl.max(*par + parc);
    }
}

fn qrfac(a: &mut [f64], m: usize, n: usize) -> (Vec<usize>, Vec<f64>, Vec<f64>) {
    let acnorm = (0..n)
        .map(|column| enorm_column(a, m, n, 0, column))
        .collect::<Vec<_>>();
    let mut rdiag = acnorm.clone();
    let mut wa = acnorm.clone();
    let mut ipvt = (0..n).collect::<Vec<_>>();
    for j in 0..m.min(n) {
        let mut largest = j;
        for column in j + 1..n {
            if rdiag[largest] < rdiag[column] {
                largest = column;
            }
        }
        if largest != j {
            for row in 0..m {
                a.swap(row * n + j, row * n + largest);
            }
            rdiag.swap(j, largest);
            wa.swap(j, largest);
            ipvt.swap(j, largest);
        }
        let mut norm = enorm_column(a, m, n, j, j);
        if norm != 0.0 {
            if a[j * n + j] < 0.0 {
                norm = -norm;
            }
            for row in j..m {
                a[row * n + j] /= norm;
            }
            a[j * n + j] += 1.0;
            for column in j + 1..n {
                // Production MOD_Utils fuses the ordered dot and rank-one
                // update. Separate rounding perturbs nearly dependent columns.
                let dot = (j..m).fold(0.0, |sum, row| {
                    a[row * n + j].mul_add(a[row * n + column], sum)
                });
                let scale = dot / a[j * n + j];
                for row in j..m {
                    a[row * n + column] = (-scale).mul_add(a[row * n + j], a[row * n + column]);
                }
                if rdiag[column] != 0.0 {
                    let temp = a[j * n + column] / rdiag[column];
                    rdiag[column] *= (1.0 - temp * temp).max(0.0).sqrt();
                    if 0.05 * (rdiag[column] / wa[column]).powi(2) <= f64::EPSILON {
                        rdiag[column] = enorm_column(a, m, n, j + 1, column);
                        wa[column] = rdiag[column];
                    }
                }
            }
        }
        rdiag[j] = -norm;
    }
    (ipvt, rdiag, acnorm)
}

fn qrsolv(
    r: &mut [f64],
    n: usize,
    ipvt: &[usize],
    diag: &[f64],
    qtb: &[f64],
    x: &mut [f64],
    sdiag: &mut [f64],
) {
    let mut wa = qtb.to_vec();
    for j in 0..n {
        for row in j..n {
            r[row * n + j] = r[j * n + row];
        }
        x[j] = r[j * n + j];
    }
    for j in 0..n {
        let column = ipvt[j];
        if diag[column] != 0.0 {
            sdiag[j..].fill(0.0);
            sdiag[j] = diag[column];
            let mut qtbpj = 0.0;
            for k in j..n {
                if sdiag[k] == 0.0 {
                    continue;
                }
                let (sine, cosine) = if r[k * n + k].abs() < sdiag[k].abs() {
                    let cotan = r[k * n + k] / sdiag[k];
                    let sine = qrsolv_rotation_coefficient(cotan);
                    (sine, sine * cotan)
                } else {
                    let tangent = sdiag[k] / r[k * n + k];
                    let cosine = qrsolv_rotation_coefficient(tangent);
                    (cosine * tangent, cosine)
                };
                // MOD_Utils fuses the cosine product after rounding the sine product.
                r[k * n + k] = cosine.mul_add(r[k * n + k], sine * sdiag[k]);
                let temp = cosine.mul_add(wa[k], sine * qtbpj);
                qtbpj = cosine.mul_add(qtbpj, -sine * wa[k]);
                wa[k] = temp;
                for row in k + 1..n {
                    let temp = cosine.mul_add(r[row * n + k], sine * sdiag[row]);
                    sdiag[row] = cosine.mul_add(sdiag[row], -sine * r[row * n + k]);
                    r[row * n + k] = temp;
                }
            }
        }
        sdiag[j] = r[j * n + j];
        r[j * n + j] = x[j];
    }
    let mut nsing = n;
    for j in 0..n {
        if sdiag[j] == 0.0 && nsing == n {
            nsing = j;
        }
        if nsing < n {
            wa[j] = 0.0;
        }
    }
    for offset in 0..nsing {
        let j = nsing - 1 - offset;
        let sum = (j + 1..nsing).fold(0.0, |sum, row| wa[row].mul_add(r[row * n + j], sum));
        wa[j] = (wa[j] - sum) / sdiag[j];
    }
    for j in 0..n {
        x[ipvt[j]] = wa[j];
    }
}

fn qrsolv_rotation_coefficient(t: f64) -> f64 {
    debug_assert!(t.is_finite() && t.abs() <= 1.0);
    // Original 0.5D/sqrt(0.25D + 0.25D*t**2) rounds t*t in f64, then
    // promotes sqrt/div to REAL16. Recover the low-part residual in f64
    // instead of rounding 1+q and its reciprocal sqrt independently.
    let q = t * t;
    let u = 1.0 + q;
    let low = q - (u - 1.0);
    let y = 1.0 / u.sqrt();
    let yy = y * y;
    let yy_error = y.mul_add(y, -yy);
    let residual = (-u).mul_add(yy, 1.0) - u * yy_error - low * yy;
    (0.5 * y).mul_add(residual, y)
}

fn enorm(values: &[f64]) -> f64 {
    // Production Fortran contracts SUM(x**2); retain its single rounding.
    values
        .iter()
        .fold(0.0, |sum, value| value.mul_add(*value, sum))
        .sqrt()
}

fn enorm_column(a: &[f64], m: usize, n: usize, start: usize, column: usize) -> f64 {
    (start..m)
        .fold(0.0, |sum, row| {
            a[row * n + column].mul_add(a[row * n + column], sum)
        })
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_step_shrink_matches_original_mixed_precision() {
        // MOD_Utils::lmder evaluates .5D*d/(d+.5D*a) in REAL16. First
        // four pairs are rejected steps from the original soil trajectories.
        for (dirder, actred, expected) in [
            (0xbf60988fda3e6a11, 0xbf73274f3cc3fd10, 0x3fcdb5ec8a80b114),
            (0xbf2b68c51b146f1b, 0xbee140ca2335dc3f, 0x3fdf61f764480f30),
            (0xbf87b1d699ddf93a, 0xbfb1f979a6099eb9, 0x3fbfba3df9bf4e1b),
            (0xbf4a8416f2ae0619, 0xbf6140bf622c2420, 0x3fcbcf624c3a3828),
            (0xffe0000000000000, 0xffe0000000000000, 0x3fd5555555555555),
            (0x8000000000000001, 0x8000000000000002, 0x3fd0000000000000),
            (0x8000000000000000, 0xbff0000000000000, 0x0000000000000000),
        ] {
            assert_eq!(
                lm_step_shrink(f64::from_bits(dirder), f64::from_bits(actred)).to_bits(),
                expected
            );
        }
    }

    #[test]
    fn damped_qr_angle_matches_original_mixed_precision_rounding() {
        // Independent original expression at -O2 -fdefault-real-8: t*t is
        // rounded in f64 before the D literals promote sqrt/div to REAL16.
        for (input, expected) in [
            (0x0000000000000000, 0x3ff0000000000000),
            (0x0000000000000001, 0x3ff0000000000000),
            (0x8000000000000000, 0x3ff0000000000000),
            (0x3ff0000000000000, 0x3fe6a09e667f3bcd),
            (0xbff0000000000000, 0x3fe6a09e667f3bcd),
            (0x3fefffffffffffff, 0x3fe6a09e667f3bcd),
            (0x3fe8000000000000, 0x3fe999999999999a),
            (0x3fe0000000000000, 0x3fec9f25c5bfedd9),
            (0x3fb999999999999a, 0x3fefd7583bc82e29),
            (0x3e40000000000000, 0x3ff0000000000000),
            (0x3e3fffffffffffff, 0x3ff0000000000000),
            (0x1e60000000000000, 0x3ff0000000000000),
        ] {
            assert_eq!(
                qrsolv_rotation_coefficient(f64::from_bits(input)).to_bits(),
                expected
            );
        }
    }

    #[test]
    fn damped_qr_rotations_retain_original_single_rounding() {
        // Original MOD_Utils::qrsolv linked unchanged at -O2 -fdefault-real-8.
        // This fixture isolates fused rotation updates; it does not prove the
        // remaining REAL16-literal rotation-angle expressions match in f64.
        let mut r = [
            0xc0114057a409ce54,
            0xc004302890adc765,
            0x3fe36101dc4d04f0,
            0x0000000000000000,
            0xc0136789aee71586,
            0x4012dbd94697ebec,
            0x0000000000000000,
            0x0000000000000000,
            0xbfe57b3c083344c0,
        ]
        .map(f64::from_bits);
        let diag = [0x4007487c4975f1a4, 0x4004be34f78500ef, 0x3ff0d17c1d1cec5e].map(f64::from_bits);
        let qtb = [0xbfdb491fec382330, 0xbff8a16b9d3c8b0e, 0x3ff4fc98dc78ca9a].map(f64::from_bits);
        let mut x = [0.0; 3];
        let mut sdiag = [0.0; 3];
        qrsolv(&mut r, 3, &[1, 2, 0], &diag, &qtb, &mut x, &mut sdiag);
        let expected = [
            0xbfc0394faa01fcfd,
            0xbf9351ca77309cdf,
            0x3fc759ca86a499a6,
            0xc014210616548d98,
            0xc014864f3b12598f,
            0x400a20f957729a29,
            0xc0114057a409ce54,
            0xc004302890adc765,
            0x3fe36101dc4d04f0,
            0xc0014d50382e9524,
            0xc0136789aee71586,
            0x4012dbd94697ebec,
            0x3fe09bc6d6608a6e,
            0x40122551d895d0d1,
            0xbfe57b3c083344c0,
        ];
        for (actual, expected) in x.iter().chain(&sdiag).chain(&r).zip(expected) {
            assert_eq!(actual.to_bits(), expected);
        }
    }

    #[test]
    fn norms_retain_original_square_sum_single_rounding() {
        // Independently linked original MOD_Utils::enorm, production -O2.
        // Separate multiply/add gives ...095 instead; one ULP in xnorm can
        // terminate LM at its initial point on the xtol boundary.
        let expected = f64::from_bits(0x3fcb9271769ab094);
        assert_eq!(enorm(&[0.08, 0.2]), expected);
        assert_eq!(
            enorm_column(&[999.0, 888.0, 1.0, 0.08, 2.0, 0.2], 3, 2, 1, 1),
            expected
        );
    }

    #[test]
    fn pivoted_qr_retains_original_dot_and_householder_rounding() {
        // Original MOD_Utils::qrfac, -O2 -fdefault-real-8. Nearly dependent
        // columns expose separate rounding of the dot and rank-one update.
        let mut a = [
            0x3ff0000000000001,
            0x3fefffffffffffff,
            0x3fd999999999999a,
            0xbfd5555555555555,
            0xbfd5555555555554,
            0x3fe3333333333333,
            0x3fa999999999999a,
            0x3fa999999999999b,
            0xbfe199999999999a,
            0x3fd0000000000001,
            0x3fcfffffffffffff,
            0x3fb999999999999a,
        ]
        .map(f64::from_bits);
        let (ipvt, rdiag, acnorm) = qrfac(&mut a, 4, 3);
        assert_eq!(ipvt, [0, 2, 1]);
        assert_eq!(
            a.map(f64::to_bits),
            [
                0x3ffec0e708f74b55,
                0xbfc74f8183ed2001,
                0xbff15a0e95c7d2c5,
                0xbfd3abdeb69f0f1b,
                0x3ffc6749e447a6f4,
                0x3c3d6c6776ed5f73,
                0x3fa79b0b418babba,
                0xbfe42f73247be97d,
                0x3ffa1ee6c01bee82,
                0x3fcd81ce11ee96aa,
                0x3fa1493ce73266ac,
                0xbfe8c8e6f8f460a3,
            ]
        );
        assert_eq!(
            rdiag.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [0xbff15a0e95c7d2c5, 0xbfec9c197a74d0c7, 0xbc852407e48866ea]
        );
        assert_eq!(
            acnorm.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [0x3ff15a0e95c7d2c5, 0x3ff15a0e95c7d2c4, 0x3fed327fa4116eac]
        );
    }

    struct Linear;

    impl LeastSquaresProblem for Linear {
        fn residual(&self, x: &[f64], output: &mut [f64]) -> bool {
            output.copy_from_slice(&[x[0] - 3.0, 2.0 * x[1] + 8.0]);
            true
        }

        fn jacobian(&self, _x: &[f64], output: &mut [f64]) -> bool {
            output.copy_from_slice(&[1.0, 0.0, 0.0, 2.0]);
            true
        }
    }

    #[test]
    fn lmder_keeps_colm_scaling_and_solves_an_analytic_problem() {
        let mut values = [0.5, 0.5];
        assert!(lmder(&Linear, &mut values, 2));
        assert!((values[0] - 3.0).abs() < 1.0e-10);
        assert!((values[1] + 4.0).abs() < 1.0e-10);
    }
}
