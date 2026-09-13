//! Clip a runnable spatial landdata tree to a case-domain rectangle.
//!
//! This is the maintained Rust counterpart of `MOD_RegionClip.F90`.  A
//! clipped tree keeps the original pixel/block lattice and selects whole mesh
//! elements, just as the upstream routine does; it does not resample data.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use netcdf::types::{FloatType, IntType, NcVariableType};

/// Geographic bounds used by the existing-surface-data clipping path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialBounds {
    pub south: f64,
    pub north: f64,
    pub west: f64,
    pub east: f64,
}

impl SpatialBounds {
    fn validate(self) -> Result<()> {
        ensure!(
            self.south.is_finite()
                && self.north.is_finite()
                && self.west.is_finite()
                && self.east.is_finite(),
            "region bounds must be finite"
        );
        ensure!(
            (-90.0..=90.0).contains(&self.south)
                && (-90.0..=90.0).contains(&self.north)
                && self.south < self.north,
            "region latitude bounds must satisfy -90 <= south < north <= 90"
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct BlockSelection {
    elements: Vec<bool>,
    ids: HashSet<i64>,
}

/// Copy the selected elements and every dependent patch/PFT/urban vector from
/// a larger spatial surface-data tree.
///
/// The destination must not exist, so clipping cannot overwrite a runnable
/// landdata tree.
pub fn clip_existing_surface(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    bounds: SpatialBounds,
) -> Result<()> {
    bounds.validate()?;
    let source = source.as_ref();
    let destination = destination.as_ref();
    ensure!(
        source.is_dir(),
        "existing landdata is not a directory: {}",
        source.display()
    );
    ensure!(
        !destination.exists(),
        "region-clip destination already exists: {}",
        destination.display()
    );
    ensure!(
        source.canonicalize()? != destination,
        "region-clip source and destination must differ"
    );

    let pixel = source.join("pixel.nc");
    let block = source.join("block.nc");
    ensure!(
        pixel.is_file(),
        "existing landdata is missing {}",
        pixel.display()
    );
    ensure!(
        block.is_file(),
        "existing landdata is missing {}",
        block.display()
    );
    let pixel_axes = read_pixel_axes(&pixel)?;
    let block_layout = read_block_layout(&block)?;
    let files = netcdf_files(source)?;
    let selections = mesh_selections(source, &files, &pixel_axes, bounds)?;
    ensure!(
        !selections.is_empty()
            && selections
                .values()
                .any(|selection| selection.elements.iter().any(|&v| v)),
        "region bounds select no existing land elements"
    );

    std::fs::create_dir_all(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    std::fs::copy(&block, destination.join("block.nc"))?;
    std::fs::copy(&pixel, destination.join("pixel.nc"))?;

    for source_file in files {
        let relative = source_file.strip_prefix(source)?;
        if relative == Path::new("block.nc") || relative == Path::new("pixel.nc") {
            continue;
        }
        let target = destination.join(relative);
        if mesh_index(relative) {
            copy_file(&source_file, &target)?;
            continue;
        }
        let Some(key) = block_key(relative) else {
            continue;
        };
        let Some(selection) = selections.get(&key) else {
            continue;
        };
        if !selection.elements.iter().any(|&selected| selected) {
            continue;
        }
        if mesh_block(relative) {
            clip_mesh_block(&source_file, &target, &selection.elements)?;
            continue;
        }
        let masks = masks_for_vector(&source_file, source, relative, &key, &selection.ids)?;
        if masks.is_empty() {
            continue;
        }
        clip_vector_file(&source_file, &target, &masks)?;
    }

    for source_file in netcdf_files(source)? {
        let relative = source_file.strip_prefix(source)?;
        if mesh_index(relative) {
            let target = destination.join(relative);
            update_mesh_counts(&target, &block_layout, &selections)?;
        }
    }
    Ok(())
}

fn netcdf_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn visit(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in
            std::fs::read_dir(path).with_context(|| format!("cannot list {}", path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(&path, files)?;
            } else if path.extension().is_some_and(|extension| extension == "nc") {
                files.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn copy_file(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().context("NetCDF target has no parent")?;
    std::fs::create_dir_all(parent)?;
    std::fs::copy(source, target)
        .with_context(|| format!("cannot copy {} to {}", source.display(), target.display()))?;
    Ok(())
}

fn mesh_selections(
    root: &Path,
    files: &[PathBuf],
    pixel: &PixelAxes,
    bounds: SpatialBounds,
) -> Result<HashMap<String, BlockSelection>> {
    let mut selections = HashMap::new();
    for file in files {
        let relative = file.strip_prefix(root)?;
        if !mesh_block(relative) {
            continue;
        }
        let key = block_key(relative).expect("mesh block has a block key");
        let selection = mesh_selection(file, pixel, bounds)?;
        if let Some(previous) = selections.insert(key.clone(), selection.clone()) {
            ensure!(
                previous.elements == selection.elements,
                "mesh topology differs between land-cover years for block {key}"
            );
        }
    }
    Ok(selections)
}

fn mesh_selection(file: &Path, pixel: &PixelAxes, bounds: SpatialBounds) -> Result<BlockSelection> {
    let input = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let ids = values_i64(&input, "elmindex")?;
    let counts = values_i32(&input, "elmnpxl")?;
    let coordinates = values_i32(&input, "elmpixels")?;
    ensure!(
        ids.len() == counts.len(),
        "{} has inconsistent mesh vectors",
        file.display()
    );
    let mut offset = 0_usize;
    let mut selected = Vec::with_capacity(ids.len());
    for &count in &counts {
        let count = usize::try_from(count).context("mesh pixel count is negative")?;
        let width = count
            .checked_mul(2)
            .context("mesh coordinate count overflows")?;
        ensure!(
            offset + width <= coordinates.len(),
            "{} has truncated mesh coordinates",
            file.display()
        );
        let contains = coordinates[offset..offset + width]
            .chunks_exact(2)
            .any(|coordinate| {
                let longitude = usize::try_from(coordinate[0])
                    .ok()
                    .and_then(|index| index.checked_sub(1));
                let latitude = usize::try_from(coordinate[1])
                    .ok()
                    .and_then(|index| index.checked_sub(1));
                match (longitude, latitude) {
                    (Some(longitude), Some(latitude))
                        if longitude < pixel.lon_w.len() && latitude < pixel.lat_s.len() =>
                    {
                        latitude_overlap(pixel.lat_s[latitude], pixel.lat_n[latitude], bounds)
                            && longitude_overlap(
                                pixel.lon_w[longitude],
                                pixel.lon_e[longitude],
                                bounds,
                            )
                    }
                    _ => false,
                }
            });
        selected.push(contains);
        offset += width;
    }
    ensure!(
        offset == coordinates.len(),
        "{} has trailing mesh coordinates",
        file.display()
    );
    let ids = ids
        .iter()
        .zip(&selected)
        .filter_map(|(&id, &keep)| keep.then_some(id))
        .collect();
    Ok(BlockSelection {
        elements: selected,
        ids,
    })
}

fn read_pixel_axes(file: &Path) -> Result<PixelAxes> {
    let input = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let lon_w = values_f64(&input, "lon_w")?;
    let lon_e = values_f64(&input, "lon_e")?;
    let lat_s = values_f64(&input, "lat_s")?;
    let lat_n = values_f64(&input, "lat_n")?;
    ensure!(
        lon_w.len() == lon_e.len() && lat_s.len() == lat_n.len(),
        "pixel coordinate edges have inconsistent dimensions"
    );
    Ok(PixelAxes {
        lon_w,
        lon_e,
        lat_s,
        lat_n,
    })
}

struct PixelAxes {
    lon_w: Vec<f64>,
    lon_e: Vec<f64>,
    lat_s: Vec<f64>,
    lat_n: Vec<f64>,
}

struct BlockLayout {
    lon_w: Vec<f64>,
    lat_s: Vec<f64>,
}

fn read_block_layout(file: &Path) -> Result<BlockLayout> {
    let input = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    Ok(BlockLayout {
        lon_w: values_f64(&input, "lon_w")?,
        lat_s: values_f64(&input, "lat_s")?,
    })
}

fn latitude_overlap(south: f64, north: f64, bounds: SpatialBounds) -> bool {
    south < bounds.north && north > bounds.south
}

fn longitude_overlap(west: f64, east: f64, bounds: SpatialBounds) -> bool {
    if same_longitude(bounds.west, bounds.east) || same_longitude(west, east) {
        return true;
    }
    longitude_intervals(west, east)
        .into_iter()
        .any(|(west, east)| {
            longitude_intervals(bounds.west, bounds.east)
                .into_iter()
                .any(|(left, right)| west < right && east > left)
        })
}

fn same_longitude(left: f64, right: f64) -> bool {
    (normalize_longitude(left) - normalize_longitude(right)).abs() < 1e-9
}

fn normalize_longitude(value: f64) -> f64 {
    let value = value.rem_euclid(360.0);
    if value >= 180.0 { value - 360.0 } else { value }
}

fn longitude_intervals(west: f64, east: f64) -> Vec<(f64, f64)> {
    let west = normalize_longitude(west);
    let east = normalize_longitude(east);
    if west < east {
        vec![(west, east)]
    } else {
        vec![(west, 180.0), (-180.0, east)]
    }
}

fn mesh_index(relative: &Path) -> bool {
    relative.file_name().is_some_and(|name| name == "mesh.nc")
        && relative
            .parent()
            .and_then(Path::parent)
            .is_some_and(|parent| parent == Path::new("mesh"))
}

fn mesh_block(relative: &Path) -> bool {
    relative
        .components()
        .next()
        .is_some_and(|component| component.as_os_str() == "mesh")
        && block_key(relative).is_some()
}

fn block_key(relative: &Path) -> Option<String> {
    let stem = relative.file_stem()?.to_str()?;
    let start = stem.rfind("_w").or_else(|| stem.rfind("_e"))?;
    let key = stem.get(start + 1..)?;
    key.contains('_').then(|| key.to_owned())
}

fn masks_for_vector(
    file: &Path,
    root: &Path,
    relative: &Path,
    block: &str,
    selected_elements: &HashSet<i64>,
) -> Result<BTreeMap<String, Vec<bool>>> {
    let input = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let mut dimensions = HashSet::new();
    for variable in input.variables() {
        for dimension in variable.dimensions() {
            dimensions.insert(dimension.name());
        }
    }
    let mut masks = BTreeMap::new();
    for (dimension, pixelset) in [
        ("landelm", "landelm"),
        ("landpatch", "landpatch"),
        ("landhru", "landhru"),
        ("landpft", "landpft"),
        ("landurban", "landurban"),
        ("patch", "landpatch"),
        ("pft", "landpft"),
        ("urban", "landurban"),
    ] {
        if dimensions.contains(dimension) {
            masks.insert(
                dimension.to_owned(),
                pixelset_mask(root, relative, pixelset, block, selected_elements)?,
            );
        }
    }
    Ok(masks)
}

fn pixelset_mask(
    root: &Path,
    relative: &Path,
    pixelset: &str,
    block: &str,
    selected: &HashSet<i64>,
) -> Result<Vec<bool>> {
    let year = relative
        .components()
        .nth(1)
        .and_then(|component| component.as_os_str().to_str())
        .context("blocked landdata file is missing its land-cover year")?;
    let path = root
        .join(pixelset)
        .join(year)
        .join(format!("{pixelset}_{block}.nc"));
    ensure!(
        path.is_file(),
        "{} needs {} to select its {pixelset} records",
        relative.display(),
        path.display()
    );
    let input = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    Ok(values_i64(&input, "eindex")?
        .into_iter()
        .map(|element| selected.contains(&element))
        .collect())
}

fn clip_mesh_block(source: &Path, target: &Path, element_mask: &[bool]) -> Result<()> {
    let input =
        netcdf::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    let ids = values_i64(&input, "elmindex")?;
    let counts = values_i32(&input, "elmnpxl")?;
    let coordinates = values_i32(&input, "elmpixels")?;
    ensure!(
        ids.len() == counts.len() && ids.len() == element_mask.len(),
        "{} has inconsistent mesh vectors",
        source.display()
    );
    let mut output_ids = Vec::new();
    let mut output_counts = Vec::new();
    let mut output_coordinates = Vec::new();
    let mut offset = 0_usize;
    for ((&id, &count), &keep) in ids.iter().zip(&counts).zip(element_mask) {
        let count = usize::try_from(count).context("mesh pixel count is negative")?;
        let width = count
            .checked_mul(2)
            .context("mesh coordinate count overflows")?;
        ensure!(
            offset + width <= coordinates.len(),
            "{} has truncated mesh coordinates",
            source.display()
        );
        if keep {
            output_ids.push(id);
            output_counts.push(i32::try_from(count)?);
            output_coordinates.extend_from_slice(&coordinates[offset..offset + width]);
        }
        offset += width;
    }
    ensure!(
        offset == coordinates.len(),
        "{} has trailing mesh coordinates",
        source.display()
    );
    let parent = target.parent().context("NetCDF target has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut output =
        netcdf::create(target).with_context(|| format!("cannot create {}", target.display()))?;
    output.add_dimension("element", output_ids.len())?;
    output.add_dimension("ncoor", 2)?;
    output.add_dimension("pixel", output_coordinates.len() / 2)?;
    output
        .add_variable::<i64>("elmindex", &["element"])?
        .put_values(&output_ids, ..)?;
    output
        .add_variable::<i32>("elmnpxl", &["element"])?
        .put_values(&output_counts, ..)?;
    output
        .add_variable::<i32>("elmpixels", &["pixel", "ncoor"])?
        .put_values(&output_coordinates, (.., ..))?;
    output.close()?;
    Ok(())
}

fn clip_vector_file(
    source: &Path,
    target: &Path,
    masks: &BTreeMap<String, Vec<bool>>,
) -> Result<()> {
    let input =
        netcdf::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    let parent = target.parent().context("NetCDF target has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut output =
        netcdf::create(target).with_context(|| format!("cannot create {}", target.display()))?;
    for dimension in input.dimensions() {
        let name = dimension.name();
        let length = masks
            .get(&name)
            .map(|mask| mask.iter().filter(|&&selected| selected).count())
            .unwrap_or_else(|| dimension.len());
        output.add_dimension(&name, length)?;
    }
    for variable in input.variables() {
        let name = variable.name();
        let dimensions = variable
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect::<Vec<_>>();
        let shape = variable
            .dimensions()
            .iter()
            .map(|dimension| dimension.len())
            .collect::<Vec<_>>();
        let mask_axis = dimensions
            .iter()
            .enumerate()
            .filter_map(|(axis, dimension)| {
                masks.get(dimension).map(|mask| (axis, mask.as_slice()))
            })
            .collect::<Vec<_>>();
        ensure!(
            mask_axis.len() <= 1,
            "{} has a vector with more than one selected pixelset dimension",
            source.display()
        );
        write_variable(
            &variable,
            &mut output,
            &name,
            &dimensions,
            &shape,
            mask_axis.first().copied(),
        )?;
    }
    output.close()?;
    Ok(())
}

fn write_variable(
    variable: &netcdf::Variable<'_>,
    output: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[String],
    shape: &[usize],
    mask_axis: Option<(usize, &[bool])>,
) -> Result<()> {
    let dimensions = dimensions.iter().map(String::as_str).collect::<Vec<_>>();
    macro_rules! write {
        ($type:ty) => {{
            let values = variable.get_values::<$type, _>(..)?;
            let values = clip_values(&values, shape, mask_axis)?;
            output
                .add_variable::<$type>(name, &dimensions)?
                .put_values(&values, ..)?;
            Ok(())
        }};
    }
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => write!(f64),
        NcVariableType::Float(FloatType::F32) => write!(f32),
        NcVariableType::Int(IntType::I8) => write!(i8),
        NcVariableType::Int(IntType::I16) => write!(i16),
        NcVariableType::Int(IntType::I32) => write!(i32),
        NcVariableType::Int(IntType::I64) => write!(i64),
        NcVariableType::Int(IntType::U8) => write!(u8),
        NcVariableType::Int(IntType::U16) => write!(u16),
        NcVariableType::Int(IntType::U32) => write!(u32),
        NcVariableType::Int(IntType::U64) => write!(u64),
        kind => bail!("{} has unsupported NetCDF type {kind:?}", variable.name()),
    }
}

fn clip_values<T: Copy>(
    values: &[T],
    shape: &[usize],
    mask_axis: Option<(usize, &[bool])>,
) -> Result<Vec<T>> {
    let Some((axis, mask)) = mask_axis else {
        return Ok(values.to_vec());
    };
    ensure!(
        mask.len() == shape[axis],
        "vector mask length does not match its NetCDF dimension"
    );
    let before = shape[..axis].iter().product::<usize>();
    let after = shape[axis + 1..].iter().product::<usize>();
    let selected = mask.iter().filter(|&&keep| keep).count();
    let mut result = Vec::with_capacity(before * selected * after);
    for outer in 0..before {
        let base = outer * mask.len() * after;
        for (index, &keep) in mask.iter().enumerate() {
            if keep {
                result.extend_from_slice(&values[base + index * after..base + (index + 1) * after]);
            }
        }
    }
    Ok(result)
}

fn update_mesh_counts(
    file: &Path,
    blocks: &BlockLayout,
    selections: &HashMap<String, BlockSelection>,
) -> Result<()> {
    let mut output =
        netcdf::append(file).with_context(|| format!("cannot append {}", file.display()))?;
    let mut variable = output
        .variable_mut("nelm_blk")
        .with_context(|| format!("{} is missing nelm_blk", file.display()))?;
    let dimensions = variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    ensure!(dimensions.len() == 2, "nelm_blk must have two dimensions");
    let mut counts = vec![0_i32; variable.len()];
    for (y, &south) in blocks.lat_s.iter().enumerate() {
        for (x, &west) in blocks.lon_w.iter().enumerate() {
            let key = block_name(west, south);
            let count = selections
                .get(&key)
                .map(|selection| {
                    selection
                        .elements
                        .iter()
                        .filter(|&&selected| selected)
                        .count()
                })
                .unwrap_or(0);
            let index = match dimensions.as_slice() {
                [first, second] if first == "yblk" && second == "xblk" => {
                    y * blocks.lon_w.len() + x
                }
                [first, second] if first == "xblk" && second == "yblk" => {
                    x * blocks.lat_s.len() + y
                }
                _ => bail!("nelm_blk dimensions must be (yblk,xblk) or (xblk,yblk)"),
            };
            counts[index] = i32::try_from(count)?;
        }
    }
    variable.put_values(&counts, ..)?;
    output.close()?;
    Ok(())
}

fn block_name(west: f64, south: f64) -> String {
    let longitude = if west < 0.0 {
        format!("w{:03}", -west.floor() as i32)
    } else {
        format!("e{:03}", west.floor() as i32)
    };
    let latitude = if south < 0.0 {
        format!("s{:02}", -south.floor() as i32)
    } else {
        format!("n{:02}", south.floor() as i32)
    };
    format!("{longitude}_{latitude}")
}

fn values_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => Ok(variable.get_values::<f64, _>(..)?),
        NcVariableType::Float(FloatType::F32) => Ok(variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect()),
        kind => bail!("{name} must be floating point, got {kind:?}"),
    }
}

fn values_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Int(IntType::I32) => Ok(variable.get_values::<i32, _>(..)?),
        kind => bail!("{name} must be int32, got {kind:?}"),
    }
}

fn values_i64(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Int(IntType::I64) => Ok(variable.get_values::<i64, _>(..)?),
        NcVariableType::Int(IntType::I32) => Ok(variable
            .get_values::<i32, _>(..)?
            .into_iter()
            .map(i64::from)
            .collect()),
        kind => bail!("{name} must be int64, got {kind:?}"),
    }
}

#[cfg(test)]
#[path = "region_tests.rs"]
mod tests;
