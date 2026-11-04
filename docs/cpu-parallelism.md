# CPU parallelism contract

This repository uses **Rayon for production data parallelism** and keeps model
values as IEEE-754 `f64`.

## Remote resource budgets

For a single spatial case, the run page supports automatic parallel allocation
from a node count and the usable CPU cores per node. Existing saved manual
settings remain manual. Independent station batches keep their node-packing
path; a single station has no spatial work to divide between ranks.

The equivalent CLI request is:

```sh
colm-cli remote-run /path/to/case --host cluster --root /data/colm \
  --kernel /path/to/kernel --scheduler slurm \
  --parallel auto --nodes 2 --cores-per-node 48 --dry-run 1
```

Automatic mode resolves both the scheduler request and process launcher from
one plan. Do not combine it with manual `--ranks`, `--threads` or `--cpus`.

| Engine | Two nodes, 48 cores per node |
| --- | --- |
| Rust | 2 ranks, 48 threads/CPUs per rank |
| Fortran | 96 MPI ranks, 1 thread/CPU per rank |

Rust prefers one rank per node; when a node has more than 256 usable cores,
the planner chooses the smallest evenly dividing rank count that keeps each
rank at 256 threads or fewer. This is a deterministic feasible layout, not a
measured performance optimum. With `--parallel manual` (or no mode flag),
explicit process/thread settings retain their meaning, and omitted node counts
are planned from `--cores-per-node` when known.

Automatic multi-node allocation currently requires Slurm and `srun`. Other
schedulers support single-node automatic allocation; multi-node jobs need
manual configuration until their MPI placement is verified. Bare execution
cannot allocate additional hosts merely from a node count.

The requested `--mem-gb` remains a per-node memory request. The planner does not
invent a model memory estimate. Actual remote landdata block counts and
unsupported multi-process model configurations remain subject to the engine's
startup checks; a resource plan does not certify those scientific inputs.

## Required shape

- Parallelize independent work units (patches, mesh elements, raster blocks)
  with `rayon::prelude::*` and indexed `par_iter` / `into_par_iter`.
- Preserve the scalar operation order *inside* each work unit. Collect results
  by their original index, then perform dependency-sensitive copies and NetCDF
  writes in their required serial order.
- Keep WMO copies, ordered recurrence relations, reductions whose order affects
  results, and NetCDF/HDF I/O outside the parallel iterator unless their
  thread-safety and ordering contract has been verified.
- Do not introduce bespoke `std::thread` worker pools or atomic work queues
  for production numerical loops. Standard threads remain appropriate for
  blocking subprocess supervision and GUI/service housekeeping.

## SIMD and precision

- Model state and numerical kernels remain `f64`; never trade correctness for
  `f32` lanes.
- Let LLVM autovectorize ordinary `f64` arithmetic first. Explicit SIMD is
  permitted only for a measured hot loop with a stable, safe implementation
  that preserves the scalar `f64` result contract.
- Do not replace `powf`, `log10`, or other transcendental calculations with
  approximate vector math without an upstream-reference error budget approved
  by regression tests.
- Reference rounding is explicit where it affects topology or nonlinear fits:
  raw-grid edges, soil input means/observations, LM norms and ordered QR
  dot/rank updates use `f64::mul_add`
  where independent goldens establish the original production Fortran build's
  single rounding on ARM64. Other Fortran compiler/target settings may differ.
  Do not enable blanket contraction or fast-math. Area aggregation retains
  `areaquad`'s degree conversion, km² units and multiplication order; cancelling
  the radius scale can change a nonlinear fit's accept/reject decision.

## Acceptance checks

A new parallel kernel must prove that one-thread and multi-thread Rayon pools
produce identical indexed outputs, retain the upstream/reference regression
checks, and record a representative performance measurement before claiming a
speedup.
