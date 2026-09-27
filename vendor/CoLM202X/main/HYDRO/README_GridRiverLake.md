# GridRiverLakeFlow: namelist notes for long runs

These notes cover land-river water exchange settings for `GridRiverLakeFlow`
builds. They are based on a 3-year regional test (36-44N, 96-84W, 0.25 deg,
15 min river network, 2017-2019).

## Two-way coupling and runoff scheme

- `DEF_GridRiverLake_FloodFeedback = .true.` (or `DEF_CaMa_FloodFeedback`)
  sets `DEF_Runoff_SCHEME = 0` (TOPMODEL with baseflow) in mksrfdata,
  mkinidata and colm. The default scheme 3 (simplified VIC) has no baseflow
  (`rsubst = 0`): with feedback the water table rises to the surface and
  re-infiltrated floodwater leaves again as surface runoff.
- The runoff scheme changes what mksrfdata and mkinidata produce. Rerun both
  when switching feedback on or off. Scheme-0 preprocessing lacks `BVIC`, so a
  scheme-3 run on it stops in the RangeCheck.

## Floodplain re-infiltration cap

- `DEF_GridRiverLake_FloodInfiltMax` (mm/day, default 5; -1 disables) caps
  the sub-grid flood infiltration when feedback is on. The capped part stays
  as flood surface runoff.
- Without a cap, floodwater infiltrates at the top-soil saturated
  conductivity and returns to the river as baseflow: in the test this
  recirculated 2000-2800 mm/yr through flooded cells. The extra regional
  deficit caused by feedback was -467 (no cap), -151 (20), -54 (5) and
  -24 (1) mm/yr.

## Lakes: use the dynamic lake for water-budget studies

- With the default static lake (`DEF_USE_Dynamic_Lake = .false.`), lake
  storage is fixed: any step surplus leaves as runoff at once, and any
  evaporation shortfall is booked as `lake_deficit` and never repaid. Lake
  runoff therefore exceeds P - ET. In the test, lake cells had
  P - ET - R - dS = -509 mm/yr, offset by `lake_deficit` = +609 mm/yr.
- `DEF_USE_Dynamic_Lake = .true.` stores lake water as `wdsrf` and spills
  only above the lake depth. In the same test the lake-cell residual fell
  to -6 mm/yr, lake runoff from 994 to 523 mm/yr, and lake depths stayed
  within their seasonal range over 3 years; restart continuation was
  bitwise.
- Lake patches receive no river inflow in grid mode (the model prints
  "Dynamic Lake is not well supported without lateral flow"). The test
  region is humid (lake P > E); in dry regions a dynamic lake with E > P
  loses water continuously. Check lake depths in such regions.
- The default stays static to keep mainline results. For long runs whose
  water budget matters, set `DEF_USE_Dynamic_Lake = .true.` explicitly.
