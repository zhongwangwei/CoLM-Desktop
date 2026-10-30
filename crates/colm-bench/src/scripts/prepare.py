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
- 变量按维度分两类：额外维度只有层维度（soilsnow、soil、lake）的**保留**（带维度写进站点合并
  文件），其它额外维度（band、rtyp、vegnodes、ens…）的跳过并逐个打印原因。层变量不能丢：
  OpenBench 3.0.8 的 CoLM2024 profile 用 `compute:` 从 `f_wliq_soisno.isel(soilsnow=5:13)` 这类
  原始层变量算 Root_Zone_Soil_Moisture/Surface_Soil_Moisture，旧实现把层变量全跳过，profile
  就读不到。代价是峰值内存随层变量数×层数增长（早期实测把层/类别维变量全并进来时 AT-Neu 48 个
  文件 360 MB→17 GB 十分钟没完），所以只放行层维度，类别维仍跳过。
- patch 只有 1 个时 squeeze 掉；>1 个（多 patch 单点）明确报错——旧实现直接 squeeze 崩。
- 格点与 element 向量文件不能当站点文件制备，明确报错而不是过滤完变量后 KeyError: time。
- 经纬度优先用命令行覆盖值（文件缺坐标时也能制备）；否则读文件里的单点坐标。
- 站点清单用 csv.writer 写：站点名或输出路径含逗号/引号时不会被拆成七列。
- 非公历日历（noleap、360_day…）经 xarray 解出来是 cftime 对象，不是 datetime64：文件排序、
  重复时刻分组、CSV 年份都要走日历感知的分支，输出时间轴的 units/calendar 编码从首文件搬过去。
  同一目录混日历、或混标准日历与 cftime，明确报错（与 INSPECT 的探测判据一致）。
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

try:
    # 非公历日历（noleap/360_day…）由 xarray 解成 cftime 对象；与 inspect.py 一样按可选依赖处理：
    # 标准日历的机器没装它也必须能制备，is_cftime_values 就是恒 False。
    import cftime
except ImportError:  # pragma: no cover - 装了 xarray 的机器通常也有；没有时只影响非公历文件
    cftime = None

# 主 history 的文件名后缀：DEF_HIST_groupby 只产生这三种（colm-hist/src/schedule.rs）。
MAIN_FILE = re.compile(r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?\.nc$")
HIST_MARK = "_hist_"
UNSTRUCTURED_DIMS = ("element", "ncol", "cell", "mesh")
# 站点合并要原样保留的层维度（与 inspect.py 的 LAYER_DIMS 同一口径）：OpenBench 3.0.8 的
# CoLM2024 profile 在 model_catalog.yaml 里用 `compute:` 从原始层变量算地表量，例如
# `f_wliq_soisno.isel(soilsnow=5:13)` → Root_Zone_Soil_Moisture / Surface_Soil_Moisture。
# 制备若把这些变量跳过，profile compute 在站点文件里找不到变量。其余额外维度
# （band/rtyp/vegnodes/ens/sensor…）在站点评估里没有对应语义，继续跳过并打印。
LAYER_DIMS = ("soilsnow", "soil", "lake")
# 输出时间轴只沿用首文件这几项编码：units/calendar 是 cftime 序列化的全部依据，dtype 保持原数值类型。
# 压缩与分块参数绑定首文件的时间轴形状，合并后长度已经变了，搬过来没有意义。
TIME_ENCODING_KEYS = ("units", "calendar", "dtype")


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
    """读一个文件的「站点合并要保留的变量」；返回 (dataset, 被跳过的变量->原因)。

    保留判据与 inspect.py 的 layered_variables 同一口径：维度只有 time/patch，或额外维度全部落在
    层维度 {soilsnow, soil, lake} 上。层变量必须带维度保留——OpenBench 3.0.8 的 CoLM2024 profile
    用 compute 从原始层变量算地表量，制备丢掉它们 profile 就读不到；层维度不 squeeze。
    band/rtyp/vegnodes 这类其它额外维度在站点评估里没有对应语义，跳过并逐个打印原因。
    """
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
            if extra and not all(dim in LAYER_DIMS for dim in extra):
                skipped[name] = extra
                continue
            keep.append(name)
        if not keep:
            raise SystemExit(
                f"{path} has no variable that varies only with time/patch or the layer dimensions "
                f"({', '.join(LAYER_DIMS)}); cannot build a station file "
                f"(skipped {', '.join(skipped) or 'none'})"
            )
        part = dataset[keep]
        if "patch" in part.sizes:
            part = part.squeeze("patch", drop=True)
        return part.load(), skipped


def is_cftime_values(values):
    """时间轴的取值是不是 cftime 对象。

    xarray 只在 time:calendar 是非标准日历（noleap、360_day…）时才解成 cftime 对象，标准日历
    仍是 datetime64。这里与 inspect.py 的同名判断刻意重复：两个脚本在本机走 `python -c`、在
    服务器走 heredoc，各自是一份完整源码，没有可共享的模块；判据必须一致，否则探测端说能用、
    制备端却另走一条路。cftime 是可选依赖，没装时恒 False。
    """
    return (
        cftime is not None
        and values.dtype.kind == "O"
        and values.size > 0
        and isinstance(values.ravel()[0], cftime.datetime)
    )


def time_year(value):
    """时间点的年份。

    cftime 没有 datetime64 表示（360_day 的 2 月 30 日也不是合法公历日期，不能先转 datetime64
    再取年份），只能读 .year；datetime64 沿用原来的 str(...)[:4]，站点清单的年份写法不变。
    """
    if cftime is not None and isinstance(value, cftime.datetime):
        return int(value.year)
    return int(str(value)[:4])


def time_axis_kind(dataset):
    """时间轴的解码表示："datetime64" 或 "cftime"；其他类型明确拒绝，不猜成 datetime64。"""
    values = np.asarray(dataset["time"].values)
    if values.dtype.kind == "M":
        return "datetime64"
    if is_cftime_values(values):
        return "cftime"
    raise SystemExit(
        f"time variable does not decode to datetime64 or cftime ({values.dtype}); "
        "check the time:units/calendar attributes"
    )


def check_shared_time_axis(parts):
    """合并前确认所有文件共用一条时间轴；混日期/混日历明确拒绝。

    为什么先验再排：`np.datetime64(cftime)` 对 noleap 会把日期悄悄按公历解释（闰年语义就没了），
    对 360_day 的 2 月 30 日直接 `ValueError: Invalid date (2000,2,30)`；不同日历的"同一天"也
    不是同一时刻，xarray 会在 sortby 里抛 `TypeError: cannot compare ... (different calendars)`。
    inspect.py 已在探测端按同一判据拒绝（R06），制备端再守一次，用户拿到的是可行动的错误，不是栈。
    """
    kinds = set()
    calendars = set()
    for part in parts:
        kind = time_axis_kind(part)
        kinds.add(kind)
        if kind == "cftime":
            calendars.add(str(part["time"].values.ravel()[0].calendar))
    if len(kinds) > 1:
        raise SystemExit(
            "the main history files mix standard-calendar datetime64 with non-standard cftime "
            "time axes; they cannot be ordered on one axis — keep one calendar per history"
        )
    if len(calendars) > 1:
        raise SystemExit(
            f"the main history files use different calendars ({', '.join(sorted(calendars))}); "
            "they cannot share one time axis — keep one calendar per history"
        )


def first_time(dataset):
    """文件里的最早时刻，用来给文件排序。

    同一日历的 cftime 对象可直接比较（就是 Python 对象比较），numpy datetime64 也一样；调用前
    check_shared_time_axis 已保证所有文件是同一种解码表示/日历，所以这些排序键可以互相比较。
    """
    values = np.asarray(dataset["time"].values)
    if values.dtype.kind == "O":
        return min(values.ravel())
    return values.min()


def copy_time_encoding(dataset, encoding, attrs):
    """把首文件 time 变量的 units/calendar/dtype 编码与缺失属性搬到合并结果上。

    为什么必须显式搬：解码时 xarray 把 units/calendar 从属性移进 encoding；合并后的时间变量若
    没有它们，旧版 xarray 无法把 cftime 编码回 NetCDF 时间轴，新版会按合并后的时间轴另猜一套
    units（实测：首文件 `days since 2000-12-31 00:30:00` 被换成 `days since 2000-12-30T00:30:00`，
    参考时刻丢了）。首文件有、合并结果缺的属性（long_name 等）补回去，避免输出文件少元数据。
    """
    for key in TIME_ENCODING_KEYS:
        value = encoding.get(key)
        if value is not None:
            dataset["time"].encoding[key] = value
    for key, value in attrs.items():
        dataset["time"].attrs.setdefault(key, value)


def merge_duplicate_times(dataset):
    """同一时刻在多个文件里出现（不同变量或部分重复文件）：按坐标合并成一条记录。

    xarray 的 concat 会把重复时刻原样留下（6 条记录 3 个唯一时刻、主变量一半 NaN）；
    这里按时刻分组，用 `compat="no_conflicts"` 合并同名变量：只有一边有值的补齐，
    两边都有且不一致时报错（不替用户挑一个）。分组键与 `sorted(groups)` 直接用时刻对象：
    同一日历的 cftime 可哈希、可比较（混日历已由 check_shared_time_axis 在合并前拒绝）。
    保留的层维度变量也走这里：每片是 (time=1, 层…) 的切片，merge 的 join="outer" 负责把层维度
    与层坐标对齐，no_conflicts 只比较同名变量在共同位置上的值。
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
    """按时间排序、去重后合并；返回 (合并结果, 被跳过变量->原因)。"""
    skipped = {}
    parts = []
    for path in files:
        part, file_skipped = read_part(path, patch_size)
        for name, extra in file_skipped.items():
            skipped.setdefault(name, set()).update(extra)
        parts.append(part)
    # 先验日历再排序：同一解码表示/日历下 first_time 的键才可比较；混用要给可行动的错误。
    check_shared_time_axis(parts)
    parts.sort(key=first_time)
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
        # 首文件的 time 编码/属性：合并结果要靠它们把 cftime 时间轴写回 NetCDF（见 copy_time_encoding）。
        time_encoding = dict(first["time"].encoding)
        time_attrs = dict(first["time"].attrs)
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
    # 先补回时间轴编码再写：cftime 没有 datetime64 的数值表示，units/calendar 丢了就写不出可解码的时间轴。
    copy_time_encoding(merged, time_encoding, time_attrs)
    # 文件里若把 lat/lon 存成坐标（而不是 0 维数据变量），先去掉，统一写我们确认过的那一对。
    merged = merged.drop_vars(["lat", "lon"], errors="ignore")
    merged["lat"] = np.float64(lat)
    merged["lon"] = np.float64(lon)
    merged.attrs["station"] = station
    merged.attrs["prepared_by"] = "colm-bench prepare.py"

    out_nc = os.path.abspath(out_nc)
    os.makedirs(os.path.dirname(out_nc) or ".", exist_ok=True)
    merged.to_netcdf(out_nc)

    times = np.asarray(merged["time"].values)
    start = time_year(times.min())
    end = time_year(times.max())
    out_csv = os.path.abspath(out_csv)
    os.makedirs(os.path.dirname(out_csv) or ".", exist_ok=True)
    with open(out_csv, "w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["ID", "SYEAR", "EYEAR", "LON", "LAT", "DIR"])
        writer.writerow([station, start, end, f"{lon:.6f}", f"{lat:.6f}", out_nc])

    print(f"{station}: {len(files)} files, {merged.sizes['time']} steps, {len(merged.data_vars)} variables")
    # 层变量是故意保留的（OpenBench profile compute 的输入），要在日志里说清楚，别让用户以为它们
    # 和以前一样被跳过了；被跳过的类别维变量仍逐个打印原因。
    kept_layers = sorted(
        f"{name}({', '.join(str(dim) for dim in merged[name].dims if str(dim) != 'time')})"
        for name in merged.data_vars
        if any(str(dim) != "time" for dim in merged[name].dims)
    )
    if kept_layers:
        print(
            f"kept {len(kept_layers)} layer variable(s) with their layer dimensions for the "
            f"OpenBench profile compute: {', '.join(kept_layers)}"
        )
    if skipped:
        detail = ", ".join(
            f"{name}({', '.join(sorted(extra))})" for name, extra in sorted(skipped.items())
        )
        print(
            f"skipped {len(skipped)} variable(s) whose dimensions go beyond time/patch and the "
            f"{'/'.join(LAYER_DIMS)} layer dimensions: {detail}"
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
