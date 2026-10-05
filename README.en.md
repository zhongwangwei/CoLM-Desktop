<p align="center">
  <img src="gui/dist/assets/colm-icon.png" width="96" alt="CoLM Desktop icon">
</p>

<h1 align="center">CoLM Desktop</h1>

<p align="center">
  A cross-platform desktop workbench for CoLM202X site and spatial simulations<br>
  <a href="README.md">中文</a> | <strong>English</strong>
</p>

<p align="center">
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml"><img src="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.1"><img src="https://img.shields.io/badge/version-v0.3.0--beta.1-orange" alt="Current beta version: v0.3.0-beta.1"></a>
  <img src="https://img.shields.io/badge/status-Beta-orange" alt="Status: Beta">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="Platforms">
</p>

CoLM Desktop brings CoLM202X site and spatial case setup, parameter constraints, the three-stage run and result evaluation together in one graphical interface. All three stages — surface data (`mksrfdata`), initial conditions (`mkinidata`) and the model main loop (`colm`) — run on the **Rust engine**, and the results are **bit-for-bit identical** to the original CoLM202X. The installers work out of the box: you do not need a Rust, Fortran, MPI or NetCDF toolchain.

> [!WARNING]
> **This is the `0.3.0-beta.1` prerelease.** Features are still changing quickly and there may be known or unknown defects. Keep backups of your original data and cases, and validate results independently before research use.

> [!IMPORTANT]
> Both **site** and **spatial** simulations are supported. Spatial runs cover watershed, regional and global extents on lat-lon, unstructured and catchment grids. Several sites can run concurrently as independent cases. **Parameter tuning and uncertainty analysis are not yet available in spatial mode.** They come back when you switch to site mode.

## Download

Get the installer for your platform from the [v0.3.0-beta.1 prerelease page](https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.1):

| Platform | Architecture | Format |
|---|---|---|
| macOS | Apple Silicon | `.dmg` |
| macOS | Intel x86_64 | `.dmg` |
| Windows | x86_64 | `.msi` / `.exe` |
| Linux | x86_64 | `.AppImage` / `.deb` / `.rpm` |

Each installer contains the desktop app, the `colm-cli` command line, the Rust engine (`mksrfdata-rs`, `mkinidata-rs`, `colm-rs`) and example sites. For platforms without a release, or for development builds, see "Running from source" below.

## What's new in 0.3.0-beta.1

- **Catchment grids on the Rust engine.** Hillslope flow, river-lake routing (with quadruple-precision sums), subsurface lateral flow and dynamic lakes on catchments and HRU hydrologic units are all ported. Basin history, the basin restart and `catch_parameters.nc` are written, bit-for-bit identical to the original.
- **The Rust engine is the default for every grid type:** site, lat-lon, unstructured and catchment.
- **Faster setup and restart writing for large spatial cases.** Data is now read and written per patch. Assembling a block of tens of thousands of patches takes tens of seconds instead of tens of minutes, and memory no longer grows with the square of the patch count.
- **Shared-memory parallelism.** Patch updates and large river systems run on multiple threads. When several cases run at once, they share the CPU cores automatically.

## Features

| Feature | Description |
|---|---|
| Guided model setup | Cards for spatial structure, land classification, subgrid and physics set the model constraints |
| Site and case management | Scan site folders, match forcing and observations automatically, create cases for the session in bulk |
| Constraint-aware parameters | Unified catalog search, common/expert tiers, IGBP/USGS/PFT/PC scopes, sparse overrides, safe import and export |
| CSV/TXT multi-site preparation | Detects comma, tab, semicolon or whitespace long tables, splits them by site and writes standard single-site NetCDF; can also generate site files in bulk |
| Forcing gap repair | Short gaps are interpolated according to each variable's physics. Long gaps are converted to UTC, matched to the nearest ERA5-Land cell and bias-corrected against overlapping observations, with an hourly QC trail |
| Rust model engine | Surface data, initial conditions and the main loop are all in Rust. Coverage includes LCT / PFT / PC, BGC, crops, urban, lakes, ozone, tracers, LULCC, grid river/reservoir routing and catchment lateral flow |
| Spatial simulation | Watershed, regional and global extents on lat-lon, unstructured and catchment grids. Regional cases turn on regional unit-catchment routing by default |
| Parallelism | Shared-memory multithreading in one process (patch updates and large river systems run per cell). `RAYON_NUM_THREADS` sets the thread count; the default is every logical core |
| Concurrent sites | Run independent cases at the same time with CPU cores shared automatically; each site shows its own progress, stage and log |
| Staged runs | Run `mksrfdata`, `mkinidata` and `colm` separately, or all stages in one go |
| Results workbench | Seven panes: overview, variable catalog, time series, selectable evaluation, multi-site ranking, process diagnostics, and PDF/HTML/CSV/JSON/Markdown export |
| Bilingual interface | The home page and main workflow switch between Chinese and English, with standard and expert modes |

The configuration covers IGBP / USGS, LCT / PFT / PC, hydrology and heat, BGC, crops, urban, river and tracer processes. The GUI hides parameters that do not apply to the current site/spatial mode and process constraints. Combinations the Rust engine does not cover yet, such as reservoir operation, LULCC, tracers or vector history on catchment grids, are reported before the run starts, not after preprocessing finishes.
See [`docs/parameter-catalog.md`](docs/parameter-catalog.md), [`docs/parameter-gui-guide.md`](docs/parameter-gui-guide.md) and [`docs/parameter-default-preservation.md`](docs/parameter-default-preservation.md) for the parameter catalog and guides. Upstream defects found and fixed in this repository are listed in [`docs/upstream-bugs.md`](docs/upstream-bugs.md).

## Workflow

1. **Choose the simulation type.** Pick the site, land classification, subgrid and physics on the start cards.
2. **Prepare your own data (optional).** Choose a single-site NetCDF file, or a CSV, TXT or TSV file with one or more sites. Tables are split by site, converted to UTC and checked site by site. Short gaps are interpolated directly. For long gaps, ERA5-Land is downloaded or read from the cache and corrected over the overlap period; the result goes into new standard files and never overwrites the originals. Skip this step when using the built-in examples.
3. **Set files and folders.** Choose the prepared `Sitedata` and `Forcing` folders and the root folder for cases.
4. **Create cases.** Scan the sites, tick one or more, and let forcing and observation files be matched automatically.
5. **Check the basic settings.** Configure spin-up, surface data, initial conditions, forcing and parallel options.
6. **Configure process parameters.** Only the processes active under the current constraints are shown.
7. **Choose outputs and run.** Run stage by stage or all at once, with live progress and logs for every site.
8. **Analyze results.** Browse the actual history variables and dimensions, plot and zoom per site, and export full CSV.
9. **Evaluate and diagnose.** Choose from the available observations (net radiation, energy fluxes, friction velocity, GPP, ecosystem respiration, NEE and others). Compute RMSE, MAE, Bias, R², Pearson r, NSE and KGE with its components, compare sites, and export a report or PDF.

The results workbench only includes cases created in the current task, never old results left in the case root. Long series and model–observation pair plots are downsampled while keeping extremes; metrics still use every sample. Multi-site evaluation runs in a bounded worker pool, so one site with missing observations or a failure does not stop the others.

### Recommended data layout

When sites, forcing and observations sit in sibling folders, the GUI matches them by site name:

```text
data-root/
├── Sitedata/
│   └── <site>_site.nc
├── Forcing/
│   └── <site>_Met.nc
└── Observation/
    └── <site>_Flux.nc
```

After you choose `Sitedata`, the forcing folder and its availability update automatically. You can also choose other folders explicitly in the basic settings.

## Running from source

### Requirements

- Rust **1.85.1** or newer
- Git
- The [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your platform
- A kernel folder for running cases (included in the installers; see the note at the end to build one from source)

### Start the desktop app

```bash
git clone https://github.com/zhongwangwei/CoLM-Desktop.git
cd CoLM-Desktop
cargo run --manifest-path gui/src-tauri/Cargo.toml
```

The startup log reports whether the WebView reached the backend, which `colm-cli` path was resolved and how many kernels are available. The app opens on the card selection home page.

### Command line

The GUI drives cases through the same `colm-cli`, which you can also use on its own:

```bash
# Scan sites and check forcing matches
cargo run -p colm-cli -- scan \
  --dir /path/to/Sitedata \
  --forcing-dir /path/to/Forcing

# Probe a single- or multi-site CSV/TXT long table
cargo run -p colm-cli -- forcing-table-probe /path/to/sites.csv --json 1

# Create a case
cargo run -p colm-cli -- new \
  --site /path/to/Sitedata/site.nc \
  --out /path/to/cases/site-name

# Run every stage (Rust engine)
cargo run -p colm-cli -- run /path/to/cases/site-name \
  --kernel kernels/default

# Compute metrics against observations
cargo run -p colm-cli -- metrics /path/to/cases/site-name \
  --obs /path/to/Observation/site_Flux.nc \
  --pairs-var Rnet --pairs-var GPP --pairs-var NEE

# List the evaluation variables shared by the case and the observations
cargo run -p colm-cli -- evaluation-catalog /path/to/cases/site-name \
  --obs /path/to/Observation/site_Flux.nc

# Browse history and export an extreme-preserving downsampled series
cargo run -p colm-cli -- history-catalog /path/to/cases/site-name
cargo run -p colm-cli -- series /path/to/cases/site-name \
  --vars f_rnet --max-points 2400
```

The `--kernel` folder supplies the compile-time configuration (site, lat-lon, unstructured or catchment grid; IGBP or USGS; crop). Run `cargo run -p colm-cli --` for every command and option.

## Architecture

```text
Static HTML/CSS/JS GUI
          │ Tauri IPC
          ▼
Rust window backend ──► colm-cli sidecar ──► mksrfdata-rs / mkinidata-rs / colm-rs
          │                    │
          ├── schema           ├── forcing / surface data
          ├── namelist         ├── stage orchestration
          └── history gates    └── metrics / time series
```

| Module | Role |
|---|---|
| `gui/src-tauri` | Tauri window, file pickers, IPC, batch jobs and event forwarding |
| `crates/colm-cli` | The single orchestration entry shared by the GUI and the command line |
| `crates/colm-case` | Case folders and namelist generation |
| `crates/colm-namelist` | Format-preserving CoLM namelist reading and writing |
| `crates/colm-schema` | Configuration fields and defaults generated from the CoLM source |
| `crates/colm-kernel` | Kernel identity checks, three-stage execution, success detection and override messages |
| `crates/colm-core` | CoLM physics (energy, hydrology, snow, canopy, BGC, crops, urban, lakes, quadruple-precision arithmetic and more) |
| `crates/colm-runtime` | The main loop `colm-rs`: site and spatial drivers, forcing, river/reservoir routing, catchment lateral flow, history and restarts |
| `crates/colm-init` | Initial conditions `mkinidata-rs`, including the catchment network and cold start |
| `crates/colm-srfdata` | Surface data `mksrfdata-rs` (site and spatial aggregation), single-point surface data filling and provenance records |
| `crates/colm-lapack` | Urban radiation matrix inversion, reproducing the LAPACK results the original links against on each platform |
| `crates/colm-forcing` | Forcing probing, gap repair, ERA5-Land correction, validation and configuration |
| `crates/colm-hist` | Output variable gates, time series and evaluation metrics |
| `oracle` | Golden-result regression and numerical consistency checks |

The window process does not link NetCDF/HDF5. Anything that reads NetCDF goes through the `colm-cli` sidecar, which keeps the desktop process light and isolates native I/O dependencies.

## Development and verification

```bash
# Engine tests
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Static GUI interface and behavior tests
cargo run -q -p xtask -- check-gui
for test in gui/tests/*.mjs; do node "$test"; done

# Tauri backend
cargo test --manifest-path gui/src-tauri/Cargo.toml
cargo clippy --manifest-path gui/src-tauri/Cargo.toml --all-targets -- -D warnings
```

CI builds and tests the whole Rust workspace on macOS, Windows and Linux. That includes the Rust engine's physics, main loop, initial-condition and surface-data crates, plus the GUI interface, formatting and static analysis. It also checks tolerance classification and NetCDF I/O against committed golden files. Each round of bit-for-bit comparison with the original (site, lat-lon, unstructured and catchment cases) is recorded in the verification log.

## Documentation

- [Design and overall architecture](docs/design.md)
- [GUI entry and constraint design](docs/design-gate.md)
- [GUI workflow design](docs/design-gui3.md)
- [Preprocessing design](docs/design-prep.md)
- [Forcing gap repair design and acceptance matrix](docs/plan-forcing-gap-repair.md)
- [CSV/TXT multi-site preparation contract and acceptance matrix](docs/plan-tabular-multisite-prep.md)
- [Results workbench design](docs/plan-results-workbench.md)
- [Implementation, defect reviews and verification log](docs/implementation-verification.md)
- [Upstream CoLM202X defects and local fixes](docs/upstream-bugs.md)
- [CPU parallelism conventions](docs/cpu-parallelism.md)

The design documents are in Chinese.

## Releases

When `main` carries a version that has not been released yet, or a `v*` tag is pushed, `.github/workflows/release.yml` builds installers for macOS (Apple Silicon and Intel), Windows and Linux. It checks that the sidecars and example data are bundled, then publishes the release. To package locally:

```bash
cargo run -p xtask -- stage-sidecar
cd gui/src-tauri
cargo tauri build --config tauri.bundle.conf.json
```

## Maintainers

- **Development and maintenance**: Zhongwang Wei @ CoLM land surface model development team
- **Affiliation**: School of Atmospheric Sciences, Sun Yat-sen University
- **Email**: [weizhw6@mail.sysu.edu.cn](mailto:weizhw6@mail.sysu.edu.cn)
- **Project page**: <https://github.com/zhongwangwei/CoLM-Desktop>

**Copyright: CoLM land surface model development team, School of Atmospheric Sciences, Sun Yat-sen University.**

## License

The Rust and desktop code is dual-licensed under `MIT OR Apache-2.0`. See each third-party dependency and the upstream CoLM202X source for their licenses.

---

> Note: The installers also include the original CoLM202X Fortran kernels. You can switch to them under "Model engine" on the run page, or with `colm-cli run … --engine fortran`. The Rust engine is the default. When running from source, build the kernel folders under `kernels/` with `./oracle/scripts/build_kernel.sh <preset>` (this needs gfortran, NetCDF-Fortran, LAPACK and MPI).
