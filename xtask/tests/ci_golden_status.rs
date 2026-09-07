#[test]
fn golden_status_uses_the_job_result_not_data_availability() {
    let workflow = include_str!("../../.github/workflows/ci.yml");
    let job = workflow.split_once("\n  golden-status:\n").unwrap().1;
    assert!(job.contains("needs: [rust, golden]"));
    assert!(job.contains("if: always()"));
    assert!(job.contains("GOLDEN_RESULT: ${{ needs.golden.result }}"));
    assert!(!job.contains("vars.HAS_PLUMBER2"));

    #[cfg(unix)]
    {
        let script = job.split_once("run: |\n").unwrap().1;
        for result in ["success", "failure", "skipped", "cancelled"] {
            let output = std::process::Command::new("bash")
                .args(["-eu", "-c", script])
                .env("GOLDEN_RESULT", result)
                .output()
                .unwrap();
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(
                text.contains("Golden regression passed"),
                result == "success",
                "{text}"
            );
            assert_eq!(text.contains("::warning::"), result != "success", "{text}");
        }
    }
}
