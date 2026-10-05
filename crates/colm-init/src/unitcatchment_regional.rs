//! 区域单元流域网络（`DEF_UnitCatchment_regional`）。
//!
//! 上游在 mksrfdata 里建完 landpatch 后调 `unitcatchment_regional_build`
//! （`mksrfdata/MOD_UnitCatchmentRegional.F90`）：
//! 1. 陆面 patch 覆盖到的汇流输入网格（`build_worker_remapdata (landpatch, gridro)` 的 `ids_me`）记为 `touched`；
//! 2. `unitcatchment_subset_write`（`mksrfdata/MOD_UnitCatchmentSubset.F90:300-534`）按 `inpmat_x/y` 找出
//!    接收这些格子径流的单元流域，保留它们所在的整条河流系统（同一个河口）；开 `bif_closure` 时再沿
//!    分汊路径闭包；
//! 3. 沿 `nseqmax`/`npthout`/`dam_ndams` 三个维裁剪全部变量，6 个存单元流域号的变量重编号，
//!    另写 `seq_src_index` 与几个全局属性，落在 `<landdata>/riverlake/unitcatchment_regional.nc`。
//!
//! Rust 的 mksrfdata 拿不到面积映射（`colm-srfdata` 不依赖本 crate），所以由 mkinidata-rs 在冷启动前
//! 生成；产物与纯 Fortran 预处理的那份逐变量、逐属性比对。字符变量不写（见 `subset_write`）。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{NcTypeDescriptor, NcVariableType};

use crate::spatial_grid::LatLonGrid;
use crate::spatial_mapping::{AreaWeightedMapping, PixelAxes};

/// `REGIONAL_UNITCATCHMENT_SUFFIX`（`share/MOD_Namelist.F90:395`）。
pub const REGIONAL_SUFFIX: &str = "riverlake/unitcatchment_regional.nc";

const SEQ_DIM: &str = "nseqmax";
const PATH_DIM: &str = "npthout";
const DAM_DIM: &str = "dam_ndams";

/// `regional_unitcatchment_file`：`trim(DEF_dir_landdata) // '/riverlake/unitcatchment_regional.nc'`。
pub fn regional_file(landdata: &Path) -> PathBuf {
    landdata.join(REGIONAL_SUFFIX)
}

/// 汇流输入网格的大小（`nx`/`ny` 维）。
pub fn grid_size(file: &Path) -> Result<(usize, usize)> {
    let network = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let length = |name: &str| -> Result<usize> {
        Ok(network
            .dimension(name)
            .with_context(|| format!("{} has no dimension {name}", file.display()))?
            .len())
    };
    Ok((length("nx")?, length("ny")?))
}

/// 陆面 patch 覆盖到的汇流输入格子（`touched(ix,iy)`，按 `iy*nlon + ix` 平铺，0 起）。
///
/// 与运行期 `RunoffRouting` 的 `grids` 同一个映射：`build_arealweighted (gridro, landpatch)` 的全部份。
pub fn touched_cells(landdata: &Path, year: i32, nlon: usize, nlat: usize) -> Result<Vec<bool>> {
    let grid = LatLonGrid::define_by_ndims(nlon, nlat)?;
    let directory = landdata.join("landpatch").join(format!("{year:04}"));
    let mut blocks = std::fs::read_dir(&directory)
        .with_context(|| format!("cannot list {}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_prefix("landpatch_")
                .and_then(|rest| rest.strip_suffix(".nc"))
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    blocks.sort();
    ensure!(
        !blocks.is_empty(),
        "{} holds no landpatch blocks",
        directory.display()
    );
    let mut touched = vec![false; nlon * nlat];
    for block in blocks {
        let patches = crate::spatial_static::read_patches(landdata, year, &block)?;
        let sets = crate::spatial_static::read_spatial_pixel_sets(
            landdata,
            year,
            &block,
            &patches.element,
            &patches.start,
            &patches.end,
            &patches.shared_fraction,
            "landpatch",
        )?;
        let axes = PixelAxes {
            lon_w: sets.lon_w.clone(),
            lon_e: sets.lon_e.clone(),
            lat_s: sets.lat_s.clone(),
            lat_n: sets.lat_n.clone(),
        };
        let mapping = AreaWeightedMapping::build(&grid, &axes, &sets.cells, &sets.shared_fraction)?;
        for part in mapping.parts.iter().flatten() {
            touched[part.ilat * nlon + part.ilon] = true;
        }
    }
    Ok(touched)
}

/// 选中的单元流域与它们的新下标。
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// 保留的单元流域（源网络里的 1 起序号，递增）。
    pub seq: Vec<usize>,
    /// 保留的分汊路径（1 起）。
    pub path: Vec<usize>,
    /// 保留的水库（1 起）。
    pub dam: Vec<usize>,
    /// 源序号 → 新序号（1 起，0 为未选）；下标 0 不用。
    pub new_index: Vec<i32>,
    /// 保留的河流系统个数。
    pub systems: usize,
}

/// 选择逻辑（`unitcatchment_subset_write` 的前半，`:320-420`），与文件读写分开，便于单测。
#[allow(clippy::too_many_arguments)]
pub fn select(
    seq_next: &[i32],
    inpmat: &[(Vec<i32>, Vec<i32>)],
    nlon: usize,
    nlat: usize,
    touched: &[bool],
    bifurcation: Option<(&[i32], &[i32])>,
    dam_seq: Option<&[i32]>,
    bif_closure: bool,
) -> Result<Selection> {
    let nseq = seq_next.len();
    ensure!(
        inpmat.len() == nseq,
        "seq_next and the input matrix do not agree on the number of unit catchments"
    );
    // 河口：`seq_next` 自上游向下游有序，从下往上回溯。
    let mut mouth = (0..nseq).collect::<Vec<_>>();
    for (i, &next) in seq_next.iter().enumerate() {
        if next > 0 {
            ensure!(
                (next as usize) > i + 1,
                "seq_next is not ordered from upstream to downstream"
            );
        }
    }
    for i in (0..nseq).rev() {
        if seq_next[i] > 0 {
            mouth[i] = mouth[seq_next[i] as usize - 1];
        }
    }
    let receives = inpmat
        .iter()
        .map(|(xs, ys)| {
            xs.iter().zip(ys).any(|(&x, &y)| {
                x >= 1
                    && (x as usize) <= nlon
                    && y >= 1
                    && (y as usize) <= nlat
                    && touched[(y as usize - 1) * nlon + (x as usize - 1)]
            })
        })
        .collect::<Vec<_>>();
    ensure!(
        receives.iter().any(|&r| r),
        "no unit catchment receives runoff from the land domain; check that the network matches the domain"
    );
    let mut system_kept = vec![false; nseq];
    for i in 0..nseq {
        if receives[i] {
            system_kept[mouth[i]] = true;
        }
    }
    if let Some((up, down)) = bifurcation {
        ensure!(
            up.iter()
                .chain(down)
                .all(|&s| s >= 1 && (s as usize) <= nseq),
            "a bifurcation pathway refers to a unit catchment outside the network"
        );
        if bif_closure {
            let mut changed = true;
            while changed {
                changed = false;
                for (&u, &d) in up.iter().zip(down) {
                    let (i, j) = (mouth[u as usize - 1], mouth[d as usize - 1]);
                    if system_kept[i] != system_kept[j] {
                        system_kept[i] = true;
                        system_kept[j] = true;
                        changed = true;
                    }
                }
            }
        }
    }
    let keep = (0..nseq).map(|i| system_kept[mouth[i]]).collect::<Vec<_>>();
    let mut new_index = vec![0_i32; nseq + 1];
    let mut seq = Vec::new();
    for i in 0..nseq {
        if keep[i] {
            seq.push(i + 1);
            new_index[i + 1] = i32::try_from(seq.len())?;
        }
    }
    let path = match bifurcation {
        Some((up, down)) => (0..up.len())
            .filter(|&p| keep[up[p] as usize - 1] && keep[down[p] as usize - 1])
            .map(|p| p + 1)
            .collect(),
        None => Vec::new(),
    };
    let dam = match dam_seq {
        Some(dams) => (0..dams.len())
            .filter(|&k| dams[k] >= 1 && (dams[k] as usize) <= nseq && keep[dams[k] as usize - 1])
            .map(|k| k + 1)
            .collect(),
        None => Vec::new(),
    };
    Ok(Selection {
        seq,
        path,
        dam,
        new_index,
        systems: system_kept.iter().filter(|&&kept| kept).count(),
    })
}

/// `holds_unitcatchment_numbers`：存单元流域号、要重编号的变量。
fn holds_unitcatchment_numbers(name: &str) -> bool {
    matches!(
        name,
        "seq" | "seq_next" | "seq_upst" | "bifurcation_upst" | "bifurcation_down" | "dam_seq"
    )
}

/// 沿 `axis` 只取 `keep`（1 起）里的那些下标；其余轴原样。`shape` 是 C 序。
fn cut<T: Copy>(values: &[T], shape: &[usize], axis: Option<(usize, &[usize])>) -> Vec<T> {
    let Some((axis, keep)) = axis else {
        return values.to_vec();
    };
    let inner: usize = shape[axis + 1..].iter().product();
    let outer: usize = shape[..axis].iter().product();
    let length = shape[axis];
    let mut out = Vec::with_capacity(outer * keep.len() * inner);
    for o in 0..outer {
        for &k in keep {
            let start = (o * length + (k - 1)) * inner;
            out.extend_from_slice(&values[start..start + inner]);
        }
    }
    out
}

fn copy_typed<T: NcTypeDescriptor + Copy>(
    from: &netcdf::Variable,
    to: &mut netcdf::FileMut,
    dims: &[String],
    shape: &[usize],
    axis: Option<(usize, &[usize])>,
    map: impl Fn(Vec<T>) -> Result<Vec<T>>,
) -> Result<()> {
    let values: Vec<T> = from.get_values(netcdf::Extents::All)?;
    let values = map(cut(&values, shape, axis))?;
    let names = dims.iter().map(String::as_str).collect::<Vec<_>>();
    let mut output = to.add_variable::<T>(&from.name(), &names)?;
    for attribute in from.attributes() {
        output.put_attribute(attribute.name(), attribute.value()?)?;
    }
    if !values.is_empty() {
        output.put_values(&values, netcdf::Extents::All)?;
    }
    Ok(())
}

/// `unitcatchment_subset_write`：读源网络、选择、按选择裁剪写出。返回 (保留的单元流域数, 河流系统数)。
pub fn subset_write(
    file_in: &Path,
    file_out: &Path,
    nlon: usize,
    nlat: usize,
    touched: &[bool],
    bif_closure: bool,
) -> Result<(usize, usize)> {
    let source =
        netcdf::open(file_in).with_context(|| format!("cannot open {}", file_in.display()))?;
    let length = |name: &str| source.dimension(name).map(|d| d.len());
    let nseq = length(SEQ_DIM).with_context(|| format!("dimension {SEQ_DIM} not found"))?;
    let ints = |name: &str| -> Result<Vec<i32>> {
        source
            .variable(name)
            .with_context(|| format!("variable {name} not found"))?
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {name}"))
    };
    let seq_next = ints("seq_next")?;
    let (inpmat_x, inpmat_y) = (ints("inpmat_x")?, ints("inpmat_y")?);
    ensure!(
        seq_next.len() == nseq && inpmat_x.len() == inpmat_y.len() && inpmat_x.len() % nseq == 0,
        "seq_next and the input matrix do not agree on the number of unit catchments"
    );
    // `inpmat_x(nseqmax, inpn)`：C 序里单元流域在前。
    let inpn = inpmat_x.len() / nseq;
    let inpmat = (0..nseq)
        .map(|i| {
            (
                inpmat_x[i * inpn..(i + 1) * inpn].to_vec(),
                inpmat_y[i * inpn..(i + 1) * inpn].to_vec(),
            )
        })
        .collect::<Vec<_>>();
    let npth = length(PATH_DIM);
    let bifurcation = match npth {
        Some(_) => Some((ints("bifurcation_upst")?, ints("bifurcation_down")?)),
        None => None,
    };
    let dam_seq = match length(DAM_DIM) {
        Some(n) if n > 0 => Some(ints("dam_seq")?),
        _ => None,
    };
    let selection = select(
        &seq_next,
        &inpmat,
        nlon,
        nlat,
        touched,
        bifurcation
            .as_ref()
            .map(|(up, down)| (up.as_slice(), down.as_slice())),
        dam_seq.as_deref(),
        bif_closure,
    )?;

    if let Some(parent) = file_out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut out = netcdf::create(file_out)
        .with_context(|| format!("cannot create {}", file_out.display()))?;
    for dimension in source.dimensions() {
        let name = dimension.name();
        let size = match name.as_str() {
            SEQ_DIM => selection.seq.len(),
            PATH_DIM => selection.path.len(),
            DAM_DIM => selection.dam.len(),
            _ => dimension.len(),
        };
        out.add_dimension(&name, size)?;
    }
    for attribute in source.attributes() {
        out.add_attribute(attribute.name(), attribute.value()?)?;
    }
    let nseqriv_old = match source.attribute("nseqriv").map(|a| a.value()) {
        Some(Ok(netcdf::AttributeValue::Longlong(value))) => value as usize,
        Some(Ok(netcdf::AttributeValue::Int(value))) => value as usize,
        _ => nseq,
    };
    let nseqriv_new = selection
        .seq
        .iter()
        .filter(|&&s| s <= nseqriv_old.min(nseq))
        .count();
    let count = |value: usize| netcdf::AttributeValue::Longlong(value as i64);
    out.add_attribute("nseqall", count(selection.seq.len()))?;
    out.add_attribute("nseqmax", count(selection.seq.len()))?;
    out.add_attribute("nseqriv", count(nseqriv_new))?;
    if npth.is_some() {
        out.add_attribute("npthout", count(selection.path.len()))?;
    }
    if length(DAM_DIM).is_some() {
        out.add_attribute("dam_ndams", count(selection.dam.len()))?;
    }
    out.add_attribute("source_nseqmax", count(nseq))?;
    out.add_attribute(
        "subset_bif_mode",
        if bif_closure { "closure" } else { "drop" },
    )?;
    out.add_attribute(
        "subset_note",
        "regional subset written by mksrfdata (DEF_UnitCatchment_regional)",
    )?;

    for variable in source.variables() {
        let name = variable.name();
        let dims = variable
            .dimensions()
            .iter()
            .map(|d| d.name())
            .collect::<Vec<_>>();
        let shape = variable
            .dimensions()
            .iter()
            .map(|d| d.len())
            .collect::<Vec<_>>();
        ensure!(
            dims.len() <= 2,
            "variable {name} has more than two dimensions"
        );
        let cut_axes = dims
            .iter()
            .enumerate()
            .filter(|(_, d)| matches!(d.as_str(), SEQ_DIM | PATH_DIM | DAM_DIM))
            .collect::<Vec<_>>();
        ensure!(
            cut_axes.len() <= 1,
            "variable {name} has two cut dimensions"
        );
        let axis = cut_axes.first().map(|(axis, dim)| {
            let keep: &[usize] = match dim.as_str() {
                SEQ_DIM => &selection.seq,
                PATH_DIM => &selection.path,
                _ => &selection.dam,
            };
            (*axis, keep)
        });
        let renumber = holds_unitcatchment_numbers(&name);
        match variable.vartype() {
            NcVariableType::Int(netcdf::types::IntType::I32) => {
                let new_index = &selection.new_index;
                copy_typed::<i32>(&variable, &mut out, &dims, &shape, axis, |mut values| {
                    if renumber {
                        for value in &mut values {
                            if *value > 0 {
                                let index = *value as usize;
                                ensure!(index <= nseq, "{name} refers beyond the network");
                                ensure!(
                                    new_index[index] > 0,
                                    "{name} refers to a unit catchment outside the selection"
                                );
                                *value = new_index[index];
                            }
                        }
                    }
                    Ok(values)
                })?;
            }
            NcVariableType::Float(netcdf::types::FloatType::F64) => {
                ensure!(!renumber, "index variable {name} must be integer");
                copy_typed::<f64>(&variable, &mut out, &dims, &shape, axis, Ok)?;
            }
            // 字符变量（如 `dam_DamName`）按 `NC_CHAR` 原样裁剪拷贝（`colm_ncchar::NcChar`）。
            NcVariableType::Char => {
                ensure!(!renumber, "index variable {name} must be integer");
                copy_typed::<colm_ncchar::NcChar>(&variable, &mut out, &dims, &shape, axis, Ok)?;
            }
            kind => bail!("variable {name} has an unsupported data type {kind:?}"),
        }
    }
    let src_index = selection
        .seq
        .iter()
        .map(|&s| i32::try_from(s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut variable = out.add_variable::<i32>("seq_src_index", &[SEQ_DIM])?;
    variable.put_attribute(
        "long_name",
        "index of this unit catchment in the source network",
    )?;
    variable.put_values(&src_index, netcdf::Extents::All)?;
    Ok((selection.seq.len(), selection.systems))
}

/// 区域模式下水库表的 `dam_seq` 换成区域网络的编号（`MOD_Grid_Reservoir.F90:91-110`）：
/// 参数表本身必须是源网络编号（带 `seq_src_index` 的表拒绝），按区域文件的 `seq_src_index` 反查；
/// 不在区域网络里的坝记为 `-行号`，不会匹配到任何单元流域。
pub fn translate_dam_seq(dam_seq: &mut [i32], parameters: &Path, regional: &Path) -> Result<()> {
    let table = netcdf::open(parameters)
        .with_context(|| format!("cannot open {}", parameters.display()))?;
    ensure!(
        table.variable("seq_src_index").is_none(),
        "DEF_ReservoirPara_file must use the source unit catchment numbering"
    );
    let network =
        netcdf::open(regional).with_context(|| format!("cannot open {}", regional.display()))?;
    let src_index: Vec<i32> = network
        .variable("seq_src_index")
        .with_context(|| format!("{} has no seq_src_index", regional.display()))?
        .get_values(netcdf::Extents::All)?;
    let size = src_index.iter().copied().max().unwrap_or(1).max(1) as usize;
    let mut regional_index = vec![0_i32; size + 1];
    for (i, &source) in src_index.iter().enumerate() {
        regional_index[source as usize] = i32::try_from(i + 1)?;
    }
    for (row, seq) in dam_seq.iter_mut().enumerate() {
        let local = if *seq >= 1 && (*seq as usize) <= size {
            regional_index[*seq as usize]
        } else {
            0
        };
        *seq = if local > 0 {
            local
        } else {
            -i32::try_from(row + 1)?
        };
    }
    Ok(())
}

/// `verify_regional_network`（`MOD_Grid_RiverLakeNetwork.F90:1322-1353`）：区域网络的
/// `seq_x/seq_y` 必须就是源网络在 `seq_src_index` 处的值，否则区域文件不属于当前的源网络。
pub fn verify(regional: &Path, source: &Path) -> Result<()> {
    let ints = |file: &Path, name: &str| -> Result<Vec<i32>> {
        netcdf::open(file)
            .with_context(|| format!("cannot open {}", file.display()))?
            .variable(name)
            .with_context(|| format!("{} has no {name}", file.display()))?
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {name} from {}", file.display()))
    };
    let src_index = ints(regional, "seq_src_index")?;
    let (x, y) = (ints(regional, "seq_x")?, ints(regional, "seq_y")?);
    let (x_source, y_source) = (ints(source, "seq_x")?, ints(source, "seq_y")?);
    let consistent = src_index.len() == x.len()
        && src_index
            .iter()
            .all(|&i| i >= 1 && (i as usize) <= x_source.len())
        && src_index
            .iter()
            .zip(x.iter().zip(&y))
            .all(|(&i, (&xr, &yr))| {
                x_source[i as usize - 1] == xr && y_source[i as usize - 1] == yr
            });
    ensure!(
        consistent,
        "the regional unit-catchment network {} does not belong to DEF_UnitCatchment_file {}; \
         run mksrfdata again with the current DEF_UnitCatchment_file",
        regional.display(),
        source.display()
    );
    Ok(())
}

/// `unitcatchment_regional_build`：由 landpatch 求 `touched`，写区域网络（`bif_closure = .true.`）。
/// `bifurcation`（`DEF_USE_BIFURCATION`）：只有分汊打开时才沿分汊通道做闭包。分汊关着时通道不参与
/// 汇流，闭包只会顺着 CaMa 的跨河系通道把与区域无关的河系一串串拉进来（区域 96–104°E 的算例
/// 带上了塔里木与华北，116 个河系里 39 个在区域里一个单元流域都没有）；一端在外的通道直接丢掉。
pub fn build(
    unitcatchment_file: &Path,
    landdata: &Path,
    year: i32,
    bifurcation: bool,
) -> Result<(usize, usize)> {
    let (nlon, nlat) = grid_size(unitcatchment_file)?;
    let touched = touched_cells(landdata, year, nlon, nlat)?;
    subset_write(
        unitcatchment_file,
        &regional_file(landdata),
        nlon,
        nlat,
        &touched,
        bifurcation,
    )
}

#[cfg(test)]
#[path = "unitcatchment_regional_tests.rs"]
mod unitcatchment_regional_tests;
