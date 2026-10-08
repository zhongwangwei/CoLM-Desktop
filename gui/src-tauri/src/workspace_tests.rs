use super::*;

#[test]
fn workspace_names_are_checked_before_they_reach_the_command_line() {
    for ok in ["emis", "fix-1", "a_b", "A1"] {
        assert!(valid_name(ok).is_ok(), "{ok}");
    }
    for bad in ["", "../x", "a/b", "-x", "a b", "a;b", &"x".repeat(41)] {
        assert!(valid_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_command_line_is_the_command_then_the_arguments() {
    assert_eq!(args("ws-list", &[]), ["ws-list"]);
    assert_eq!(
        args("ws-delete", &["--name", "emis"]),
        ["ws-delete", "--name", "emis"]
    );
}
