//! `MOD_AssimStomataConductance:sortin` 的差分探针（配对物
//! `oracle/scripts/sortin_diff.f90`）。
//!
//! 第 330–332 轮把 `stomata` 闭环从 498 推到 61，剩下的全在模型 0/1 的 `assim`，
//! 而它们唯一的共同路径就是这个 `sortin`（`stomata` 里没有 FMA 的函数）。
//! Rust 侧通过 `sortin_for_probe`（`#[doc(hidden)]`）调用私有的 `sortin`。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐。输出：`ic` 后 12 列十六进制
//! —— `errors[0..6]` 与 `guesses[0..6]`。
use colm_core::sortin_for_probe;

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
        sortin_for_probe(&mut errors, &mut guesses, range, gammas, ic);
        out.push_str(&format!("{ic}"));
        for j in 0..6 {
            out.push_str(&format!(
                " {:016X} {:016X}",
                errors[j].to_bits(),
                guesses[j].to_bits()
            ));
        }
        out.push('\n');
    }
    print!("{out}");
}
