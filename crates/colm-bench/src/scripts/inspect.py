#!/usr/bin/env python3
"""比较页「读一个 history 目录」：本机与服务器共用同一份脚本。

调用（本机是 `python -c <源代码> <目录> [前缀]`，服务器是 `python - <目录> [前缀] <<EOF`；
两种方式 sys.argv 都以脚本名或 `-` 起算，所以参数位置相同）：

    inspect.py 目录 [主 history 前缀]

成功时 stdout 只有一行 JSON（窗口进程整段反序列化，不能混别的输出）；失败用 SystemExit，
消息放最后一行（界面只显示 stderr 最后一行）。

为什么这么判（每一条都是实测踩出来的）：

- 目录里可能同时有主 history 与 tracer/unitcat/basin/restart 等族。`*_hist_*.nc` 通配会把
  `<case>_hist_tracer_<date>.nc` 也吃进来；旧实现按时间把两族拼在一起，得到 6 条记录 3 个唯一时刻、
  主变量一半 NaN。文件名里 `_hist_` 之后紧跟日期（YYYY/YYYY-MM/YYYY-MM-DD）的才是主 history。
- 时间分辨率不能只看第一个文件的前两条记录：每天一个文件时每文件只有一条记录，旧逻辑落到默认
  月级；逐年数据也会被误判 Month。这里把主族所有文件的 time 采样出来排序取中位步长。
- `data_groupby` 必须与 OpenBench 找文件的方式一致，只能按文件名后缀判；旧实现看第一个文件的
  尾部，同目录换一种分组就错。
- 空间类型要显式分类：单格 lat×lon 网格也是 `grid`（旧逻辑「某维长度>1」把它当单点），
  element/ncol 向量是 `unstructured`，patch>1 是 `patch`，没有坐标也看不出是单点的是 `unknown`。
  上层必须能先拒绝，而不是先显示「可比较」再在制备时 KeyError。
- 经度按环绕距离归一化到 [-180,180) 再算范围与分辨率：0–360 文件里的 200.5° 归一成 -159.5°，
  [-179,179] 这种跨日界线的两点网格分辨率是 2° 而不是 358°。OpenBench 的 [西,东] 表示法写不出
  跨日界线的范围（必须 西<东），所以这种网格给 extent=null、crosses_dateline=true，让上层拒绝，
  而不是给一个 199.5>180 的假范围。
- 土层/积雪层/类别维（soil、soilsnow、lake、band…）超出站点评估的 {time,patch}：不能静默丢掉，
  要逐个列进 unsupported_variables，制备时再明确打印跳过原因。
"""

import glob
import json
import os
import re
import sys

# 直接 `python3 inspect.py DIR` 运行时，Python 会把本文件所在目录插到 sys.path[0]；本文件恰好叫
# inspect.py，numpy 们内部的 `import inspect` 就会导入这个文件自己（实测 numpy.ma 直接 AttributeError）。
# 生产路径（`python -c <源代码>`、`python - args <<EOF`）的 sys.path[0] 是工作目录，不会踩到；
# 这里把脚本目录摘掉，让它可以被独立运行、也能被测试脚本直接执行。
if __name__ == "__main__" and "__file__" in globals():
    # `python -c <源代码>` 时没有 __file__（sys.path[0] 是工作目录，本来也不会踩到），跳过。
    _script_dir = os.path.dirname(os.path.abspath(__file__))
    sys.path[:] = [
        entry for entry in sys.path if os.path.abspath(entry or os.getcwd()) != _script_dir
    ]

import numpy as np
import xarray as xr

# DEF_HIST_groupby 只产生这三种后缀（colm-hist/src/schedule.rs 的 HistoryGrouping）。
MAIN_SUFFIX = re.compile(r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?$")
HIST_MARK = "_hist_"
RESTART_MARK = "_restart_hist_"
# 非结构网格可能用的维度名；目前 CoLM 的向量写出用 element，其余为防将来换名。
UNSTRUCTURED_DIMS = ("element", "ncol", "cell", "mesh")

# OpenBench 模拟条目能消化的维度：站点是 time(+patch)，格点是 time+lat+lon。
SUPPORTED_DIMS = {
    "station": {"time", "patch"},
    "patch": {"time", "patch"},
    "grid": {"time", "lat", "lon"},
    "unstructured": {"time", "element"},
    "unknown": {"time"},
}

# 只归一化经度用；纬度不绕圈。
DAY_SECONDS = 86400.0


def normalize_lon(value):
    """把任意约定（0–360、±180、甚至 ±540）的经度卷进 [-180, 180)。"""
    wrapped = ((float(value) + 180.0) % 360.0) - 180.0
    # 浮点取模在 -180 附近可能给出 -180.00000000000003；固定成恰好 -180，避免出现 < -180 的范围。
    return -180.0 if abs(wrapped + 180.0) < 1e-9 else wrapped


def parse_name(path):
    """文件名 -> (前缀, 族名, 日期后缀)；不是 `*_hist_*.nc` 的返回 None。

    族名 "main" 是主 history（`_hist_` 后紧跟日期），其余是 tracer/unitcat/basin…
    这些族单独报告，不参与探测与制备。
    """
    base = os.path.basename(path)
    if not base.endswith(".nc"):
        return None
    stem = base[: -len(".nc")]
    # restart 累加器旁车 `<case>_restart_hist_<cdate>_<block>.nc` 也含 `_hist_`，先认它，
    # 免得被当成主 history（它的后缀 2010-032-00000 不是 YYYY-MM-DD）。
    index = stem.rfind(RESTART_MARK)
    if index >= 0:
        head = stem[: index + len(RESTART_MARK)]
        return head, "restart", stem[index + len(RESTART_MARK) :]
    index = stem.rfind(HIST_MARK)
    if index < 0:
        return None
    head = stem[: index + len(HIST_MARK)]
    tail = stem[index + len(HIST_MARK) :]
    if MAIN_SUFFIX.fullmatch(tail):
        return head, "main", tail
    token = tail.split("_", 1)[0]
    if not token or token[0].isdigit():
        # 形如 2010-032-00000：日期样的东西但不是主 history 的分组后缀，归到 other。
        return head, "other", tail
    return head + token + "_", token, tail[len(token) + 1 :]


def scan(folder):
    """目录里所有能认出来的 history 文件：[(路径, 前缀, 族名, 后缀)]。"""
    found = []
    for path in glob.glob(os.path.join(folder, "*_hist_*.nc")):
        parsed = parse_name(path)
        if parsed is not None:
            found.append((path, parsed[0], parsed[1], parsed[2]))
    found.sort(key=lambda item: item[0])
    return found


def resolve_folder(folder):
    """给的是算例目录时，history 在 `out/<DEF_CASE_NAME>/history`（名字不一定等于目录名）。

    多个算例输出都在 `out/` 下时报错列候选，不拿第一个冒充。
    """
    if scan(folder):
        return folder
    nested = [
        path
        for path in sorted(glob.glob(os.path.join(folder, "out", "*", "history")))
        if os.path.isdir(path) and scan(path)
    ]
    if not nested:
        raise SystemExit(f"no *_hist_*.nc in {folder}")
    if len(nested) > 1:
        names = ", ".join(os.path.dirname(path) for path in nested)
        raise SystemExit(
            f"multiple case histories under {os.path.join(folder, 'out')}: {names}; "
            "point the inspection at one of them or pass the main history prefix"
        )
    return nested[0]


def file_times(path):
    """只取 time 变量（xarray 懒打开，读到的只有时间轴）。"""
    with xr.open_dataset(path) as dataset:
        if "time" not in dataset.variables:
            raise SystemExit(f"{path} has no time variable")
        values = np.asarray(dataset["time"].values)
    if values.dtype.kind != "M":
        raise SystemExit(f"{path}: time variable does not decode to datetime ({values.dtype})")
    return np.sort(values)


def collect_times(paths):
    chunks = [file_times(path) for path in paths]
    return np.sort(np.concatenate(chunks)) if chunks else np.array([], dtype="datetime64[ns]")


def median_step_seconds(times):
    """相邻记录的中位步长；只有一个时刻时返回 None（交给文件名兜底）。"""
    unique = np.unique(times)
    if unique.size < 2:
        return None
    seconds = np.diff(unique) / np.timedelta64(1, "s")
    return float(np.median(seconds))


def classify_tim_res(seconds):
    """中位步长 -> OpenBench 的 tim_res。

    OpenBench 3.0.8 只接受 Month/Day/Hour/Year/3Hour/6Hour/8Day（config/loader.py 的
    VALID_TIM_RES_VALUES）；CoLM 的 DEF_HIST_FREQ 只会出 hourly/3 小时/日/月/年这几档。
    月与年按日历步长给容差（闰年、28/31 天），落不到任何一档的步长（例如 5 天）明确报错，
    不能猜成 Month 让它悄悄算错。
    """
    if seconds <= 3600:
        return "Hour"
    if seconds <= 3 * 3600:
        return "3Hour"
    if seconds <= DAY_SECONDS:
        return "Day"
    if 27 * DAY_SECONDS <= seconds <= 32 * DAY_SECONDS:
        return "Month"
    if 350 * DAY_SECONDS <= seconds <= 380 * DAY_SECONDS:
        return "Year"
    raise SystemExit(
        f"cannot map a median history step of {seconds:.0f} s onto an OpenBench tim_res "
        "(supported: Hour/3Hour/Day/Month/Year)"
    )


def classify_suffix(suffix):
    """文件名日期后缀 -> OpenBench data_groupby 的候选。"""
    match = MAIN_SUFFIX.fullmatch(suffix)
    if match is None:
        return None
    if match.group(3) is not None:
        return "Day"
    if match.group(2) is not None:
        return "Month"
    return "Year"


def spatial_type_of(dataset):
    """显式空间分类，判断顺序就是「先看最可信的维度」。

    单点文件（CoLM 站点写出）有 patch 维与 0 维 lat/lon；格点文件有 lat/lon 维（哪怕只有 1 格）；
    向量写出有 element 维。都没有时才看有 0 维 lat/lon 没有，否则 unknown。
    """
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


def scalar_value(dataset, name):
    """取 0 维（或只有 1 个值）坐标的值；不是单点坐标时返回 None。"""
    if name not in dataset.variables:
        return None
    var = dataset[name]
    if var.dims and all(dataset.sizes[dim] == 1 for dim in var.dims):
        values = np.asarray(var.values, dtype=float).ravel()
        return float(values[0]) if values.size else None
    if not var.dims:
        return float(np.asarray(var.values))
    return None


def circular_spacing(values, period=360.0):
    """环绕距离下每个点的最近邻距离的中位数（分辨率的定义）。

    普通 np.diff 会把 [-179,179] 算成 358；环绕最近邻给 2，这正是跨日界线相邻的含义。
    """
    unique = np.unique(np.mod(np.asarray(values, dtype=float), period))
    if unique.size < 2:
        return None
    nearest = []
    for value in unique:
        distance = np.abs(unique - value)
        distance = np.minimum(distance, period - distance)
        distance = distance[distance > 0]
        if distance.size:
            nearest.append(float(distance.min()))
    return float(np.median(nearest)) if nearest else None


def linear_spacing(values):
    unique = np.unique(np.asarray(values, dtype=float))
    if unique.size < 2:
        return None
    return float(np.median(np.diff(unique)))


def boundary_spacing(dataset, low, high):
    """用格边界（lat_s/lat_n、lon_w/lon_e）求分辨率，中心点只有一格时也能得到格子大小。"""
    if low not in dataset.variables or high not in dataset.variables:
        return None
    low_values = np.asarray(dataset[low].values, dtype=float).ravel()
    high_values = np.asarray(dataset[high].values, dtype=float).ravel()
    if low_values.size == 0 or low_values.size != high_values.size:
        return None
    spans = np.abs(high_values - low_values)
    spans = spans[np.isfinite(spans)]
    return float(np.median(spans)) if spans.size else None


def nearest_wrap(value, reference, period=360.0):
    """把边界值卷到离它自己那个格心最近的一圈（0–360 与 ±180 混用时必须这么对齐）。"""
    return value + period * np.round((reference - value) / period)


def crosses_dateline(lon_centers, spacing):
    """只由经度中心点判断这套格点是否绕过了日界线。

    把「最大空隙」当作没覆盖的经度带：覆盖宽度 = 360 - 最大空隙。普通区域/全球网格里最大空隙
    是首尾之间的回绕段（覆盖宽度 = 朴素跨度）；跨日界线时最大空隙落在内部，朴素跨度远大于实际
    覆盖宽度——旧实现就会把 [-179,179] 的 2° 相邻算成 358° 分辨率、把范围算成 [-179,179]。
    """
    unique = np.sort(np.unique(np.asarray(lon_centers, dtype=float)))
    if unique.size < 2:
        return False
    gaps = np.diff(unique)
    wrap_gap = unique[0] + 360.0 - unique[-1]
    covered = 360.0 - max(float(gaps.max()) if gaps.size else 0.0, float(wrap_gap))
    naive = float(unique[-1] - unique[0])
    tolerance = max((spacing or 0.0) * 0.5, 1e-6)
    return naive - covered > tolerance


def grid_geometry(dataset):
    """格点文件 -> (extent, resolution, crosses_dateline, lon_centers)。读不出给 None。"""
    if "lon" not in dataset.variables or "lat" not in dataset.variables:
        return None, None, False, None
    lon_raw = np.asarray(dataset["lon"].values, dtype=float).ravel()
    lat_raw = np.asarray(dataset["lat"].values, dtype=float).ravel()
    if lon_raw.size == 0 or lat_raw.size == 0:
        return None, None, False, None
    lon_centers = np.array([normalize_lon(value) for value in lon_raw])

    lon_spacing = circular_spacing(lon_centers)
    lat_spacing = linear_spacing(lat_raw)
    if lon_spacing is None:
        lon_spacing = boundary_spacing(dataset, "lon_w", "lon_e")
    if lat_spacing is None:
        lat_spacing = boundary_spacing(dataset, "lat_s", "lat_n")
    spacings = [value for value in (lon_spacing, lat_spacing) if value]
    # 格点 resolution 取 max：OpenBench 的 grid_res 是单一值，取大的一边保证覆盖粗方向。
    resolution = max(spacings) if spacings else None

    crossing = crosses_dateline(lon_centers, lon_spacing)
    if crossing:
        return None, resolution, True, lon_centers

    extent = None
    if (
        "lon_w" in dataset.variables
        and "lon_e" in dataset.variables
        and "lat_s" in dataset.variables
        and "lat_n" in dataset.variables
    ):
        lon_w = np.asarray(dataset["lon_w"].values, dtype=float).ravel()
        lon_e = np.asarray(dataset["lon_e"].values, dtype=float).ravel()
        lat_s = np.asarray(dataset["lat_s"].values, dtype=float).ravel()
        lat_n = np.asarray(dataset["lat_n"].values, dtype=float).ravel()
        if lon_w.size == lon_raw.size and lon_e.size == lon_raw.size:
            west = min(
                float(nearest_wrap(normalize_lon(value), center))
                for value, center in zip(lon_w, lon_centers)
            )
            east = max(
                float(nearest_wrap(normalize_lon(value), center))
                for value, center in zip(lon_e, lon_centers)
            )
            # 自检：边界必须真的包住中心点；0–360 文件单格在日界线附近时容易把两圈混起来，
            # 出现了就当作跨日界线，不给假范围。
            if not (west <= float(lon_centers.min()) and east >= float(lon_centers.max())):
                return None, resolution, True, lon_centers
            south = float(np.min(lat_s)) if lat_s.size else None
            north = float(np.max(lat_n)) if lat_n.size else None
        else:
            west = east = south = north = None
    else:
        west = east = south = north = None

    if west is None or south is None:
        # 没有边界变量时用中心点 ± 半个分辨率；只有一组中心点时推不出格子边界。
        if lon_spacing is None or lat_spacing is None:
            return None, resolution, False, lon_centers
        west = float(lon_centers.min()) - lon_spacing / 2.0
        east = float(lon_centers.max()) + lon_spacing / 2.0
        south = float(lat_raw.min()) - lat_spacing / 2.0
        north = float(lat_raw.max()) + lat_spacing / 2.0

    west = max(-180.0, west)
    east = min(180.0, east)
    south = max(-90.0, south)
    north = min(90.0, north)
    if west < east and south < north:
        extent = [west, east, south, north]
    return extent, resolution, False, lon_centers


def family_report(folder, entries):
    """每一族单独报告：主族之外（tracer/unitcat/…）在界面里要能看见，不参与探测。"""
    groups = {}
    for path, prefix, family, suffix in entries:
        groups.setdefault((prefix, family), []).append(path)
    report = []
    for (prefix, family), paths in sorted(groups.items()):
        paths = sorted(paths)
        entry = {"family": family, "prefix": prefix, "files": len(paths), "syear": None, "eyear": None}
        try:
            # 其他族只报范围：读首尾两个文件的 time 就够，没必要把几百个 tracer 文件全打开。
            start = int(str(file_times(paths[0]).min())[:4])
            end = int(str(file_times(paths[-1]).max())[:4])
            entry["syear"] = start
            entry["eyear"] = end
        except (SystemExit, ValueError, OSError, KeyError):
            # 旁车文件可能没有可解码的 time；族报告只是展示，不因此让整个探测失败。
            pass
        report.append(entry)
    return report


def inspect(folder, prefix_argument):
    entries = scan(folder)
    if not entries:
        raise SystemExit(f"no *_hist_*.nc in {folder}")

    main_prefixes = sorted({prefix for _, prefix, family, _ in entries if family == "main"})
    if not main_prefixes:
        families = sorted({family for _, _, family, _ in entries})
        raise SystemExit(
            f"no main history files (<case>_hist_<YYYY[-MM[-DD]]>.nc) in {folder}; "
            f"found only {', '.join(families)} files"
        )
    if len(main_prefixes) > 1:
        if prefix_argument:
            wanted = (
                prefix_argument
                if prefix_argument.endswith(HIST_MARK)
                else prefix_argument + HIST_MARK
            )
            if wanted not in main_prefixes:
                raise SystemExit(
                    f"no main history prefix {prefix_argument!r} in {folder}; "
                    f"candidates: {', '.join(main_prefixes)}"
                )
            prefix = wanted
        else:
            # 同一目录里混着不同算例（例如 A 的 2000 与 Z 的 2010）：不能拿首尾文件拼年份。
            raise SystemExit(
                f"multiple main history prefixes in {folder}: {', '.join(main_prefixes)}; "
                "pass the main history prefix to choose one (inspect.py DIR PREFIX)"
            )
    else:
        prefix = main_prefixes[0]
        if prefix_argument:
            wanted = (
                prefix_argument
                if prefix_argument.endswith(HIST_MARK)
                else prefix_argument + HIST_MARK
            )
            if wanted != prefix:
                raise SystemExit(
                    f"no main history prefix {prefix_argument!r} in {folder}; "
                    f"candidates: {prefix}"
                )

    main_paths = [
        path
        for path, file_prefix, family, _ in entries
        if family == "main" and file_prefix == prefix
    ]
    suffixes = [
        suffix for path, file_prefix, family, suffix in entries if family == "main" and file_prefix == prefix
    ]

    # data_groupby：按族内文件名后缀判；单文件是 single；后缀混着时就报错，不猜。
    if len(main_paths) == 1:
        data_groupby = "single"
    else:
        patterns = {classify_suffix(suffix) for suffix in suffixes}
        if None in patterns:
            bad = [os.path.basename(path) for path, suffix in zip(main_paths, suffixes) if classify_suffix(suffix) is None]
            raise SystemExit(
                f"cannot tell data_groupby from file names (need YYYY / YYYY-MM / YYYY-MM-DD): {', '.join(bad)}"
            )
        if len(patterns) > 1:
            raise SystemExit(
                f"mixed history file date patterns in {folder}: {', '.join(sorted(patterns))}; "
                "the files must all use the same DEF_HIST_groupby"
            )
        data_groupby = patterns.pop()

    times = collect_times(main_paths)
    if times.size == 0:
        raise SystemExit(f"the main history files in {folder} have no time records")
    start = int(str(times.min())[:4])
    end = int(str(times.max())[:4])

    step = median_step_seconds(times)
    tim_res_source = "time"
    if step is None:
        # 真的只有一个时刻（一个文件一条记录）：没有时间轴可采样，只能按文件名后缀猜一次，
        # 并明确标出来源，界面/上层可以把它当提示而不是实测结论。
        tim_res_source = "filename"
        fallback = {suffix: classify_suffix(suffix) for suffix in suffixes}
        guesses = {value for value in fallback.values() if value}
        if len(guesses) != 1:
            raise SystemExit(
                "the history has a single time step and the file name does not tell the resolution; "
                "cannot infer tim_res"
            )
        tim_res = guesses.pop()
    else:
        tim_res = classify_tim_res(step)

    with xr.open_dataset(main_paths[0]) as first:
        spatial_type = spatial_type_of(first)
        patch = int(first.sizes["patch"]) if "patch" in first.sizes else None
        variables = []
        unsupported = []
        supported = SUPPORTED_DIMS.get(spatial_type, {"time"})
        for name in first.data_vars:
            var = first[name]
            if "time" not in var.dims:
                continue
            variables.append({"name": name, "dims": [str(dim) for dim in var.dims]})
            extra = [str(dim) for dim in var.dims if dim not in supported]
            if extra:
                unsupported.append(
                    {
                        "name": name,
                        "dims": [str(dim) for dim in var.dims],
                        "extra_dims": extra,
                        "reason": f"dimensions {', '.join(extra)} are beyond "
                        f"{{{', '.join(sorted(supported))}}}",
                    }
                )
        lon = lat = None
        extent = resolution = None
        crossing = False
        if spatial_type == "grid":
            extent, resolution, crossing, _ = grid_geometry(first)
        elif spatial_type in ("station", "patch"):
            lon_value = scalar_value(first, "lon")
            lat_value = scalar_value(first, "lat")
            if lon_value is not None and lat_value is not None:
                lon = normalize_lon(lon_value)
                lat = lat_value

    blockers = []
    if spatial_type == "patch":
        blockers.append(
            {
                "code": "multi_patch",
                "message": f"the history has {patch} patches per grid cell; "
                "the station comparison supports a single patch only",
            }
        )
    elif spatial_type == "unstructured":
        blockers.append(
            {
                "code": "unstructured",
                "message": "unstructured (element/ncol) history cannot be compared as a station; "
                "OpenBench station data needs a single point",
            }
        )
    elif spatial_type == "unknown":
        blockers.append(
            {
                "code": "unknown_spatial_type",
                "message": "cannot tell from the file whether it is a station or a grid "
                "(no lat/lon dimensions, no patch dimension and no coordinates)",
            }
        )
    elif crossing:
        blockers.append(
            {
                "code": "dateline_crossing",
                "message": "the grid crosses the antimeridian; OpenBench's [west, east] region cannot "
                "express it, so no extent is reported",
            }
        )

    warnings = []
    if tim_res_source == "filename":
        warnings.append("the history has a single time step; tim_res was inferred from the file name")
    if spatial_type in ("station", "patch") and lon is None:
        warnings.append(
            "the history files have no coordinates; pass lon/lat when preparing a station file"
        )
    if spatial_type == "grid" and extent is None and not crossing:
        warnings.append("could not read the grid extent; enter the region manually")

    return {
        "history_dir": os.path.abspath(folder),
        "prefix": prefix,
        "files": len(main_paths),
        "families": family_report(folder, entries),
        "spatial": spatial_type == "grid",
        "spatial_type": spatial_type,
        "patch": patch,
        "lon": lon,
        "lat": lat,
        "syear": start,
        "eyear": end,
        "tim_res": tim_res,
        "tim_res_source": tim_res_source,
        "data_groupby": data_groupby,
        "extent": extent,
        "resolution": resolution,
        "crosses_dateline": crossing,
        "comparable": not blockers,
        "reason": "; ".join(blocker["message"] for blocker in blockers) if blockers else None,
        "blockers": blockers,
        "variables": variables,
        "unsupported_variables": unsupported,
        "warnings": warnings,
    }


def main(argv):
    if len(argv) < 2:
        raise SystemExit("usage: inspect.py HISTORY_DIR [MAIN_HISTORY_PREFIX]")
    folder = resolve_folder(argv[1])
    prefix_argument = argv[2].strip() if len(argv) > 2 and argv[2].strip() else None
    print(json.dumps(inspect(folder, prefix_argument), ensure_ascii=False))


if __name__ == "__main__":
    main(sys.argv)
