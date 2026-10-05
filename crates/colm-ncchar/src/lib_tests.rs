use super::NcChar;

#[test]
fn char_arrays_round_trip_as_nc_char() {
    let dir = std::env::temp_dir().join(format!("colm-ncchar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("names.nc");
    let names = ["Longyangxia", "Liujiaxia"];
    let width = 12;
    let mut bytes = vec![NcChar(0); names.len() * width];
    for (row, name) in names.iter().enumerate() {
        for (col, byte) in name.bytes().enumerate() {
            bytes[row * width + col] = NcChar(byte as i8);
        }
    }
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("dam", names.len()).unwrap();
        file.add_dimension("len", width).unwrap();
        let mut var = file
            .add_variable::<NcChar>("name", &["dam", "len"])
            .unwrap();
        var.put_values(&bytes, ..).unwrap();
    }
    let file = netcdf::open(&path).unwrap();
    let var = file.variable("name").unwrap();
    assert_eq!(var.vartype(), netcdf::types::NcVariableType::Char);
    let back: Vec<NcChar> = var.get_values(..).unwrap();
    assert_eq!(back, bytes);
    std::fs::remove_dir_all(&dir).unwrap();
}
