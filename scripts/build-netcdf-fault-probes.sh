#!/usr/bin/env bash
# Link the test-only C close boundary into the final executables, not dependencies.
set -euo pipefail
test "$(uname -s)" = Linux || { echo 'NetCDF fault probes require Linux' >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
test "$#" = 1 || { echo 'usage: build-netcdf-fault-probes.sh OUTPUT_DIR' >&2; exit 2; }
mkdir -p "$1"
destination=$(cd "$1" && pwd)
cc -fPIC -Wall -Wextra -Werror -c "$root/scripts/fixtures/netcdf_api_close_fault.c" \
  -o "$destination/netcdf_api_close_fault.o"
cd "$root"
# Cargo supplies a hashed suffix; the harness requires these exact output names.
cargo rustc --offline --locked -p colm-cli --bin colm-cli -- \
  -C "link-arg=$destination/netcdf_api_close_fault.o" -C link-arg=-Wl,--wrap=nc_close \
  -C extra-filename= -o "$destination/colm-cli"
cargo rustc --offline --locked -p colm-runtime --example netcdf-close-probe -- \
  -C "link-arg=$destination/netcdf_api_close_fault.o" -C link-arg=-Wl,--wrap=nc_close \
  -C extra-filename= -o "$destination/netcdf-close-probe"
test -x "$destination/colm-cli"
test -x "$destination/netcdf-close-probe"
