# Local close-error fix

Upstream package: `netcdf-src` 0.5.3 from crates.io, package SHA-256
`9f8a4c2a96491452c12be9fe0ad0e6e20fd94cb5996eabd4cfcec51dfd4d898a`.
Georust revision: `933f646750ed3f2bba12046b48bd4d19ab0711da` (`netcdf-src/`).
Bundled netCDF-C: 4.10.1. License: [source/COPYRIGHT](source/COPYRIGHT).
`UPSTREAM-SHA256.json` records the unmodified packaged file digests; Cargo's cache
marker, redundant dependency lock, and original manifest are omitted.

The only executable C change removes `dumpopenobjects(h5)` after `H5Fclose()` fails in
`source/libhdf5/hdf5file.c`. HDF5 may already have destroyed its internal file
state on a close error. Querying that state again through `H5Iget_name` aborts in
`H5G_rootof` instead of returning `NC_EHDFERR`. The error return is preserved.
`build.rs` guards the fix against accidental removal during a vendor refresh.

The comment beside `libdap4/ncd4types.h`'s `checksumignore` records the provenance
of an unchanged upstream field, not a CoLM checksum bypass. Upstream sets it only
for an explicit URL `#hyrax` fragment (`NCD4_applychecksumcontrols` in `d4file.c`);
otherwise `d4data.c` reports a checksum mismatch as `NC_EDAP`.
`cargo tree --offline --locked -e features -i netcdf-src` confirms CoLM enables
only the crate's default feature. `build.rs` sets `NETCDF_ENABLE_DAP=OFF`, and
the upstream CMake DAP-off branch also sets DAP2/DAP4 off. This code is therefore
not compiled into the current CoLM binaries. Enabling DAP requires a separate
remote-integrity review; do not use the compatibility option to suppress errors.
The source-integrity test removes only this exact annotation before checking the
original header SHA-256 (`d30250671992801902398ce467e06da8c7b96a70502f41e36530bb74718ec601`),
so an implementation change to that field or its controls still fails the guard.

Diagnosis used actual static netCDF-C 4.10.1 / HDF5 2.2.0 and private files: raw
libc close returning EIO or ENOSPC aborts before this fix; replacing this single
C line returns `NC_EHDFERR (-101)` for both. Healthy close succeeds in both builds.
Build the instrumented probes with `scripts/build-netcdf-fault-probes.sh`.
Run `scripts/test-netcdf-libc-close-faults.py` with those probes as `--cli` and
`--runtime-probe`, plus the normal CLI as `--plain-cli`, for actual libc boundary
regression. The probes only mark the real `nc_close` interval in this test.
Run `scripts/test-netcdf-close-faults.py` with the instrumented probes for the
separate C API close-return propagation checks.

Upstream references:
- https://github.com/Unidata/netcdf-c/blob/v4.10.1/libhdf5/hdf5file.c#L229-L235
- https://github.com/HDFGroup/hdf5/blob/2.2.0/src/H5Fint.c#L1620-L1680
- https://github.com/HDFGroup/hdf5/blob/2.2.0/src/H5Groot.c#L83-L98

No upstream fix was available at the time of this patch. Remove the local patch
only after an upstream replacement passes both raw-close regression errors.
