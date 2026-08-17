//! `namelist /group/ a, b, c &` 语句 —— CoLM 自己对「什么是可设字段」的定义。
//!
//! 原先按 `DEF_` 前缀白名单收顶层字段，那条判据是错的：`MOD_Namelist.F90`
//! 的 **Part 3: For Single Point** 整段用 `SITE_` / `USE_SITE_` 前缀，
//! 于是一个专做单点的项目，schema 恰好缺了单点那一节（21 个字段），
//! 另外还缺 3 个 `USE_srfdata_*` / `USE_zip_*`。
//!
//! 换成 namelist 判据还顺带解决了反方向的问题：有 6 个字段有声明有默认值
//! 但不在任何组里（用户改不了），以及 `ieee_arithmetic` 这种 `USE` 语句
//! 被误当成声明的情况。

use std::collections::BTreeMap;

/// 扫全文，得出「字段名 -> 它所属的 namelist 组名」。
///
/// 三处需要当心，都在真实文件里实测到：
/// 1. 续行里夹**空行**（`DEF_domain, &` 之后隔一行才是 `SITE_fsitedata`）——
///    按「上一行以 `&` 结尾就取下一行」会在这里断掉，只解析出 2 个成员；
/// 2. 续行符之后带**行尾注释**（`DEF_LAI_MONTHLY, & !add by ...`）；
/// 3. 有一个成员被宏包住（`DEF_file_GIEMS` 在 TRACER+BGC 下才可设）——
///    守卫行本身跳过，成员照收，因为它确实是字段。
pub fn groups(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        let line = strip_comment(raw);
        let trimmed = line.trim_start();
        let low = trimmed.to_ascii_lowercase();
        let Some(rest) = low.strip_prefix("namelist /") else {
            continue;
        };
        let Some(slash) = rest.find('/') else {
            continue;
        };
        let group = rest[..slash].trim().to_string();

        let mut body = trimmed["namelist /".len() + slash + 1..].to_string();
        loop {
            let t = body.trim_end();
            if !t.ends_with('&') {
                break;
            }
            body = t.trim_end_matches('&').to_string();
            let mut next = None;
            for l in lines.by_ref() {
                let s = strip_comment(l);
                if s.trim().is_empty() || s.trim_start().starts_with('#') {
                    continue;
                }
                next = Some(s.to_string());
                break;
            }
            let Some(next) = next else { break };
            body.push(' ');
            body.push_str(&next);
        }
        for name in body.split(',') {
            let n = name.trim();
            if !n.is_empty() {
                out.insert(n.to_string(), group.clone());
            }
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    match line.find('!') {
        Some(p) => &line[..p],
        None => line,
    }
}
