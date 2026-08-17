//! 儒略日与公历日期的互换。
//!
//! 强迫场的时间轴是「自某个时刻起的秒数」，而 namelist 要的是 `startyr`/`endyr`
//! 这样的年月。中间这一步换算自己写，不引入 chrono —— 本仓库的依赖每多一个，
//! 三个平台的静态构建就多一处可能出岔的地方，而这里要的只是两个函数。
//!
//! 算法是 Howard Hinnant 的 `days_from_civil` / `civil_from_days`，
//! 对公历前推有效，不做闰秒。`civil_tests.rs` 对 1900–2100 逐日往返验证。

/// 1970-01-01 起的天数。公历，可为负。
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// `days_from_civil` 的逆。
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

/// 一个公历时刻。不带时区 —— PLUMBER2 的时间轴是**地方时**，
/// 而 CoLM 单点正是靠 `DEF_simulation_time%greenwich = .FALSE.` 接受这一点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    /// 往后走整数秒。
    pub fn plus_seconds(&self, secs: i64) -> Stamp {
        let day = days_from_civil(self.year, self.month, self.day);
        let tod = self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64;
        let total = day * 86400 + tod + secs;
        let (d, rem) = (total.div_euclid(86400), total.rem_euclid(86400));
        let (year, month, dayn) = civil_from_days(d);
        Stamp {
            year,
            month,
            day: dayn,
            hour: (rem / 3600) as u32,
            minute: ((rem % 3600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }
}

#[cfg(test)]
#[path = "civil_tests.rs"]
mod civil_tests;
