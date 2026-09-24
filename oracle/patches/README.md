# 未落地的形状补丁（挂起候选）

这个目录放**已经逐条读出、量过、但按"口径指标不许变差"没能落地**的形状补丁。
它们是 `docs/implementation-verification.md` 里那些"负结果"的可执行形态 ——
单独落任何一处都会让黄金湿窗的 `over_tol`/`ot_vars` 变差，因为湿窗由 `f_vegwp` 混沌主导：
改对末位会换一条轨道，聚合指标可以反向（第 301/351/361/369/371/372 轮）。

**用法**（在 `HEAD` 上）：

```bash
git apply oracle/patches/wet-seed-working-set.diff
cargo fmt --all && cargo test -p colm-core --lib     # 359 passed
oracle/scripts/compare_all.sh                        # 17 通过 / 3 失败（光合那三个仍失败，见下）
bash /tmp/gf/wet_ts.sh 432 && python3 /tmp/gf/rdiff.py   # 0/68
```

## `wet-seed-working-set.diff`

三块，都是 GIMPLE 逐条核对过的：

1. **`photosynthesis.rs` 的 30 处**（`sortin` 11 + `calc_photo_params` 4 + `stomata` 16 +
   `update_photosyn` 8 + `WUE_solver` 1）。补上后三个闭环：
   `compare_sortin` 3000/3000、`compare_update_photosyn` 3000/3000、
   `compare_stomata` 的 `assim` 1/4000（改前分别是 `bterm` 853 / `assim` 225 /
   `rst` 114+`assim` 171）。**这个文件里的改动落地后，那三个闭环会从"一直失败"变成"基本全过"**
   —— 单独落它会让黄金湿窗 1197 → 1968。
2. **`leaf_temperature.rs` 的 `pco2a`**（`MOD_LeafTemperature_Extended.F90:1244` 的 `.FNMA`）。
   它把**湿窗 restart 首分歧从 N=63（10/68）关到 0/68**（N=72 也 0/68），
   干窗黄金位型逐位不变；单独落它会让黄金湿窗 1197 → 1650。
3. **`runoff.rs` 的 `WaterDepthInit + watin`**（`MOD_Runoff.F90:342` 的
   `.FMA(_29, waterdepthmax, watin)`）。这一条**同时进 `ELSEIF ((WaterDepthInit+watin) >
   WaterDepthMax)` 的分支判定**，只有湿季（`watin > 0`）才走到 ⇒ 干窗永远看不到它。
   补上后湿窗 restart 的逐位范围从 N=63 推到 **N=432**；单独落它黄金湿窗 1197 → 1876。

**当前状态**：三块合起来，湿窗 restart 在 **N=1–432 逐位相同**（干窗 N=1–528 逐位相同）；
下一个种子钉在 **N=489**（`emis`/`t_grnd`/`t_soisno` 各 1 ULP），
`vsfr_wet.sh 489` 的内部探针显示**第一条不同的记录又是 `WSF1` 入场的 `qgtop`**
（即再往上游：`qseva`/`fevpg` 那条叶/热链）。

**要落地它的条件**：把湿窗也做到逐位相同（那时口径自然是 0），
或者由维护者明确为湿窗开"口径例外"。**不要**单独落其中任何一块。
