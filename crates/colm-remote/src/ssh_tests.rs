use super::*;

#[test]
fn hosts_are_checked_and_strings_quoted_for_the_remote_shell() {
    assert_eq!(Ssh::new(" 7920land ").unwrap().host, "7920land");
    assert!(Ssh::new("zhwei@172.16.100.17").is_ok());
    for bad in ["", "-oProxyCommand=evil", "a b"] {
        assert!(Ssh::new(bad).is_err(), "{bad}");
    }
    assert_eq!(quote("/media/data02/x y"), "'/media/data02/x y'");
    assert_eq!(quote("it's"), r"'it'\''s'");
}

#[test]
fn connection_failures_say_what_to_do() {
    let output = |code, stderr: &str| Output {
        success: false,
        code: Some(code),
        stdout: String::new(),
        stderr: stderr.into(),
    };
    let unreachable = connection_hint(
        "7920l",
        &output(
            255,
            "kex_exchange_identification: Connection closed by remote host",
        ),
    );
    assert!(
        unreachable.contains("cannot connect") && unreachable.contains("7920l"),
        "{unreachable}"
    );
    let password = connection_hint(
        "tianhe",
        &output(255, "Permission denied (publickey,keyboard-interactive)."),
    );
    assert!(
        password.contains("log in once in a terminal (ssh tianhe)"),
        "{password}"
    );
    let hostkey = connection_hint("x", &output(255, "Host key verification failed."));
    assert!(hostkey.contains("host key"), "{hostkey}");
    assert!(connection_hint("x", &output(1, "boom")).starts_with("the remote command failed"));
}
