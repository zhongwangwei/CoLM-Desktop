#!/usr/bin/env python3
"""比较页「把单点算例的 history 并成一个站点文件」：本机与服务器共用同一份脚本。

调用（本机 `python -c <源代码> 参数...`，服务器 `python - 参数... <<EOF`，参数位置相同）：

    prepare.py HISTORY_DIR PREFIX OUT_NC OUT_CSV STATION [LON] [LAT]

OUT_NC 是并出来的站点 NetCDF，OUT_CSV 是 OpenBench 的站点清单（ID,SYEAR,EYEAR,LON,LAT,DIR）。
LON/LAT 可选：history 文件里没有坐标时必须给（两者要么都给要么都不给）。

为什么这么并（对比页审查第 2、4 条）：

- 同一个 history 目录里常有 tracer/unitcat/basin 等族。旧实现 `prefix + "*.nc"` 会把
  `<case>_hist_tracer_<date>.nc` 也并进来，同一时刻出现两次、主变量一半 NaN。这里只并主族：
  PREFIX（INSPECT 给的 `<case>_hist_`）之后必须紧跟 YYYY[-MM[-DD]]。
- 跨文件拼接前先按时间排序，再去重；同一时刻出现在不同文件里、各自带不同变量时按坐标合并
  （变量取非 NaN 值），不再产生重复时刻或半 NaN 记录。
- 只并维度不超过 {time, patch} 的变量（土壤层/积雪层/类别维的变量会让内存从 360 MB 涨到 17 GB，
  实测 AT-Neu 48 个文件十分钟没完）。被跳过的变量逐个打印原因，不静默丢。
- patch 只有 1 个时 squeeze 掉；>1 个（多 patch 单点）明确报错——旧实现直接 squeeze 崩。
- 格点与 element 向量文件不能当站点文件制备，明确报错而不是过滤完变量后 KeyError: time。
- 经纬度优先用命令行覆盖值（文件缺坐标时也能制备）；否则读文件里的单点坐标。
- 站点清单用 csv.writer 写：站点名或输出路径含逗号/引号时不会被拆成七列。
"""

import csv
import glob
import os
import re
import sys

# 与 inspect.py 同样处理：直接 `python3 prepare.py ...` 时本目录在 sys.path[0]，目录里的
# inspect.py 会挡住 numpy/pandas 内部 `import inspect`。生产路径（-c / stdin）的 sys.path[0]
# 是工作目录，不会踩到；摘掉脚本目录后两个脚本都能独立运行。
if __name__ == "__main__" and "__file__" in globals():
    # `python -c <源代码>` 时没有 __file__（sys.path[0] 是工作目录，本来也不会踩到），跳过。
    _script_dir = os.path.dirname(os.path.abspath(__file__))
    sys.path[:] = [
        entry for entry in sys.path if os.path.abspath(entry or os.getcwd()) != _script_dir
    ]

import numpy as np
import xarray as xr

# 主 history 的文件名后缀：DEF_HIST_groupby 只产生这三种（colm-hist/src/schedule.rs）。
MAIN_FILE = re.compile(r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?\.nc$")
HIST_MARK = "_hist_"
UNSTRUCTURED_DIMS = ("element", "ncol", "cell", "mesh")


def normalize_lon(value):
    """站点清单里统一用 ±180 约定；0–360 的 200.5° 写成 -159.5°，OpenBench 才对得上参考站点。"""
    return ((float(value) + 180.0) % 360.0) - 180.0


def number_argument(name, text):
    try:
        value = float(text)
    except (TypeError, ValueError):
        raise SystemExit(f"{name} must be a number, got {text!r}")
    if not np.isfinite(value):
        raise SystemExit(f"{name} must be finite, got {text!r}")
    return value


def main_files(folder, prefix):
    """目录里属于主族的文件（按文件名排序）；其他族不参与合并。"""
    if prefix.endswith("_hist"):
        prefix += "_"
    if not prefix.endswith("_hist_"):
        if HIST_MARK in prefix:
            # 传进来的像 `<case>_hist_tracer_`：这不是主族前缀，直接说清楚。
            raise SystemExit(
                f"prepare only merges the main history family; {prefix!r} is not a "
                f"<case>{HIST_MARK} prefix"
            )
        prefix = prefix + HIST_MARK
    pattern = os.path.join(folder, prefix + "*.nc")
    files = []
    others = set()
    for path in sorted(glob.glob(pattern)):
        remainder = os.path.basename(path)[len(prefix) :]
        if MAIN_FILE.fullmatch(remainder):
            files.append(path)
        else:
            family = remainder.split("_", 1)[0]
            others.add(family if family and not family[0].isdigit() else "other")
    if not files:
        detail = f"; found other families: {', '.join(sorted(others))}" if others else ""
        raise SystemExit(
            f"no main history files in {folder} matching {prefix}<YYYY[-MM[-DD]]>.nc{detail}"
        )
    return files


def spatial_type_of(dataset):
    """与 inspect.py 同一判据：先维、再 patch、最后看有没有坐标。"""
    dims = set(dataset.sizes)
    if "lat" in dims and "lon" in dims:
        return "grid"
    if any(name in dims for name in UNSTRUCTURED_DIMS):
        return "unstructured"
    if "patch" in dims:
        return "station" if dataset.sizes["patch"] == 1 else "patch"
    if "lat" in dataset.variables and "lon" in dataset.variables:
        return "station"
    return "unknown"


def coordinate_value(dataset, name):
    """0 维（或只有 1 个值）的 lat/lon；格点坐标不是单点，返回 None。"""
    if name not in dataset.variables:
        return None
    variable = dataset[name]
    if variable.dims and all(dataset.sizes[dim] == 1 for dim in variable.dims):
        values = np.asarray(variable.values, dtype=float).ravel()
        return float(values[0]) if values.size else None
    if not variable.dims:
        return float(np.asarray(variable.values))
    return None


def read_part(path, patch_size):
    """读一个文件的「只随时间变」的变量；返回 (dataset, 被跳过的变量->原因)。"""
    skipped = {}
    with xr.open_dataset(path) as dataset:
        if "time" not in dataset.variables:
            raise SystemExit(f"{path} has no time variable; cannot build a station file")
        if "patch" in dataset.sizes and dataset.sizes["patch"] != patch_size:
            raise SystemExit(
                f"{path} has {dataset.sizes['patch']} patches, expected {patch_size}"
            )
        keep = []
        for name in dataset.data_vars:
            variable = dataset[name]
            if "time" not in variable.dims:
                continue
            extra = [str(dim) for dim in variable.dims if dim not in ("time", "patch")]
            if extra:
                skipped[name] = extra
                continue
            keep.append(name)
        if not keep:
            raise SystemExit(
                f"{path} has no variable that varies only with time/patch; "
                f"cannot build a station file (skipped {', '.join(skipped) or 'none'})"
            )
        part = dataset[keep]
        if "patch" in part.sizes:
            part = part.squeeze("patch", drop=True)
        return part.load(), skipped


def merge_duplicate_times(dataset):
    """同一时刻在多个文件里出现（不同变量或部分重复文件）：按坐标合并成一条记录。

    xarray 的 concat 会把重复时刻原样留下（6 条记录 3 个唯一时刻、主变量一半 NaN）；
    这里按时刻分组，用 `compat="no_conflicts"` 合并同名变量：只有一边有值的补齐，
    两边都有且不一致时报错（不替用户挑一个）。
    """
    index = dataset.indexes["time"]
    groups = {}
    for position, value in enumerate(index):
        groups.setdefault(value, []).append(position)
    merged = []
    for value in sorted(groups):
        pieces = [dataset.isel(time=[position]) for position in groups[value]]
        try:
            merged.append(xr.merge(pieces, compat="no_conflicts", join="outer"))
        except ValueError as error:
            raise SystemExit(f"conflicting values at time {value}: {error}")
    combined = xr.concat(merged, dim="time")
    return combined.sortby("time")


def merge_files(files, patch_size):
    """按时间排序、去重后合并；返回 (合并结果, 被跳过变量->原因, 文件数)。"""
    skipped = {}
    parts = []
    for path in files:
        part, file_skipped = read_part(path, patch_size)
        for name, extra in file_skipped.items():
            skipped.setdefault(name, set()).update(extra)
        parts.append(part)
    parts.sort(key=lambda dataset: np.datetime64(dataset["time"].values.min()))
    merged = xr.concat(
        parts,
        dim="time",
        data_vars="all",
        coords="minimal",
        compat="override",
        join="outer",
    )
    merged = merged.sortby("time")
    if not merged.indexes["time"].is_unique:
        merged = merge_duplicate_times(merged)
    if not merged.indexes["time"].is_unique:
        raise SystemExit("the prepared history still has duplicate time records")
    return merged, skipped


def read_coordinates(dataset):
    lon = coordinate_value(dataset, "lon")
    lat = coordinate_value(dataset, "lat")
    if lon is None or lat is None:
        return None, None
    return normalize_lon(lon), lat


def prepare(folder, prefix, out_nc, out_csv, station, lon_argument, lat_argument):
    if (lon_argument is None) != (lat_argument is None):
        raise SystemExit("give both lon and lat overrides, or neither")
    override = None
    if lon_argument is not None:
        lon = number_argument("lon", lon_argument)
        lat = number_argument("lat", lat_argument)
        if abs(lat) > 90:
            raise SystemExit(f"lat must be within [-90, 90], got {lat}")
        if abs(lon) > 360:
            raise SystemExit(f"lon must be within [-360, 360], got {lon}")
        override = (normalize_lon(lon), lat)

    files = main_files(folder, prefix)
    with xr.open_dataset(files[0]) as first:
        spatial_type = spatial_type_of(first)
        patch_size = int(first.sizes["patch"]) if "patch" in first.sizes else None
        if spatial_type == "grid":
            raise SystemExit(
                f"{folder} holds gridded history (lat/lon dimensions); a gridded simulation is "
                "compared directly and cannot be prepared as a station file"
            )
        if spatial_type == "unstructured":
            raise SystemExit(
                f"{folder} holds unstructured (element/ncol) history; it cannot be prepared as a "
                "station file"
            )
        if patch_size is not None and patch_size > 1:
            raise SystemExit(
                f"{folder} has patch={patch_size}; multi-patch single-point history is not "
                "supported by the station comparison (needs exactly one patch)"
            )
        if spatial_type == "unknown" and override is None:
            raise SystemExit(
                f"cannot tell whether {folder} is a single point (no patch dimension and no "
                "lat/lon); pass LON LAT if it is a station"
            )
        file_lon, file_lat = read_coordinates(first)
    if override is not None:
        lon, lat = override
    elif file_lon is not None and file_lat is not None:
        lon, lat = file_lon, file_lat
    else:
        raise SystemExit(
            f"the history files in {folder} carry no lon/lat; pass LON LAT after STATION "
            "(prepare.py FOLDER PREFIX OUT_NC OUT_CSV STATION [LON] [LAT])"
        )

    merged, skipped = merge_files(files, patch_size)
    # 文件里若把 lat/lon 存成坐标（而不是 0 维数据变量），先去掉，统一写我们确认过的那一对。
    merged = merged.drop_vars(["lat", "lon"], errors="ignore")
    merged["lat"] = np.float64(lat)
    merged["lon"] = np.float64(lon)
    merged.attrs["station"] = station
    merged.attrs["prepared_by"] = "colm-bench prepare.py"

    out_nc = os.path.abspath(out_nc)
    os.makedirs(os.path.dirname(out_nc) or ".", exist_ok=True)
    merged.to_netcdf(out_nc)

    times = np.sort(merged["time"].values)
    start = int(str(times.min())[:4])
    end = int(str(times.max())[:4])
    out_csv = os.path.abspath(out_csv)
    os.makedirs(os.path.dirname(out_csv) or ".", exist_ok=True)
    with open(out_csv, "w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["ID", "SYEAR", "EYEAR", "LON", "LAT", "DIR"])
        writer.writerow([station, start, end, f"{lon:.6f}", f"{lat:.6f}", out_nc])

    print(f"{station}: {len(files)} files, {merged.sizes['time']} steps, {len(merged.data_vars)} variables")
    if skipped:
        detail = ", ".join(
            f"{name}({', '.join(sorted(extra))})" for name, extra in sorted(skipped.items())
        )
        print(
            f"skipped {len(skipped)} variable(s) whose dimensions go beyond time/patch: {detail}"
        )
    return merged


def main(argv):
    if not 6 <= len(argv) <= 8:
        raise SystemExit(
            "usage: prepare.py HISTORY_DIR PREFIX OUT_NC OUT_CSV STATION [LON] [LAT]"
        )
    folder = argv[1]
    prepare(
        folder,
        argv[2],
        argv[3],
        argv[4],
        argv[5],
        argv[6] if len(argv) > 6 else None,
        argv[7] if len(argv) > 7 else None,
    )


if __name__ == "__main__":
    main(sys.argv)
