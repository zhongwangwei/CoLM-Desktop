use super::*;

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
