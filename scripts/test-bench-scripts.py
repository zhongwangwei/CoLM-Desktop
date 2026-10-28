#!/usr/bin/env python3
"""`crates/colm-bench/src/scripts/{inspect,prepare}.py` 的独立回归测试（python3 + numpy + xarray，不用 pytest）。

比较页的这两个脚本在本机是 `python -c <源代码>` 执行、在服务器是 heredoc 喂给 `python -`，
出问题只能靠合成 NetCDF 先把每一类输入走一遍。这里构造用例、实际起子进程执行脚本、检查
stdout 的 JSON / stderr 的错误 / 并出来的 NetCDF 与站点清单：

    python3 scripts/test-bench-scripts.py

每类用例对应比较页审查报告第 2、4 条里的一个现状缺陷：
主 history 与 tracer 同目录、每天一个文件、逐年文件、patch=2、单格 lat×lon 网格、
element 向量、土层变量、同目录混前缀、0–360 经度、跨日界线、缺坐标 + 经纬度覆盖、
站点名/路径带逗号、跨文件重复时刻按变量合并。
"""

import csv
import json
import os
import shutil
import subprocess
import sys
import tempfile
import traceback

try:
    import numpy as np
    import xarray as xr
except ImportError as error:  # pragma: no cover - 本机环境已装；缺了给清楚提示
    print(f"this test needs python3 with numpy and xarray: {error}", file=sys.stderr)
    sys.exit(2)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INSPECT = os.path.join(ROOT, "crates", "colm-bench", "src", "scripts", "inspect.py")
PREPARE = os.path.join(ROOT, "crates", "colm-bench", "src", "scripts", "prepare.py")
PYTHON = sys.executable

TESTS = []


def test(function):
    """登记一个用例；每个用例拿到一个空的临时目录。"""
    TESTS.append(function)
    return function


# ---------------------------------------------------------------- 子进程与断言

def run(script, *arguments):
    return subprocess.run(
        [PYTHON, script, *[str(argument) for argument in arguments]],
        capture_output=True,
        text=True,
    )


def run_stdin(script, *arguments):
    """服务器上的真实调用方式：脚本走 stdin，参数在命令行。"""
    with open(script, "r", encoding="utf-8") as handle:
        source = handle.read()
    return subprocess.run(
        [PYTHON, "-", *[str(argument) for argument in arguments]],
        input=source,
        capture_output=True,
        text=True,
    )


def run_dash_c(script, *arguments):
    """本机的真实调用方式：`python -c <源代码> 参数...`。"""
    with open(script, "r", encoding="utf-8") as handle:
        source = handle.read()
    return subprocess.run(
        [PYTHON, "-c", source, *[str(argument) for argument in arguments]],
        capture_output=True,
        text=True,
    )


def ok(result, what):
    assert result.returncode == 0, f"{what} failed ({result.returncode}):\n{result.stderr}"
    return result


def failed(result, what):
    assert result.returncode != 0, f"{what} should fail but exited 0:\n{result.stdout}"
    return result.stderr


def inspect(folder, prefix=None):
    arguments = [folder] + ([prefix] if prefix is not None else [])
    result = ok(run(INSPECT, *arguments), f"inspect {folder}")
    # 窗口进程对 stdout 整段反序列化：这里也必须整段能解析，而不是「最后一行是 JSON」。
    return json.loads(result.stdout)


def inspect_stderr(folder, prefix=None):
    arguments = [folder] + ([prefix] if prefix is not None else [])
    return failed(run(INSPECT, *arguments), f"inspect {folder}")


def prepare(folder, prefix, out_dir, station="AT-Neu", lon=None, lat=None):
    out_nc = os.path.join(out_dir, "sim.nc")
    out_csv = os.path.join(out_dir, "sim.csv")
    arguments = [folder, prefix, out_nc, out_csv, station]
    if lon is not None and lat is not None:
        arguments.extend([lon, lat])
    result = run(PREPARE, *arguments)
    return result, out_nc, out_csv


def prepare_ok(folder, prefix, out_dir, station="AT-Neu", lon=None, lat=None):
    result, out_nc, out_csv = prepare(folder, prefix, out_dir, station, lon, lat)
    ok(result, f"prepare {folder}")
    return result, out_nc, out_csv


def prepare_stderr(folder, prefix, out_dir, station="AT-Neu", lon=None, lat=None):
    result, _, _ = prepare(folder, prefix, out_dir, station, lon, lat)
    return failed(result, f"prepare {folder}")


def approx(actual, expected, what, tolerance=1e-6):
    assert actual is not None, f"{what}: expected ~{expected}, got None"
    assert abs(float(actual) - float(expected)) <= tolerance, f"{what}: expected ~{expected}, got {actual}"


def variable_names(dataset):
    return [name for name in dataset.data_vars]


# ---------------------------------------------------------------- 造 NetCDF

def times(start, count, step_minutes):
    """从 start 起、每 step_minutes 一条，共 count 条（datetime64，xarray 会按 CF 编码写出）。"""
    base = np.datetime64(start, "s")
    return base + np.arange(count) * np.timedelta64(step_minutes, "m")


def write_station(path, time_values, variables, patch=1, coordinate=(20.5, 110.25), extra_sizes=None):
    """站点文件：patch 维 + 0 维 lat/lon（与 CoLM 的写出方式一致），变量维度是 time/patch/层。"""
    sizes = {"time": np.asarray(time_values).size}
    if patch is not None:
        sizes["patch"] = patch
    if extra_sizes:
        sizes.update(extra_sizes)
    data = {}
    for name, dims in variables.items():
        shape = tuple(sizes[dim] for dim in dims)
        data[name] = (dims, np.arange(max(1, int(np.prod(shape))), dtype=float).reshape(shape))
    dataset = xr.Dataset(data, coords={"time": np.asarray(time_values)})
    if patch is not None and coordinate is not None:
        dataset["lat"] = np.float64(coordinate[0])
        dataset["lon"] = np.float64(coordinate[1])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    dataset.to_netcdf(path)


def write_grid(path, time_values, lat, lon, variables, boundaries=True):
    """格点文件：lat/lon 是维度坐标（单格也是 grid），另写可选的格子边界变量。"""
    lat = np.asarray(lat, dtype=float)
    lon = np.asarray(lon, dtype=float)
    sizes = {"time": np.asarray(time_values).size, "lat": lat.size, "lon": lon.size}
    data = {}
    for name, dims in variables.items():
        shape = tuple(sizes[dim] for dim in dims)
        data[name] = (dims, np.arange(max(1, int(np.prod(shape))), dtype=float).reshape(shape))
    dataset = xr.Dataset(
        data, coords={"time": np.asarray(time_values), "lat": lat, "lon": lon}
    )
    if boundaries:
        dataset["lat_s"] = ("lat", lat - 0.5)
        dataset["lat_n"] = ("lat", lat + 0.5)
        dataset["lon_w"] = ("lon", lon - 0.5)
        dataset["lon_e"] = ("lon", lon + 0.5)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    dataset.to_netcdf(path)


def write_vector(path, time_values, elements=3):
    """非结构网格：element 维（河流/向量写出），没有 patch 也没有 lat/lon 维。"""
    dataset = xr.Dataset(
        {
            "elmindex": ("element", np.arange(elements)),
            "f_xy_t": (("time", "element"), np.arange(np.asarray(time_values).size * elements, dtype=float).reshape(-1, elements)),
        },
        coords={"time": np.asarray(time_values)},
    )
    os.makedirs(os.path.dirname(path), exist_ok=True)
    dataset.to_netcdf(path)


# ---------------------------------------------------------------- 用例

@test
def main_and_tracer_families(workdir):
    """同一目录里主 history 与 tracer：只探测/并主族，tracer 单独报告，时刻不重复。"""
    folder = os.path.join(workdir, "history")
    hourly = times("2000-01-01T00:30", 3, 60)
    february = times("2000-02-01T00:30", 3, 60)
    write_station(os.path.join(folder, "AT-Neu_hist_2000-01.nc"), hourly, {"f_xy_t": ("time", "patch")})
    write_station(os.path.join(folder, "AT-Neu_hist_2000-02.nc"), february, {"f_xy_t": ("time", "patch")})
    write_station(
        os.path.join(folder, "AT-Neu_hist_tracer_2000-01.nc"), hourly, {"f_trc_co2": ("time", "patch")}
    )
    write_station(
        os.path.join(folder, "AT-Neu_hist_tracer_2000-02.nc"), february, {"f_trc_co2": ("time", "patch")}
    )

    info = inspect(folder)
    assert info["prefix"] == "AT-Neu_hist_", info["prefix"]
    assert info["files"] == 2, info["files"]
    assert info["spatial_type"] == "station", info["spatial_type"]
    assert info["patch"] == 1, info["patch"]
    assert info["tim_res"] == "Hour", info["tim_res"]
    assert info["data_groupby"] == "Month", info["data_groupby"]
    assert [entry["name"] for entry in info["variables"]] == ["f_xy_t"], info["variables"]
    families = {entry["family"]: entry for entry in info["families"]}
    assert set(families) == {"main", "tracer"}, families
    assert families["tracer"]["files"] == 2, families
    assert info["comparable"] is True
    assert info["syear"] == 2000 and info["eyear"] == 2000

    result, out_nc, _ = prepare_ok(folder, info["prefix"], workdir)
    with xr.open_dataset(out_nc) as merged:
        assert merged.sizes["time"] == 6, dict(merged.sizes)
        assert "patch" not in merged.sizes, dict(merged.sizes)
        assert "f_trc_co2" not in merged.data_vars, variable_names(merged)
        assert "f_xy_t" in merged.data_vars, variable_names(merged)
        stamps = np.sort(merged["time"].values)
        assert np.unique(stamps).size == 6, "duplicate timestamps reached the merged file"
    assert "2 files" in result.stdout, result.stdout


@test
def daily_files_with_one_record_each(workdir):
    """每天一个文件、每文件一条记录：时间轴要跨文件采样才判得出 Day。"""
    folder = os.path.join(workdir, "history")
    for day in (1, 2, 3):
        write_station(
            os.path.join(folder, f"Daily_hist_2000-01-0{day}.nc"),
            times(f"2000-01-0{day}T00:30", 1, 60),
            {"f_xy_t": ("time", "patch")},
        )
    info = inspect(folder)
    assert info["tim_res"] == "Day", info["tim_res"]
    assert info["data_groupby"] == "Day", info["data_groupby"]
    assert info["files"] == 3, info["files"]
    _, out_nc, _ = prepare_ok(folder, info["prefix"], workdir)
    with xr.open_dataset(out_nc) as merged:
        assert merged.sizes["time"] == 3, dict(merged.sizes)


@test
def yearly_files_are_year_not_month(workdir):
    """逐年一个文件、每文件一条记录：旧逻辑按首条记录默认月级；这里要判 Year。"""
    folder = os.path.join(workdir, "history")
    for year in (2000, 2001, 2002):
        write_station(
            os.path.join(folder, f"Yearly_hist_{year}.nc"),
            times(f"{year}-07-02T00:00", 1, 60),
            {"f_xy_t": ("time", "patch")},
        )
    info = inspect(folder)
    assert info["tim_res"] == "Year", info["tim_res"]
    assert info["data_groupby"] == "Year", info["data_groupby"]
    assert info["syear"] == 2000 and info["eyear"] == 2002


@test
def monthly_files_are_month(workdir):
    """每月一个文件（每文件一条）：月级分组要判 Month。"""
    folder = os.path.join(workdir, "history")
    for month, day in (("01", 16), ("02", 15), ("03", 16)):
        write_station(
            os.path.join(folder, f"Monthly_hist_2000-{month}.nc"),
            times(f"2000-{month}-{day:02d}T00:30", 1, 60),
            {"f_xy_t": ("time", "patch")},
        )
    info = inspect(folder)
    assert info["tim_res"] == "Month", info["tim_res"]
    assert info["data_groupby"] == "Month", info["data_groupby"]


@test
def three_hourly_single_file(workdir):
    """3 小时步长映射 3Hour；单文件 data_groupby 是 single。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Three_hist_2000-01.nc"),
        times("2000-01-01T00:30", 3, 180),
        {"f_xy_t": ("time", "patch")},
    )
    info = inspect(folder)
    assert info["tim_res"] == "3Hour", info["tim_res"]
    assert info["data_groupby"] == "single", info["data_groupby"]


@test
def single_record_resolution_comes_from_filename(workdir):
    """只有一个时刻时时间轴无从采样：按文件名后缀给出 tim_res，并明确标 tim_res_source=filename。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Solo_hist_2000-01.nc"),
        times("2000-01-16T00:30", 1, 60),
        {"f_xy_t": ("time", "patch")},
    )
    info = inspect(folder)
    assert info["files"] == 1 and info["data_groupby"] == "single", info
    assert info["tim_res"] == "Month", info["tim_res"]
    assert info["tim_res_source"] == "filename", info["tim_res_source"]
    assert any("inferred" in warning for warning in info["warnings"]), info["warnings"]


@test
def mixed_case_prefixes_are_an_error(workdir):
    """同一目录混着 A（2000）与 Z（2010）两套前缀：不得混成 2000–2010，报错列候选。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "A_hist_2000.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(folder, "Z_hist_2010.nc"),
        times("2010-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    stderr = inspect_stderr(folder)
    assert "A_hist_" in stderr and "Z_hist_" in stderr, stderr
    assert "multiple main history prefixes" in stderr, stderr
    # 给第二个参数（前缀）后要能只看其中一套。
    info = inspect(folder, "A_hist_")
    assert info["prefix"] == "A_hist_", info["prefix"]
    assert info["syear"] == 2000 and info["eyear"] == 2000, (info["syear"], info["eyear"])


@test
def nested_case_directory_resolves_history(workdir):
    """给算例目录时到 out/<名字>/history 找；多个算例输出时列候选。"""
    case = os.path.join(workdir, "case")
    folder = os.path.join(case, "out", "some-case", "history")
    write_station(
        os.path.join(folder, "Nested_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    info = inspect(case)
    assert os.path.basename(info["history_dir"]) == "history", info["history_dir"]
    assert info["prefix"] == "Nested_hist_", info["prefix"]

    second = os.path.join(case, "out", "other-case", "history")
    write_station(
        os.path.join(second, "Nested_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    stderr = inspect_stderr(case)
    assert "multiple case histories" in stderr, stderr


@test
def single_cell_grid_is_grid_not_station(workdir):
    """1×1 的 lat×lon 网格：显式判 grid，范围按格子边界归一化，0–360 经度要落在 ±180 内。"""
    folder = os.path.join(workdir, "history")
    write_grid(
        os.path.join(folder, "Grid_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[20.0],
        lon=[200.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
    )
    info = inspect(folder)
    assert info["spatial_type"] == "grid", info["spatial_type"]
    assert info["spatial"] is True
    assert info["patch"] is None, info["patch"]
    assert info["lon"] is None and info["lat"] is None, (info["lon"], info["lat"])
    approx(info["resolution"], 1.0, "resolution")
    extent = info["extent"]
    assert extent is not None, info
    approx(extent[0], -160.5, "extent west")
    approx(extent[1], -159.5, "extent east")
    approx(extent[2], 19.5, "extent south")
    approx(extent[3], 20.5, "extent north")
    assert extent[0] < extent[1] and extent[2] < extent[3], extent
    assert all(-180.0 <= extent[index] <= 180.0 for index in (0, 1)), extent
    assert info["crosses_dateline"] is False
    assert info["comparable"] is True


@test
def zero_to_360_longitudes_are_normalized(workdir):
    """0–360 经度（200/201/202）归一成 -160/-159/-158；extent 必须是合法 min<max。"""
    folder = os.path.join(workdir, "history")
    write_grid(
        os.path.join(folder, "Pacific_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[19.0, 20.0, 21.0],
        lon=[200.0, 201.0, 202.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
        boundaries=False,
    )
    info = inspect(folder)
    assert info["spatial_type"] == "grid", info["spatial_type"]
    extent = info["extent"]
    assert extent is not None, info
    approx(extent[0], -160.5, "extent west")
    approx(extent[1], -157.5, "extent east")
    assert extent[0] < extent[1]
    assert all(-180.0 <= extent[index] <= 180.0 for index in (0, 1)), extent
    approx(info["resolution"], 1.0, "resolution")


@test
def dateline_crossing_is_marked_not_mixed(workdir):
    """[-179,179]（以及 0–360 的 179/181）是跨日界线相邻：分辨率 2°，extent 不能是 358°。"""
    folder = os.path.join(workdir, "history")
    write_grid(
        os.path.join(folder, "Two_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[20.0],
        lon=[-179.0, 179.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
    )
    info = inspect(folder)
    assert info["spatial_type"] == "grid", info["spatial_type"]
    assert info["crosses_dateline"] is True
    assert info["extent"] is None, info["extent"]
    approx(info["resolution"], 2.0, "resolution")
    assert info["comparable"] is False
    assert any(blocker["code"] == "dateline_crossing" for blocker in info["blockers"]), info["blockers"]

    other = os.path.join(workdir, "history-360")
    write_grid(
        os.path.join(other, "Two360_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[20.0],
        lon=[179.0, 181.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
    )
    info = inspect(other)
    assert info["crosses_dateline"] is True, info
    assert info["extent"] is None, info["extent"]
    approx(info["resolution"], 2.0, "resolution (0-360 spelling)")


@test
def multi_patch_is_rejected_explicitly(workdir):
    """patch=2：探测报 patch 类型与原因；制备明确报错，不许 squeeze 崩。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Patchy_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
        patch=2,
    )
    info = inspect(folder)
    assert info["spatial_type"] == "patch", info["spatial_type"]
    assert info["patch"] == 2, info["patch"]
    assert info["comparable"] is False
    assert any(blocker["code"] == "multi_patch" for blocker in info["blockers"]), info["blockers"]
    stderr = prepare_stderr(folder, info["prefix"], workdir)
    assert "patch=2" in stderr and "multi-patch" in stderr, stderr


@test
def element_vector_is_unstructured(workdir):
    """element 向量：显式判 unstructured，制备拒绝（旧实现过滤完变量后 KeyError: time）。"""
    folder = os.path.join(workdir, "history")
    write_vector(os.path.join(folder, "River_hist_2000-01.nc"), times("2000-01-01T00:30", 2, 60))
    info = inspect(folder)
    assert info["spatial_type"] == "unstructured", info["spatial_type"]
    assert info["comparable"] is False
    assert any(blocker["code"] == "unstructured" for blocker in info["blockers"]), info["blockers"]
    stderr = prepare_stderr(folder, "River_hist_", workdir)
    assert "unstructured" in stderr, stderr


@test
def layered_variables_are_reported_and_skipped(workdir):
    """土层/类别维变量：探测列进 unsupported_variables；制备打印跳过原因，不静默丢。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Layered_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {
            "f_xy_t": ("time", "patch"),
            "f_wliq_soisno": ("time", "patch", "soil"),
            "f_alb": ("time", "patch", "rtyp", "band"),
        },
        extra_sizes={"soil": 10, "rtyp": 2, "band": 2},
    )
    info = inspect(folder)
    names = [entry["name"] for entry in info["unsupported_variables"]]
    assert "f_wliq_soisno" in names, info["unsupported_variables"]
    assert "f_alb" in names, info["unsupported_variables"]
    extra = {entry["name"]: entry["extra_dims"] for entry in info["unsupported_variables"]}
    assert extra["f_wliq_soisno"] == ["soil"], extra
    assert extra["f_alb"] == ["rtyp", "band"], extra
    dims = {entry["name"]: entry["dims"] for entry in info["variables"]}
    assert dims["f_wliq_soisno"] == ["time", "patch", "soil"], dims
    assert info["comparable"] is True

    result, out_nc, _ = prepare_ok(folder, info["prefix"], workdir)
    assert "f_wliq_soisno" in result.stdout and "skipped" in result.stdout, result.stdout
    with xr.open_dataset(out_nc) as merged:
        assert "f_wliq_soisno" not in merged.data_vars, variable_names(merged)
        assert "f_xy_t" in merged.data_vars, variable_names(merged)
        assert merged.sizes["time"] == 2


@test
def missing_coordinates_need_override(workdir):
    """文件没有 lat/lon：没有覆盖参数时明确报错；给了覆盖参数就按覆盖值并归一化经度。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "NoCoord_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
        coordinate=None,
    )
    info = inspect(folder)
    assert info["spatial_type"] == "station", info["spatial_type"]
    assert info["lon"] is None and info["lat"] is None, (info["lon"], info["lat"])
    assert any("no coordinates" in warning for warning in info["warnings"]), info["warnings"]

    stderr = prepare_stderr(folder, info["prefix"], workdir)
    assert "no lon/lat" in stderr and "LON LAT" in stderr, stderr

    result, out_nc, out_csv = prepare_ok(
        folder, info["prefix"], workdir, lon=200.5, lat=-20.0
    )
    with xr.open_dataset(out_nc) as merged:
        approx(float(merged["lon"]), -159.5, "merged lon")
        approx(float(merged["lat"]), -20.0, "merged lat")
    with open(out_csv, newline="", encoding="utf-8") as handle:
        row = list(csv.reader(handle))[1]
    assert len(row) == 6, row
    approx(float(row[3]), -159.5, "csv LON")
    approx(float(row[4]), -20.0, "csv LAT")
    assert "2 steps" in result.stdout, result.stdout


@test
def comma_in_station_and_path_survives_csv(workdir):
    """站点名与输出路径含逗号：csv.writer 要写成一个六列 CSV，不能裂成七列。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Comma_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    out_dir = os.path.join(workdir, "out,commas")
    _, out_nc, out_csv = prepare_ok(
        folder, "Comma_hist_", out_dir, station="AT,Neu"
    )
    with open(out_csv, newline="", encoding="utf-8") as handle:
        rows = list(csv.reader(handle))
    assert len(rows) == 2, rows
    assert len(rows[0]) == 6 and len(rows[1]) == 6, rows
    assert rows[1][0] == "AT,Neu", rows[1]
    assert rows[1][5] == os.path.abspath(out_nc), rows[1]
    assert "," in os.path.basename(os.path.dirname(rows[1][5])), rows[1]


@test
def prepare_sorts_and_merges_overlapping_times(workdir):
    """文件按名字排序 ≠ 按时间排序，且同一时刻各自带不同变量：要排序、去重、按坐标合并。"""
    folder = os.path.join(workdir, "history")
    # 名字序：`2000-01-02.nc` 排在 `2000-01.nc` 前面（'-' < '.'），时间序相反。
    write_station(
        os.path.join(folder, "Overlap_hist_2000-01.nc"),
        np.array(["2000-01-01T00:30", "2000-01-02T00:30"], dtype="datetime64[s]"),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(folder, "Overlap_hist_2000-01-02.nc"),
        np.array(["2000-01-02T00:30", "2000-01-03T00:30"], dtype="datetime64[s]"),
        {"f_xy_q": ("time", "patch")},
    )
    _, out_nc, _ = prepare_ok(folder, "Overlap_hist_", workdir)
    with xr.open_dataset(out_nc) as merged:
        assert merged.sizes["time"] == 3, dict(merged.sizes)
        stamps = merged["time"].values
        assert np.all(np.diff(stamps) > np.timedelta64(0, "s")), "time axis is not sorted"
        assert np.unique(stamps).size == 3, "duplicate timestamps reached the merged file"
        values_t = merged["f_xy_t"].values.ravel()
        values_q = merged["f_xy_q"].values.ravel()
        # 中间那一秒两个文件都有：t 来自第一个文件、q 来自第二个，都要是非 NaN。
        assert np.isfinite(values_t[1]) and np.isfinite(values_q[1]), (values_t, values_q)
        assert np.isnan(values_q[0]) and np.isnan(values_t[2]), (values_t, values_q)


@test
def unknown_spatial_type_is_rejected(workdir):
    """既没有 patch、也没有坐标：unknown，上层要拒绝（不得显示可比较再 KeyError）。"""
    folder = os.path.join(workdir, "history")
    dataset = xr.Dataset(
        {"f_xy_t": ("time", np.arange(2, dtype=float))},
        coords={"time": times("2000-01-01T00:30", 2, 60)},
    )
    os.makedirs(folder, exist_ok=True)
    dataset.to_netcdf(os.path.join(folder, "Odd_hist_2000.nc"))
    info = inspect(folder)
    assert info["spatial_type"] == "unknown", info["spatial_type"]
    assert info["comparable"] is False
    assert any(blocker["code"] == "unknown_spatial_type" for blocker in info["blockers"]), info["blockers"]
    stderr = prepare_stderr(folder, info["prefix"], workdir)
    assert "cannot tell" in stderr, stderr


@test
def grid_history_is_not_prepared_as_station(workdir):
    """格点 history 不能并成站点文件：明确报错指向直接按格点比较。"""
    folder = os.path.join(workdir, "history")
    write_grid(
        os.path.join(folder, "Grid_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[19.0, 20.0],
        lon=[110.0, 111.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
    )
    stderr = prepare_stderr(folder, "Grid_hist_", workdir)
    assert "gridded" in stderr, stderr


@test
def only_tracer_files_report_family_error(workdir):
    """目录里只有 tracer 文件：报「没有主 history」并列出发现了哪些族，不静默当空目录。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Only_hist_tracer_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_trc_co2": ("time", "patch")},
    )
    stderr = inspect_stderr(folder)
    assert "no main history files" in stderr and "tracer" in stderr, stderr


@test
def remote_stdin_and_local_dash_c_invocations(workdir):
    """服务器 heredoc（stdin）与本机 `-c` 两种真实调用方式都与直接执行结果一致。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Invoke_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {"f_xy_t": ("time", "patch")},
    )
    direct = ok(run(INSPECT, folder), "inspect direct")
    by_stdin = ok(run_stdin(INSPECT, folder), "inspect via stdin")
    by_dash_c = ok(run_dash_c(INSPECT, folder), "inspect via -c")
    assert json.loads(direct.stdout) == json.loads(by_stdin.stdout), "stdin invocation differs"
    assert json.loads(direct.stdout) == json.loads(by_dash_c.stdout), "-c invocation differs"

    out_dir = os.path.join(workdir, "sim")
    local, out_nc, out_csv = prepare_ok(folder, "Invoke_hist_", out_dir, station="Invoke")
    for label, invoke in (
        ("stdin", run_stdin),
        ("-c", run_dash_c),
    ):
        target = os.path.join(workdir, f"sim-{label}")
        result = invoke(
            PREPARE,
            folder,
            "Invoke_hist_",
            os.path.join(target, "sim.nc"),
            os.path.join(target, "sim.csv"),
            "Invoke",
        )
        ok(result, f"prepare via {label}")
    with xr.open_dataset(out_nc) as merged:
        assert np.isfinite(float(merged["lon"])), "prepared file has no lon"


# ---------------------------------------------------------------- 主流程

def main():
    passed = 0
    failures = []
    for function in TESTS:
        workdir = tempfile.mkdtemp(prefix="colm-bench-scripts-")
        try:
            function(workdir)
        except Exception:  # noqa: BLE001 - 测试需要把任意断言/子进程失败都收进摘要
            failures.append((function.__name__, traceback.format_exc()))
            print(f"[FAIL] {function.__name__}")
        else:
            passed += 1
            print(f"[PASS] {function.__name__}")
        finally:
            shutil.rmtree(workdir, ignore_errors=True)
    print(f"\n{passed} passed, {len(failures)} failed, {len(TESTS)} total")
    for name, detail in failures:
        print(f"\n--- {name} ---\n{detail}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
