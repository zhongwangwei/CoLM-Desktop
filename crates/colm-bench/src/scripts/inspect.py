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
- 频率判定先看实测时间轴，再看文件自述：dataset/time 属性里的 tim_res/time_res/frequency/
  time_frequency/cell_methods 等是"元数据"，而 OpenBench 重采样的是文件里的时间轴。所以采样
  得到步长时用实测（元数据一致只作佐证，冲突以实测为准并在 warnings 里给出两个值——静默采用
  冲突元数据会让 OpenBench 按错误周期重采样）；只有整份数据只有一个时刻、采不出步长时才用明确
  元数据（source=metadata，界面据此提示用户确认）；两者都没有才退回文件名后缀推断。
- `data_groupby` 必须与 OpenBench 找文件的方式一致，只能按文件名后缀判；旧实现看第一个文件的
  尾部，同目录换一种分组就错。
- 空间类型要显式分类：单格 lat×lon 网格也是 `grid`（旧逻辑「某维长度>1」把它当单点），
  element/ncol 向量是 `unstructured`，patch>1 是 `patch`，没有坐标也看不出是单点的是 `unknown`。
  上层必须能先拒绝，而不是先显示「可比较」再在制备时 KeyError。
- 经度按环绕距离归一化到 [-180,180) 再算范围与分辨率：0–360 文件里的 200.5° 归一成 -159.5°，
  [-179,179] 这种跨日界线的两点网格分辨率是 2° 而不是 358°。OpenBench 的 [西,东] 表示法写不出
  跨日界线的范围（必须 西<东），所以这种网格给 extent=null、crosses_dateline=true，让上层拒绝，
  而不是给一个 199.5>180 的假范围。
- 只有一个中心点时（1×1 格）中心间距法失灵：分辨率要退回格子边界并按环绕距离算，跨缝也只能由
  边界判断。中心 180°、西 179.5°、东 -179.5° 的 1×1 格是 1° 宽、横跨日界线，旧实现报 359°、
  crosses_dateline=false 并把 extent 夹成半格，上层就静默放行了一个省掉半格的区域（R05）。
- 非公历日历（noleap、360_day…）经 xarray 解出来是 cftime 对象，不是 datetime64：排序、步长、
  年份都要走日历感知的分支（cftime 相减给 datetime.timedelta），不能 np.diff 后除 timedelta64，
  那会抛裸 TypeError；同一目录混不同日历/标准日历则明确拒绝（R06）。
- 土层/积雪层/湖泊层维度（soil、soilsnow、lake）的变量不是"制备会丢的变量"：OpenBench 3.0.8
  的 CoLM2024 profile 用 `compute:` 直接从原始层变量算地表量（`f_wliq_soisno.isel(soilsnow=5:13)`
  → Root_Zone_Soil_Moisture 等），制备会原样保留这些维度，所以单独报进 `layered_variables`
  （界面据此提示"profile 可能 compute/需要层处理"）。其它超出站点评估 {time,patch} 的维度
  （band、rtyp、vegnodes 等）仍逐个列进 `unsupported_variables`，制备时跳过。
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

try:
    # 非公历日历（noleap/360_day…）由 xarray 解成 cftime 对象；它是可选依赖，标准日历的
    # 环境没装也必须能探测。用不到时保持 None，is_cftime_values 就是恒 False。
    import cftime
except ImportError:  # pragma: no cover - 装了 xarray 的机器通常也有；没有时只影响非公历文件
    cftime = None

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

# CoLM histmap 的层维度（colm-hist/src/generated.rs 的 soi/soilsnow/lake 三套）。它们不是制备会丢的
# 维度：OpenBench 3.0.8 的 CoLM2024 profile 在 model_catalog.yaml 里用 `compute:` 从原始层变量算
# 地表量（`f_wliq_soisno.isel(soilsnow=5:13)` → Root_Zone_Soil_Moisture 等），PREPARE 会原样保留
# 这些维度写进站点合并文件。带这些额外维度的变量单独报进 layered_variables，不混进
# unsupported_variables——后者在前端被当成"变量映射命中就会被制备跳过"，层变量不该再触发那个拒绝。
LAYER_DIMS = ("soilsnow", "soil", "lake")

# 只归一化经度用；纬度不绕圈。
DAY_SECONDS = 86400.0

# 能从时间轴/频率元数据推断出来的 OpenBench 3.0.8 tim_res 档位。v3.0.8 的 config/loader.py 里
# VALID_TIM_RES_VALUES 还含 climatology-month/climatology-year 与正则 Nmonth（如 2month），但那
# 三者分别描述比较语义/不固定的日历步长，不能从一条时间轴可靠反推（理由见 classify_tim_res），
# 所以这里只收"步长能解释"的七个档位。
# 区间用显式 (下限秒, 上限秒, tim_res)：
# - 要容得下真实日历抖动（月 28–31 天、年 365/366 天，闰日都算）；
# - 又不能把相邻档位吞进来：7/9 天不落 8Day，2/4 小时不落 3Hour/6Hour，5 天不落任何档；
# - Hour 的区间是 54–66 分钟。比它还细的输出（CoLM DEF_HIST_FREQ=TIMESTEP 默认 1800 s）没有
#   任何 OpenBench 档位可选，由 decide_tim_res 显式粗化成 Hour 并写 warning，不在这里静默匹配。
TIM_RES_STEP_RANGES = (
    (3600.0 * 0.9, 3600.0 * 1.1, "Hour"),
    (3.0 * 3600.0 * 0.9, 3.0 * 3600.0 * 1.1, "3Hour"),
    (6.0 * 3600.0 * 0.9, 6.0 * 3600.0 * 1.1, "6Hour"),
    (DAY_SECONDS * 0.9, DAY_SECONDS * 1.1, "Day"),
    # 8 天合成产物通常是整 8 天；按 ±0.5 天留容差（而不是按比例放大），7 天/9 天不是 OpenBench 的 8Day。
    (7.5 * DAY_SECONDS, 8.5 * DAY_SECONDS, "8Day"),
    (27.0 * DAY_SECONDS, 32.0 * DAY_SECONDS, "Month"),
    (350.0 * DAY_SECONDS, 380.0 * DAY_SECONDS, "Year"),
)

# 频率元数据可能落在 dataset 全局属性或 time 变量属性上，不同后处理工具写法不一；属性名按小写
# 匹配（NetCDF 属性名大小写敏感，后处理工具常写成 Frequency/Time_Frequency）。
# 顺序即同一来源内的优先顺序：更专门的 tim_res/time_res 先于泛化的 frequency，cell_methods 最后。
FREQUENCY_ATTR_NAMES = (
    "tim_res",
    "tim_resolution",
    "time_res",
    "time_resolution",
    "temporal_resolution",
    "time_frequency",
    "time_freq",
    "frequency",
    "freq",
    "cell_methods",
    "cell_method",
)

# 固定长度的频率单位（供 "1h"/"3-hourly"/"daily" 这类文字解析）。月/年单独判断：它们的步长随日历
# 变化，不能折成固定秒数再分档（2 月与 8 月的"一个月"不是同一长度）。
FREQUENCY_FIXED_UNITS = {
    "s": 1.0,
    "sec": 1.0,
    "secs": 1.0,
    "second": 1.0,
    "seconds": 1.0,
    "min": 60.0,
    "mins": 60.0,
    "minute": 60.0,
    "minutes": 60.0,
    "h": 3600.0,
    "hr": 3600.0,
    "hrs": 3600.0,
    "hour": 3600.0,
    "hours": 3600.0,
    "hourly": 3600.0,
    "d": DAY_SECONDS,
    "day": DAY_SECONDS,
    "days": DAY_SECONDS,
    "daily": DAY_SECONDS,
    "w": 7.0 * DAY_SECONDS,
    "week": 7.0 * DAY_SECONDS,
    "weeks": 7.0 * DAY_SECONDS,
    "weekly": 7.0 * DAY_SECONDS,
}
FREQUENCY_MONTH_UNITS = ("mon", "mons", "month", "months", "monthly")
FREQUENCY_YEAR_UNITS = ("y", "yr", "yrs", "year", "years", "yearly", "annual", "annually")


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


def is_cftime_values(values):
    """xarray 把非公历日历（noleap、360_day…）解成 cftime 对象，标准日历仍是 datetime64。

    cftime 只在遇到非公历文件时才会被 xarray 用到，标准日历的机器不装它也必须能探测，
    所以 import 是可选依赖；这里没装就恒为 False。
    """
    return (
        cftime is not None
        and values.dtype.kind == "O"
        and values.size > 0
        and isinstance(values.ravel()[0], cftime.datetime)
    )


def time_year(value):
    """时间点的年份。cftime 不能 astype 成 datetime64（日历不同），直接读 .year。"""
    if cftime is not None and isinstance(value, cftime.datetime):
        return int(value.year)
    return int(str(value)[:4])


def file_times(path):
    """只取 time 变量（xarray 懒打开，读到的只有时间轴）。"""
    with xr.open_dataset(path) as dataset:
        if "time" not in dataset.variables:
            raise SystemExit(f"{path} has no time variable")
        values = np.asarray(dataset["time"].values)
    if values.dtype.kind == "M":
        return np.sort(values)
    if is_cftime_values(values):
        # cftime 对象没有 datetime64 的纳秒表示，np.sort 走对象比较（同一文件内日历相同）。
        return np.asarray(sorted(values.ravel()), dtype=object)
    raise SystemExit(
        f"{path}: time variable does not decode to datetime64 or cftime ({values.dtype}); "
        "check the time:units/calendar attributes"
    )


def collect_times(paths):
    chunks = [file_times(path) for path in paths]
    if not chunks:
        return np.array([], dtype="datetime64[ns]")
    kinds = {"cftime" if is_cftime_values(chunk) else "datetime64" for chunk in chunks if chunk.size}
    if len(kinds) > 1:
        raise SystemExit(
            "the main history files mix standard-calendar datetime64 with non-standard cftime "
            "time axes; they cannot be ordered on one axis — keep one calendar per history"
        )
    if "cftime" in kinds:
        calendars = sorted(
            {chunk.ravel()[0].calendar for chunk in chunks if is_cftime_values(chunk)}
        )
        if len(calendars) > 1:
            # 不同日历的"同一天"不是同一时刻，拼接后排序没有意义；明确拒绝，不猜。
            raise SystemExit(
                f"the main history files use different calendars ({', '.join(calendars)}); "
                "they cannot share one time axis — keep one calendar per history"
            )
        return np.asarray(sorted(np.concatenate(chunks)), dtype=object)
    return np.sort(np.concatenate(chunks))


def median_step_seconds(times):
    """相邻记录的中位步长；只有一个时刻时返回 None（调用方再试频率元数据/文件名）。"""
    if is_cftime_values(times):
        # 不能用 np.diff(times)/np.timedelta64(1,'s')：cftime 相减给 datetime.timedelta，
        # 除 numpy timedelta64 会抛 TypeError（R06）。
        unique = sorted(set(times.ravel()))
        if len(unique) < 2:
            return None
        seconds = [
            (later - earlier).total_seconds()
            for earlier, later in zip(unique, unique[1:])
        ]
        return float(np.median(seconds))
    unique = np.unique(times)
    if unique.size < 2:
        return None
    seconds = np.diff(unique) / np.timedelta64(1, "s")
    return float(np.median(seconds))


def frequency_text(raw):
    """频率属性值 -> 小写、空白折叠后的文本；读不出来给空串。"""
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8", "replace")
    return " ".join(str(raw).strip().lower().split())


def classify_seconds(seconds):
    """步长秒数 -> 可推断的 tim_res；落不到任何档位返回 None（元数据解析用，不抛异常）。"""
    for low, high, name in TIM_RES_STEP_RANGES:
        if low <= seconds <= high:
            return name
    return None


def classify_tim_res(seconds):
    """实测中位步长 -> OpenBench 的 tim_res。

    OpenBench 3.0.8（config/loader.py 的 VALID_TIM_RES_VALUES）接受
    Month/Day/Hour/Year/3Hour/6Hour/8Day/climatology-month/climatology-year，外加 Nmonth（如
    2month）。这里只返回"一条时间轴能解释"的档位：climatology-* 描述的是比较语义（同一份月气候
    态文件的时间轴步长仍是 1 个月，但气候态语义不能从一个步长推出来），Nmonth 的日历步长不固定
    （28–31 天 × N），按中位步长反推 N 会静默给错重采样周期——这些只能由用户显式选。落不到任何
    档位（例如 5 天、2 小时、30 分钟）明确报错，不猜成相邻档位让它悄悄算错。
    """
    tim_res = classify_seconds(seconds)
    if tim_res is not None:
        return tim_res
    supported = "/".join(name for _, _, name in TIM_RES_STEP_RANGES)
    raise SystemExit(
        f"cannot map the median history step of {seconds:.0f} s onto an OpenBench tim_res "
        f"(supported: {supported}); OpenBench resamples the file's own time axis, so fix the "
        "output frequency or enter tim_res manually"
    )


def parse_frequency_text(raw):
    """频率元数据原文 -> 规范 tim_res（Hour/3Hour/6Hour/Day/8Day/Month/Year）或 None。

    认识的写法：hourly/1h/1hour/3600（纯数字按秒）、3-hourly/3 hourly、6-hourly、daily/1d/
    1day、8-day/8d、monthly/mon/1month、yearly/annual/1yr，以及 CF cell_methods 里的
    `interval: 6 hours`。只认能落到上面档位的写法；认不出返回 None（由调用方决定提示还是忽略），
    绝不在这里硬映射——例如 30 分钟不能当成 Hour 猜，那会让 OpenBench 按错误周期重采样。
    """
    text = frequency_text(raw)
    if not text:
        return None
    # 纯数字按秒："frequency: 3600"。
    if re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", text):
        return classify_seconds(float(text))
    # CF cell_methods 的 interval 最明确、最无歧义："time: mean (interval: 6 hours)"。
    match = re.search(r"interval\s*[:=]\s*([0-9]+(?:\.[0-9]+)?)\s*([a-z]+)", text)
    if match is None:
        # CF 的 "mean within days"/"mean over years" 是气候态聚合语义，不是固定输出步长，不猜。
        if "within" in text or "over" in text or "climatolog" in text:
            return None
        # 折平 "3-hourly"/"6_hourly" 这类分隔写法；带多余词的（"daily mean"）整体视为不认识。
        match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)?\s*([a-z]+)", re.sub(r"[-_]+", " ", text))
        if match is None:
            return None
    amount_text, unit = match.group(1), match.group(2)
    if unit in FREQUENCY_FIXED_UNITS:
        amount = float(amount_text) if amount_text is not None else 1.0
        return classify_seconds(amount * FREQUENCY_FIXED_UNITS[unit])
    # 月/年只接受 1 个：Nmonth 是 OpenBench 合法值但日历步长不固定，不能从文字里的 N 静默反推。
    if unit in FREQUENCY_MONTH_UNITS and (amount_text is None or float(amount_text) == 1.0):
        return "Month"
    if unit in FREQUENCY_YEAR_UNITS and (amount_text is None or float(amount_text) == 1.0):
        return "Year"
    return None


def frequency_metadata(dataset):
    """dataset 与 time 变量上的频率元数据：[(来源, 属性名, 原始值, 规范 tim_res 或 None)]。

    都收集而不是只取第一个：同一个文件里 dataset 与 time 自相矛盾时要能报出来（文件自述都打架
    就更不能静默挑一个）。time 变量上的属性比 dataset 全局属性更具体，排在前面当首选。
    """
    found = []
    for source, attrs in (("time", dataset["time"].attrs), ("dataset", dataset.attrs)):
        lowered = {}
        for key in attrs:
            lowered.setdefault(str(key).lower(), key)
        for name in FREQUENCY_ATTR_NAMES:
            if name in lowered:
                raw = attrs[lowered[name]]
                found.append((source, name, raw, parse_frequency_text(raw)))
    return found


def decide_tim_res(step, metadata, suffixes):
    """判定 tim_res -> (tim_res, source, metadata_tim_res, warnings)。

    策略与理由：
    1. 时间轴采样得到中位步长 -> 用实测，source=time。OpenBench 重采样的是文件里的时间轴，元数据
       只是文件自述；一致时元数据只作佐证（source 仍是 time），冲突时必须以实测为准并把两个值都
       写进 warnings —— 静默采用冲突元数据会让 OpenBench 按错误周期重采样却看不出任何异常，这比
       明确报冲突更糟。
    2. 采不到步长（整份数据只有一个时刻）但元数据明确 -> 用元数据，source=metadata。这是
       "信息不足时要求确认"的落点：只剩文件自述一条证据，必须在 warnings 与 source 里讲清楚，
       界面据此提示用户确认。
    3. 都没有 -> 保留旧行为：按文件名日期后缀推断（source=filename，明确标注是推断）；文件名也
       说不清就明确报错，不猜。
    """
    usable = [item for item in metadata if item[3] is not None]
    distinct = []
    for item in usable:
        if item[3] not in distinct:
            distinct.append(item[3])
    inconsistent = []
    if len(distinct) > 1:
        detail = ", ".join(f"{item[1]}={item[2]!r}->{item[3]}" for item in usable)
        inconsistent.append(
            f"frequency metadata is inconsistent ({detail}); using "
            f"{usable[0][1]}={usable[0][2]!r} as the declared frequency"
        )
    metadata_tim_res = usable[0][3] if usable else None

    if step is not None:
        warnings = list(inconsistent)
        if step < TIM_RES_STEP_RANGES[0][0]:
            # OpenBench 最细的档位只有 Hour，而 CoLM 的 DEF_HIST_FREQ=TIMESTEP 默认按 1800 s 写
            # history（本仓库 AU-Preston 等 17 个历史文件实测中位步长 1800 s）。按 Hour 报是
            # "只能这么报"的粗化，不是猜周期：对比目标恒为小时或更粗，重采样均值不因采样更密而错；
            # 若硬报错，这类最常见的时间步输出就完全进不了比较。用 warnings 明说，别让用户以为
            # 文件真是逐时的。
            measured = "Hour"
            warnings.append(
                f"the history output is finer than 1 h (median step {step:.0f} s) but OpenBench has "
                "no sub-hourly tim_res; reporting Hour, the finest supported resolution"
            )
        else:
            measured = classify_tim_res(step)
        if metadata_tim_res is not None and metadata_tim_res != measured:
            warnings.append(
                f"frequency metadata says {metadata_tim_res} "
                f"({usable[0][1]}={usable[0][2]!r}) but the sampled time axis is {measured} "
                f"(median step {step:.0f} s); using the sampled time axis because OpenBench reads "
                "the files, not the metadata"
            )
        return measured, "time", metadata_tim_res, warnings

    if usable:
        warnings = [
            "the history has a single time step; tim_res was taken from the frequency metadata "
            f"({usable[0][1]}={usable[0][2]!r}) because there is no time axis to sample; "
            "confirm it before running"
        ]
        warnings.extend(inconsistent)
        return metadata_tim_res, "metadata", metadata_tim_res, warnings

    ignored = [item for item in metadata if item[3] is None]
    ignored_detail = ""
    if ignored:
        ignored_detail = "; frequency metadata " + ", ".join(
            f"{item[1]}={item[2]!r}" for item in ignored
        ) + " is not an OpenBench tim_res"
    fallback = {suffix: classify_suffix(suffix) for suffix in suffixes}
    guesses = {value for value in fallback.values() if value}
    if len(guesses) != 1:
        # 单时刻、元数据不可用、文件名也说不清：三条证据都没有，只能明确报错让用户手填。
        raise SystemExit(
            "the history has a single time step and the file name does not tell the resolution; "
            "cannot infer tim_res" + ignored_detail
        )
    warnings = ["the history has a single time step; tim_res was inferred from the file name"]
    if ignored:
        detail = ", ".join(f"{item[1]}={item[2]!r}" for item in ignored)
        warnings.append(f"ignored frequency metadata that is not an OpenBench tim_res ({detail})")
    return guesses.pop(), "filename", None, warnings


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


def boundary_spacing(dataset, low, high, period=None):
    """用格边界（lat_s/lat_n、lon_w/lon_e）求分辨率，中心点只有一格时也能得到格子大小。

    period=360 时按环绕距离算：一个 1° 宽的格子把西边界写成 179.5、东边界写成 -179.5 时，
    朴素差是 359°，会把分辨率报成 359（R05）；环绕距离 `((high-low+180) % 360) - 180`
    给出的是跨缝的 1°。纬度不绕圈，period=None 时仍直接相减。
    """
    if low not in dataset.variables or high not in dataset.variables:
        return None
    low_values = np.asarray(dataset[low].values, dtype=float).ravel()
    high_values = np.asarray(dataset[high].values, dtype=float).ravel()
    if low_values.size == 0 or low_values.size != high_values.size:
        return None
    if period is None:
        spans = np.abs(high_values - low_values)
    else:
        spans = np.abs(((high_values - low_values + period / 2.0) % period) - period / 2.0)
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


def boundary_crosses_dateline(boundary_pairs):
    """边界对（已对齐到各自格心）里是否有一格越过了 ±180。

    中心点法要求至少两个中心：单格横跨日界线时只有一个中心，永远看不出缝（R05）。这时只能
    看边界——西 179.5、东 -179.5 对齐后落在 -180 两侧；把东边界写成 180.5 的等价写法归一化
    后同样落在 -180 的另一侧，所以两种写法结论一致。容差只吸收浮点往返（真实越缝是边界间距
    量级），不能大到把半格越缝放过去、再让 extent 被夹回 ±180。
    """
    tolerance = 1e-6
    return any(
        west < -180.0 - tolerance or east > 180.0 + tolerance
        for west, east in boundary_pairs
    )


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
        lon_spacing = boundary_spacing(dataset, "lon_w", "lon_e", period=360.0)
    if lat_spacing is None:
        lat_spacing = boundary_spacing(dataset, "lat_s", "lat_n")
    spacings = [value for value in (lon_spacing, lat_spacing) if value]
    # 格点 resolution 取 max：OpenBench 的 grid_res 是单一值，取大的一边保证覆盖粗方向。
    resolution = max(spacings) if spacings else None

    west = east = south = north = None
    boundary_pairs = None
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
            # 边界对齐到各自格心所在的那一圈；0–360 与 ±180 混写时才能和中心点比较。
            boundary_pairs = [
                (
                    float(nearest_wrap(normalize_lon(low), center)),
                    float(nearest_wrap(normalize_lon(high), center)),
                )
                for low, high, center in zip(lon_w, lon_e, lon_centers)
            ]
            west = min(pair[0] for pair in boundary_pairs)
            east = max(pair[1] for pair in boundary_pairs)
            south = float(np.min(lat_s)) if lat_s.size else None
            north = float(np.max(lat_n)) if lat_n.size else None
            # 自检：边界必须真的包住中心点；0–360 文件单格在日界线附近时容易把两圈混起来，
            # 出现了就当作跨日界线，不给假范围。
            if not (west <= float(lon_centers.min()) and east >= float(lon_centers.max())):
                return None, resolution, True, lon_centers

    crossing = crosses_dateline(lon_centers, lon_spacing)
    if not crossing and boundary_pairs is not None:
        # 只有一个中心点时中心间距法永远返回 False；单格横跨日界线只能由边界看出来（R05）。
        crossing = boundary_crosses_dateline(boundary_pairs)
    if crossing:
        return None, resolution, True, lon_centers

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
            # cftime 的年份只能读 .year（没有 datetime64 表示），所以统一走 time_year。
            start = time_year(file_times(paths[0])[0])
            end = time_year(file_times(paths[-1])[-1])
            entry["syear"] = start
            entry["eyear"] = end
        except (SystemExit, ValueError, TypeError, OSError, KeyError):
            # 旁车文件可能没有可解码的 time；族报告只是展示，不因此让整个探测失败。
            # TypeError 也收进来：混合日历的对象数组排序会抛它，不能让它裸奔到上层。
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
    start = time_year(times[0])
    end = time_year(times[-1])

    step = median_step_seconds(times)

    with xr.open_dataset(main_paths[0]) as first:
        # 频率判定需要 dataset/time 属性，和下面的空间/变量探测共用这一次打开。
        metadata = frequency_metadata(first)
        spatial_type = spatial_type_of(first)
        patch = int(first.sizes["patch"]) if "patch" in first.sizes else None
        variables = []
        unsupported = []
        layered = []
        supported = SUPPORTED_DIMS.get(spatial_type, {"time"})
        for name in first.data_vars:
            var = first[name]
            if "time" not in var.dims:
                continue
            variables.append({"name": name, "dims": [str(dim) for dim in var.dims]})
            extra = [str(dim) for dim in var.dims if dim not in supported]
            if not extra:
                continue
            if all(dim in LAYER_DIMS for dim in extra):
                # 层维度由 OpenBench profile 的 compute 消费，PREPARE 会保留维度；不是"制备会丢"的变量。
                layered.append({"name": name, "dims": [str(dim) for dim in var.dims]})
                continue
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

    # 时间分辨率：实测时间轴优先，单时刻才用明确元数据，再退文件名推断（理由见 decide_tim_res）。
    tim_res, tim_res_source, tim_res_metadata, tim_res_warnings = decide_tim_res(
        step, metadata, suffixes
    )

    blockers = []
    if spatial_type == "patch":
        blockers.append(
            {
                "code": "multi_patch",
                # 为什么不能"取第一个 patch 就行"：history 只有逐 patch 的数值变量，没有 patch 的
                # 类型/面积/权重这类元数据变量，OpenBench 的站点条目也没有逐 patch 选择/加权的入口；
                # 合成一个平均站点会把不同地表的通量混在一起，所以这里只阻断、不猜一个聚合方式。
                "message": f"the history has {patch} patches per grid cell; the files carry no "
                "patch type/area/weight variable and OpenBench has no per-patch selection or "
                "weighting, so the station comparison cannot select or weight patches and "
                "supports a single patch only",
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

    warnings = list(tim_res_warnings)
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
        # 文件自述的频率（规范化成 OpenBench tim_res），null 表示没有可用元数据；与 tim_res 不同
        # 即为"元数据冲突、以实测为准"，界面可据此提示，两个值都在 JSON 里。
        "tim_res_metadata": tim_res_metadata,
        "data_groupby": data_groupby,
        "extent": extent,
        "resolution": resolution,
        "crosses_dateline": crossing,
        "comparable": not blockers,
        "reason": "; ".join(blocker["message"] for blocker in blockers) if blockers else None,
        "blockers": blockers,
        "variables": variables,
        # 带 soilsnow/soil/lake 额外维度的变量（[{name, dims}]）：制备会原样保留维度，OpenBench
        # CoLM2024 profile 的 compute 直接读；前端提示"profile 可能 compute/需要层处理"，不要当成
        # unsupported_variables 那种"映射命中就会丢"的变量。
        "layered_variables": layered,
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
