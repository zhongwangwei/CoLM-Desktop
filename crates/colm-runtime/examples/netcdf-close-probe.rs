//! Used only by scripts/test-netcdf-close-faults.py; no external datasets.
use anyhow::{bail, ensure, Context, Result};
use colm_runtime::river::{restart::write_river_history, RiverHistory};
use std::path::Path;

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let [_, mode, path] = args.as_slice() else {
        bail!("usage: netcdf-close-probe history|unchecked|observation|observation-updated PATH");
    };
    match mode.as_str() {
        "history" => write_river_history(Path::new(path), &RiverHistory::zeros(8))
            .context("actual river history writer failed")?,
        "observation" | "observation-updated" => {
            let file = netcdf::open(path)?;
            let values = file
                .variable("Qle")
                .context("converted observation has no Qle")?
                .get_values::<f64, _>(..)?;
            let expected = if mode == "observation-updated" {
                [20.0, 21.0]
            } else {
                [10.0, 11.0]
            };
            ensure!(values.as_slice() == expected, "unexpected observation data");
        }
        // Sensitivity control: the exact same C-level close failure is swallowed by Drop.
        "unchecked" => {
            let mut file = netcdf::create(path)?;
            file.add_dimension("time", 2048)?;
            file.add_variable::<f64>("value", &["time"])?
                .put_values(&vec![1.0; 2048], ..)?;
            drop(file);
        }
        _ => bail!("unknown probe mode: {mode}"),
    }
    println!("writer returned success");
    Ok(())
}
