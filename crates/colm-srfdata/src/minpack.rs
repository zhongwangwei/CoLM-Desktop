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
                let sum = (j..m).map(|row| wa4[row] * fjac[row * n + j]).sum::<f64>();
                let scale = -sum / diagonal;
                for row in j..m {
                    wa4[row] += fjac[row * n + j] * scale;
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
                    let sum = (0..=j).map(|row| qtf[row] * fjac[row * n + j]).sum::<f64>();
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
                1.0 - (fnorm1 / fnorm).powi(2)
            } else {
                -1.0
            };
            for j in 0..n {
                wa3[j] = 0.0;
                let column = ipvt[j];
                let step = wa1[column];
                for row in 0..=j {
                    wa3[row] += fjac[row * n + j] * step;
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
                    0.5 * dirder / (dirder + 0.5 * actred)
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
            wa1[row] -= r[row * n + j] * value;
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
            let sum = (0..j).map(|row| wa1[row] * r[row * n + j]).sum::<f64>();
            wa1[j] = (wa1[j] - sum) / r[j * n + j];
        }
        let norm = enorm(&wa1);
        parl = (fp / delta / norm) / norm;
    }
    for j in 0..n {
        let sum = (0..=j).map(|row| qtb[row] * r[row * n + j]).sum::<f64>();
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
                wa1[row] -= r[row * n + j] * value;
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
                let dot = (j..m)
                    .map(|row| a[row * n + j] * a[row * n + column])
                    .sum::<f64>();
                let scale = dot / a[j * n + j];
                for row in j..m {
                    a[row * n + column] -= scale * a[row * n + j];
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
                    let sine = 0.5 / (0.25 + 0.25 * cotan * cotan).sqrt();
                    (sine, sine * cotan)
                } else {
                    let tangent = sdiag[k] / r[k * n + k];
                    let cosine = 0.5 / (0.25 + 0.25 * tangent * tangent).sqrt();
                    (cosine * tangent, cosine)
                };
                r[k * n + k] = cosine * r[k * n + k] + sine * sdiag[k];
                let temp = cosine * wa[k] + sine * qtbpj;
                qtbpj = -sine * wa[k] + cosine * qtbpj;
                wa[k] = temp;
                for row in k + 1..n {
                    let temp = cosine * r[row * n + k] + sine * sdiag[row];
                    sdiag[row] = -sine * r[row * n + k] + cosine * sdiag[row];
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
        let sum = (j + 1..nsing)
            .map(|row| wa[row] * r[row * n + j])
            .sum::<f64>();
        wa[j] = (wa[j] - sum) / sdiag[j];
    }
    for j in 0..n {
        x[ipvt[j]] = wa[j];
    }
}

fn enorm(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn enorm_column(a: &[f64], m: usize, n: usize, start: usize, column: usize) -> f64 {
    (start..m)
        .map(|row| a[row * n + column].powi(2))
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

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
