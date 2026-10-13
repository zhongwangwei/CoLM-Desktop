#!/usr/bin/env python3
"""Restore the complete local FLUXNET-CH4 AT-Neu observations into the example.

Requires development tools numpy/netCDF4. No download, interpolation, gap filling,
forcing change, or shift of the already distributed example clock is performed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

import netCDF4 as nc
import numpy as np

RAW = {'Qle': 'LE', 'Qh': 'H', 'NEE': 'NEE', 'Ustar': 'USTAR',
       'Rnet': 'NETRAD', 'SWup': 'SW_OUT', 'LWup': 'LW_OUT', 'Qg': 'G',
       'SWdown': 'SW_IN', 'LWdown': 'LW_IN'}
CH4 = ('FCH4', 'FCH4_f', 'FCH4_f_ann', 'FCH4_f_ann_qc')
POLICY = 'https://fluxnet.org/data/fluxnet-ch4-community-product/data-variables/'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--examples', type=Path, default=Path(__file__).resolve().parents[1] / 'examples')
    args = parser.parse_args()
    stem = 'AT-Neu_2010-2012_FLUXNET-CH4'
    target = args.examples / 'Observation' / f'{stem}_Flux.nc'
    forcing = args.examples / 'Forcing' / f'{stem}_Met.nc'
    manifest = {'schema': 1, 'source_file': args.source.name, 'source_sha256': sha(args.source),
                'forcing_sha256': sha(forcing), 'variable_policy': POLICY,
                'time_policy': 'Preserve the already distributed observation/forcing numeric clock and time metadata; no timestamp shift.',
                'qc_policy': '0 denotes a present finite raw (non-_F) observation, 3 denotes missing. This is a measured/missing classification, not a new instrument quality test.'}
    with nc.Dataset(args.source) as source, nc.Dataset(target) as baseline, nc.Dataset(forcing) as met:
        assert str(source.getncattr('site_id')) == 'AT-Neu'
        for other in (baseline, met):
            assert source['time'].units == other['time'].units
            np.testing.assert_array_equal(source['time'][:], other['time'][:])
        for name in CH4:
            np.testing.assert_array_equal(source[name][:].filled(-9999), baseline[name][:].filled(-9999))
        manifest['source_utc_offset'] = float(source.UTC_OFFSET)
        manifest['distributed_utc_offset'] = float(baseline.UTC_OFFSET)
        manifest['records'] = len(source.dimensions['time'])
        manifest['existing_methane_arrays_unchanged'] = list(CH4)
        manifest['raw_counts'] = {}
        with tempfile.NamedTemporaryFile(dir=target.parent, suffix='.nc', delete=False) as temp:
            temp_path = Path(temp.name)
        try:
            with nc.Dataset(temp_path, 'w', format='NETCDF4') as out:
                out.setncatts({name: source.getncattr(name) for name in source.ncattrs()})
                for name in ('UTC_OFFSET', 'utc_offset', 'time_reference', 'time_shown_in'):
                    if name in baseline.ncattrs():
                        out.setncattr(name, baseline.getncattr(name))
                out.source_utc_offset = manifest['source_utc_offset']
                out.observation_source_sha256 = manifest['source_sha256']
                out.observation_variable_policy = POLICY
                out.clock_policy = manifest['time_policy']
                out.history = str(source.getncattr('history')) + '\nComplete observation variables restored; existing distributed example clock and methane values preserved. Raw-value presence masks added for evaluator QC compatibility.'
                for name, dim in source.dimensions.items():
                    out.createDimension(name, len(dim))
                for name, var in source.variables.items():
                    kw = {'fill_value': var.getncattr('_FillValue')} if '_FillValue' in var.ncattrs() else {}
                    dest = out.createVariable(name, var.dtype, var.dimensions, zlib=True, complevel=9, **kw)
                    dest.setncatts({key: var.getncattr(key) for key in var.ncattrs() if key != '_FillValue'})
                    dest[:] = var[:]
                for name, original in RAW.items():
                    var = source[name]
                    assert var.Fluxnet_name == original, (name, var.Fluxnet_name)
                    values = var[:]
                    valid = ~np.ma.getmaskarray(values) & np.isfinite(values.filled(np.nan)) & (values.filled(-9999) != -9999)
                    qc = out.createVariable(name + '_qc', 'i1', var.dimensions, zlib=True, complevel=9)
                    qc[:] = np.where(valid, 0, 3)
                    qc.long_name = f'Raw {original} observation presence classification'
                    qc.flag_values = np.array([0, 3], dtype='i1')
                    qc.flag_meanings = 'present_raw_observation missing'
                    qc.comment = manifest['qc_policy']
                    out[name].ancillary_variables = name + '_qc'
                    manifest['raw_counts'][name] = int(valid.sum())
            with nc.Dataset(temp_path) as check:
                for name in source.variables:
                    a, b = source[name][:], check[name][:]
                    np.testing.assert_array_equal(a.filled(-9999), b.filled(-9999))
            temp_path.replace(target)
        finally:
            temp_path.unlink(missing_ok=True)
    manifest['output_sha256'] = sha(target)
    manifest['output_bytes'] = target.stat().st_size
    (args.examples / 'AT-Neu-observation-provenance.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
