//! 空间降尺度的映射部件：`topo_grid`/`maxelv_grid` 的初始化与 `part2pset`。

use super::*;
use colm_init::spatial_mapping::MappingPart;

fn simple_patch(elevation_m: f64, patch_mask: bool) -> DownscalingPatch {
    DownscalingPatch {
        elevation_m,
        glacier: false,
        patch_mask,
        terrain: PatchTerrain::Simple {
            slope: [0.0; ASPECT_TYPES],
            aspect: [0.0; ASPECT_TYPES],
            curvature: 0.0,
        },
    }
}

fn settings() -> DownscalingSettings {
    DownscalingSettings {
        simple: true,
        config: ForcingDownscalingConfig::default(),
    }
}

/// 两个 patch 共用格子 (0,0)；第二个还覆盖 (1,0)。`patchmask` 为假的 patch 不进 `topo_grid`/`maxelv_grid`，
/// 但它的份面积仍计入 `areagrid`。只被遮蔽 patch 覆盖的格子 `topo_grid` 是 `spval`。
#[test]
fn grid_elevation_uses_masked_patches_only() {
    let part = |ilon, ilat, area| MappingPart { ilon, ilat, area };
    let mapping = AreaWeightedMapping {
        parts: vec![
            vec![part(0, 0, 1.0)],
            vec![part(0, 0, 3.0), part(1, 0, 2.0)],
            vec![part(2, 0, 4.0)],
        ],
        area: vec![1.0, 5.0, 4.0],
    };
    let state = SpatialDownscaling::new(
        settings(),
        vec![
            simple_patch(100.0, true),
            simple_patch(500.0, true),
            simple_patch(900.0, false),
        ],
        &mapping,
    )
    .unwrap();
    let cell = |key| state.cell_of[&key];
    assert_eq!(state.topo_grid[cell((0, 0))], (100.0 + 500.0 * 3.0) / 4.0);
    assert_eq!(state.topo_grid[cell((1, 0))], 500.0);
    assert_eq!(state.topo_grid[cell((2, 0))], MISSING);
    assert_eq!(state.maxelv_grid[cell((0, 0))], 500.0);
    assert_eq!(state.maxelv_grid[cell((2, 0))], MISSING);
    assert_eq!(state.area_grid[cell((2, 0))], 4.0);
}

/// `part2pset` 跳过未赋值（面积为 0）的份，面积为 0 的 set 得 `spval`。
#[test]
fn part_to_set_is_an_area_weighted_mean() {
    let part = |area| MappingPart {
        ilon: 0,
        ilat: 0,
        area,
    };
    let mapping = AreaWeightedMapping {
        parts: vec![vec![part(1.0), part(0.0), part(3.0)], vec![part(0.0)]],
        area: vec![4.0, 0.0],
    };
    let value = |t| PartForcing {
        t,
        q: 0.0,
        pbot: 0.0,
        rhoair: 0.0,
        prc: 0.0,
        prl: 0.0,
        frl: 0.0,
        swrad: 0.0,
        us: 0.0,
        vs: 0.0,
    };
    let row = [Some(value(280.0)), None, Some(value(284.0))];
    assert_eq!(
        part_to_set(&mapping, 0, &row, |p| p.t),
        (280.0 + 284.0 * 3.0) / 4.0
    );
    assert_eq!(part_to_set(&mapping, 1, &[None], |p| p.t), MISSING);
}
