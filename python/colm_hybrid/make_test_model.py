"""生成 colm-hybrid 单元测试用的极小 ONNX 模型。

    python python/colm_hybrid/make_test_model.py crates/colm-hybrid/tests/data/linear.onnx

模型：y = x @ W + b，x 为 [N, 2]（N 可变），y 为 [N, 1]，W = [[2], [-3]]，b = [0.5]。
权重都是 float32 能精确表示的数，测试里可以按位比较。
"""

import sys

import numpy as np
import onnx
from onnx import TensorProto, helper, numpy_helper


def main(path: str) -> None:
    weight = numpy_helper.from_array(np.array([[2.0], [-3.0]], dtype=np.float32), "W")
    bias = numpy_helper.from_array(np.array([0.5], dtype=np.float32), "b")
    graph = helper.make_graph(
        [
            helper.make_node("MatMul", ["x", "W"], ["xw"]),
            helper.make_node("Add", ["xw", "b"], ["y"]),
        ],
        "colm_hybrid_linear",
        [helper.make_tensor_value_info("x", TensorProto.FLOAT, ["N", 2])],
        [helper.make_tensor_value_info("y", TensorProto.FLOAT, ["N", 1])],
        initializer=[weight, bias],
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 13)])
    model.ir_version = 8
    onnx.checker.check_model(model)
    onnx.save(model, path)


if __name__ == "__main__":
    main(sys.argv[1])
