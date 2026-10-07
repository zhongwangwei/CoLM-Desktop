"""把小网络导出成 colm-hybrid 能加载的 ONNX。

输入 x 为 [N, F] float32（N 可变），输出 y 为 [N, O] float32。隐藏层用 tanh。
`layers` 是 [(W, b), ...]，W 为 [in, out]，最后一层不加激活。只有一层时就是线性模型。
"""

import numpy as np
import onnx
from onnx import TensorProto, helper, numpy_helper


def export_mlp(layers, path: str) -> None:
    nodes, initializers = [], []
    current = "x"
    for index, (weight, bias) in enumerate(layers):
        weight = np.asarray(weight, dtype=np.float32)
        bias = np.asarray(bias, dtype=np.float32).reshape(-1)
        w_name, b_name = f"W{index}", f"b{index}"
        initializers += [numpy_helper.from_array(weight, w_name), numpy_helper.from_array(bias, b_name)]
        nodes.append(helper.make_node("MatMul", [current, w_name], [f"m{index}"]))
        last = index == len(layers) - 1
        added = "y" if last else f"a{index}"
        nodes.append(helper.make_node("Add", [f"m{index}", b_name], [added]))
        if not last:
            nodes.append(helper.make_node("Tanh", [added], [f"h{index}"]))
            current = f"h{index}"
    inputs = np.asarray(layers[0][0]).shape[0]
    outputs = np.asarray(layers[-1][0]).shape[1]
    graph = helper.make_graph(
        nodes,
        "colm_hybrid_mlp",
        [helper.make_tensor_value_info("x", TensorProto.FLOAT, ["N", inputs])],
        [helper.make_tensor_value_info("y", TensorProto.FLOAT, ["N", outputs])],
        initializer=initializers,
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 13)])
    model.ir_version = 8
    onnx.checker.check_model(model)
    onnx.save(model, path)


def forward(layers, x):
    """与 ONNX 图相同的前向（float32），用于自检。"""
    h = np.asarray(x, dtype=np.float32)
    for index, (weight, bias) in enumerate(layers):
        h = h @ np.asarray(weight, dtype=np.float32) + np.asarray(bias, dtype=np.float32)
        if index < len(layers) - 1:
            h = np.tanh(h)
    return h


def export_mlp_json(layers, path: str, activation: str = "tanh") -> None:
    """同样的网络写成 colm-hybrid 的原生格式（`*.mlp.json`，引擎用 f64 计算，不需要 ONNX）。

    隐藏层用 `activation`（tanh/relu/identity），最后一层是 identity。
    """
    import json

    out = []
    for index, (weight, bias) in enumerate(layers):
        last = index == len(layers) - 1
        out.append({
            "weights": np.asarray(weight, dtype=np.float64).tolist(),
            "bias": np.asarray(bias, dtype=np.float64).reshape(-1).tolist(),
            "activation": "identity" if last else activation,
        })
    with open(path, "w") as f:
        json.dump({"format": "colm-mlp-1", "layers": out}, f, indent=1)
