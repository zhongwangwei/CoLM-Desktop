use super::*;

#[test]
fn land_filter_compacts_leading_empty_and_mixed_elements_in_place() {
    let mut mesh = FlatMesh::new(
        vec![1, 2, 3, 4],
        vec![0, 2, 3, 5, 6],
        vec![1, 2, 3, 4, 5, 6],
        vec![1; 6],
    )
    .unwrap();
    let mut types = vec![0, -1, 2, 0, 17, 0];
    mesh.retain_land_pixels(&mut types).unwrap();
    assert_eq!(mesh.land_elements().element_ids, [2, 3]);
    assert_eq!(mesh.land_elements().pixel_end, [1, 1]);
    assert_eq!(mesh.pixels(0).unwrap().0, [3]);
    assert_eq!(mesh.pixels(1).unwrap().0, [5]);
    assert_eq!(types, [2, 17]);
    types.fill(0);
    mesh.retain_land_pixels(&mut types).unwrap();
    assert!(mesh.is_empty());
    assert!(types.is_empty());
}

#[test]
fn mesh_uses_one_flat_coordinate_store_for_variable_size_elements() {
    let mesh = FlatMesh::new(
        vec![42, 9001],
        vec![0, 2, 5],
        vec![10, 11, 4, 5, 6],
        vec![20, 20, 7, 8, 9],
    )
    .unwrap();
    assert_eq!(mesh.pixel_count(0).unwrap(), 2);
    assert_eq!(mesh.pixel_count(1).unwrap(), 3);
    assert_eq!(mesh.pixels(1).unwrap(), (&[4, 5, 6][..], &[7, 8, 9][..]));
}

#[test]
fn landelm_fields_match_the_fortran_builder() {
    let mesh = FlatMesh::new(
        vec![11, 13],
        vec![0, 3, 4],
        vec![1, 2, 3, 4],
        vec![5, 5, 5, 6],
    )
    .unwrap();
    let landelm = mesh.land_elements();
    assert_eq!(landelm.element_ids, vec![11, 13]);
    assert_eq!(landelm.pixel_start, vec![1, 1]);
    assert_eq!(landelm.pixel_end, vec![3, 1]);
    assert_eq!(landelm.set_type, vec![0, 0]);
    assert_eq!(landelm.element_index, vec![1, 2]);
}

#[test]
fn mesh_rejects_empty_elements_duplicate_ids_and_zero_based_pixels() {
    assert!(FlatMesh::new(vec![1], vec![0, 0], vec![], vec![]).is_err());
    assert!(FlatMesh::new(vec![1, 1], vec![0, 1, 2], vec![1, 2], vec![1, 2]).is_err());
    assert!(FlatMesh::new(vec![1], vec![0, 1], vec![0], vec![1]).is_err());
}

#[test]
fn landpatch_partitions_reordered_pixels_with_colms_quicksort_permutation() {
    let mesh = FlatMesh::new(
        vec![100],
        vec![0, 4],
        vec![10, 11, 12, 13],
        vec![20, 20, 20, 20],
    )
    .unwrap();
    let (mesh, patches) = mesh.into_land_patches(&[2, 1, 2, 0], false).unwrap();
    assert_eq!(patches.element_ids, vec![100, 100, 100]);
    assert_eq!(patches.pixel_start, vec![1, 2, 3]);
    assert_eq!(patches.pixel_end, vec![1, 2, 4]);
    assert_eq!(patches.set_type, vec![0, 1, 2]);
    assert_eq!(patches.element_index, vec![1, 1, 1]);
    assert_eq!(mesh.pixels(0).unwrap().0, &[13, 11, 10, 12]);

    let layout = patches
        .aggregation_layout(&mesh, vec![None, None, None])
        .unwrap();
    assert_eq!(
        layout.aggregate_soil_texture(&[8, 9, 10, 11]).unwrap(),
        vec![8, 9, 10]
    );
}

#[test]
fn dominant_patch_type_preserves_ocean_but_merges_positive_land_types() {
    let mesh = FlatMesh::new(
        vec![100],
        vec![0, 4],
        vec![10, 11, 12, 13],
        vec![20, 20, 20, 20],
    )
    .unwrap();
    let (_, patches) = mesh.into_land_patches(&[2, 1, 1, 0], true).unwrap();
    assert_eq!(patches.set_type, vec![0, 1]);
    assert_eq!(patches.pixel_start, vec![1, 2]);
    assert_eq!(patches.pixel_end, vec![1, 4]);
}

#[test]
fn catchment_hrus_follow_the_mesh_sort_and_lake_sign_contract() {
    let mesh = FlatMesh::new(
        vec![100, 200],
        vec![0, 3, 5],
        vec![10, 11, 12, 20, 21],
        vec![1, 1, 1, 2, 2],
    )
    .unwrap();

    let (mesh, hrus) = mesh.into_land_hrus(&[2, 1, 2, 2, 1], &[0, 7]).unwrap();

    assert_eq!(hrus.element_ids, vec![100, 100, 200, 200]);
    assert_eq!(hrus.pixel_start, vec![1, 2, 1, 2]);
    assert_eq!(hrus.pixel_end, vec![1, 3, 1, 2]);
    assert_eq!(hrus.set_type, vec![1, 2, -1, -2]);
    assert_eq!(hrus.element_index, vec![1, 1, 2, 2]);
    assert_eq!(mesh.pixels(0).unwrap().0, &[11, 12, 10]);
    assert_eq!(mesh.pixels(1).unwrap().0, &[21, 20]);
}

#[test]
fn urban_refinement_splits_only_urban_patches_and_preserves_fortran_missing_fill() {
    let mesh = FlatMesh::new(
        vec![100, 200],
        vec![0, 4, 5],
        vec![10, 11, 12, 13, 20],
        vec![1, 1, 1, 1, 2],
    )
    .unwrap();
    let patches = FlatLandPatches {
        element_ids: vec![100, 200],
        pixel_start: vec![1, 1],
        pixel_end: vec![4, 1],
        set_type: vec![13, 17],
        element_index: vec![1, 2],
    };

    let (mesh, patches, urban) = mesh
        .into_urban_land_patches(&patches, &[3, 0, 2, 7, 1], &[1.0; 5], 13, 3)
        .unwrap();

    // The upstream fill assigns the two missing values as 2 then 3 because
    // only class 2 contributes to its historic proportion calculation.
    assert_eq!(mesh.pixels(0).unwrap().0, &[11, 12, 13, 10]);
    assert_eq!(patches.set_type, vec![13, 13, 17]);
    assert_eq!(patches.pixel_start, vec![1, 3, 1]);
    assert_eq!(patches.pixel_end, vec![2, 4, 1]);
    assert_eq!(urban.set_type, vec![2, 3]);
    assert_eq!(urban.element_ids, vec![100, 100]);
    assert_eq!(urban.pixel_start, vec![1, 3]);
    assert_eq!(urban.pixel_end, vec![2, 4]);
}

#[test]
fn urban_refinement_assigns_the_final_class_when_every_class_is_missing() {
    let mesh = FlatMesh::new(vec![100], vec![0, 2], vec![10, 11], vec![1, 1]).unwrap();
    let patches = FlatLandPatches {
        element_ids: vec![100],
        pixel_start: vec![1],
        pixel_end: vec![2],
        set_type: vec![13],
        element_index: vec![1],
    };
    let (_, patches, urban) = mesh
        .into_urban_land_patches(&patches, &[0, 99], &[1.0, 1.0], 13, 10)
        .unwrap();
    assert_eq!(patches.set_type, vec![13]);
    // `buff_count(N_URB)` gets the unallocated remainder in the Fortran
    // implementation, so its later nominal fallback branch is unreachable.
    assert_eq!(urban.set_type, vec![10]);
    assert_eq!(urban.pixel_start, vec![1]);
    assert_eq!(urban.pixel_end, vec![2]);
}

#[test]
fn wmo_patches_keep_first_largest_source_and_shift_later_element_indices() {
    let mesh = FlatMesh::new(
        vec![1, 2, 3],
        vec![0, 6, 8, 11],
        (1..=11).collect(),
        vec![1; 11],
    )
    .unwrap();
    let (mesh, patches) = mesh
        .into_land_patches(&[1, 1, 2, 2, 17, 17, 17, 17, 10, 10, 13], false)
        .unwrap();
    let mut elements = mesh.land_elements();
    let wmo = patches.with_wmo_patches(&mut elements).unwrap();
    assert_eq!(elements.set_type, [1, 0, 1]);
    assert_eq!(wmo.set_type, [1, 2, 17, 1, 17, 10, 13, 10]);
    assert_eq!(wmo.pixel_start, [1, 3, 5, 0, 1, 1, 3, 0]);
    assert_eq!(wmo.pixel_end, [2, 4, 6, 0, 2, 2, 3, 0]);
    let sources = vec![None, None, None, Some(0), None, None, None, Some(5)];
    assert_eq!(wmo.wmo_sources().unwrap(), sources);
    let layout = wmo.aggregation_layout(&mesh, sources).unwrap();
    assert_eq!(
        layout
            .aggregate_soil_texture(&(1..=11).collect::<Vec<_>>())
            .unwrap()[3],
        layout
            .aggregate_soil_texture(&(1..=11).collect::<Vec<_>>())
            .unwrap()[0]
    );
    assert!(wmo
        .aggregation_layout(&mesh, vec![None; wmo.len()])
        .is_err());
    assert!(wmo.with_wmo_patches(&mut elements).is_err());
    let mut invalid = wmo.clone();
    invalid.pixel_end[3] = 1;
    assert!(invalid.wmo_sources().is_err());
    invalid = wmo.clone();
    invalid.set_type[3] = 2;
    assert!(invalid.wmo_sources().is_err());
}
