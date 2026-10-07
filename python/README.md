# python/

混合模型（AI + 物理）的训练侧代码。设计见 `docs/design-hybrid.md`。

- `colm_hybrid/make_test_model.py`：生成 `crates/colm-hybrid/tests/data/linear.onnx`（单元测试用的线性模型）。需要 `onnx` 与 `numpy`，建议放在独立的 venv 里装。

训练数据从引擎抓取，不改参数、结果与纯物理逐位相同：

    colm-rs <case>/case.nml --land-cover igbp --case-outputs --hybrid <case>/hybrid.toml --hybrid-tap tap.csv

`hybrid.toml` 在抓取时可以不写 `model`/`sha256`。

## 试点训练（`colm_hybrid.pilot`）

`land_class` 插槽：PLUMBER2 站点上学"静态特征 → Vcmax（`DEF_LC_VMAX25`）"，在留出站点上与纯物理比较。

    python -m venv .venv && .venv/bin/pip install -r requirements.txt
    cd python
    ../.venv/bin/python -m colm_hybrid.pilot prepare  --sites sites.json --work WORK --cli colm-cli --kernel KERNEL
    ../.venv/bin/python -m colm_hybrid.pilot train    --work WORK --cli colm-cli --kernel KERNEL --jobs 12
    ../.venv/bin/python -m colm_hybrid.pilot evaluate --work WORK --cli colm-cli --kernel KERNEL

- 每个站点两年：第一年 spin-up，第二年写 history 并评价。
- 训练是 CMA-ES 无梯度优化：每个候选导出 ONNX、写进各算例的 `hybrid.toml`，只重跑 colm 段（前处理靠指纹跳过）。
- 损失是训练站点上 `1 − KGE(Qle)` 的平均。
- 特征的读法与引擎一致（`features.py`）。
