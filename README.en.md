<p align="center">
  <img src="gui/public/assets/colm-icon.png" width="96" alt="CoLM Desktop icon">
</p>

<h1 align="center">CoLM Desktop</h1>

<p align="center">
  A cross-platform desktop workbench for CoLM202X site, regional and global simulations<br>
  <a href="README.md">中文</a> | <strong>English</strong>
</p>

<p align="center">
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml"><img src="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.5"><img src="https://img.shields.io/badge/version-v0.3.0--beta.5-orange" alt="Current beta version: v0.3.0-beta.5"></a>
  <img src="https://img.shields.io/badge/status-Beta-orange" alt="Status: Beta">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="Platforms">
</p>

CoLM Desktop brings CoLM202X site and spatial case setup, parameter constraints, the three-stage run and result evaluation together in one graphical interface. All three stages — surface data (`mksrfdata`), initial conditions (`mkinidata`) and the model main loop (`colm`) — run on the **Rust engine**, and the results are **bit-for-bit identical** to the original CoLM202X. The installers work out of the box: you do not need a Rust, Fortran, MPI or NetCDF toolchain.

> [!WARNING]
> **This is the `0.3.0-beta.5` prerelease.** Features are still changing quickly and there may be known or unknown defects. Keep backups of your original data and cases, and validate results independently before research use.

> [!IMPORTANT]
> Both **site** and **spatial** simulations are supported. Spatial runs cover watershed, regional and **0.1° global** extents on lat-lon, unstructured and catchment grids. The Rust engine's spatial loop supports **Linux multi-node** execution (`srun`/`mpiexec`) with block-level SPMD, validated by a 10-day 0.1° global run on 8 tianhe nodes. Several sites can run concurrently as independent cases. Calibration, uncertainty analysis and AI studies also work on basin, regional and global cases.

## Download

Get the installer for your platform from the [v0.3.0-beta.5 prerelease page](https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.5):

| Platform | Architecture | Format |
|---|---|---|
| macOS | Apple Silicon | `.dmg` |
| macOS | Intel x86_64 | `.dmg` |
| Windows | x86_64 | `.msi` / `.exe` |
| Linux | x86_64 | `.AppImage` / `.deb` / `.rpm` |

Each installer contains the desktop app, the `colm-cli` command line, the Rust engine (`mksrfdata-rs`, `mkinidata-rs`, `colm-rs`) and example sites. For platforms without a release, or for development builds, see "Running from source" below.

## Terminal assistant

Run `colm-agent chat --project-root /your/project` without the desktop window. It supports streaming responses, explicit approvals and saved conversations. `colm-agent config` sets the provider, model, API format and reasoning options, or selects the existing Codex, Claude Code or OpenCode backend.

pip distribution is temporarily paused: wheel generation and publishing are disabled. The desktop app and native CLI remain available. See [terminal usage](docs/terminal-assistant.md).

## What's new in 0.3.0-beta.5 (release notes)

- **Compare page (OpenBench evaluation)**: OpenBench is installed privately in the app's own folder; compare simulations across projects and versions on this computer or a server (live logs, Stop, only the report comes back). Register model variables, reference data and non-standard simulations; choose several references per variable; tick metrics and scores read from the installed OpenBench; annual and monthly climatology time scales.
- **Research (calibration, uncertainty, AI)**: now part of New modified version as a four-step wizard whose results become a new version through a draft; works on basin, regional and global cases; shows convergence and how tightly observations pin each parameter; site targets can be compared as daily or monthly means, variables weighted, and shared parameters cross-validated by site; results no better than the defaults are refused.
- **Model versions**: a version library beside a workbench; edit a version's source in the app or any external editor with changes recorded automatically; build and test checks before freezing; compare any two versions.
- **Land-class parameters**: land-class table overrides apply per class at every scale; stomatal tables are tuned per land class; tracer and river-sediment parameters are editable in the parameter dialog.
- **Server runs**: station batches are packed onto nodes on Slurm, LSF, PBS and machines without a scheduler; the engine is built on the login node, and servers without internet get dependencies sent from the desktop; results are read on the server and only what is plotted comes back; built-in presets for common clusters.
- **AI assistant**: attach local PDF, DOCX and text documents; web search uses DeepSeek's Search API; documentation search can optionally use a Xerj node the user already runs locally (off by default; candidates are still re-read and hash/version/knowledge-binding checked by CoLM); every page button is registered as an assistant action, with dangerous ones still confirmed by hand; built-in API requests are budgeted against the configured context window for the current model, with a warning near the window and folding of older tool results, and a blank window for an unknown model must be filled in rather than guessed; 40 generations per turn with a one-click Continue.
- **Interface and safety**: closing or quitting asks first when edits are unsaved or work is running; runs a crash left behind are stopped at the next start; the sidebar reads History and Resources; a batch of interface fixes from page-by-page click-throughs and external audits.
- Both engines still give bit-identical results.

## What's new in 0.3.0-beta.4

- **Rebuilt interface**: a version-first UI; History shows records as cards whose details open in a dialog, and can be cleared (optionally with local case folders and records whose end was never confirmed).
- **Default paths per compute resource**: this machine and each server can store default project, surface data, runtime and forcing paths; new simulations fill them in and sync edits back.
- **Model versions**: modified versions can be deleted, and older official revisions cleaned up together with source snapshots only they used.
- **Remote runs**: server jobs are checked every 10 seconds; remote case creation reuses an existing folder; cleanup on NFS no longer hides the real error; a `.prj` whose name differs only in case is accepted.
- **Routing**: watershed/region lat-lon cases default to the regional river network; global routing drops inefficient nested parallelism.
- **Research and history**: replay of recorded settings works again as a non-strict rerun; several data-safety fixes in Study state and locks, fetching and deletion.
- Both engines still give bit-identical results.

## What's new in 0.3.0-beta.3

This section preserves the released interface description. For the current source interface, see Workflow below.

- **River-lake routing is optional.** A "River-lake routing" card next to BGC on the start page's process step decides whether rivers are computed (new namelist switch `DEF_USE_GridRiverLakeFlow`). Leave it off and no river network is built, no routing runs and the "Rivers and reservoirs" step disappears; land results are unchanged. The card warns that a watershed boundary, a region or an unstructured mesh may not match the unit catchments of the river network.
- **Regional river networks no longer pull in unrelated river systems.** River systems are merged along bifurcation pathways only when bifurcation is on. The unitcat river output is cropped to the model domain and lines up cell by cell with the land history. New lat-lon cases write history at the model resolution.
- **Reservoir operation** now works on catchment meshes, and lat-lon grid reservoirs read the GRanD table carried by the unit-catchment file by default.
- **Open existing cases** from the start page or the files page. The case's grid, subgrid, process switches, kernel and setup inputs are restored.
- **Runs and logs.** The progress bar advances during Rust runs; the log is split by stage and written files are summed up on one line; spatial runs show threads or MPI ranks depending on the engine; starting a run no longer checks hundreds of thousands of RawData files one by one.
- **A leaner interface.** The preprocessing entry is hidden, and options users need not decide (block partition, MPI IO grouping, regional unit catchments) are no longer shown.
- Both engines remain bit-for-bit identical.

## What's new in 0.3.0-beta.2

- **Faster large spatial cases.** History accumulation now runs per patch in parallel, and the restart snapshot is only taken when a restart is written. Quadruple-precision division in catchment river-lake routing is faster. Setup no longer resolves paths or looks up PFT parameters repeatedly. Plant hydraulics, photosynthesis and variably saturated soil water reuse `pow` results for identical arguments. A 3-day Pearl River catchment run (36,000 patches) drops from about 9–10 minutes to about 6.5 minutes.
- **No change to any result.** Catchment, site and lat-lon grid cases stay bit-for-bit identical to the previous release and to the original Fortran.
- Run `colm-rs` with `COLM_RS_TIMING=1` to print how long each part of the main loop took.

## What's new in 0.3.0-beta.1

- **Catchment grids on the Rust engine.** Hillslope flow, river-lake routing (with quadruple-precision sums), subsurface lateral flow and dynamic lakes on catchments and HRU hydrologic units are all ported. Basin history, the basin restart and `catch_parameters.nc` are written, bit-for-bit identical to the original.
- **The Rust engine is the default for every grid type:** site, lat-lon, unstructured and catchment.
- **Faster setup and restart writing for large spatial cases.** Data is now read and written per patch. Assembling a block of tens of thousands of patches takes tens of seconds instead of tens of minutes, and memory no longer grows with the square of the patch count.
- **Shared-memory parallelism.** Patch updates and large river systems run on multiple threads. When several cases run at once, they share the CPU cores automatically.

## Features

| Feature | Description |
|---|---|
| Models and cases | A model revision fixes code, parameters, physics and networks; a case supplies location, input, period and output data. Runs use independent copies |
| Site and case management | Scan site folders and match inputs, open existing cases, or create independent cases from bundled examples |
| Parameter overrides | Search the version catalog, record IGBP/USGS/PFT/PC and process-file scopes, save sparse overrides, import and export |
| CSV/TXT multi-site preparation | Detects comma, tab, semicolon or whitespace long tables, splits them by site and writes standard single-site NetCDF; can also generate site files in bulk |
| Forcing gap repair | Short gaps are interpolated according to each variable's physics. Long gaps are converted to UTC, matched to the nearest ERA5-Land cell and bias-corrected against overlapping observations, with an hourly QC trail |
| Rust model engine | Surface data, initial conditions and the main loop are all in Rust. Coverage includes LCT / PFT / PC, BGC, crops, urban, lakes, ozone, tracers, LULCC, grid river/reservoir routing and catchment lateral flow |
| Spatial simulation | Watershed, regional and 0.1° global extents on lat-lon, unstructured and catchment grids. Regional cases turn on regional unit-catchment routing by default |
| Multi-node | Block-level SPMD for the Rust spatial loop; `srun`/`mpiexec` launch; validated with a 10-day 0.1° global run on 8 tianhe nodes |
| Parallelism | Shared-memory multithreading in one process (patch updates and large river systems run per cell). `RAYON_NUM_THREADS` sets the thread count; the default is every logical core |
| Concurrent sites | Run independent cases at the same time with CPU cores shared automatically; each site shows its own progress, stage and log |
| Staged runs | Run `mksrfdata`, `mkinidata` and `colm` separately, or all stages in one go |
| Results workbench | Data catalog, time series, observation evaluation, multi-site comparison, closure diagnosis and reports, using recorded run provenance |
| Bilingual interface | Sidebar controls for Chinese/English, theme and expert settings, with an optional assistant panel on the right |

The configuration covers IGBP / USGS, LCT / PFT / PC, hydrology and heat, BGC, crops, urban, river and tracer processes. Version overrides retain land-class, PFT and process scopes; materialization and execution check compatibility with the selected case and kernel. Lat-lon grids with river-lake routing (including watershed extents cut from a Shapefile) support reservoir operation, LULCC and tracers; reservoir parameters default to the GRanD table carried by the unit-catchment file `DEF_UnitCatchment_file`. Catchment meshes (`DEF_CatchmentMesh_data`) support reservoir operation, but not yet LULCC, tracers or vector history; such combinations are reported before the run starts, not after preprocessing finishes.
See [`docs/parameter-catalog.md`](docs/parameter-catalog.md), [`docs/parameter-gui-guide.md`](docs/parameter-gui-guide.md) and [`docs/parameter-default-preservation.md`](docs/parameter-default-preservation.md) for the parameter catalog and guides. Upstream defects found and fixed in this repository are listed in [`docs/upstream-bugs.md`](docs/upstream-bugs.md).

## Workflow

These instructions describe the current React source interface. The app opens at Project, where you first choose a model revision. Simulation, project, result, example and recent-project entries are gray and disabled until you choose. A separate tinted model panel sits above “Then choose an action.” The sidebar contains Project, Simulation, Results, Research and History, with Compute resources and Settings at the bottom. AI assistant and language controls are in the window toolbar. There is no extra start page or top workflow bar.

1. **Choose a model, then open or try a case.** Select Official model in the separate model panel or choose an exact frozen revision from Existing versions to unlock the actions below. The choice lasts for this app session, including returning home; restarting requires choosing again. New modified model opens the manager before selection, and selecting or freezing a revision unlocks tasks. Choose a folder under Project, or select New simulation → Examples, then choose a site, name, output folder and period before creating it. Spatial cases select a regional, watershed or global extent and input grid on the same page.
2. **Prepare your data (optional).** Project → Prepare input data supports single-site NetCDF and single-/multi-site CSV, TXT and TSV. The existing UTC normalization, short-gap interpolation and ERA5-Land correction produce standard files without overwriting the original data.
3. **Change the model when needed.** Open Project → Manage versions and derive a draft from a frozen revision. Parameters, Physics, AI network and Code are separate blocks. Save a draft to continue editing; Freeze creates an immutable revision, and later edits produce another revision. Record sites, land classes and calibration/validation periods under applicability. Entering Research from a draft saves and freezes that draft first.
4. **Configure the simulation.** Follow Range and grid → Model structure and schemes → Data and period → Spinup and output → Review and run. New simulations use cards for single-site, watershed, regional and global ranges and spatial grid types. Start and end dates share one row and use `YYYY-MM-DD`, such as `2001-01-01`. Step 2 selects IGBP/USGS classification and groups model structure, parameters and schemes; Adjust parameters and schemes opens version management, and returning after selecting or freezing a revision preserves the pending setup. Routing, BGC and TRACER expose their conditional options, with separate real process-file and indexed PFT/PC editors; unknown applicability remains pending. Step 3 groups input data by process and sets periods. Step 4 provides time steps, spinup cutoff, output and restart. More settings retain initial conditions, forcing processing, diagnostics and kernel-supported output variables. Frozen models remain immutable; existing time-of-day seconds are preserved, and local case edits must be saved or discarded before leaving.
5. **Check resources and run.** Use the top cards in Compute resources to inspect the local machine or a registered server. Edit the selected connection's address, account and SSH config/key/password authentication; expand data directories and maintenance as needed. Check connection shows evidence for this connection, available engine snapshots and mapped paths. Changed settings require another check. Resource selection controls this detail view; execution location is still chosen under Simulation → Check and run. Every run materializes an independent model-revision × case-data copy and records its actual provenance and content identities.
6. **Analyze recorded results.** Results automatically reads the recorded simulation curve. Compare other runs opens the run dialog; Evaluation settings selects observations and updates metrics. The overview shows actual provenance, evidence, the chart with bias/RMSE/valid pairs, then next steps. Catalog, series, observation evaluation, multi-site comparison, closure diagnosis and reports remain in the footer. Plots may be downsampled; metrics use complete data. Parent comparison requires actual results with matching input, period and evaluation conditions; a missing baseline does not trigger another run.
7. **Research and save its products.** Research offers Calibration, Uncertainty, AI parameter learning and AI process replacement. Review sites, observations, ranges, training/validation periods and budget, then preflight, create and run. Save a selected member or trained network as a model version; select that version to run and compare. Networks require Rust, and changes to engines or compile-time switches still require compatible programs and kernels.
8. **Trace operations.** History filters run, study, model, resource and assistant operations. Inspect objects, state, duration, provenance and referenced files; export one record or the full list. Interrupted or unconfirmed submissions require reconciliation with the actual process/server job before retrying.

The results page exports complete series CSV and JSON, CSV, Markdown or HTML reports. PDF uses the system print dialog; choose Save as PDF. A download or print request does not prove that a file was saved.

Open AI assistant from the window toolbar and drag its left edge to resize it. Use Settings to choose an API service or local Codex, Claude Code or OpenCode. Register an API key, discover or enter a model, and configure its service URL and supported reasoning options; protocol and budget overrides are under expert settings. Restart opens a new conversation; saved conversations can be resumed, and ↑/↓ selects previous inputs. Only model and reasoning controls remain below the conversation; service and backend options are in Settings. Example suggestions fill the message first and execute only after sending. Analysis, calibration and AI tasks use the corresponding visible pages and follow operation approval rules.

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

Enter the `Sitedata` and `Forcing` folders on New case and scan the sites. Selecting a site fills its matching forcing and coverage period. Edit existing case data paths under Cases → Settings.

## Running from source

### Requirements

- Rust **1.85.1** or newer
- Git
- Node.js **24** (matching CI) and npm
- Tauri 2 CLI (`cargo tauri`)
- The [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your platform
- A kernel folder for running cases (included in the installers; see the note at the end to build one from source)

### Start the desktop app

```bash
git clone https://github.com/zhongwangwei/CoLM-Desktop.git
cd CoLM-Desktop
npm --prefix gui ci
npm --prefix gui run build
cargo run -p xtask -- stage-sidecar
cd gui
cargo tauri dev
```

`cargo tauri dev` starts the Vite server and native window. Staging builds the CLI, engines and assistant and binds their source/program hashes. Restage after Rust source changes so old binaries are not treated as results from new source. A browser preview has no native model or assistant backend.

To use the built frontend without Vite, run `cargo tauri build --no-bundle` in `gui/`, then start the native program reported by the build (default `src-tauri/target/release/colm-desktop-gui`, with `.exe` on Windows). The configured command rebuilds the frontend; Cargo settings may change the output directory.

The startup log reports the WebView/backend connection, resolved `colm-cli` path and available kernels. Model, case and assistant entry points follow the workbench workflow above.

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
React + TypeScript GUI (Vite)
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
| `gui/src` | React pages, model/case context, charts and assistant page contracts |
| `gui/src-tauri` | Tauri window, file pickers, IPC, batch jobs and event forwarding |
| `crates/colm-model` | Model drafts, immutable revisions, sparse overlays and run copies |
| `crates/colm-history` | SQLite operation ledger, legacy imports and evidence references |
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

# React type, interface and behavior checks
npm --prefix gui ci
npm --prefix gui run lint
npm --prefix gui test
npm --prefix gui run build
cargo run -q -p xtask -- check-gui

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
npm --prefix gui ci
cd gui
cargo tauri build --config src-tauri/tauri.bundle.conf.json
```

The bundle configuration rebuilds the React frontend and sidecars and includes documentation for the packaged source version. Release notes do not substitute for current-source build and remote acceptance checks.

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
