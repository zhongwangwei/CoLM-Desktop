use super::*;

#[test]
fn fetched_results_land_in_a_folder_named_after_comparison_and_server() {
    assert_eq!(
        mirror_name("qfo_era5", "7920ts").unwrap(),
        "qfo_era5@7920ts"
    );
    // 服务器名里的点、@ 之类换掉，不会多出一层目录。
    assert_eq!(
        mirror_name("a", "user@host.example/x").unwrap(),
        "a@user_host_example_x"
    );
    assert!(mirror_name("../a", "h").is_err());
    assert!(mirror_name("a", "").is_err());
}

#[test]
fn only_this_pages_jobs_can_be_watched_or_cancelled() {
    check_job("bench-run-84692-1792749735388362000").unwrap();
    for bad in ["run-123", "bench-../../x", "bench-a b", ""] {
        assert!(check_job(bad).is_err(), "{bad:?}");
    }
    assert!(server_dir("/data/x").is_ok());
    assert!(server_dir("data/x").is_err());
}
