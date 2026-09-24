//! `MOD_AssimStomataConductance:sortin` 的差分探针（配对物
//! `oracle/scripts/sortin_diff.f90`）。
//!
//! 第 330–332 轮把 `stomata` 闭环从 498 推到 61，剩下的全在模型 0/1 的 `assim`，
//! 而它们唯一的共同路径就是这个 `sortin`。Rust 侧通过 `sortin_for_probe`（`#[doc(hidden)]`）
//! 调用私有的 `sortin`。
//!
//! 第 335 轮起再导出一份**二次拟合分支的中间量**（`sortin_intermediates_for_probe`）：
//! 同一份**调用前**输入、同一个 `sortin_impl` 跑两遍（一份取中间量、一份取输出），
//! 两遍不会分叉。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐。输出：`ic` 后 21 列十六进制
//! —— `errors[0..6]`、`guesses[0..6]`，再 `ac1,ac2,bc1,bc2,cc1,cc2,bterm,aterm,cterm`。
use colm_core::sortin_intermediates_for_probe;

struct Lcg(u64);
impl Lcg {
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / 9007199254740992.0
    }
}

fn main() {
    let mut s = Lcg(20260924);
    let mut out = String::new();
    for _ in 1..=3000u32 {
        let ic = 1 + (s.uni() * 6.0) as usize;
        let range = (s.uni() - 0.3) * 200.0;
        let gammas = 10.0 + s.uni() * 80.0;
        let mut errors = [0.0f64; 6];
        let mut guesses = [0.0f64; 6];
        for j in 0..6 {
            errors[j] = (s.uni() - 0.5) * 200.0;
            guesses[j] = gammas + s.uni() * range;
        }
        // 一次调用同时拿到**更新后的数组**和中间量（`sortin_impl` 只跑一遍）。
        let debug = sortin_intermediates_for_probe(&mut errors, &mut guesses, range, gammas, ic);
        out.push_str(&format!("{ic}"));
        for j in 0..6 {
            out.push_str(&format!(
                " {:016X} {:016X}",
                errors[j].to_bits(),
                guesses[j].to_bits()
            ));
        }
        for value in &debug {
            out.push_str(&format!(" {:016X}", value.to_bits()));
        }
        out.push('\n');
    }
    print!("{out}");
}
