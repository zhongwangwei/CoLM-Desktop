# Extended canopy interception schemes

This directory keeps the extended canopy interception parameterizations
(CLM4/CLM5/Noah-MP/MATSIRO/VIC/JULES/CoLM202x) outside the `main/` physics
path.

The current repository default defines `extend_interception` in
`include/define.h`.  The Makefile therefore replaces the four same-name
`main/` modules with:

- `MOD_LeafInterception_Extended.F90`
- `MOD_LeafTemperature_Extended.F90`
- `MOD_LeafTemperaturePC_Extended.F90`
- `MOD_Thermal_CanopyPhase_Extended.F90`

Set `DEF_Interception_scheme = 6` in the runtime namelist to select VIC.  VIC
uses its native vegetation-tile convention (`F=1`); `fsno` is passed separately
for snow-process activation.  `DEF_VEG_SNOW = .false.` is recommended for the
closest match to upstream VIC.  Change the switch back to
`#undef extend_interception` and run a clean rebuild to restore the `main/`
CoLM2014 path.
