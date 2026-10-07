use std::path::{Path, PathBuf};

use super::*;

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-hybrid-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const LINEAR_SHA256: &str = "cd50301fbac066c77decd7397f922ec1b91602879a56f599a3acf3bb7997955a";

/// 写一份引用 `linear.onnx` 的配置（模型拷到配置旁边，验证相对路径解析）。
fn write_config(dir: &Path, extra: &str) -> PathBuf {
    std::fs::copy(data("linear.onnx"), dir.join("linear.onnx")).unwrap();
    let path = dir.join("hybrid.toml");
    std::fs::write(
        &path,
        format!(
            r#"
[[slot]]
name = "land_class"
kind = "param"
model = "linear.onnx"
sha256 = "{LINEAR_SHA256}"
features = ["a", "b"]
{extra}
outputs = [{{ name = "DEF_LC_VMAX25" }}]
"#
        ),
    )
    .unwrap();
    path
}

#[test]
#[cfg(feature = "inference")]
fn the_onnx_linear_model_runs_on_any_batch_size() {
    let dir = scratch("onnx");
    let config = HybridConfig::load(&write_config(&dir, "")).unwrap();
    let slot = Slot::load(config.slots[0].clone()).unwrap();
    // y = 2a - 3b + 0.5，都能用 f32 精确表示。
    for rows in [1usize, 3, 17] {
        let data: Vec<f64> = (0..rows).flat_map(|i| [i as f64, 1.0]).collect();
        let out = slot.evaluate(&Matrix::new(rows, 2, data).unwrap()).unwrap();
        assert_eq!(out.rows, rows);
        let expected: Vec<f64> = (0..rows).map(|i| 2.0 * i as f64 - 3.0 + 0.5).collect();
        assert_eq!(out.data, expected);
    }
    // 同一输入两次给出同一位型。
    let input = Matrix::new(2, 2, vec![0.1, 0.2, 0.3, 0.4]).unwrap();
    let a = slot.evaluate(&input).unwrap();
    let b = slot.evaluate(&input).unwrap();
    assert_eq!(
        a.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
#[cfg(feature = "inference")]
fn normalization_and_transforms_apply_in_order() {
    let dir = scratch("normalize");
    std::fs::write(
        dir.join("norm.json"),
        r#"{"mean": [1.0, 0.0], "std": [2.0, 1.0]}"#,
    )
    .unwrap();
    let config = HybridConfig::load(&write_config(&dir, r#"normalize = "norm.json""#)).unwrap();
    let slot = Slot::load(config.slots[0].clone()).unwrap();
    // a = 5 → (5-1)/2 = 2；b = 1 → 1；y = 4 - 3 + 0.5 = 1.5。
    let out = slot
        .evaluate(&Matrix::new(1, 2, vec![5.0, 1.0]).unwrap())
        .unwrap();
    assert_eq!(out.data, [1.5]);

    let spec = |transform, range| OutputSpec {
        name: "x".into(),
        range,
        transform,
        relative: false,
    };
    assert_eq!(
        apply_output(&spec(Transform::Sigmoid, Some([10.0, 20.0])), 0.0).unwrap(),
        15.0
    );
    assert_eq!(
        apply_output(&spec(Transform::Clamp, Some([0.0, 1.0])), 3.0).unwrap(),
        1.0
    );
    assert_eq!(
        apply_output(&spec(Transform::Softplus, Some([2.0, 100.0])), 0.0).unwrap(),
        2.0 + 2f64.ln()
    );
    assert!(apply_output(&spec(Transform::Identity, Some([0.0, 1.0])), 3.0).is_err());
    assert!(apply_output(&spec(Transform::Identity, None), f64::NAN).is_err());
}

#[test]
fn a_wrong_hash_or_bad_config_is_rejected() {
    let dir = scratch("reject");
    let path = write_config(&dir, "");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(LINEAR_SHA256, &"0".repeat(64));
    std::fs::write(&path, text).unwrap();
    let error = HybridConfig::load(&path).unwrap_err().to_string();
    assert!(error.contains("sha256"), "{error}");

    let path = write_config(&scratch("reject-range"), "");
    let text = std::fs::read_to_string(&path).unwrap().replace(
        r#"{ name = "DEF_LC_VMAX25" }"#,
        r#"{ name = "DEF_LC_VMAX25", transform = "sigmoid" }"#,
    );
    std::fs::write(&path, text).unwrap();
    assert!(HybridConfig::load(&path).is_err());

    let path = write_config(&scratch("reject-field"), "unknown = 1");
    assert!(HybridConfig::load(&path).is_err());

    // 没有模型：正式加载拒绝，特征抓取可以。
    let dir = scratch("spec-only");
    let path = dir.join("hybrid.toml");
    std::fs::write(
        &path,
        r#"
[[slot]]
name = "land_class"
kind = "param"
features = ["a"]
outputs = [{ name = "DEF_LC_VMAX25" }]
"#,
    )
    .unwrap();
    assert!(HybridConfig::load(&path).is_err());
    assert_eq!(HybridConfig::load_spec(&path).unwrap().slots[0].model, None);
}

#[test]
fn the_fingerprint_follows_the_model_bytes() {
    let dir = scratch("fingerprint");
    let path = write_config(&dir, "");
    let first = HybridConfig::load(&path).unwrap().fingerprint;
    assert_eq!(first, HybridConfig::load(&path).unwrap().fingerprint);
    // 改配置文本（加注释）也算变化。
    let text = std::fs::read_to_string(&path).unwrap() + "\n# note\n";
    std::fs::write(&path, text).unwrap();
    assert_ne!(first, HybridConfig::load(&path).unwrap().fingerprint);
}

#[test]
fn a_function_backend_passes_physics_values_through_exactly() {
    let dir = scratch("echo");
    let config = HybridConfig::load(&write_config(&dir, "")).unwrap();
    // "模仿物理"：直接给出 f64，不经 f32，identity 变换也不动它。
    let echo = FnBackend(|input: &Matrix| {
        Matrix::new(
            input.rows,
            1,
            (0..input.rows).map(|row| input.row(row)[0] / 3.0).collect(),
        )
    });
    let slot = Slot::with_backend(config.slots[0].clone(), Arc::new(echo)).unwrap();
    let out = slot
        .evaluate(&Matrix::new(2, 2, vec![1.0, 0.0, 7.0, 0.0]).unwrap())
        .unwrap();
    assert_eq!(out.data, [1.0 / 3.0, 7.0 / 3.0]);
}

/// 原生 MLP：与等价的 ONNX 给出同样的结果（权重都是 f32 精确的数，比较位型），格式错误会被拒绝。
#[test]
#[cfg(feature = "inference")]
fn a_native_mlp_matches_the_equivalent_onnx_model() {
    let dir = scratch("mlp");
    // linear.onnx：y = 2a - 3b + 0.5。
    let mlp = Mlp::new(vec![Layer {
        weights: vec![vec![2.0], vec![-3.0]],
        bias: vec![0.5],
        activation: Activation::Identity,
    }])
    .unwrap();
    mlp.save(&dir.join("linear.mlp.json")).unwrap();
    let path = dir.join("hybrid.toml");
    std::fs::write(
        &path,
        format!(
            "[[slot]]\nname = \"land_class\"\nkind = \"param\"\nmodel = \"linear.mlp.json\"\nsha256 = \"{}\"\nfeatures = [\"a\", \"b\"]\noutputs = [{{ name = \"DEF_LC_VMAX25\" }}]\n",
            sha256_hex(&std::fs::read(dir.join("linear.mlp.json")).unwrap())
        ),
    )
    .unwrap();
    let native = Slot::load(HybridConfig::load(&path).unwrap().slots[0].clone()).unwrap();
    let onnx = Slot::load(
        HybridConfig::load(&write_config(&scratch("mlp-onnx"), ""))
            .unwrap()
            .slots[0]
            .clone(),
    )
    .unwrap();
    let input = Matrix::new(3, 2, vec![0.0, 1.0, 4.0, 2.0, -1.5, 0.25]).unwrap();
    assert_eq!(
        native.evaluate(&input).unwrap(),
        onnx.evaluate(&input).unwrap()
    );
}

#[test]
fn native_mlps_validate_their_shapes_and_run_hidden_layers() {
    let two_layer = Mlp::new(vec![
        Layer {
            weights: vec![vec![1.0, -1.0], vec![0.5, 0.5]],
            bias: vec![0.0, 0.1],
            activation: Activation::Tanh,
        },
        Layer {
            weights: vec![vec![2.0], vec![3.0]],
            bias: vec![-1.0],
            activation: Activation::Identity,
        },
    ])
    .unwrap();
    assert_eq!((two_layer.inputs(), two_layer.outputs()), (2, 1));
    assert_eq!(two_layer.parameter_count(), 6 + 3);
    let y = two_layer
        .infer(&Matrix::new(1, 2, vec![0.4, 0.2]).unwrap())
        .unwrap();
    let h = [(0.4f64 + 0.1).tanh(), (-0.4f64 + 0.1 + 0.1).tanh()];
    assert_eq!(y.data, [-1.0 + 2.0 * h[0] + 3.0 * h[1]]);
    // 层与层对不上、权重行长度不对、格式名不对。
    assert!(Mlp::new(vec![
        Layer {
            weights: vec![vec![1.0, 2.0]],
            bias: vec![0.0, 0.0],
            activation: Activation::Relu,
        },
        Layer {
            weights: vec![vec![1.0]],
            bias: vec![0.0],
            activation: Activation::Identity,
        },
    ])
    .is_err());
    assert!(Mlp::new(vec![Layer {
        weights: vec![vec![1.0], vec![1.0, 2.0]],
        bias: vec![0.0],
        activation: Activation::Identity,
    }])
    .is_err());
    let dir = scratch("mlp-format");
    std::fs::write(
        dir.join("x.mlp.json"),
        r#"{"format": "other", "layers": []}"#,
    )
    .unwrap();
    assert!(Mlp::load(&dir.join("x.mlp.json")).is_err());
}
