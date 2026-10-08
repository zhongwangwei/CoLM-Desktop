use super::*;

#[test]
fn kernel_listing_is_parsed_and_the_newest_full_one_is_chosen() {
    let text = "@@default-aaaa1111\nfull=1\nmtime=1000\nmanifest={\"preset\":\"default\",\"platform\":\"Linux-x86_64\",\"colm_git_sha\":\"1d41ad09\",\"macros\":[\"LULC_IGBP\",\"SinglePoint\"]}\n\
@@default-bbbb2222\nfull=1\nmtime=2000\nmanifest={\"preset\":\"default\",\"macros\":[]}\n\
@@latlon-cccc3333\nfull=1\nmtime=1500\nmanifest={\"preset\":\"latlon\",\"macros\":[\"GRIDBASED\",\"USEMPI\"]}\n\
@@default-dddd4444\nfull=0\nmtime=3000\nmanifest={\"preset\":\"default\",\"macros\":[]}\n\
@@broken\nfull=1\nmtime=5\nmanifest=not json\n";
    let kernels = parse_list("/data/colm/", text);
    assert_eq!(kernels.len(), 5);
    let first = kernels
        .iter()
        .find(|k| k.name == "default-aaaa1111")
        .unwrap();
    assert_eq!(first.dir, "/data/colm/kernels/default-aaaa1111");
    assert_eq!(first.colm_git_sha, "1d41ad09");
    assert_eq!(first.macros, ["LULC_IGBP", "SinglePoint"]);
    assert!(first.full);
    // 只有清单的目录（full=0）不会被选；坏清单的预设为空，也不会被选。
    assert_eq!(
        newest_full(&kernels, "default").unwrap().name,
        "default-bbbb2222"
    );
    assert_eq!(
        newest_full(&kernels, "latlon").unwrap().name,
        "latlon-cccc3333"
    );
    assert!(newest_full(&kernels, "crop").is_none());
    assert!(parse_list("/r", "").is_empty());
    let script = list_script("/data/colm");
    assert!(script.contains("cd '/data/colm/kernels'") && script.contains("-x \"$d/colm.x\""));
}
