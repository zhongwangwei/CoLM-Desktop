# TRACER three-layer architecture

TRACER is flat by design. A configured row is described by three orthogonal
values, optional species code attaches to that same row, and host components
dispatch through one index-aligned lifecycle table. Adding a species must not
add another top-level tracer hierarchy or another reactive/particle registry.

## Layer 1: descriptor

`MOD_Tracer_Defs` builds one `tracer_info_type` row per configured tracer. Its
classification is the product of three dimensions rather than a subtype tree:

| `family_id` | `state_owner` | Supported reaction shape |
| --- | --- | --- |
| `FAMILY_ISOTOPE` | `STATE_OWNER_GENERIC_WATER` | not applicable (`REACTION_NONE` internally) |
| `FAMILY_SOLUTE` | `STATE_OWNER_GENERIC_WATER` by default; `PROVIDER` for species-owned state | `REACTION_NONE`, `REACTION_FIRST_ORDER`, or `REACTION_PROVIDER` |
| `FAMILY_PARTICLE` | `STATE_OWNER_PROVIDER` | not applicable (`REACTION_NONE` internally) |
| `FAMILY_GAS` | `STATE_OWNER_PROVIDER` | `REACTION_NONE` or `REACTION_PROVIDER` |

The reaction vocabulary is `REACTION_NONE`, `REACTION_FIRST_ORDER`, and
`REACTION_PROVIDER`. The current invariant is stricter than that vocabulary:
first-order decay is available only to a generic-water solute. Isotopes and
particles do not have a generic reaction dimension; their stored `NONE` value
means "not applicable", not "conservative chemistry".

The canonical `DEF_TRACER_TYPES` values are therefore:

```text
isotope, solute, particle, gas
```

`conservative` is accepted as a legacy spelling of `solute`. Legacy `reactive`
rows are accepted only to ease migration; new configurations should name the
physical family and let a positive `reactive_decay_rate` or compiled provider
determine the reaction shape. `DEF_TRACER_TYPES` must contain exactly
`DEF_TRACER_NUM` entries.

### Capabilities are not free booleans

Do not add per-tracer namelist switches named `volatile`, `carrier`,
`reactive`, or `fractionates`:

- **carrier** follows state ownership. Generic isotope/solute state is carried
  by the land-water pools; a provider-owned gas/particle defines its own state
  and transport substrate.
- **volatile** is phase/process behavior. Current generic solutes are retained
  as nonvolatile dissolved material, while gas phase exchange belongs to the
  gas provider. It is not an independent family flag.
- **reactive** is derived from `reaction_mode`. A zero decay rate gives `NONE`,
  a positive generic-solute rate gives `FIRST_ORDER`, and species code may
  declare `PROVIDER`.
- **fractionates** requires an isotope descriptor and registered isotope
  physics in `MOD_Tracer_Isotope_Registry`. The existing
  `DEF_TRACER_USE_FRACTIONATION` is only a global experimental gate; it is not
  a per-species capability declaration. `DEF_TRACER_KINETIC_SCHEME` selects
  one internally consistent gas-diffusion dataset for all isotope processes:
  `MERLIVAT1978` (default) or `CAPPA2003`.

This keeps invalid combinations out of the namelist, such as a fractionating
sediment row or a generic-water CH4 row.

### Fractionation namelist knobs

Aligned with the IsoGSM reference model (`gsml/ISOTOPE/`, `gsml/CLD1/lrgscl.F`,
`gsml/moninp.F`). The equilibrium coefficients themselves (Majoube 1971,
Merlivat & Nief 1967) already matched IsoGSM exactly; these control the
kinetic and transport terms.

| Knob | Default | What it controls |
|---|---|---|
| `DEF_TRACER_KINETIC_SCHEME` | `MERLIVAT1978` | Air diffusivity pair. `MERLIVAT1978` reproduces the IsoGSM experiment; `CAPPA2003` is an alternative laboratory-based sensitivity choice. The selected pair is used consistently by every CoLM isotope process, but it does not have to match the atmospheric forcing producer as a physical identity. |
| `DEF_TRACER_ICE_SUPERSAT_SLOPE` | `0.003` | Jouzel & Merlivat (1984) supersaturation kinetics for vapour→ice deposition, `S = 1 - slope*T[C]`. 0 recovers pure equilibrium deposition (which over-enriches sub-freezing frost). |
| `DEF_TRACER_CG_RELHUM_MAX` | `0.99` | Humidity cap in the Craig-Gordon `1/(1-h)` factor. A numerical guard, not physics: every 0.01 shaved off truncates real depletion (the former 0.95 cost ~176 permil in the humid regime). |
| `DEF_TRACER_OPEN_WATER_KINETIC` | `MJ79` | Lake / water-body evaporation. `MJ79` is the wind-dependent Merlivat & Jouzel (1979) law (~6 permil at low wind); `EXPONENT` restores the n=2/3 pore-diffusion value (~19 permil), which overstates open water threefold. |
| `DEF_TRACER_SUBL_SKIN_MM` | `5.0` | Exchanging surface-skin mass for sublimation. A whole snow layer is not a well-mixed reservoir. Very large recovers layer-mixed behaviour; 0 recovers IsoGSM's non-fractionating land sublimation. |
| `DEF_TRACER_SOIL_KINETIC` | `RESISTANCE` | Soil evaporation kinetics. `RESISTANCE` weights the turbulent (n=2/3) and pore-diffusion (n=1) exponents by `ra` and `rss`, so the effect grows from ~19 to ~28.5 permil as the front retreats into the pores. `EXPONENT` pins it at the wet value. |
| `DEF_TRACER_SOIL_DIFFUSION` | `.true.` | Liquid-phase molecular diffusion between soil layers (Millington-Quirk tortuosity). Internal exchange, exactly conserving. |
| `DEF_TRACER_SOIL_VAPOR_DIFFUSION` | `.true.` | Also gates **firn** vapour diffusion between snow layers, which acts on the *ice* inventory with `alpha_ice_vap` and saturation over ice (~1.6e-11 m2/s at 300 kg/m3 and -10 C, inside the Johnsen et al. 2000 range). The implied timescale is years at seasonal layer thickness, so it matters for multi-year packs far more than for snow that melts out annually. Vapour-phase diffusion through soil pores, folded into the same diffusivity as an equivalent liquid term. Its moisture dependence is the *opposite* of the liquid term (air-filled porosity), so it dominates exactly where the liquid film shuts down: ~30x the liquid term at `theta=0.05`, ~0.5% at `theta=0.40`. Both terms together are what produce the Barnes & Allison (1983) evaporation-front profile. |
| `DEF_TRACER_SNOWMELT_EQUILIBRATION` | `0.0` | Degree of exchange, per snow layer traversed, between percolating meltwater and that layer's ice, relaxing toward `R_ice/alpha_ice_liq`. Since `alpha_ice_liq > 1` the meltwater leaves lighter than the pack, reproducing the observed early-melt depletion and residual-pack enrichment (Taylor et al. 2001) — several permil, and clearly present in observations, so enabling it is usually the more realistic choice. Ships at 0 only because the per-layer degree is empirical; values near 1 suit thin layers. Internal ice↔water exchange, exactly conserving. |
| `DEF_TRACER_CANOPY_EQUILIBRATION` | `0.0` | Per-step degree of two-way equilibrium exchange between intercepted liquid water and ambient vapour (zero net water flux, non-zero net tracer flux; booked into `a_trc_vapor_exchange`). Ships off: unlike the others there is no reference land-surface implementation to calibrate the degree against. |

Runtime forcing diagnostics report the number of fractionating-tracer patches
that fall back from invalid or near-dry vapour isotope forcing. Near-zero
specific humidity can legitimately make a normalized isotope ratio undefined;
the warning exists so missing moist-air isotope data cannot silently look like
valid forcing.

Freezing currently has two explicitly different host diagnostics. THERMAL/WATER
phase changes provide a pool temperature and use equilibrium Rayleigh
fractionation. Interception reports only an aggregate rain-to-snow transfer
across PFTs, without a temperature for that event, so `tracer_precip` moves its
tracer conservatively. Do not replace that transfer with an arbitrary patch
temperature; unify it only when the host exposes a temperature-weighted
freeze-event diagnostic.

## Layer 2: provider and lifecycle table

`MOD_Tracer_Lifecycle` allocates exactly one lifecycle row per descriptor. At
`tracer_lifecycle_init`, `register_all_tracer_providers` reads the single
compiled-provider manifest:

```text
include/tracer_lifecycle_providers.inc
```

A species registrar constructs `tracer_lifecycle_hooks_type` and calls
`register_tracer_provider`. Registration resolves the configured name/aliases
once, writes the declared family/owner/reaction shape into that descriptor,
and attaches hooks at the same tracer index. Runtime dispatch does not rescan
species names and there are no separate reactive and particle callback tables.
After validation, the master process prints one startup row per tracer with its
resolved family, state owner, reaction mode, provider, and active hook groups.
This makes namelist/provider mismatches visible without reintroducing a second
runtime registry.

Only species that own state, kinetics, or host-phase callbacks need a lifecycle
provider. Parameter-only isotope and solute rows do not belong in the provider
manifest.

### Current providers

- **CH4** is registered in code by
  `MOD_Tracer_Reactive_Methane:ch4_register_tracer_provider` as
  `GAS / PROVIDER / PROVIDER`. The manifest includes it when `BGC` is compiled.
  The main namelist selects CH4 and maps its parameter file; it does not
  register the module.
- **SEDIMENT** is registered by
  `MOD_Tracer_Particle_Sediment:register_sediment_tracer_provider` as
  `PARTICLE / PROVIDER / NONE` and owns the routing hooks and sediment state.
  Sediment is a concrete particle species, not a synonym for the particle
  family.

A future microplastic tracer should be another concrete particle provider with
its own state and hooks. It should not be added as a sediment mode. Add only
the species-owned module/registrar and one manifest entry; the descriptor and
host dispatch layers remain unchanged.

The same rule applies to chemistry. CL and a simple NO3 tracer do not gain
anything from empty species modules: generic transport, optional dissolved
limits, and optional first-order loss are already descriptor-driven. A NO3
provider becomes meaningful only when it owns distinct kinetics or state, for
example coupled nitrification/denitrification pools or species-specific source
and restart logic.

## Layer 3: host dispatch

Host code imports `MOD_Tracer_Lifecycle`, never concrete species modules:

- `MOD_Tracer_LandPhase` calls the `tracer_lifecycle_land_*`, lake, wetland,
  and soil dispatch routines.
- HYDRO calls the `tracer_lifecycle_route_*` routines.
- restart, history, LULCC, and flood publication use the corresponding
  lifecycle entry points.

Each dispatcher iterates the same lifecycle table and calls only associated
hooks. A new provider therefore changes neither land nor HYDRO call sites.

## Configuration examples

### Descriptor-only solute

Chloride needs no compiled CL module:

```fortran
DEF_TRACER_NUM         = 1
DEF_TRACER_NAMES       = 'CL'
DEF_TRACER_TYPES       = 'solute'
DEF_TRACER_PARAM_FILES = 'CL:run/standard_chloride_parameter.nml'
```

A simple NO3 tracer uses the same shape. In its mapped
`&nl_colm_tracer_parameter`, set `DEF_TRACER%reactive_decay_rate = 0.0` for no
reaction or a positive rate for generic first-order loss. Do not add a
`reactive = .true.` switch.

### CH4 gas provider

```fortran
DEF_TRACER_NUM         = 1
DEF_TRACER_NAMES       = 'CH4'
DEF_TRACER_TYPES       = 'gas'
DEF_TRACER_PARAM_FILES = 'CH4:run/standard_ch4_parameter.nml'
```

`run/standard_ch4_parameter.nml` contains both generic tracer metadata and the
CH4-owned `&nl_colm_methane_parameter` controls. Selecting `gas` does not create
a methane model: the compiled BGC provider must be present, and validation
fails otherwise. A configured CH4 tracer is offline with respect to atmospheric
CH4 state, but it is not offline with respect to land BGC: once CH4 is active,
the provider reads BGC carbon inputs and updates methane-specific land state and
flux diagnostics.

### Sediment particle provider

```fortran
DEF_TRACER_NUM         = 1
DEF_TRACER_NAMES       = 'SEDIMENT'
DEF_TRACER_TYPES       = 'particle'
DEF_TRACER_PARAM_FILES = 'SEDIMENT:run/standard_sediment_parameter.nml'
```

The mapped file contains generic descriptor metadata plus
`&nl_colm_sediment_parameter`, which is read by the sediment provider. There
is no separate top-level sediment switch.

Suspended sediment is prognosed as per-size-class solid volume (`sedsto`, m3),
not as concentration times whatever water volume happens to be current.
`sedcon` is derived from that canonical mass and HYDRO storage at the routing
period boundary. Sediment restart schema 1 writes `sedsto`, a complete physics
descriptor, and a transaction-complete marker; nonempty legacy concentration/
bed/accumulator checkpoints are rejected because their physical interpretation
cannot be recovered exactly.

Sediment diagnostics use a bed-increment convention. `netflw < 0` and
`exch_d_eff > 0` include every effective solid-volume addition to the bed:
water-column settling, concentration-cap/dry-carrier deposition, and shallow or
dry hillslope input placed directly on the bed. The last path is an **external
hillslope source**, not suspended-water settling; it is included so the reported
effective bed increment closes against the actual layer change.

## Migration and lifecycle order

For an older configuration:

1. Replace `conservative` with `solute`.
2. Replace generic `reactive` with `solute` plus a decay rate, or with the real
   provider-owned family when species code exists.
3. Declare sediment as `particle`; do not restore a sediment-specific registry.
4. Remove per-species transport/capability booleans and let descriptor
   derivation plus provider validation reject unsupported combinations.

Initialization order is:

```text
tracer_defs_init -> tracer_lifecycle_init -> land/route provider init
```

Shutdown must keep the table and descriptors alive until every provider has
finished:

```text
land_tracer_final
-> grid_riverlake_flow_final (including tracer_lifecycle_route_final)
-> tracer_lifecycle_reset
-> tracer_defs_final
```

`tracer_lifecycle_reset` owns only the lifecycle hooks/provider names;
`tracer_defs_final` owns the descriptor table and therefore remains last.

## Current physics boundaries

- The CH4 provider is offline only with respect to atmospheric CH4 feedback. It
  predicts soil, wetland, rice-paddy, and reduced-order lake-sediment CH4 fluxes
  without updating an atmospheric CH4 state, so `DEF_METHANE%methane_offline`
  must remain true. When a CH4 gas tracer is configured, the provider is coupled
  to land BGC carbon inputs and methane-owned land state.
- `allowlakeprod` enables the reduced-order lake-sediment pathway together with
  a prognostic, well-mixed lake-water CH4/O2 box and piston-velocity air-water
  exchange. It is not a vertically resolved lake-water column or an online
  atmospheric coupling.
- A positive solute `max_dissolved_conc` transfers excess land-pool mass to the
  existing immobile solid inventory; it is a safety ceiling, not a
  thermodynamic salt or electroneutrality model.
- Isotope fractionation algorithms remain in `MOD_Tracer_Frac`; concrete O18
  and HDO physics are registered through the isotope registry, independently
  of lifecycle providers. Leaf NSS includes the host model's aerodynamic
  moisture resistance between forcing height and the canopy, in addition to
  leaf-boundary and stomatal resistance.
