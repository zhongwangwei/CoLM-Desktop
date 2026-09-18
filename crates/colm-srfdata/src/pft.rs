//! PFT-fraction aggregation from `Aggregation_PercentagesPFT.F90`.

use anyhow::{ensure, Context, Result};

use crate::{
    surface::FlatPatches,
    topology::{FlatLandElements, FlatLandPatches, FlatMesh},
};

/// CoLM IGBP's crop land-cover type (`CROPLAND`).
pub const IGBP_CROPLAND: i32 = 12;

/// Land-patch partition mode for IGBP PFT/PC preprocessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PftPatchMode {
    Merged,
    Separate,
    FastPc,
}

/// True for IGBP classes with soil-ground data (`patchtypes(class) == 0`).
pub(crate) fn is_igbp_soil_ground(kind: i32) -> Result<bool> {
    const IGBP_PATCH_TYPES: [i32; 18] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
    let index =
        usize::try_from(kind).with_context(|| format!("IGBP land type {kind} is negative"))?;
    ensure!(
        index < IGBP_PATCH_TYPES.len(),
        "IGBP land type {kind} is outside 0..=17"
    );
    Ok(index > 0 && IGBP_PATCH_TYPES[index] == 0)
}

/// A crop-refined `landpatch` pixelset and its shared-area metadata.
///
/// This is the sequential `MOD_LandCrop::landcrop_build` partition: natural
/// and crop shares are first split with `PCT_CROP`, then crop shares are split
/// again by `PCT_CFT`.  Children intentionally retain their parent's raw-cell
/// range; `pctshared` carries their fractional ownership.
#[derive(Debug, Clone, PartialEq)]
pub struct CropLandPatchTopology {
    pub land_patches: FlatLandPatches,
    pub layout: FlatPatches,
    pub pctshared: Vec<f64>,
    /// One-based CFT class for crop patches, otherwise `None`.
    pub crop_class: Vec<Option<usize>>,
}

impl CropLandPatchTopology {
    /// Append virtual WMO patches after crop partitioning, copying the source
    /// patch's shared-area metadata for consumers that index every patch.
    pub fn with_wmo_patches(
        self,
        mesh: &FlatMesh,
        elements: &mut FlatLandElements,
    ) -> Result<Self> {
        ensure!(
            self.pctshared.len() == self.land_patches.len()
                && self.crop_class.len() == self.land_patches.len(),
            "crop metadata must match the land-patch topology"
        );
        let land_patches = self.land_patches.with_wmo_patches(elements)?;
        let sources = land_patches.wmo_sources()?;
        let mut pctshared = Vec::with_capacity(land_patches.len());
        let mut crop_class = Vec::with_capacity(land_patches.len());
        let mut physical = 0;
        for source in &sources {
            if let Some(source) = source {
                pctshared.push(pctshared[*source]);
                crop_class.push(crop_class[*source]);
            } else {
                pctshared.push(self.pctshared[physical]);
                crop_class.push(self.crop_class[physical]);
                physical += 1;
            }
        }
        ensure!(
            physical == self.pctshared.len(),
            "WMO insertion did not preserve every physical crop patch"
        );
        let layout = land_patches.aggregation_layout(mesh, sources)?;
        Ok(Self {
            land_patches,
            layout,
            pctshared,
            crop_class,
        })
    }
}

/// The branch chosen by `Aggregation_PercentagesPFT` for one land patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PftPatchKind {
    Natural,
    Crop,
    Other,
}

/// The `landpft` pixelset and its mapping back to `landpatch`.
///
/// `patch_offsets` partitions `pft_classes` once for every land patch.  The
/// structural vectors use the same one-based pixel ranges as CoLM's saved
/// `landpft` pixelset.
#[derive(Debug, Clone, PartialEq)]
pub struct PftTopology {
    pub land_pfts: FlatLandPatches,
    pub patch_offsets: Vec<usize>,
    pub pft_classes: Vec<usize>,
    pub patch_kind: Vec<PftPatchKind>,
    /// MOD_LandPFT's raw-weighted fractions, distinct from pct_pfts' mean of
    /// per-cell fractions. CROP callers replace CFT entries with patch shares.
    pub pctshared: Vec<f64>,
}

/// Split IGBP natural patches into natural/CROP/CFT shared patches.
///
/// `crop_percent` is one `PCT_CROP` value per raw cell and `cft_percent` is
/// class-major (`cft * raw_cells + cell`) `PCT_CFT`.  As in
/// `pixelsetshared_build`, only positive area-weighted shares become child
/// patches and their shares are normalized within their parent patch.
pub fn build_crop_land_patches(
    land_patches: &FlatLandPatches,
    patches: &FlatPatches,
    crop_percent: &[f64],
    cft_class_count: usize,
    cft_percent: &[f64],
    land_area: &[f64],
) -> Result<CropLandPatchTopology> {
    ensure!(
        land_patches.len() == patches.len(),
        "crop topology needs matching structural and aggregation patch layouts"
    );
    ensure!(
        crop_percent.len() == land_area.len()
            && cft_class_count > 0
            && cft_percent.len() == cft_class_count * land_area.len(),
        "crop and CFT percentages must match the raw cell layout"
    );
    ensure!(
        land_area
            .iter()
            .all(|area| area.is_finite() && *area >= 0.0)
            && crop_percent.iter().all(|value| value.is_finite())
            && cft_percent.iter().all(|value| value.is_finite()),
        "crop topology inputs must be finite and land areas non-negative"
    );

    #[derive(Clone, Copy)]
    struct Child {
        source: usize,
        set_type: i32,
        pctshared: f64,
        crop_class: Option<usize>,
    }

    let mut first = Vec::new();
    for patch in 0..land_patches.len() {
        ensure!(
            patches.wmo_source_for(patch).is_none(),
            "crop sharing needs the upstream land2mWMO topology"
        );
        if land_patches.set_type[patch] != 1 {
            first.push(Child {
                source: patch,
                set_type: land_patches.set_type[patch],
                pctshared: 1.0,
                crop_class: None,
            });
            continue;
        }
        let mut shares = [0.0; 2];
        for &cell in patches.raw_cells(patch) {
            let area = area(land_area, cell, patch)?;
            let crop = crop_percent[cell] / 100.0;
            shares[0] += (1.0 - crop) * area;
            shares[1] += crop * area;
        }
        let total = shares.iter().sum::<f64>();
        ensure!(
            total.is_finite() && total > 0.0,
            "crop patch {patch} has no positive total area share"
        );
        for (class, share) in shares.into_iter().enumerate() {
            if share > 0.0 {
                first.push(Child {
                    source: patch,
                    set_type: if class == 0 { 1 } else { IGBP_CROPLAND },
                    pctshared: share / total,
                    crop_class: None,
                });
            }
        }
    }

    let mut children = Vec::new();
    for parent in first {
        if parent.set_type != IGBP_CROPLAND {
            children.push(parent);
            continue;
        }
        let mut shares = vec![0.0; cft_class_count];
        for &cell in patches.raw_cells(parent.source) {
            let area = area(land_area, cell, parent.source)?;
            for class in 0..cft_class_count {
                shares[class] += cft_percent[class * land_area.len() + cell] * area;
            }
        }
        let total = shares.iter().sum::<f64>();
        ensure!(
            total.is_finite() && total > 0.0,
            "crop patch {} has no positive CFT share",
            parent.source
        );
        for (class, share) in shares.into_iter().enumerate() {
            if share > 0.0 {
                children.push(Child {
                    source: parent.source,
                    set_type: IGBP_CROPLAND,
                    pctshared: parent.pctshared * share / total,
                    crop_class: Some(class + 1),
                });
            }
        }
    }

    let mut element_ids = Vec::with_capacity(children.len());
    let mut pixel_start = Vec::with_capacity(children.len());
    let mut pixel_end = Vec::with_capacity(children.len());
    let mut set_type = Vec::with_capacity(children.len());
    let mut element_index = Vec::with_capacity(children.len());
    let mut offsets = Vec::with_capacity(children.len() + 1);
    let mut cells = Vec::new();
    let mut pctshared = Vec::with_capacity(children.len());
    let mut crop_class = Vec::with_capacity(children.len());
    offsets.push(0);
    for child in children {
        let source = child.source;
        element_ids.push(land_patches.element_ids[source]);
        pixel_start.push(land_patches.pixel_start[source]);
        pixel_end.push(land_patches.pixel_end[source]);
        set_type.push(child.set_type);
        element_index.push(land_patches.element_index[source]);
        cells.extend_from_slice(patches.raw_cells(source));
        offsets.push(cells.len());
        pctshared.push(child.pctshared);
        crop_class.push(child.crop_class);
    }
    let layout = FlatPatches::new(set_type.clone(), offsets, cells, vec![None; set_type.len()])?;
    Ok(CropLandPatchTopology {
        land_patches: FlatLandPatches {
            element_ids,
            pixel_start,
            pixel_end,
            set_type,
            element_index,
        },
        layout,
        pctshared,
        crop_class,
    })
}

/// Build the CROP-aware `landpft` partition from a shared crop topology.
///
/// Natural PFT classes remain zero-based; a one-based CFT class is stored as
/// `natural_pft_class_count + cft - 1`, matching `MOD_LandPFT.F90`.
#[allow(clippy::too_many_arguments)]
pub fn build_crop_pft_topology(
    land_patches: &FlatLandPatches,
    patches: &FlatPatches,
    crop_class: &[Option<usize>],
    raw_class_count: usize,
    natural_pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
) -> Result<PftTopology> {
    build_pft_topology_inner(
        land_patches,
        patches,
        raw_class_count,
        natural_pft_class_count,
        raw_percent,
        land_area,
        Some(crop_class),
    )
}

/// Build CoLM's non-CROP `landpft` partition from class-major PFT fractions.
///
/// PFT land patches are created for every natural IGBP soil-ground class
/// (`patchtypes(settyp) == 0`).  The weighted positive-class test and bare-soil fallback
/// are the `MOD_LandPFT::landpft_build` rules. `pft_class_count` permits
/// CoLM's 16-class MODIS source to retain only its 15 natural PFT types; the
/// `pctshared` retains these raw-weighted fractions; the distinct surface
/// `pct_pfts` percentages come from [`aggregate_pft_fractions`].
pub fn build_pft_topology(
    land_patches: &FlatLandPatches,
    patches: &FlatPatches,
    raw_class_count: usize,
    pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
) -> Result<PftTopology> {
    build_pft_topology_inner(
        land_patches,
        patches,
        raw_class_count,
        pft_class_count,
        raw_percent,
        land_area,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_pft_topology_inner(
    land_patches: &FlatLandPatches,
    patches: &FlatPatches,
    raw_class_count: usize,
    pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
    crop_class: Option<&[Option<usize>]>,
) -> Result<PftTopology> {
    ensure!(
        raw_class_count > 0
            && pft_class_count > 0
            && pft_class_count <= raw_class_count
            && raw_percent.len() == raw_class_count * land_area.len(),
        "raw PFT percentages must be raw_class_count x raw cell count"
    );
    if let Some(crop_class) = crop_class {
        ensure!(
            crop_class.len() == land_patches.len(),
            "crop classes must have one entry per land patch"
        );
    }
    ensure!(
        land_patches.len() == patches.len(),
        "landpft needs matching structural and aggregation patch layouts"
    );
    ensure!(
        land_area
            .iter()
            .all(|area| area.is_finite() && *area >= 0.0)
            && raw_percent.iter().all(|value| value.is_finite()),
        "landpft inputs must be finite and land areas non-negative"
    );

    let mut element_ids = Vec::new();
    let mut pixel_start = Vec::new();
    let mut pixel_end = Vec::new();
    let mut set_type = Vec::new();
    let mut element_index = Vec::new();
    let mut patch_offsets = Vec::with_capacity(land_patches.len() + 1);
    let mut pft_classes = Vec::new();
    let mut patch_kind = Vec::with_capacity(land_patches.len());
    let mut pctshared = Vec::new();
    patch_offsets.push(0);

    for patch in 0..land_patches.len() {
        let (kind, classes) = if let Some(source) = patches.wmo_source_for(patch) {
            (
                PftPatchKind::Natural,
                vec![(
                    wmo_pft_class(
                        patches,
                        source,
                        raw_class_count,
                        pft_class_count,
                        raw_percent,
                        land_area,
                    )?,
                    1.0,
                )],
            )
        } else {
            let land_type = land_patches.set_type[patch];
            let kind = if crop_class.is_some() && land_type == IGBP_CROPLAND {
                PftPatchKind::Crop
            } else if is_igbp_soil_ground(land_type)? {
                PftPatchKind::Natural
            } else {
                PftPatchKind::Other
            };
            let classes = match kind {
                PftPatchKind::Natural => {
                    let fractions = normalized_patch_pft_fractions(
                        patches.raw_cells(patch),
                        patch,
                        raw_class_count,
                        pft_class_count,
                        raw_percent,
                        land_area,
                    )?;
                    let classes = fractions
                        .iter()
                        .enumerate()
                        .filter_map(|(class, value)| (*value > 0.0).then_some((class, *value)))
                        .collect::<Vec<_>>();
                    if classes.is_empty() {
                        vec![(0, 1.0)]
                    } else {
                        classes
                    }
                }
                PftPatchKind::Crop => {
                    let crop = crop_class.expect("CROP kind requires crop classes")[patch]
                        .with_context(|| format!("crop patch {patch} has no CFT class"))?;
                    vec![(
                        pft_class_count
                            .checked_add(crop)
                            .and_then(|value| value.checked_sub(1))
                            .context("crop PFT class overflows usize")?,
                        1.0,
                    )]
                }
                PftPatchKind::Other => Vec::new(),
            };
            (kind, classes)
        };
        patch_kind.push(kind);
        for (class, fraction) in classes {
            pctshared.push(fraction);
            element_ids.push(land_patches.element_ids[patch]);
            pixel_start.push(land_patches.pixel_start[patch]);
            pixel_end.push(land_patches.pixel_end[patch]);
            set_type.push(i32::try_from(class)?);
            element_index.push(land_patches.element_index[patch]);
            pft_classes.push(class);
        }
        patch_offsets.push(pft_classes.len());
    }
    Ok(PftTopology {
        land_pfts: FlatLandPatches {
            element_ids,
            pixel_start,
            pixel_end,
            set_type,
            element_index,
        },
        patch_offsets,
        pft_classes,
        patch_kind,
        pctshared,
    })
}

fn wmo_pft_class(
    patches: &FlatPatches,
    source: usize,
    raw_class_count: usize,
    pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
) -> Result<usize> {
    let fractions = normalized_patch_pft_fractions(
        patches.raw_cells(source),
        source,
        raw_class_count,
        pft_class_count,
        raw_percent,
        land_area,
    )?;
    let mut best = None;
    for class in 12..=14 {
        let Some(&fraction) = fractions.get(class) else {
            continue;
        };
        if fraction > 0.0
            && best
                .map(|(_, best_fraction)| fraction > best_fraction)
                .unwrap_or(true)
        {
            best = Some((class, fraction));
        }
    }
    Ok(best.map(|(class, _)| class).unwrap_or(0))
}

fn normalized_patch_pft_fractions(
    cells: &[usize],
    patch: usize,
    raw_class_count: usize,
    pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
) -> Result<Vec<f64>> {
    ensure!(
        pft_class_count <= raw_class_count,
        "PFT output class count exceeds raw class count"
    );
    let mut weighted = vec![0.0; pft_class_count];
    let mut total = 0.0;
    for &cell in cells {
        let area = area(land_area, cell, patch)?;
        let mut sum = 0.0;
        for class in 0..pft_class_count {
            let value = raw_percent[class * land_area.len() + cell];
            sum += value;
            weighted[class] = value.mul_add(area, weighted[class]);
        }
        total = area.mul_add(sum, total);
    }
    if total > 0.0 {
        for value in &mut weighted {
            *value /= total;
        }
    } else {
        weighted.fill(0.0);
        weighted[0] = 1.0;
    }
    Ok(weighted)
}

/// Compose the `landpft%pctshared` vector for a CROP build.
///
/// Natural PFT children retain their topology's raw-weighted fraction; a
/// crop PFT directly inherits its CFT-resolved `landpatch%pctshared` value.
pub fn crop_pft_pctshared(topology: &PftTopology, patch_pctshared: &[f64]) -> Result<Vec<f64>> {
    ensure!(
        topology.pctshared.len() == topology.pft_classes.len()
            && patch_pctshared.len() == topology.patch_kind.len(),
        "CROP PFT shared fractions need matching PFT and land-patch vectors"
    );
    ensure!(
        topology
            .pctshared
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && patch_pctshared
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "CROP PFT shared fractions must be finite and non-negative"
    );
    let mut output = topology.pctshared.clone();
    for (patch, kind) in topology.patch_kind.iter().enumerate() {
        if *kind == PftPatchKind::Crop {
            output[topology.patch_offsets[patch]..topology.patch_offsets[patch + 1]]
                .fill(patch_pctshared[patch]);
        }
    }
    Ok(output)
}

/// PFT topology and raw PFT-percentage fields.
///
/// `raw_percent` is `raw_class * raw_cells + raw_cell`; output is in the CSR
/// order defined by `pft_offsets` and `pft_classes`.
#[derive(Debug, Clone, Copy)]
pub struct PftFractionInput<'a> {
    pub pft_offsets: &'a [usize],
    pub pft_classes: &'a [usize],
    pub patch_kind: &'a [PftPatchKind],
    pub raw_class_count: usize,
    pub raw_percent: &'a [f64],
    pub land_area: &'a [f64],
    /// CROP builds clear this raw PFT class before aggregation.
    pub crop_excluded_class: Option<usize>,
}

/// Raw PFT abundance and a monthly PFT LAI or SAI field.
///
/// Both raw fields are class-major: `raw_class * raw_cells + raw_cell`.
/// This is the computational portion shared by the PFT/PC LAI and SAI loops
/// in `Aggregation_LAI.F90`.
#[derive(Debug, Clone, Copy)]
pub struct PftIndexInput<'a> {
    pub pft_offsets: &'a [usize],
    pub pft_classes: &'a [usize],
    pub patch_kind: &'a [PftPatchKind],
    pub raw_class_count: usize,
    pub raw_percent: &'a [f64],
    pub raw_index: &'a [f64],
    pub land_area: &'a [f64],
}

/// Patch and PFT vectors written by one monthly PFT/PC LAI or SAI step.
#[derive(Debug, Clone, PartialEq)]
pub struct PftIndexState {
    pub patch_index: Vec<f64>,
    pub pft_index: Vec<f64>,
}

/// Applies the computational part of `Aggregation_PercentagesPFT`.
pub fn aggregate_pft_fractions(
    patches: &FlatPatches,
    input: PftFractionInput<'_>,
) -> Result<Vec<f64>> {
    validate_input(patches, input)?;
    let mut output = vec![0.0; input.pft_classes.len()];
    for patch in 0..patches.len() {
        let range = input.pft_offsets[patch]..input.pft_offsets[patch + 1];
        if patches.wmo_source_for(patch).is_some() {
            let first = single_wmo_pft(range, patch)?;
            output[first] = 1.0;
            continue;
        }
        match input.patch_kind[patch] {
            PftPatchKind::Natural => {
                aggregate_natural_patch(patches.raw_cells(patch), range, input, &mut output, patch)?
            }
            PftPatchKind::Crop => output[range].fill(1.0),
            PftPatchKind::Other => {}
        }
    }
    Ok(output)
}

/// Port of the PFT-specific branch of `Aggregation_ForestHeight`.
///
/// Each PFT receives a PFT-percentage- and area-weighted canopy height.  If a
/// retained PFT has no positive source weight, CoLM falls back to the
/// area-weighted height of its land patch.
pub fn aggregate_pft_height(
    patches: &FlatPatches,
    input: PftFractionInput<'_>,
    raw_height_m: &[f64],
) -> Result<Vec<f64>> {
    validate_input(patches, input)?;
    ensure!(
        raw_height_m.len() == input.land_area.len()
            && raw_height_m.iter().all(|value| value.is_finite()),
        "PFT forest height must be one finite value per raw cell"
    );
    let mut output = vec![0.0; input.pft_classes.len()];
    for patch in 0..patches.len() {
        let range = input.pft_offsets[patch]..input.pft_offsets[patch + 1];
        if let Some(source) = patches.wmo_source_for(patch) {
            let first = single_wmo_pft(range, patch)?;
            output[first] = patch_area_weighted_height(
                patches.raw_cells(source),
                source,
                input.land_area,
                raw_height_m,
            )?;
            continue;
        }
        if range.is_empty() || input.patch_kind[patch] == PftPatchKind::Other {
            continue;
        }
        let cells = patches.raw_cells(patch);
        let patch_height = patch_area_weighted_height(cells, patch, input.land_area, raw_height_m)?;
        match input.patch_kind[patch] {
            PftPatchKind::Natural => {
                for pft in range {
                    let class = input.pft_classes[pft];
                    let mut weighted_area = 0.0;
                    let mut weighted_height = 0.0;
                    for &cell in cells {
                        let percent = percentage(input, class, cell, patch)?.max(0.0);
                        let area = area(input.land_area, cell, patch)?;
                        weighted_area = percent.mul_add(area, weighted_area);
                        weighted_height =
                            (raw_height_m[cell] * percent).mul_add(area, weighted_height);
                    }
                    output[pft] = if weighted_area > 0.0 {
                        weighted_height / weighted_area
                    } else {
                        patch_height
                    };
                }
            }
            PftPatchKind::Crop => output[range.start] = patch_height,
            PftPatchKind::Other => unreachable!(),
        }
    }
    Ok(output)
}

fn patch_area_weighted_height(
    cells: &[usize],
    patch: usize,
    land_area: &[f64],
    raw_height_m: &[f64],
) -> Result<f64> {
    let mut patch_area = 0.0;
    let mut patch_height = 0.0;
    for &cell in cells {
        let area = area(land_area, cell, patch)?;
        let height = *raw_height_m.get(cell).with_context(|| {
            format!("PFT forest-height patch {patch} references raw height cell {cell}")
        })?;
        patch_area += area;
        patch_height = height.mul_add(area, patch_height);
    }
    ensure!(
        patch_area > 0.0 && patch_area.is_finite(),
        "PFT forest-height patch {patch} has zero or non-finite land area"
    );
    Ok(patch_height / patch_area)
}

fn single_wmo_pft(range: std::ops::Range<usize>, patch: usize) -> Result<usize> {
    ensure!(
        range.len() == 1,
        "WMO patch {patch} must have exactly one virtual PFT"
    );
    Ok(range.start)
}

/// Applies the PFT/PC branch of `Aggregation_LAI` to one LAI or SAI field.
pub fn aggregate_pft_index(
    patches: &FlatPatches,
    input: PftIndexInput<'_>,
) -> Result<PftIndexState> {
    validate_index_input(patches, input)?;
    let mut output = PftIndexState {
        patch_index: vec![0.0; patches.len()],
        pft_index: vec![0.0; input.pft_classes.len()],
    };
    for patch in 0..patches.len() {
        let range = input.pft_offsets[patch]..input.pft_offsets[patch + 1];
        if range.is_empty() {
            ensure!(
                input.patch_kind[patch] == PftPatchKind::Other
                    && patches.wmo_source_for(patch).is_none(),
                "PFT/PC patch {patch} has no PFT"
            );
            continue;
        }
        let first = range
            .clone()
            .next()
            .with_context(|| format!("PFT/PC patch {patch} has no PFT"))?;
        if let Some(source) = patches.wmo_source_for(patch) {
            let first = single_wmo_pft(range, patch)?;
            let class = input.pft_classes[first];
            if (12..=14).contains(&class) {
                let source_range = input.pft_offsets[source]..input.pft_offsets[source + 1];
                if let Some(source_pft) = source_range
                    .clone()
                    .find(|&pft| input.pft_classes[pft] == class)
                {
                    output.pft_index[first] = output.pft_index[source_pft];
                }
            }
            output.patch_index[patch] = output.pft_index[first];
            continue;
        }

        let (patch_index, area_sum) =
            aggregate_patch_index(patches.raw_cells(patch), input, patch)?;
        output.patch_index[patch] = patch_index / area_sum;
        match input.patch_kind[patch] {
            PftPatchKind::Natural => {
                for pft in range {
                    let class = input.pft_classes[pft];
                    let mut weighted_area = 0.0;
                    let mut weighted_index = 0.0;
                    for &cell in patches.raw_cells(patch) {
                        let percent = percentage_index(input, class, cell, patch)?.max(0.0);
                        let area = area(input.land_area, cell, patch)?;
                        weighted_area = percent.mul_add(area, weighted_area);
                        weighted_index = (index(input, class, cell, patch)? * percent)
                            .mul_add(area, weighted_index);
                    }
                    if weighted_area > 0.0 {
                        output.pft_index[pft] = weighted_index / weighted_area;
                    }
                }
            }
            PftPatchKind::Crop => output.pft_index[first] = output.patch_index[patch],
            PftPatchKind::Other => {}
        }
    }
    Ok(output)
}

fn validate_input(patches: &FlatPatches, input: PftFractionInput<'_>) -> Result<()> {
    ensure!(
        input.pft_offsets.len() == patches.len() + 1
            && input.pft_offsets.first() == Some(&0)
            && input.pft_offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && input.pft_offsets.last() == Some(&input.pft_classes.len()),
        "PFT offsets must partition the PFT vector once per patch"
    );
    ensure!(
        input.patch_kind.len() == patches.len(),
        "PFT patch kinds must have one entry per patch"
    );
    ensure!(
        input.raw_class_count > 0
            && input.raw_percent.len() == input.raw_class_count * input.land_area.len(),
        "raw PFT percentages must be raw_class_count x raw cell count"
    );
    validate_natural_classes(
        input.pft_offsets,
        input.pft_classes,
        input.patch_kind,
        input.raw_class_count,
    )?;
    ensure!(
        input
            .crop_excluded_class
            .is_none_or(|class| class < input.raw_class_count),
        "excluded crop PFT class is outside the raw PFT class range"
    );
    Ok(())
}

fn validate_index_input(patches: &FlatPatches, input: PftIndexInput<'_>) -> Result<()> {
    ensure!(
        input.pft_offsets.len() == patches.len() + 1
            && input.pft_offsets.first() == Some(&0)
            && input.pft_offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && input.pft_offsets.last() == Some(&input.pft_classes.len()),
        "PFT offsets must partition the PFT vector once per patch"
    );
    ensure!(
        input.patch_kind.len() == patches.len(),
        "PFT patch kinds must have one entry per patch"
    );
    ensure!(
        input.raw_class_count > 0
            && input.raw_percent.len() == input.raw_class_count * input.land_area.len()
            && input.raw_index.len() == input.raw_class_count * input.land_area.len(),
        "raw PFT percentage and index fields must be raw_class_count x raw cell count"
    );
    validate_natural_classes(
        input.pft_offsets,
        input.pft_classes,
        input.patch_kind,
        input.raw_class_count,
    )?;
    ensure!(
        input
            .land_area
            .iter()
            .all(|area| area.is_finite() && *area >= 0.0),
        "PFT land area must be finite and non-negative"
    );
    ensure!(
        input
            .raw_percent
            .iter()
            .chain(input.raw_index)
            .all(|value| value.is_finite()),
        "PFT percentage and LAI/SAI inputs must be finite"
    );
    Ok(())
}

fn validate_natural_classes(
    offsets: &[usize],
    classes: &[usize],
    patch_kind: &[PftPatchKind],
    raw_class_count: usize,
) -> Result<()> {
    for (patch, kind) in patch_kind.iter().enumerate() {
        if *kind != PftPatchKind::Crop {
            ensure!(
                classes[offsets[patch]..offsets[patch + 1]]
                    .iter()
                    .all(|&class| class < raw_class_count),
                "non-crop PFT class for patch {patch} is outside the raw PFT class range"
            );
        }
    }
    Ok(())
}

fn aggregate_patch_index(
    cells: &[usize],
    input: PftIndexInput<'_>,
    patch: usize,
) -> Result<(f64, f64)> {
    let mut area_sum = 0.0;
    let mut index_sum = 0.0;
    for &cell in cells {
        let area = area(input.land_area, cell, patch)?;
        let mut percent_sum = 0.0;
        let mut value_sum = 0.0;
        for class in 0..input.raw_class_count {
            let percent = percentage_index(input, class, cell, patch)?.max(0.0);
            percent_sum += percent;
            value_sum = index(input, class, cell, patch)?.mul_add(percent, value_sum);
        }
        index_sum = (value_sum / percent_sum.max(1.0e-6)).mul_add(area, index_sum);
        area_sum += area;
    }
    ensure!(
        area_sum > 0.0 && area_sum.is_finite(),
        "PFT/PC patch {patch} has zero or non-finite land area"
    );
    Ok((index_sum, area_sum))
}

fn aggregate_natural_patch(
    cells: &[usize],
    range: std::ops::Range<usize>,
    input: PftFractionInput<'_>,
    output: &mut [f64],
    patch: usize,
) -> Result<()> {
    let first = range
        .clone()
        .next()
        .with_context(|| format!("natural patch {patch} has no PFT"))?;
    let mut total_area = 0.0;
    for &cell in cells {
        let area = area(input.land_area, cell, patch)?;
        total_area += area;
        let mut total = 0.0;
        for class in 0..input.raw_class_count {
            if Some(class) != input.crop_excluded_class {
                total += percentage(input, class, cell, patch)?.max(0.0);
            }
        }
        total = total.max(1.0e-6);
        for pft in range.clone() {
            let class = input.pft_classes[pft];
            let value = if Some(class) == input.crop_excluded_class {
                0.0
            } else {
                percentage(input, class, cell, patch)?.max(0.0)
            };
            output[pft] = (value / total).mul_add(area, output[pft]);
        }
    }
    ensure!(
        total_area > 0.0 && total_area.is_finite(),
        "natural PFT patch {patch} has zero or non-finite land area"
    );
    for value in &mut output[range.clone()] {
        *value /= total_area;
    }
    let sum = output[range.clone()].iter().sum::<f64>();
    if sum > 0.0 {
        for value in &mut output[range] {
            *value /= sum;
        }
    } else {
        output[first] = 1.0;
    }
    Ok(())
}

fn area(values: &[f64], cell: usize, patch: usize) -> Result<f64> {
    let value = values
        .get(cell)
        .copied()
        .with_context(|| format!("PFT patch {patch} references raw area cell {cell}"))?;
    ensure!(
        value.is_finite() && value >= 0.0,
        "PFT patch {patch} has invalid raw cell area {value}"
    );
    Ok(value)
}

fn percentage(input: PftFractionInput<'_>, class: usize, cell: usize, patch: usize) -> Result<f64> {
    let index = class * input.land_area.len() + cell;
    input.raw_percent.get(index).copied().with_context(|| {
        format!(
            "PFT patch {patch} references raw class {class}, cell {cell}, outside the raw percentage field"
        )
    })
}

fn percentage_index(
    input: PftIndexInput<'_>,
    class: usize,
    cell: usize,
    patch: usize,
) -> Result<f64> {
    let index = class * input.land_area.len() + cell;
    input.raw_percent.get(index).copied().with_context(|| {
        format!(
            "PFT/PC patch {patch} references raw class {class}, cell {cell}, outside the raw percentage field"
        )
    })
}

fn index(input: PftIndexInput<'_>, class: usize, cell: usize, patch: usize) -> Result<f64> {
    let offset = class * input.land_area.len() + cell;
    input.raw_index.get(offset).copied().with_context(|| {
        format!(
            "PFT/PC patch {patch} references raw class {class}, cell {cell}, outside the raw LAI/SAI field"
        )
    })
}

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;
