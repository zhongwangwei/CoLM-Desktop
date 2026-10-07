"""从常数重启读特征，规则与引擎一致（crates/colm-runtime/src/hybrid.rs）：

- `name`：(patch,) 量，整数变量也行；
- `name[k]`：(patch, 层) 量的第 k 层（1 起）。

单点算例只有一个土壤 patch（patch 0）。
"""

import glob
import re
from pathlib import Path

import netCDF4


def constant_restart(case: Path) -> Path:
    """按 patch 存放的那一份常数重启（单点是带分块名的 `*_w180_s90.nc`，另一份没有 patch 量）。"""
    files = sorted(glob.glob(str(case / "out" / "*" / "restart" / "const" / "*.nc")))
    for path in files:
        with netCDF4.Dataset(path) as d:
            if "patchclass" in d.variables:
                return Path(path)
    raise FileNotFoundError(f"no per-patch constant restart under {case}/out/*/restart/const")


def read_features(case: Path, features, patch: int = 0):
    with netCDF4.Dataset(constant_restart(case)) as d:
        d.set_auto_mask(False)
        values = []
        for feature in features:
            m = re.fullmatch(r"([^\[\]]+)(?:\[(\d+)\])?", feature)
            if not m:
                raise ValueError(f"feature {feature} is not name or name[k]")
            name, layer = m.group(1), m.group(2)
            v = d.variables[name][:]
            if v.ndim == 1:
                if layer is not None:
                    raise ValueError(f"{feature}: {name} has no layers")
                values.append(float(v[patch]))
            else:
                if layer is None:
                    raise ValueError(f"{feature}: name a layer as {name}[k]")
                values.append(float(v[patch, int(layer) - 1]))
        return values
