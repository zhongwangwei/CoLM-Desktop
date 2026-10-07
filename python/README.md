# python/

混合模型（AI + 物理）的训练侧代码。设计见 `docs/design-hybrid.md`。

- `colm_hybrid/make_test_model.py`：生成 `crates/colm-hybrid/tests/data/linear.onnx`（单元测试用的线性模型）。需要 `onnx` 与 `numpy`，建议放在独立的 venv 里装。

训练数据从引擎抓取，不改参数、结果与纯物理逐位相同：

    colm-rs <case>/case.nml --land-cover igbp --case-outputs --hybrid <case>/hybrid.toml --hybrid-tap tap.csv

`hybrid.toml` 在抓取时可以不写 `model`/`sha256`。
