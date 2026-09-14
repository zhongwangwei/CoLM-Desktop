# CPU parallelism contract

This repository uses **Rayon for production data parallelism** and keeps model
values as IEEE-754 `f64`.

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
- Reference rounding is explicit where it affects topology: regular raw-grid
  edges use `f64::mul_add`, matching the original production Fortran build's
  single rounding on ARM64. Other Fortran compiler/target settings may differ.
  Do not enable blanket contraction or fast-math. Area aggregation retains
  `areaquad`'s degree conversion, km² units and multiplication order; cancelling
  the radius scale can change a nonlinear fit's accept/reject decision.

## Acceptance checks

A new parallel kernel must prove that one-thread and multi-thread Rayon pools
produce identical indexed outputs, retain the upstream/reference regression
checks, and record a representative performance measurement before claiming a
speedup.
