use super::*;

fn doc(body: &str) -> Document {
    colm_namelist::parse(&format!("&nl_colm\n{body}/\n")).unwrap()
}

#[test]
fn each_land_class_gets_its_own_override() {
    let document = doc(
        " DEF_LC_VMAX25(7) = 40.0\n DEF_LC_VMAX25(10) = 60.0\n DEF_LC_HTOP0(10) = 1.5\n \
         DEF_LC_C3C4(10) = 1\n DEF_LC_PSI50_ROOT(7) = -300000.0\n DEF_LC_SAI0(3) = -1.e36\n",
    );
    let (table, plant) = land_class_overrides(&document, 17).unwrap();
    assert_eq!(table.for_class(7).vmax25_umol, Some(40.0));
    assert_eq!(table.for_class(10).vmax25_umol, Some(60.0));
    assert_eq!(table.for_class(10).htop0, Some(1.5));
    assert_eq!(table.for_class(10).c3c4, Some(1));
    assert_eq!(table.for_class(7).htop0, None);
    // A slot written as the sentinel is not an override; untouched classes have none.
    assert_eq!(table.for_class(3), LandClassOverrides::default());
    assert_eq!(table.classes().collect::<Vec<_>>(), [7, 10]);
    assert_eq!(plant.for_class(7).root_psi50_mm, Some(-300000.0));
    assert_eq!(plant.for_class(10), PlantHydraulicOverrides::default());
    assert!(has_land_class_overrides(&document));
    assert!(!has_land_class_overrides(&doc(" DEF_LC_YEAR = 2005\n")));
}

#[test]
fn bare_values_and_classes_outside_the_scheme_are_refused() {
    // In Fortran a bare value only lands in slot 1: say so instead of guessing.
    let bare = land_class_overrides(&doc(" DEF_LC_VMAX25 = 40.0\n"), 17).unwrap_err();
    assert!(bare.to_string().contains("DEF_LC_VMAX25(lc)"), "{bare}");
    // IGBP has 17 classes; Fortran would silently ignore slot 20.
    assert!(land_class_overrides(&doc(" DEF_LC_VMAX25(20) = 40.0\n"), 17).is_err());
    assert!(land_class_overrides(&doc(" DEF_LC_VMAX25(20) = 40.0\n"), 24).is_ok());
    assert!(land_class_overrides(&doc(" DEF_LC_C3C4(5) = 2\n"), 17).is_err());
}

#[test]
fn canopy_tables_take_each_class_height() {
    let (table, _) = land_class_overrides(
        &doc(" DEF_LC_HTOP0(10) = 1.5\n DEF_LC_HBOT0(10) = 0.05\n DEF_LC_HTOP0(16) = 0.2\n"),
        17,
    )
    .unwrap();
    let scheme = colm_core::LandCoverScheme::Igbp;
    let (top, bottom) = crate::single_point::canopy_tables(scheme, &table);
    let (plain_top, plain_bottom) =
        crate::single_point::canopy_tables(scheme, &LandClassOverrideTable::new());
    assert_eq!((top[10], bottom[10], top[16]), (1.5, 0.05, 0.2));
    // 没写的地类、没写的列照旧是表值。
    assert_eq!(bottom[16], plain_bottom[16]);
    for class in (1..=17).filter(|class| ![10, 16].contains(class)) {
        assert_eq!(
            (top[class], bottom[class]),
            (plain_top[class], plain_bottom[class])
        );
    }
}
