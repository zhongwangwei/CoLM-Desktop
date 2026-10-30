#!/usr/bin/env python3
"""`crates/colm-bench/src/scripts/{inspect,prepare}.py` 的独立回归测试（python3 + numpy + xarray，不用 pytest）。

比较页的这两个脚本在本机是 `python -c <源代码>` 执行、在服务器是 heredoc 喂给 `python -`，
出问题只能靠合成 NetCDF 先把每一类输入走一遍。这里构造用例、实际起子进程执行脚本、检查
stdout 的 JSON / stderr 的错误 / 并出来的 NetCDF 与站点清单：

    python3 scripts/test-bench-scripts.py

每类用例对应比较页审查报告里的一个现状缺陷：
主 history 与 tracer 同目录、每天一个文件、逐年文件、patch=2（含诊断文案）、单格 lat×lon 网格、
element 向量、层维度变量（soilsnow/soil/lake 保留给 OpenBench profile compute，类别维仍跳过）、
同目录混前缀、0–360 经度、跨日界线（含中心 180° 的 1×1 格）、缺坐标 + 经纬度覆盖、
站点名/路径带逗号、跨文件重复时刻按变量合并；C04 的频率判定：
单时刻读频率元数据（source=metadata）、元数据与实测冲突以实测为准、亚小时输出粗化成 Hour、
6 小时/8 天档位、2/4 小时与 6 小时不落错档（R08）、以及落不到任何档位的步长明确报错；
R06 的非公历日历：cftime 解码出的 noleap/360_day 时间轴仍要能采样步长与判频率，制备端也要能
排序/去重/保留 units+calendar 编码/按日历读 CSV 年份，混日历或混标准与 cftime 明确报错。
"""

import csv
import datetime
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

try:
    import cftime
except ImportError:  # pragma: no cover - 只有非公历用例需要；缺了在那个用例里明确说
    cftime = None

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


def show_tim_res(info):
    """打印 C04 新增的字段：跑门禁时能直接看到判定来源、实测值与元数据值。"""
    print(
        f"    tim_res={info['tim_res']!r} source={info['tim_res_source']!r} "
        f"metadata={info['tim_res_metadata']!r}"
    )


# ---------------------------------------------------------------- 造 NetCDF

def times(start, count, step_minutes):
    """从 start 起、每 step_minutes 一条，共 count 条（datetime64，xarray 会按 CF 编码写出）。"""
    base = np.datetime64(start, "s")
    return base + np.arange(count) * np.timedelta64(step_minutes, "m")


def write_station(
    path,
    time_values,
    variables,
    patch=1,
    coordinate=(20.5, 110.25),
    extra_sizes=None,
    extra_coords=None,
    attrs=None,
    time_attrs=None,
):
    """站点文件：patch 维 + 0 维 lat/lon（与 CoLM 的写出方式一致），变量维度是 time/patch/层。

    attrs / time_attrs 用来在 dataset 全局属性或 time 变量属性上造频率元数据（C04 用例）。
    extra_coords 造像 CoLM 那样的层维度坐标变量（ncio_write_colm_dimension 把 soilsnow/soil/lake
    写成 1-D 坐标，值就是层号），层维度保留后输出必须原样带着它们。
    """
    sizes = {"time": np.asarray(time_values).size}
    if patch is not None:
        sizes["patch"] = patch
    if extra_sizes:
        sizes.update(extra_sizes)
    data = {}
    for name, dims in variables.items():
        shape = tuple(sizes[dim] for dim in dims)
        data[name] = (dims, np.arange(max(1, int(np.prod(shape))), dtype=float).reshape(shape))
    coords = {"time": np.asarray(time_values)}
    if extra_coords:
        coords.update(extra_coords)
    dataset = xr.Dataset(data, coords=coords)
    if attrs:
        dataset.attrs.update(attrs)
    if time_attrs:
        dataset["time"].attrs.update(time_attrs)
    if patch is not None and coordinate is not None:
        dataset["lat"] = np.float64(coordinate[0])
        dataset["lon"] = np.float64(coordinate[1])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    dataset.to_netcdf(path)


def write_grid(
    path,
    time_values,
    lat,
    lon,
    variables,
    boundaries=True,
    lon_bounds=None,
    lat_bounds=None,
):
    """格点文件：lat/lon 是维度坐标（单格也是 grid），另写可选的格子边界变量。

    lon_bounds/lat_bounds 是 (下边界, 上边界) 的显式覆盖：真机文件里跨日界线的格子可能把
    东边界写成 -179.5，默认的 lon+0.5 写不出这种写法。
    """
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
        lat_s, lat_n = lat_bounds if lat_bounds is not None else (lat - 0.5, lat + 0.5)
        lon_w, lon_e = lon_bounds if lon_bounds is not None else (lon - 0.5, lon + 0.5)
        dataset["lat_s"] = ("lat", np.asarray(lat_s, dtype=float))
        dataset["lat_n"] = ("lat", np.asarray(lat_n, dtype=float))
        dataset["lon_w"] = ("lon", np.asarray(lon_w, dtype=float))
        dataset["lon_e"] = ("lon", np.asarray(lon_e, dtype=float))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    dataset.to_netcdf(path)


def cftime_axis(calendar, start, count, step_minutes):
    """非公历时间轴：按日历构造 cftime 对象，xarray 写文件时会把 calendar 落成属性。

    start 是 (年, 月, 日, 时, 分)。不能用 datetime64 代替：那样写出来的就是标准日历，
    根本走不到 R06 里 cftime 的分支。
    """
    base = cftime.datetime(*start, calendar=calendar)
    return np.array(
        [base + datetime.timedelta(minutes=step_minutes * index) for index in range(count)],
        dtype=object,
    )


def cftime_months(calendar, year, months, day, minute=30):
    """按日历构造每月一条的时间轴，用来验证非固定月长下的中位步长。"""
    return np.array(
        [cftime.datetime(year, month, day, 0, minute, calendar=calendar) for month in months],
        dtype=object,
    )


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
    assert info["tim_res_metadata"] is None, info["tim_res_metadata"]


@test
def single_time_step_metadata_beats_filename(workdir):
    """单时刻 + 明确频率元数据：用元数据（source=metadata），不再按文件名后缀猜。"""
    folder = os.path.join(workdir, "history")
    # 文件名后缀 2000-01 按旧逻辑会猜 Month；dataset 属性明确写 daily，必须优先用元数据。
    write_station(
        os.path.join(folder, "MetaSolo_hist_2000-01.nc"),
        times("2000-01-16T00:00", 1, 60),
        {"f_xy_t": ("time", "patch")},
        attrs={"frequency": "daily"},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["files"] == 1 and info["data_groupby"] == "single", info
    assert info["tim_res"] == "Day", info["tim_res"]
    assert info["tim_res_source"] == "metadata", info["tim_res_source"]
    assert info["tim_res_metadata"] == "Day", info["tim_res_metadata"]
    # 单时刻只剩元数据一条证据：warnings 要讲清来源，让界面提示用户确认。
    assert any("frequency metadata" in warning for warning in info["warnings"]), info["warnings"]
    assert not any("inferred from the file name" in warning for warning in info["warnings"]), info[
        "warnings"
    ]


@test
def frequency_metadata_spellings_are_understood(workdir):
    """常见频率写法（大小写不敏感、含 CF cell_methods interval）都要落到正确档位。"""
    cases = [
        ({"frequency": "hourly"}, None, "Hour"),
        ({"frequency": "1h"}, None, "Hour"),
        ({"tim_res": "3600"}, None, "Hour"),
        ({"frequency": 3600}, None, "Hour"),
        ({"Frequency": "DAILY"}, None, "Day"),
        ({"frequency": "1d"}, None, "Day"),
        ({"frequency": "3-hourly"}, None, "3Hour"),
        ({"frequency": "3Hour"}, None, "3Hour"),
        (None, {"cell_methods": "time: mean (interval: 6 hours)"}, "6Hour"),
        ({"frequency": "8-day"}, None, "8Day"),
        ({"frequency": "monthly"}, None, "Month"),
        ({"frequency": "mon"}, None, "Month"),
        ({"frequency": "yearly"}, None, "Year"),
        ({"frequency": "annual"}, None, "Year"),
    ]
    for index, (attrs, time_attrs, expected) in enumerate(cases):
        folder = os.path.join(workdir, f"history-{index}")
        write_station(
            os.path.join(folder, "Spelling_hist_2000-01.nc"),
            times("2000-01-16T00:00", 1, 60),
            {"f_xy_t": ("time", "patch")},
            attrs=attrs,
            time_attrs=time_attrs,
        )
        info = inspect(folder)
        show_tim_res(info)
        assert info["tim_res"] == expected, (attrs, time_attrs, expected, info["tim_res"])
        assert info["tim_res_source"] == "metadata", (attrs, time_attrs, info["tim_res_source"])
        assert info["tim_res_metadata"] == expected, (attrs, time_attrs, info["tim_res_metadata"])


@test
def unusable_frequency_metadata_falls_back_to_filename(workdir):
    """元数据写的频率落不到 OpenBench 档位：不硬映射，退回文件名推断并说明忽略了元数据。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Fine_hist_2000-01.nc"),
        times("2000-01-16T00:00", 1, 60),
        {"f_xy_t": ("time", "patch")},
        attrs={"frequency": "30 minutes"},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "Month", info["tim_res"]  # 文件名后缀 2000-01
    assert info["tim_res_source"] == "filename", info["tim_res_source"]
    assert info["tim_res_metadata"] is None, info["tim_res_metadata"]
    assert any("ignored frequency metadata" in warning for warning in info["warnings"]), info["warnings"]
    assert any("inferred from the file name" in warning for warning in info["warnings"]), info["warnings"]


@test
def metadata_conflict_uses_measured_time_axis(workdir):
    """采样与元数据冲突：以实测时间轴为准（source=time），两个值都输出并在 warnings 说明冲突。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "Conflict_hist_2000-01.nc"),
        times("2000-01-01T00:00", 4, 60),  # 实测每小时
        {"f_xy_t": ("time", "patch")},
        time_attrs={"cell_methods": "time: mean (interval: 6 hours)"},  # 文件自述 6 小时
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "Hour", info["tim_res"]
    assert info["tim_res_source"] == "time", info["tim_res_source"]
    assert info["tim_res_metadata"] == "6Hour", info["tim_res_metadata"]
    conflict = [warning for warning in info["warnings"] if "frequency metadata says" in warning]
    assert conflict, info["warnings"]
    assert "6Hour" in conflict[0] and "sampled time axis is Hour" in conflict[0], conflict[0]


@test
def sub_hourly_output_is_coarsened_to_hour_with_warning(workdir):
    """亚小时输出（CoLM DEF_HIST_FREQ=TIMESTEP 默认 1800 s）没有 OpenBench 档位：报 Hour 并警告。

    本仓库 AU-Preston 等真实历史文件实测就是 1800 s；这里锁住"不因无法归类而整体报错"的行为，
    同时保证它不是静默的（warnings 明确说输出比 1 小时更细）。
    """
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "HalfHourly_hist_2000-01.nc"),
        times("2000-01-01T00:00", 4, 30),
        {"f_xy_t": ("time", "patch")},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "Hour", info["tim_res"]
    assert info["tim_res_source"] == "time", info["tim_res_source"]
    assert any("finer than 1 h" in warning for warning in info["warnings"]), info["warnings"]


@test
def six_hourly_history_maps_to_6hour(workdir):
    """6 小时步长 -> OpenBench 3.0.8 的 6Hour；元数据一致时只作佐证（source 仍是 time）。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "SixHourly_hist_2000-01.nc"),
        times("2000-01-01T00:00", 4, 6 * 60),
        {"f_xy_t": ("time", "patch")},
        attrs={"frequency": "6-hourly"},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "6Hour", info["tim_res"]
    assert info["tim_res_source"] == "time", info["tim_res_source"]
    assert info["tim_res_metadata"] == "6Hour", info["tim_res_metadata"]
    assert not any("frequency metadata says" in warning for warning in info["warnings"]), info["warnings"]


@test
def eight_day_history_maps_to_8day(workdir):
    """8 天步长 -> OpenBench 3.0.8 的 8Day（旧实现的档位表里没有它）。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "EightDay_hist_2000.nc"),
        times("2000-01-01T00:00", 4, 8 * 24 * 60),
        {"f_xy_t": ("time", "patch")},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "8Day", info["tim_res"]
    assert info["tim_res_source"] == "time", info["tim_res_source"]
    assert info["tim_res_metadata"] is None, info["tim_res_metadata"]


@test
def two_and_four_hour_steps_do_not_snap_to_three_hour(workdir):
    """R08：2 小时曾被判成 3Hour、6 小时曾被判成 Day；现在 2/4 小时明确报错、6 小时是 6Hour。

    2/4 小时不对应任何 OpenBench 档位。若按 3Hour 交给 OpenBench，它会把 2 小时一条的时间轴
    当 3 小时重采样，相位和周期都错；宁可报错让用户改输出频率或手填，也不能静默套相邻档位。
    """
    for label, step_minutes, seconds in (
        ("TwoHourly", 2 * 60, 7200),
        ("FourHourly", 4 * 60, 14400),
    ):
        folder = os.path.join(workdir, f"history-{label}")
        write_station(
            os.path.join(folder, f"{label}_hist_2000-01.nc"),
            times("2000-01-01T00:00", 4, step_minutes),
            {"f_xy_t": ("time", "patch")},
        )
        stderr = inspect_stderr(folder)
        assert "cannot map" in stderr and str(seconds) in stderr, (label, stderr)

    # 6 小时必须落 6Hour；旧档位表曾把它判成 Day（日档的下界容差太宽）。这里同时锁住
    # "不是 Day" 和元数据一致时 source=time。
    folder = os.path.join(workdir, "history-6h")
    write_station(
        os.path.join(folder, "SixHourly_hist_2000-01.nc"),
        times("2000-01-01T00:00", 4, 6 * 60),
        {"f_xy_t": ("time", "patch")},
        attrs={"frequency": "6-hourly"},
    )
    info = inspect(folder)
    show_tim_res(info)
    assert info["tim_res"] == "6Hour", info["tim_res"]
    assert info["tim_res"] != "Day", info["tim_res"]
    assert info["tim_res_source"] == "time", info["tim_res_source"]


@test
def non_gregorian_calendars_are_probed(workdir):
    """R06：noleap/360_day 经 xarray 解码成 cftime 对象后，探测仍要能采样时间轴并判频率。

    旧实现只认 datetime64（dtype.kind != "M" 直接 SystemExit），即使放过也会在 np.diff/
    astype/排序处抛裸 TypeError。这里用真实 cftime 编码的 NetCDF 覆盖两种日历：固定步长与
    月长随日历变化的步长都要算对，标准日历的用例不受影响（其余用例就是那部分回归）。
    """
    if cftime is None:
        raise AssertionError(
            "the non-Gregorian regression needs cftime; install it (pip install cftime)"
        )
    for calendar in ("noleap", "360_day"):
        folder = os.path.join(workdir, f"history-{calendar}-6h")
        write_station(
            os.path.join(folder, "Calendar_hist_2000-01.nc"),
            cftime_axis(calendar, (2000, 1, 1, 0, 30), 4, 6 * 60),
            {"f_xy_t": ("time", "patch")},
            attrs={"frequency": "6-hourly"},
        )
        info = inspect(folder)
        show_tim_res(info)
        assert info["tim_res"] == "6Hour", (calendar, info["tim_res"])
        assert info["tim_res_source"] == "time", (calendar, info["tim_res_source"])
        assert info["tim_res_metadata"] == "6Hour", (calendar, info["tim_res_metadata"])
        assert info["syear"] == 2000 and info["eyear"] == 2000, (
            calendar,
            info["syear"],
            info["eyear"],
        )
        # family_report 也读年份；cftime 没有 datetime64 表示，走错分支会被 except 吞成 None，
        # 所以这里显式核对，别让"族报告静默没年份"混过去。
        families = {entry["family"]: entry for entry in info["families"]}
        assert families["main"]["syear"] == 2000, (calendar, families["main"])

        # 月长随日历变化：noleap 的 2 月 28 天、360_day 恒 30 天。中位步长仍应落到 Month；
        # 若退回 np.diff(...)/np.timedelta64 就会在这里抛 TypeError。
        monthly = os.path.join(workdir, f"history-{calendar}-monthly")
        write_station(
            os.path.join(monthly, "CalendarMonth_hist_2000.nc"),
            cftime_months(calendar, 2000, (1, 2, 3), day=16),
            {"f_xy_t": ("time", "patch")},
        )
        info = inspect(monthly)
        show_tim_res(info)
        assert info["tim_res"] == "Month", (calendar, info["tim_res"])
        assert info["tim_res_source"] == "time", (calendar, info["tim_res_source"])

    # 不同日历不能共用一个时间轴：明确报错并点名日历，而不是在排序处抛裸 TypeError。
    mixed = os.path.join(workdir, "history-mixed-calendars")
    write_station(
        os.path.join(mixed, "Mix_hist_2000.nc"),
        cftime_axis("noleap", (2000, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(mixed, "Mix_hist_2001.nc"),
        cftime_axis("360_day", (2001, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    stderr = inspect_stderr(mixed)
    assert "different calendars" in stderr, stderr
    assert "Traceback" not in stderr and "TypeError" not in stderr, stderr

    # 同一目录混标准日历与 cftime 也不行：解码表示不同，不能按字符串/对象比较硬排。
    mixed = os.path.join(workdir, "history-mixed-decoding")
    write_station(
        os.path.join(mixed, "Mix_hist_2000.nc"),
        times("2000-01-01T00:00", 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(mixed, "Mix_hist_2001.nc"),
        cftime_axis("noleap", (2001, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    stderr = inspect_stderr(mixed)
    assert "mix standard-calendar datetime64" in stderr, stderr
    assert "Traceback" not in stderr and "TypeError" not in stderr, stderr


@test
def unmappable_step_is_an_explicit_error(workdir):
    """5 天步长不对应任何 OpenBench tim_res：明确报错，不许猜成相邻档位。"""
    folder = os.path.join(workdir, "history")
    write_station(
        os.path.join(folder, "FiveDay_hist_2000-01.nc"),
        times("2000-01-01T00:00", 3, 5 * 24 * 60),
        {"f_xy_t": ("time", "patch")},
    )
    stderr = inspect_stderr(folder)
    assert "cannot map" in stderr, stderr
    assert "432000" in stderr, stderr
    # 报错要列出支持的档位，用户知道该手填什么。
    for supported in ("3Hour", "6Hour", "8Day", "Month", "Year"):
        assert supported in stderr, (supported, stderr)


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
def single_cell_dateline_grid_is_resolved_and_rejected(workdir):
    """R05：中心 180° 的 1×1 格横跨日界线，分辨率是 1° 不是 359°，也不能给出半格范围。

    旧实现用 |179.5-(-179.5)|=359 当分辨率（只有一个中心，中心间距法失灵），又把边界对齐后的
    西边界夹到 -180，给出 [-180,-179.5] 的半格范围并判 comparable——上层看到的是"可比较"，
    真正跨缝的区域被静默裁掉一半。
    """
    folder = os.path.join(workdir, "history")
    write_grid(
        os.path.join(folder, "Seam_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[20.0],
        lon=[180.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
        lon_bounds=([179.5], [-179.5]),
    )
    info = inspect(folder)
    assert info["spatial_type"] == "grid", info["spatial_type"]
    approx(info["resolution"], 1.0, "resolution")
    assert info["crosses_dateline"] is True, info
    assert info["extent"] is None, info["extent"]
    assert info["comparable"] is False
    assert any(blocker["code"] == "dateline_crossing" for blocker in info["blockers"]), info["blockers"]

    # 同一格的不跨缝写法（东边界 180.5 而非 -179.5）：物理上是同一片区域，结论必须一致。
    other = os.path.join(workdir, "history-360")
    write_grid(
        os.path.join(other, "Seam360_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[20.0],
        lon=[180.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
        lon_bounds=([179.5], [180.5]),
    )
    info = inspect(other)
    approx(info["resolution"], 1.0, "resolution (179.5/180.5 spelling)")
    assert info["crosses_dateline"] is True, info
    assert info["extent"] is None, info["extent"]
    assert info["comparable"] is False

    # 普通网格不回归：边界全在 ±180 内时照旧给出范围，不能被跨缝判断误伤。
    normal = os.path.join(workdir, "history-normal")
    write_grid(
        os.path.join(normal, "Normal_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 1440),
        lat=[19.0, 20.0],
        lon=[110.0, 111.0],
        variables={"f_xy_t": ("time", "lat", "lon")},
    )
    info = inspect(normal)
    assert info["crosses_dateline"] is False, info
    approx(info["resolution"], 1.0, "resolution (normal grid)")
    extent = info["extent"]
    assert extent is not None, info
    approx(extent[0], 109.5, "normal extent west")
    approx(extent[1], 111.5, "normal extent east")
    assert info["comparable"] is True


@test
def multi_patch_is_rejected_explicitly(workdir):
    """patch=2：探测报 patch 类型与完整原因（没有 patch 类型/面积/权重、OpenBench 不能逐 patch 加权）；
    制备明确报错，不许 squeeze 崩。"""
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
    # 诊断要解释"为什么不能干脆取第一个 patch"：history 只有数值变量，没有 patch 类型/面积/权重，
    # OpenBench 也没有逐 patch 选择/加权的入口。只写 "supports a single patch only" 会被用户当成
    # 待补的功能而不是无法从这份数据做比较。
    blocker = next(entry for entry in info["blockers"] if entry["code"] == "multi_patch")
    message = blocker["message"]
    assert "2 patches" in message, message
    assert "no patch type/area/weight" in message, message
    assert "cannot select or weight" in message, message
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
def layered_variables_are_kept_for_profile_compute(workdir):
    """层维度变量（soilsnow/soil/lake）保留给 OpenBench CoLM2024 profile 的 compute；类别维仍跳过。

    OpenBench 3.0.8 的 CoLM2024 profile 用 `compute:` 从 `f_wliq_soisno.isel(soilsnow=5:13)` 算
    Root_Zone_Soil_Moisture/Surface_Soil_Moisture：制备若像旧实现那样把层变量整个跳过，站点合并
    文件里没有 f_wliq_soisno，profile compute 就读不到。这里按 CoLM 真实写法造（soilsnow 是
    ncio_write_colm_dimension 写的 1-D 坐标变量、20 层），锁住三件事：探测把层变量放
    layered_variables 而不是 unsupported_variables；制备保留层维度/坐标/数值；band 类维度仍跳过并打印。
    """
    folder = os.path.join(workdir, "history")
    soilsnow = np.arange(1, 21)
    write_station(
        os.path.join(folder, "Layered_hist_2000-01.nc"),
        times("2000-01-01T00:30", 2, 60),
        {
            "f_xy_t": ("time", "patch"),
            "f_wliq_soisno": ("time", "patch", "soilsnow"),
            "f_t_soisno": ("time", "patch", "soilsnow"),
            "f_alb": ("time", "patch", "rtyp", "band"),
        },
        extra_sizes={"soilsnow": 20, "rtyp": 2, "band": 2},
        extra_coords={"soilsnow": soilsnow},
    )
    info = inspect(folder)
    layered = {entry["name"]: entry["dims"] for entry in info["layered_variables"]}
    assert set(layered) == {"f_wliq_soisno", "f_t_soisno"}, layered
    assert layered["f_wliq_soisno"] == ["time", "patch", "soilsnow"], layered
    assert layered["f_t_soisno"] == ["time", "patch", "soilsnow"], layered
    # 层变量不再是"制备会丢的变量"：unsupported_variables 里只剩类别维的 f_alb。
    unsupported = {entry["name"]: entry["extra_dims"] for entry in info["unsupported_variables"]}
    assert set(unsupported) == {"f_alb"}, unsupported
    assert unsupported["f_alb"] == ["rtyp", "band"], unsupported
    # variables 仍列全部随时间变的变量，供前端做变量映射。
    dims = {entry["name"]: entry["dims"] for entry in info["variables"]}
    assert dims["f_wliq_soisno"] == ["time", "patch", "soilsnow"], dims
    assert info["comparable"] is True

    result, out_nc, _ = prepare_ok(folder, info["prefix"], workdir)
    # 保留的层变量要在日志里能看见（不是静默保留），被丢的 band 变量仍逐个打印原因。
    assert "f_wliq_soisno" in result.stdout and "kept" in result.stdout, result.stdout
    skipped_lines = "\n".join(line for line in result.stdout.splitlines() if "skipped" in line)
    assert "f_alb" in skipped_lines and "f_wliq_soisno" not in skipped_lines, result.stdout
    with xr.open_dataset(out_nc) as merged:
        assert "f_xy_t" in merged.data_vars, variable_names(merged)
        assert "f_wliq_soisno" in merged.data_vars, variable_names(merged)
        assert "f_t_soisno" in merged.data_vars, variable_names(merged)
        assert "f_alb" not in merged.data_vars, variable_names(merged)
        assert merged.sizes["time"] == 2
        # 层维度与坐标原样保留：OpenBench profile compute 按位置 isel(soilsnow=5:13) 取层。
        assert merged["f_wliq_soisno"].dims == ("time", "soilsnow"), merged["f_wliq_soisno"].dims
        assert merged.sizes["soilsnow"] == 20, dict(merged.sizes)
        assert np.array_equal(merged["soilsnow"].values, soilsnow), merged["soilsnow"].values
        # 数值是夹具的连续编号：第 0 时刻第 6 层 = 5，第 1 时刻第 13 层 = 32。
        expected = np.arange(2 * 20, dtype=float).reshape(2, 20)
        assert np.array_equal(merged["f_wliq_soisno"].transpose("time", "soilsnow").values, expected)
        assert merged["f_wliq_soisno"].values[0, 5] == 5.0
        assert merged["f_wliq_soisno"].values[1, 12] == 32.0


@test
def layered_variables_survive_multi_file_merge(workdir):
    """层变量跨文件排序/拼接/重复时刻按变量合并时不能丢维度或层坐标。

    真实 history 是每月/每天一个文件，同一时刻可能出现在两个文件里、各自带不同变量。旧实现把层
    变量整个跳过，这条路径没暴露过；保留层维度后要保证 xr.concat(join="outer") 与
    merge_duplicate_times 的 merge(join="outer", compat="no_conflicts") 在层维度/层坐标上对齐。
    """
    folder = os.path.join(workdir, "history")
    soilsnow = np.arange(1, 21)
    # 文件名序：`..._2000-01-02.nc` 排在 `..._2000-01.nc` 前面（'-' < '.'），时间序相反。
    write_station(
        os.path.join(folder, "LayerMerge_hist_2000-01.nc"),
        np.array(["2000-01-01T00:30", "2000-01-02T00:30"], dtype="datetime64[s]"),
        {"f_wliq_soisno": ("time", "patch", "soilsnow")},
        extra_sizes={"soilsnow": 20},
        extra_coords={"soilsnow": soilsnow},
    )
    write_station(
        os.path.join(folder, "LayerMerge_hist_2000-01-02.nc"),
        np.array(["2000-01-02T00:30", "2000-01-03T00:30"], dtype="datetime64[s]"),
        {"f_t_soisno": ("time", "patch", "soilsnow")},
        extra_sizes={"soilsnow": 20},
        extra_coords={"soilsnow": soilsnow},
    )
    result, out_nc, _ = prepare_ok(folder, "LayerMerge_hist_", workdir)
    assert "3 steps" in result.stdout, result.stdout
    with xr.open_dataset(out_nc) as merged:
        assert merged.sizes["time"] == 3, dict(merged.sizes)
        assert merged.sizes["soilsnow"] == 20, dict(merged.sizes)
        assert np.array_equal(merged["soilsnow"].values, soilsnow), merged["soilsnow"].values
        water = merged["f_wliq_soisno"].transpose("time", "soilsnow").values
        temperature = merged["f_t_soisno"].transpose("time", "soilsnow").values
        expected = np.arange(2 * 20, dtype=float).reshape(2, 20)
        # 中间时刻两个文件都有：水来自早文件 t0/t1 两行，温度来自晚文件 t1/t2 两行。
        assert np.array_equal(water[0], expected[0]) and np.array_equal(water[1], expected[1]), water
        assert np.array_equal(temperature[1], expected[0]), temperature
        assert np.array_equal(temperature[2], expected[1]), temperature
        assert np.isnan(water[2]).all(), water[2]
        assert np.isnan(temperature[0]).all(), temperature[0]


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
def non_gregorian_history_is_prepared(workdir):
    """R06 遗留：非公历（noleap/360_day）主 history 的制备要能真正跑通。

    旧实现 `parts.sort(key=lambda d: np.datetime64(d["time"].values.min()))` 只认 datetime64：
    noleap 的日期会被悄悄按公历解释，360_day 的 2 月 30 日不是合法公历日期，实测直接
    `ValueError: Invalid date (2000,2,30)`。这里每个日历造两个文件、文件名序与时间序相反、
    中间留一个重复时刻（各带不同变量），端到端验证：输出可 reopen、时间轴有序去重、日历/units
    沿用输入、CSV 起止年按日历读，tracer 族照旧不参与合并。
    """
    if cftime is None:
        raise AssertionError(
            "the non-Gregorian regression needs cftime; install it (pip install cftime)"
        )
    cases = (
        {
            "calendar": "noleap",
            # 文件名序：`..._2000-12-31.nc` 排在 `..._2000-12.nc` 前面（'-' < '.'），时间序相反。
            "early": ("Cal_hist_2000-12.nc", (2000, 12, 30, 0, 30), "f_xy_t"),
            "late": ("Cal_hist_2000-12-31.nc", (2000, 12, 31, 0, 30), "f_xy_q"),
            "start_year": 2000,
            "end_year": 2001,
        },
        {
            "calendar": "360_day",
            # 2 月 30 日只在 360_day 存在：旧实现在这里把 cftime 喂给 np.datetime64 就崩。
            "early": ("Cal_hist_2000-02.nc", (2000, 2, 29, 0, 30), "f_xy_t"),
            "late": ("Cal_hist_2000-02-15.nc", (2000, 2, 30, 0, 30), "f_xy_q"),
            "start_year": 2000,
            "end_year": 2000,
        },
    )
    for case in cases:
        calendar = case["calendar"]
        folder = os.path.join(workdir, f"history-{calendar}")
        early_name, early_start, early_var = case["early"]
        late_name, late_start, late_var = case["late"]
        write_station(
            os.path.join(folder, early_name),
            cftime_axis(calendar, early_start, 2, 1440),
            {early_var: ("time", "patch")},
        )
        write_station(
            os.path.join(folder, late_name),
            cftime_axis(calendar, late_start, 2, 1440),
            {late_var: ("time", "patch")},
        )
        # 同目录的 tracer 族：只顺带确认非公历路径没改坏主族识别（tracer 文件名里也有 _hist_）。
        tracer_suffix = early_name[len("Cal_hist_") :]
        write_station(
            os.path.join(folder, "Cal_hist_tracer_" + tracer_suffix),
            cftime_axis(calendar, early_start, 2, 1440),
            {"f_trc_co2": ("time", "patch")},
        )
        # 输出要沿用首文件（prepare 的 files[0]，即文件名序第一个主族文件）的 time 编码；
        # 两个主族文件里名字较小的那个就是它（tracer 以 `tracer_` 开头，排在数字后缀之后）。
        first_by_name = os.path.join(folder, min(early_name, late_name))
        with xr.open_dataset(first_by_name) as source:
            source_units = source["time"].encoding["units"]
            source_calendar = source["time"].encoding["calendar"]

        result, out_nc, out_csv = prepare_ok(
            folder, "Cal_hist_", os.path.join(workdir, f"sim-{calendar}")
        )
        assert "3 steps" in result.stdout, result.stdout
        expected = cftime_axis(calendar, early_start, 3, 1440)
        with xr.open_dataset(out_nc) as merged:
            assert merged.sizes["time"] == 3, dict(merged.sizes)
            assert "patch" not in merged.sizes, dict(merged.sizes)
            assert "f_trc_co2" not in merged.data_vars, variable_names(merged)
            values = merged["time"].values
            assert values.dtype.kind == "O", values.dtype
            assert list(values) == list(expected), (calendar, values, expected)
            encoding = merged["time"].encoding
            assert encoding.get("calendar") == source_calendar, (calendar, encoding)
            # units 的参考时刻/单位要沿用首文件：换一套参考时刻时同一时刻的换算值会不同。
            for probe in expected:
                assert cftime.date2num(probe, encoding["units"], calendar) == cftime.date2num(
                    probe, source_units, calendar
                ), (calendar, encoding["units"], source_units)
            values_t = merged[early_var].values.ravel()
            values_q = merged[late_var].values.ravel()
            # 中间那一刻两个文件都有：t 来自早文件、q 来自晚文件，合并后都应是真实值。
            assert np.isfinite(values_t[0]) and np.isfinite(values_t[1]), values_t
            assert np.isfinite(values_q[1]) and np.isfinite(values_q[2]), values_q
            assert np.isnan(values_q[0]) and np.isnan(values_t[2]), (values_t, values_q)
        with open(out_csv, newline="", encoding="utf-8") as handle:
            rows = list(csv.reader(handle))
        assert len(rows) == 2 and len(rows[1]) == 6, rows
        assert rows[1][1] == str(case["start_year"]), (calendar, rows[1])
        assert rows[1][2] == str(case["end_year"]), (calendar, rows[1])


@test
def mixing_calendars_or_decodings_is_rejected_by_prepare(workdir):
    """同一主族混日历、或混标准日历与 cftime：制备要在排序前明确报错，不是 sortby 里抛裸 TypeError。

    inspect.py 已按同一判据在探测端拒绝（R06）；这里守住制备端：错误要点名日历/两种解码表示，
    且不能写出半成品 NC。
    """
    if cftime is None:
        raise AssertionError(
            "the non-Gregorian regression needs cftime; install it (pip install cftime)"
        )
    mixed = os.path.join(workdir, "history-mixed-calendars")
    write_station(
        os.path.join(mixed, "Mix_hist_2000.nc"),
        cftime_axis("noleap", (2000, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(mixed, "Mix_hist_2001.nc"),
        cftime_axis("360_day", (2001, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    result, out_nc, _ = prepare(mixed, "Mix_hist_", os.path.join(workdir, "out-mixed-calendars"))
    stderr = failed(result, "prepare with mixed calendars")
    assert "different calendars" in stderr, stderr
    assert "noleap" in stderr and "360_day" in stderr, stderr
    assert "Traceback" not in stderr and "TypeError" not in stderr, stderr
    assert not os.path.exists(out_nc), "a failed prepare must not leave a half-written NetCDF"

    mixed = os.path.join(workdir, "history-mixed-decoding")
    write_station(
        os.path.join(mixed, "Mix_hist_2000.nc"),
        times("2000-01-01T00:00", 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    write_station(
        os.path.join(mixed, "Mix_hist_2001.nc"),
        cftime_axis("noleap", (2001, 1, 1, 0, 0), 2, 360),
        {"f_xy_t": ("time", "patch")},
    )
    result, out_nc, _ = prepare(mixed, "Mix_hist_", os.path.join(workdir, "out-mixed-decoding"))
    stderr = failed(result, "prepare with mixed datetime64/cftime")
    assert "mix standard-calendar datetime64" in stderr, stderr
    assert "Traceback" not in stderr and "TypeError" not in stderr, stderr
    assert not os.path.exists(out_nc), "a failed prepare must not leave a half-written NetCDF"


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
