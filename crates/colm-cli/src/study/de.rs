//! Deterministic rand/1/bin Differential Evolution.
use anyhow::{bail, Result};

use super::sample::unit_f64;

#[allow(clippy::needless_range_loop)] // `dim` indexes the target and all three donor rows.
pub fn trial_generation(
    seed: u64,
    generation: usize,
    population: &[Vec<f64>],
    f: f64,
    cr: f64,
) -> Result<Vec<Vec<f64>>> {
    validate(population, f, cr)?;
    let n = population.len();
    let d = population[0].len();
    let mut out = Vec::with_capacity(n);
    for target in 0..n {
        let donors = donors(seed, generation, target, n);
        let forced = (unit_f64(seed ^ 0x91e10da5, generation, target) * d as f64).floor() as usize;
        let mut trial = Vec::with_capacity(d);
        for dim in 0..d {
            let mutant = population[donors[0]][dim]
                + f * (population[donors[1]][dim] - population[donors[2]][dim]);
            let cross =
                dim == forced || unit_f64(seed ^ 0xc0ffee, target * d + dim, generation) < cr;
            trial.push(if cross {
                mutant.clamp(0.0, 1.0)
            } else {
                population[target][dim]
            });
        }
        out.push(trial);
    }
    Ok(out)
}

fn validate(population: &[Vec<f64>], f: f64, cr: f64) -> Result<()> {
    if population.len() < 4 {
        bail!("DE population must be at least 4");
    }
    let d = population[0].len();
    if d == 0 {
        bail!("DE dimension must be positive");
    }
    if !f.is_finite() || !(0.0..=2.0).contains(&f) || f == 0.0 {
        bail!("DE mutation factor F must be in (0,2]");
    }
    if !cr.is_finite() || !(0.0..=1.0).contains(&cr) {
        bail!("DE crossover rate CR must be in [0,1]");
    }
    if population
        .iter()
        .any(|row| row.len() != d || row.iter().any(|v| !v.is_finite()))
    {
        bail!("DE population must be rectangular and finite");
    }
    Ok(())
}

fn donors(seed: u64, generation: usize, target: usize, n: usize) -> [usize; 3] {
    let mut out = [usize::MAX; 3];
    let mut count = 0;
    let mut stream = 0;
    while count < 3 {
        let candidate =
            (unit_f64(seed ^ 0xde51, generation * n + target, stream) * n as f64).floor() as usize;
        stream += 1;
        if candidate == target || out[..count].contains(&candidate) {
            continue;
        }
        out[count] = candidate;
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::science::ObjectiveScore;
    use super::*;

    fn sphere(row: &[f64]) -> ObjectiveScore {
        ObjectiveScore::Feasible(row.iter().map(|x| (x - 0.25).powi(2)).sum())
    }

    fn select(
        parent: &[f64],
        parent_score: &ObjectiveScore,
        trial: Vec<f64>,
        trial_score: &ObjectiveScore,
    ) -> Vec<f64> {
        let score = |score: &ObjectiveScore| match score {
            ObjectiveScore::Feasible(v) if v.is_finite() => Some(*v),
            _ => None,
        };
        match (score(parent_score), score(trial_score)) {
            (Some(a), Some(b)) if b <= a => trial,
            (None, Some(_)) => trial,
            _ => parent.to_vec(),
        }
    }

    #[test]
    fn deterministic_trials_stay_bounded_and_exclude_target_donors() {
        let pop = vec![
            vec![0.1, 0.2],
            vec![0.3, 0.4],
            vec![0.5, 0.6],
            vec![0.7, 0.8],
        ];
        let a = trial_generation(7, 0, &pop, 0.8, 0.9).unwrap();
        let b = trial_generation(7, 0, &pop, 0.8, 0.9).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        for target in 0..pop.len() {
            let d = donors(7, 0, target, pop.len());
            assert!(!d.contains(&target));
            assert_ne!(d[0], d[1]);
            assert_ne!(d[0], d[2]);
            assert_ne!(d[1], d[2]);
        }
    }

    #[test]
    fn selection_keeps_feasible_and_rejects_worse_or_failed_trials() {
        let parent = vec![0.4, 0.4];
        let trial = vec![0.25, 0.25];
        assert_eq!(
            select(&parent, &sphere(&parent), trial.clone(), &sphere(&trial)),
            trial
        );
        assert_eq!(
            select(
                &parent,
                &sphere(&parent),
                vec![0.9, 0.9],
                &sphere(&[0.9, 0.9])
            ),
            parent
        );
        assert_eq!(
            select(
                &parent,
                &ObjectiveScore::Infeasible("x".into()),
                trial.clone(),
                &sphere(&trial)
            ),
            trial
        );
        assert_eq!(
            select(
                &parent,
                &sphere(&parent),
                trial,
                &ObjectiveScore::Infeasible("x".into())
            ),
            parent
        );
    }

    #[test]
    fn sphere_does_not_get_worse_over_generations() {
        let mut pop = vec![
            vec![0.0, 0.0],
            vec![1.0, 1.0],
            vec![0.8, 0.2],
            vec![0.2, 0.8],
            vec![0.4, 0.3],
        ];
        let mut best = pop
            .iter()
            .map(|r| match sphere(r) {
                ObjectiveScore::Feasible(v) => v,
                _ => f64::INFINITY,
            })
            .fold(f64::INFINITY, f64::min);
        for generation in 0..12 {
            let trials = trial_generation(11, generation, &pop, 0.7, 0.9).unwrap();
            pop = pop
                .iter()
                .zip(trials)
                .map(|(p, t)| select(p, &sphere(p), t.clone(), &sphere(&t)))
                .collect();
            let now = pop
                .iter()
                .map(|r| match sphere(r) {
                    ObjectiveScore::Feasible(v) => v,
                    _ => f64::INFINITY,
                })
                .fold(f64::INFINITY, f64::min);
            assert!(now <= best + 1e-12);
            best = now;
        }
        assert!(best < 0.01, "best={best}");
    }

    #[test]
    fn validates_basic_de_contract() {
        assert!(trial_generation(1, 0, &[vec![0.0], vec![0.1], vec![0.2]], 0.8, 0.9).is_err());
        assert!(trial_generation(1, 0, &[vec![], vec![], vec![], vec![]], 0.8, 0.9).is_err());
        assert!(trial_generation(
            1,
            0,
            &[vec![0.0], vec![0.1], vec![0.2], vec![0.3]],
            0.0,
            0.9
        )
        .is_err());
        assert!(trial_generation(
            1,
            0,
            &[vec![0.0], vec![0.1], vec![0.2], vec![0.3]],
            0.8,
            1.1
        )
        .is_err());
    }
}
