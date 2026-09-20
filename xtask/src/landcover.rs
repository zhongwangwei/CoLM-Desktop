//! `MOD_Const_LC.F90` -> 地类常量表。
//!
//! 这个模块的声明里**混着被注释掉的旧版本**：一条 `htop0_usgs &` 后面先跟三行
//! `!= (...) &`，再跟真正的 `= (...) `。Fortran 的规则是注释行不终止续行，所以
//! 「按 `&` 拼接物理行」是错的 —— 那样会把注释里的旧表当成取值。正确顺序是：
//! 先按 `!` 截断注释，**整行只剩空白的跳过**（它不终止也不参与续行），再判断
//! 是否以 `&` 结尾。
//!
//! 分支用栈跟踪：`#ifdef LULC_USGS` 的那一支与 `#else` 的那一支是两套表，同名
//! 只差后缀 `_usgs` / `_igbp`。`Init_LC_Const` 里还有一层同名 `#ifdef`，栈能把它
//! 与外层区分开（那些是赋值不是声明，本来也会被过滤掉）。
//!
//! 认不出的取值形态一律**报错**：静默跳过会让某张表悄悄缺失，而缺失的表现是
//! 「某个地类的参数变成 0」—— 数值上极难发现。
//!
//! **产物必须 rustfmt 稳定**：`cargo fmt --all` 会重排入库文件，而 drift 测试是
//! 逐字节比较，重排一次就会被打回。实测两处：数组要 `#[rustfmt::skip]`（竖排会把
//! 619 行那种表撑成几千行），文件末尾只留一个换行（多一个空行 rustfmt 会删）。

use std::fmt::Write as _;

use anyhow::{bail, Context, Result};

/// 一张 `parameter, dimension(N_land_classification)` 表。
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// 去掉 `_igbp` / `_usgs` 后缀的名字，与 Fortran 里被选中的数组同名。
    pub base: String,
    pub integer: bool,
    pub values: Vec<String>,
    /// `LULC_USGS` 分支还是默认（IGBP）分支。
    pub usgs: bool,
    /// 声明所在的源码行号，只用于报错。
    pub line: usize,
}

/// 一条已经按 Fortran 规则拼好、并去掉注释的逻辑语句。
struct Statement {
    text: String,
    line: usize,
    usgs: bool,
}

pub fn tables(text: &str) -> Result<Vec<Table>> {
    let mut tables = Vec::new();
    for statement in statements(text)? {
        let Some(table) = parse_declaration(&statement)? else {
            continue;
        };
        tables.push(table);
    }
    if tables.len() < 80 {
        bail!(
            "only {} land-cover tables found — the declaration style must have changed",
            tables.len()
        );
    }
    // 同一分支里每张**列表形式**的表长度必须一致，否则说明有表被截断或漏读。
    // 长度为 1 的是标量广播（Fortran 里 `= 0.1` 那种），不参与这项核对。
    for usgs in [false, true] {
        let mut lengths = tables
            .iter()
            .filter(|table| table.usgs == usgs && table.values.len() > 1)
            .map(|table| table.values.len())
            .collect::<Vec<_>>();
        lengths.sort_unstable();
        lengths.dedup();
        if lengths.len() != 1 {
            bail!(
                "the {} land-cover tables disagree on their class count: {lengths:?}",
                if usgs { "USGS" } else { "IGBP" }
            );
        }
    }
    Ok(tables)
}

fn statements(text: &str) -> Result<Vec<Statement>> {
    let mut out = Vec::new();
    let mut pending: Option<Statement> = None;
    // `#ifdef` / `#else` 的当前分支；栈保证内层同名 `#ifdef` 不会污染外层。
    let mut stack: Vec<bool> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.starts_with('#') {
            // 指令行不能夹在续行中间；真出现就报错，别猜。
            if pending.is_some() {
                bail!("line {line}: preprocessor directive inside a continued statement");
            }
            if trimmed.starts_with("#ifdef ") || trimmed.starts_with("#if ") {
                stack.push(trimmed.contains("LULC_USGS"));
            } else if trimmed.starts_with("#else") {
                let current = stack
                    .last_mut()
                    .with_context(|| format!("line {line}: #else without #if"))?;
                *current = !*current;
            } else if trimmed.starts_with("#endif") {
                stack
                    .pop()
                    .with_context(|| format!("line {line}: #endif without #if"))?;
            }
            continue;
        }
        // 注释在续行判断**之前**截掉；截完只剩空白的行既不终止也不参与续行。
        let code = match raw.find('!') {
            Some(position) => &raw[..position],
            None => raw,
        };
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let usgs = stack.last().copied().unwrap_or(false);
        match pending.as_mut() {
            Some(statement) => {
                statement.text.push(' ');
                statement.text.push_str(code);
            }
            None => {
                pending = Some(Statement {
                    text: code.to_string(),
                    line,
                    usgs,
                });
            }
        }
        let continues = code.ends_with('&');
        if continues {
            let statement = pending.as_mut().expect("just pushed");
            statement.text.pop();
        } else {
            out.push(pending.take().expect("just pushed"));
        }
    }
    if let Some(statement) = pending {
        bail!("line {}: unterminated continued statement", statement.line);
    }
    Ok(out)
}

fn parse_declaration(statement: &Statement) -> Result<Option<Table>> {
    let lower = statement.text.to_ascii_lowercase();
    let Some(marker) = lower.find("::") else {
        return Ok(None);
    };
    if !lower[..marker].contains("parameter") {
        return Ok(None);
    }
    if !lower[..marker].contains("n_land_classification") {
        bail!(
            "line {}: a parameter array of an unexpected shape: {}",
            statement.line,
            statement.text
        );
    }
    let integer = lower[..marker].contains("integer");
    let rest = statement.text[marker + 2..].trim();
    let (name, value) = match rest.split_once('=') {
        Some((name, value)) => (name.trim(), Some(value.trim())),
        None => bail!(
            "line {}: land-cover table {rest:?} has no value",
            statement.line
        ),
    };
    let base = name
        .strip_suffix("_igbp")
        .or_else(|| name.strip_suffix("_usgs"))
        .with_context(|| {
            format!(
                "line {}: land-cover table {name} has neither the _igbp nor the _usgs suffix",
                statement.line
            )
        })?
        .to_string();
    let Some(value) = value else {
        bail!("line {}: unreachable", statement.line);
    };
    let values = values(value, statement.line)?;
    Ok(Some(Table {
        base,
        integer,
        values,
        usgs: statement.usgs,
        line: statement.line,
    }))
}

/// `(/ a, b, c /)` 的列表（可带一个尾部乘数），或一个标量（Fortran 里表示整数组同值）。
fn values(value: &str, line: usize) -> Result<Vec<String>> {
    let value = value.trim();
    if let Some(body) = value.strip_prefix("(/") {
        let (inner, tail) = body.split_once("/)").with_context(|| {
            format!("line {line}: a land-cover table list is not closed with /)")
        })?;
        // 八张植物水力表在列表后面各跟一个 `*1`。它是上游自己的恒等写法，照抄即可；
        // 但换成别的乘数就**不能**默默折进表里 —— 那会把「上游写了缩放」这件事从
        // diff 里抹掉，取值该由人看一眼再决定怎么放。
        let tail = tail.trim().trim_start_matches('*').trim();
        if !tail.is_empty() {
            ensure_unit_multiplier(tail, line)?;
        }
        let out: Vec<String> = inner
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect();
        if out.is_empty() {
            bail!("line {line}: an empty land-cover table");
        }
        return Ok(out);
    }
    if value
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E' | 'd' | 'D'))
        && !value.is_empty()
    {
        return Ok(vec![value.to_string()]);
    }
    bail!("line {line}: unrecognised land-cover value {value:?}")
}

fn ensure_unit_multiplier(multiplier: &str, line: usize) -> Result<()> {
    if matches!(multiplier, "1" | "1." | "1.0") {
        return Ok(());
    }
    bail!(
        "line {line}: a land-cover table is multiplied by {multiplier:?}; fold that into the \
         generated table by hand and record why, rather than letting the generator decide"
    )
}

/// 把两张分支的表渲染成入库的 Rust。
pub fn render(tables: &[Table]) -> Result<String> {
    let count = |usgs: bool| {
        tables
            .iter()
            .find(|table| table.usgs == usgs)
            .map(|table| table.values.len())
            .expect("both branches were validated")
    };
    let igbp_classes = count(false);
    let usgs_classes = count(true);

    let mut out = String::new();
    out.push_str(
        "//! 地类常量表，由 `cargo run -p xtask -- gen-landcover` 从\n\
         //! `vendor/CoLM202X/main/MOD_Const_LC.F90` 生成。**不要手改**：\n\
         //! `crates/colm-core/tests/drift_landcover.rs` 会重新生成并逐字节比对。\n\
         //!\n\
         //! 取值是 `Init_LC_Const` 里那些 `*_igbp` / `*_usgs` 常量的**字面副本**，\n\
         //! 不含 `vmax25 * 1.e-6` 这类赋值处的缩放 —— 缩放写在访问器里，这样表\n\
         //! 与上游源码可以直接对照。`#ifdef LULC_USGS` 的两支都在这里，选哪一支\n\
         //! 由调用方决定。\n\n",
    );
    writeln!(out, "pub const IGBP_CLASSES: usize = {igbp_classes};")?;
    writeln!(out, "pub const USGS_CLASSES: usize = {usgs_classes};")?;
    out.push('\n');

    let mut sorted: Vec<&Table> = tables.iter().collect();
    sorted.sort_by(|left, right| {
        left.base
            .cmp(&right.base)
            .then_with(|| left.usgs.cmp(&right.usgs))
    });

    let mut bases: Vec<String> = Vec::new();
    for table in &sorted {
        if !bases.contains(&table.base) {
            bases.push(table.base.clone());
        }
    }
    // 两张分支必须给出同一批名字，否则访问器会少一个字段。
    for base in &bases {
        for usgs in [false, true] {
            if !sorted
                .iter()
                .any(|table| &table.base == base && table.usgs == usgs)
            {
                bail!(
                    "land-cover table {base} exists only in the {} branch",
                    if usgs { "USGS" } else { "IGBP" }
                );
            }
        }
    }

    for table in &sorted {
        let name = constant_name(&table.base, table.usgs);
        let element = if table.integer { "i32" } else { "f64" };
        let classes = if table.usgs {
            usgs_classes
        } else {
            igbp_classes
        };
        let values: Vec<String> = if table.values.len() == 1 {
            vec![table.values[0].clone(); classes]
        } else {
            table.values.clone()
        };
        if values.len() != classes {
            bail!(
                "{} has {} values but its branch has {classes} classes",
                table.base,
                values.len()
            );
        }
        // Fortran 的 `2.e-008` / `0.` / `1.d0` 都不是合法 Rust 字面量，而且
        // **必须真能解析**才算数：照抄会生成一个连编译都过不了的表。
        let values = values
            .iter()
            .map(|value| rust_number(value, table.integer, table.line))
            .collect::<Result<Vec<_>>>()?;
        // 一张表一段、每段数字自己排版，rustfmt 会把它整个重排成竖排，那样上游
        // 改一个数在 review 里就看不出改了什么。加 `#[rustfmt::skip]` 把它钉住，
        // 与 `colm-hist` 的闸门表同一个理由。
        writeln!(out, "#[rustfmt::skip]")?;
        writeln!(
            out,
            "pub const {name}: [{element}; {}] = [",
            if table.usgs {
                "USGS_CLASSES"
            } else {
                "IGBP_CLASSES"
            }
        )?;
        for chunk in values.chunks(8) {
            writeln!(out, "    {},", chunk.join(", "))?;
        }
        out.push_str("];\n\n");
    }

    out.push_str(
        "/// 一个地类分类体系下的全部常量。字段名与上游被选中的数组同名（小写保持\n\
         /// 上游拼写），单位与 `MOD_Const_LC.F90` 一致。\n\
         #[derive(Debug, Clone, Copy)]\n\
         pub struct LandCoverTables {\n",
    );
    for base in &bases {
        writeln!(
            out,
            "    pub {base}: &'static [{}],",
            element_of(&sorted, base)
        )?;
    }
    out.push_str("}\n\n");

    for (label, usgs) in [("IGBP", false), ("USGS", true)] {
        writeln!(
            out,
            "pub const {label}: LandCoverTables = LandCoverTables {{"
        )?;
        for base in &bases {
            writeln!(out, "    {base}: &{},", constant_name(base, usgs))?;
        }
        out.push_str("};\n\n");
    }

    // 末尾只留一个换行：多一个空行 rustfmt 会删掉，drift 就会逐字节打回。
    Ok(format!("{}\n", out.trim_end()))
}

fn element_of(sorted: &[&Table], base: &str) -> &'static str {
    let table = sorted
        .iter()
        .find(|table| table.base == base)
        .expect("every base was checked in both branches");
    if table.integer {
        "i32"
    } else {
        "f64"
    }
}

fn constant_name(base: &str, usgs: bool) -> String {
    format!(
        "{}_{}",
        base.to_ascii_uppercase(),
        if usgs { "USGS" } else { "IGBP" }
    )
}

/// 把一个 Fortran 数值字面量写成合法的 Rust 字面量。
///
/// `2.e-008`、`0.`、`1.d0` 在 Fortran 里都合法，在 Rust 里都不是。转换之后
/// **必须真的能解析**才返回：这条检查比"看起来对"重要得多 —— 生成器写错一个
/// 数，编译能过而取值悄悄变了。
fn rust_number(raw: &str, integer: bool, line: usize) -> Result<String> {
    let text = raw.trim().replace(['d', 'D'], "e");
    if integer {
        let value: i64 = text
            .parse()
            .with_context(|| format!("line {line}: {raw:?} is not an integer"))?;
        return Ok(value.to_string());
    }
    let mut text = text;
    match text.find(['e', 'E']) {
        // `2.e-008`：指数前的小数点后面补一个 0。
        Some(position) if text[..position].ends_with('.') => text.insert(position, '0'),
        Some(_) => {}
        None if text.ends_with('.') => text.push('0'),
        // `d50` 里有写成 `100` 的实数项；Rust 不会把整数字面量推断成 f64。
        None if !text.contains('.') => text.push_str(".0"),
        None => {}
    }
    let value: f64 = text
        .parse()
        .with_context(|| format!("line {line}: {raw:?} is not a real number"))?;
    ensure_finite(value, raw, line)?;
    Ok(text)
}

fn ensure_finite(value: f64, raw: &str, line: usize) -> Result<()> {
    if value.is_finite() {
        return Ok(());
    }
    bail!("line {line}: the land-cover literal {raw:?} is not finite")
}
