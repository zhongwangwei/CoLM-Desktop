//! `MOD_WorkerPushData` 的单进程版本：patch ↔ 汇流输入网格（`inpm`）↔ 单元流域。
//!
//! 上游每个原语都带一个填充值 `fillvalue`：等于它的输入视为缺测，不参与、也不计入
//! `average` 的面积和；输出先置成它，第一份直接赋值（`v*area`），之后逐份相加。这里的
//! 加法不收缩成 FMA（与汇流、history 已验证的形状相同）。

use super::network::RunoffRouting;

/// 逐份累加：第一份直接赋值，之后相加。
fn accumulate(slot: &mut f64, term: f64, fill: f64) {
    *slot = if *slot == fill { term } else { *slot + term };
}

/// `worker_remap_data_grid2pset`：每个 patch 取它所占网格的值；`average` 除以非缺测份的面积和。
pub fn grid_to_patches(
    routing: &RunoffRouting,
    grid: &[f64],
    fill: f64,
    average: bool,
) -> Vec<f64> {
    routing
        .patch_parts
        .iter()
        .map(|parts| {
            let mut value = fill;
            let mut area_sum = 0.0;
            for &(k, area) in parts {
                if grid[k] == fill {
                    continue;
                }
                accumulate(&mut value, grid[k] * area, fill);
                area_sum += area;
            }
            if average && value != fill && area_sum > 0.0 {
                value / area_sum
            } else {
                value
            }
        })
        .collect()
}

/// `worker_remap_data_pset2grid`：过滤掉的与缺测的 patch 不参与，按份面积累加（不平均）。
pub fn patches_to_grid(
    routing: &RunoffRouting,
    values: &[f64],
    filter: &[bool],
    fill: f64,
) -> Vec<f64> {
    let mut grid = vec![fill; routing.grids.len()];
    for ((parts, &value), &keep) in routing.patch_parts.iter().zip(values).zip(filter) {
        if !keep || value == fill {
            continue;
        }
        for &(k, area) in parts {
            accumulate(&mut grid[k], value * area, fill);
        }
    }
    grid
}

/// `push_ucat2inpm`：每个网格按单元流域的份面积（`area_uc2gd`）累加；`average` 除以面积和。
pub fn catchments_to_inpm(
    routing: &RunoffRouting,
    values: &[f64],
    fill: f64,
    average: bool,
) -> Vec<f64> {
    routing
        .grid_catchments
        .iter()
        .map(|entries| {
            let mut value = fill;
            let mut area_sum = 0.0;
            for &(i, area) in entries {
                if values[i] == fill {
                    continue;
                }
                accumulate(&mut value, values[i] * area, fill);
                area_sum += area;
            }
            if average && value != fill && area_sum > 0.0 {
                value / area_sum
            } else {
                value
            }
        })
        .collect()
}

/// `push_inpm2ucat`：每个单元流域按 `inpmat` 的次序累加它的份面积（区域外的份没有值，跳过）。
/// 返回全网络长度的向量，没有份在区域里的单元流域是 `fill`。
pub fn inpm_to_catchments(
    routing: &RunoffRouting,
    grid: &[f64],
    catchments: usize,
    fill: f64,
    average: bool,
) -> Vec<f64> {
    let mut out = vec![fill; catchments];
    for (i, entries) in &routing.catchments {
        let mut value = fill;
        let mut area_sum = 0.0;
        for &(k, area) in entries {
            let Some(k) = k else { continue };
            if grid[k] == fill {
                continue;
            }
            accumulate(&mut value, grid[k] * area, fill);
            area_sum += area;
        }
        out[*i] = if average && value != fill && area_sum > 0.0 {
            value / area_sum
        } else {
            value
        };
    }
    out
}
