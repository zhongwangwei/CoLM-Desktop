//! 比较页（OpenBench）本机与服务器共用的配方。
//!
//! 环境都装在一个根目录（本机是应用数据目录下的 `openbench/`，服务器是工作目录下的 `openbench/`）里：
//! `tools/uv`（固定版本、按校验和核对过的 uv）、`python/`（uv 管理的专用 Python）、`env/`（虚拟环境）、
//! `cache/`（uv 下载缓存）、`home/`（OpenBench 登记表与 matplotlib、cartopy 的缓存）、`projects/<名字>/`（一次比较）。
//! 本机和服务器用同一份版本、校验和、环境变量与脚本，两边装出来的东西才一样。

/// 固定的 uv 版本（2026-08-14 发布）。换版本要连同下面的校验和一起换，并用官方 `.sha256` 与实际下载核对。
pub const UV_VERSION: &str = "0.12.5";
/// (平台, 发布页上的压缩包, SHA-256)。2026-10-23 从发布页取得，并下载 aarch64-apple-darwin 一份核对过。
/// Linux 用静态链接的 musl 版：gnu 版的 uv 自己就挑 glibc，老系统上可能连 uv 都起不来；musl 版照样认出宿主是
/// glibc，下载 gnu 的 Python 和 manylinux 包（7920 上实测 `uv python list` 给出 `linux-x86_64-gnu`）。
pub const UV_ARCHIVES: [(&str, &str, &str); 6] = [
    (
        "macos-aarch64",
        "uv-aarch64-apple-darwin.tar.gz",
        "5bb0e5fe008a773c3dbcb97ff79cd89e1241464fe9d2f986d52ad8f1b037bd62",
    ),
    (
        "macos-x86_64",
        "uv-x86_64-apple-darwin.tar.gz",
        "b3b2137477cf96c9686ebfb71524614cec780c673fd73e59bce099aef02e70e8",
    ),
    (
        "linux-x86_64",
        "uv-x86_64-unknown-linux-musl.tar.gz",
        "a4742988791c9aeae68c78150d6cba762062ad2a47e53738c2779d2b596bfcdb",
    ),
    (
        "linux-aarch64",
        "uv-aarch64-unknown-linux-musl.tar.gz",
        "8767a0e77f2cd45436401b1b42bf7e9ed5a4a91a74a5305d6fe93249d0f6dbc5",
    ),
    (
        "windows-x86_64",
        "uv-x86_64-pc-windows-msvc.zip",
        "4c4d49d8738847d9b71ba319e49a5688c93eac0fe6204b1df24e98528dddf39a",
    ),
    (
        "windows-aarch64",
        "uv-aarch64-pc-windows-msvc.zip",
        "724279317fee6e5fa8ad1908e4eba2bbe764ef1ece5b3f4597927b62b1fe562a",
    ),
];
/// PyPI 上装的 OpenBench 版本。只实测过这一版（登记表 JSON 里的 Infinity、`check` 与 `run` 对相对
/// fulllist 的解析不一致都是按它处理的）；不锁版本时新发布会悄悄换掉行为（复验 D2-03）。换版本要重跑一次评估。
pub const OPENBENCH_VERSION: &str = "3.0.8";
/// OpenBench 要求 Python ≥ 3.10；3.12 的科学计算轮子（cartopy、netCDF4）三个平台都齐。
pub const PYTHON_VERSION: &str = "3.12";
/// 这个平台（`macos-aarch64` 这样的写法，同 `std::env::consts` 的 OS 与 ARCH）该下载的 uv 压缩包与它的 SHA-256。
pub fn uv_archive(platform: &str) -> Option<(&'static str, &'static str)> {
    UV_ARCHIVES
        .iter()
        .find(|(key, _, _)| *key == platform)
        .map(|(_, archive, sha)| (*archive, *sha))
}

/// 服务器 `uname -s -m` 的输出换成 [`UV_ARCHIVES`] 里的平台写法；不认得的给 `None`。
pub fn platform_from_uname(text: &str) -> Option<&'static str> {
    let mut words = text.split_whitespace();
    let os = words.next()?;
    let arch = words.next()?;
    match (os, arch) {
        ("Linux", "x86_64" | "amd64") => Some("linux-x86_64"),
        ("Linux", "aarch64" | "arm64") => Some("linux-aarch64"),
        ("Darwin", "x86_64") => Some("macos-x86_64"),
        ("Darwin", "arm64" | "aarch64") => Some("macos-aarch64"),
        _ => None,
    }
}

/// 环境变量的取值：根目录下的一个相对位置（用 `/` 分隔），或者一个固定值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvValue {
    Under(&'static str),
    Fixed(&'static str),
}

/// 跑 uv 时的环境：Python、缓存、工具都落在根目录下；不读用户自己的 `uv.toml`。
pub const UV_ENV: [(&str, EnvValue); 6] = [
    ("UV_PYTHON_INSTALL_DIR", EnvValue::Under("python")),
    ("UV_PYTHON_BIN_DIR", EnvValue::Under("python/bin")),
    ("UV_CACHE_DIR", EnvValue::Under("cache")),
    ("UV_TOOL_DIR", EnvValue::Under("tools/uv-tools")),
    ("UV_NO_CONFIG", EnvValue::Fixed("1")),
    ("UV_NO_PROGRESS", EnvValue::Fixed("1")),
];

/// 跑专用环境里的程序（python、openbench）时的环境。OpenBench 默认把参考数据登记表写进 `~/.openbench`，
/// matplotlib、cartopy 也各自往 HOME 下写缓存（实测 `~/.matplotlib`、`~/.local/share/cartopy`）；全部改指到
/// `home/`，删掉整个根目录就不留痕迹，也不和用户自己装的 OpenBench 共用登记表。
pub const PROGRAM_ENV: [(&str, EnvValue); 9] = [
    ("OPENBENCH_HOME", EnvValue::Under("home")),
    ("MPLCONFIGDIR", EnvValue::Under("home/matplotlib")),
    ("XDG_CONFIG_HOME", EnvValue::Under("home/config")),
    ("XDG_CACHE_HOME", EnvValue::Under("home/cache")),
    ("XDG_DATA_HOME", EnvValue::Under("home/share")),
    ("PYTHONNOUSERSITE", EnvValue::Fixed("1")),
    ("PYTHONUNBUFFERED", EnvValue::Fixed("1")),
    ("PYTHONIOENCODING", EnvValue::Fixed("utf-8")),
    ("NO_COLOR", EnvValue::Fixed("1")),
];

/// 服务器上的写法：`root` 是 POSIX 绝对路径。
pub fn posix_value(root: &str, value: EnvValue) -> String {
    match value {
        EnvValue::Under(relative) => format!("{}/{relative}", root.trim_end_matches('/')),
        EnvValue::Fixed(text) => text.to_owned(),
    }
}

/// 本机的写法：相对位置按本机的分隔符拼。
pub fn local_value(root: &std::path::Path, value: EnvValue) -> std::ffi::OsString {
    match value {
        EnvValue::Under(relative) => relative
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
            .into_os_string(),
        EnvValue::Fixed(text) => text.into(),
    }
}

/// 比较名、站点键：只用字母、数字、`-`、`_`，最多 60 个字符。它们要当目录名、YAML 的键和文件名。
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 60
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 装 OpenBench 用的 PyPI 镜像（交给 uv 的 `--default-index`）。国内服务器直连 PyPI 常常很慢：7920 实测下载缓存
/// 每 20 秒只长 3 MB，十分钟还没装完；同一台机器连清华镜像 1.9 MB/s。只认 http(s) 地址，不许空白、引号与控制字符。
pub fn valid_index_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 300
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '`' | '\\'))
}

/// 站点 ID 写进站点清单 CSV：不能有逗号、引号、换行。
pub fn valid_station(station: &str) -> bool {
    !station.is_empty()
        && station.len() <= 80
        && !station
            .chars()
            .any(|c| matches!(c, ',' | '"' | '\n' | '\r'))
}

/// OpenBench 用 Python 的 `json.dumps` 输出，值里会有 `NaN`、`Infinity`（实测登记表里 23 处，如
/// `max_uparea: Infinity`），严格的 JSON 解析器不认。字符串外、作为值出现的这几个记号换成 `null`。
pub fn strict_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut last = ' ';
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if matches!(last, ':' | ',' | '[') {
            if let Some(token) = ["-Infinity", "Infinity", "NaN"]
                .iter()
                .find(|token| rest.starts_with(**token))
            {
                out.push_str("null");
                rest = &rest[token.len()..];
                last = 'l';
                continue;
            }
        }
        if c == '"' {
            in_string = true;
        }
        if !c.is_whitespace() {
            last = c;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// 看一个 history 目录：主 history 文件族、前缀、时间分辨率、年份、空间类型（station/patch/grid/
/// unstructured/unknown）、单点坐标或格点范围、其余文件族、带层维度的变量清单。用专用环境里的 xarray
/// 读，窗口进程不链接 NetCDF。脚本本体在 `src/scripts/inspect.py`：放进单独 .py 源码比塞在 Rust 原始
/// 字符串里好测（`scripts/test-bench-scripts.py` 用 python3 + xarray 造合成 NetCDF 直接执行它），
/// 本机 `python -c`、服务器 heredoc 跑的都是同一份。
pub const INSPECT_SCRIPT: &str = include_str!("scripts/inspect.py");

/// OpenBench 可选指标/评分及显示名：直接问已安装的 OpenBench registry，避免 GUI 自己维护一份会过期的清单。
pub const METRIC_SCRIPT: &str = r#"
import json
from openbench.core.registry import (
    IMPLEMENTED_METRIC_NAMES, IMPLEMENTED_SCORE_NAMES, METRIC_LABELS, SCORE_LABELS,
)
print(json.dumps({
    "metrics": [{"name": n, "label": METRIC_LABELS.get(n, n)} for n in IMPLEMENTED_METRIC_NAMES],
    "scores": [{"name": n, "label": SCORE_LABELS.get(n, n)} for n in IMPLEMENTED_SCORE_NAMES],
}))
"#;

/// OpenBench 画地图时 cartopy 要 Natural Earth 底图，缺了现下载（7920 实测下载 110m 的陆地、海岸线、河流、湖泊）。
/// 集群的计算节点多半不能上网（摆渡船只有登录节点能上网），评估作业到那里画图就失败；所以装 OpenBench 时在
/// 能上网的那台机器上先取好，放进 `XDG_DATA_HOME` 下 cartopy 自己的目录，之后不再下载。取不到只警告，不算装失败。
pub const PREFETCH_SCRIPT: &str = r#"
import sys
try:
    import cartopy.io.shapereader as shp
except Exception as error:
    print(f"cartopy unavailable, skipping basemap prefetch: {error}")
    sys.exit(0)
for category, name in [("physical", "land"), ("physical", "coastline"), ("physical", "ocean"),
                       ("physical", "rivers_lake_centerlines"), ("physical", "lakes"),
                       ("cultural", "admin_0_boundary_lines_land")]:
    for scale in ("110m", "50m"):
        try:
            shp.natural_earth(resolution=scale, category=category, name=name)
            print(f"basemap {scale} {name} ready")
        except Exception as error:
            print(f"basemap {scale} {name} not fetched: {error}")
"#;

/// 把单点算例的 history 并成一个站点文件，写 OpenBench 的站点清单（ID,SYEAR,EYEAR,LON,LAT,DIR）。
/// 调用参数：`folder prefix out_nc out_csv station [lon [lat]]`；history 文件缺坐标时必须给 lon/lat。
///
/// 为什么这么并：单点算例的 history 是每月一个文件，OpenBench 的站点模拟要一个站一个文件。只并只随时间
/// 变的地表变量（维度只有 `time`、`patch`）：带上土壤层、积雪层这些维度一起并，实测 AT-Neu 48 个文件
/// 吃到 17 GB 内存十分钟还没完；只并地表变量 9.5 秒、360 MB。被跳过的变量逐个打印原因，不静默丢。
/// 目录里的 tracer/unitcat 等文件族不参与合并（同一天两族拼一起会出现重复时刻、主变量一半 NaN）。
/// 脚本本体在 `src/scripts/prepare.py`，测试同 [`INSPECT_SCRIPT`]。
pub const PREPARE_SCRIPT: &str = include_str!("scripts/prepare.py");

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
