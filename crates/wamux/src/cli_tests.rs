//! #165: the command line is two words and a flag; anything else is a usage
//! error, and no arguments still means "run the daemon".

use super::{Command, parse, usage};

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn no_arguments_serves() {
    assert_eq!(parse(&[]).unwrap(), Command::Serve);
}

#[test]
fn store_decrypt_yes_parses() {
    assert_eq!(
        parse(&args(&["store", "decrypt", "--yes"])).unwrap(),
        Command::StoreDecrypt { yes: true }
    );
}

#[test]
fn store_decrypt_without_yes_parses_with_yes_false() {
    assert_eq!(
        parse(&args(&["store", "decrypt"])).unwrap(),
        Command::StoreDecrypt { yes: false }
    );
}

#[test]
fn unknown_arguments_are_a_usage_error() {
    for words in [
        vec!["bogus"],
        vec!["store"],
        vec!["store", "rotate"],
        vec!["store", "decrypt", "--yes", "extra"],
        vec!["store", "decrypt", "--no"],
    ] {
        let error = parse(&args(&words)).expect_err(&format!("{words:?}"));
        assert!(
            error.to_string().contains("store decrypt"),
            "{words:?}: {error}"
        );
    }
}

#[test]
fn help_is_recognized() {
    for flag in ["--help", "-h", "help"] {
        assert_eq!(parse(&args(&[flag])).unwrap(), Command::Help, "{flag}");
    }
    assert!(usage().contains("wamux store decrypt --yes"));
}

#[test]
fn store_rotate_key_parses_the_new_key_file() {
    assert_eq!(
        parse(&args(&[
            "store",
            "rotate-key",
            "--new-key-file",
            "/run/secrets/new-key"
        ]))
        .unwrap(),
        Command::StoreRotateKey {
            new_key_file: std::path::PathBuf::from("/run/secrets/new-key")
        }
    );
}

#[test]
fn store_rotate_key_without_a_path_is_a_usage_error() {
    for words in [
        vec!["store", "rotate-key"],
        vec!["store", "rotate-key", "--new-key-file"],
        vec!["store", "rotate-key", "/run/secrets/new-key"],
        vec!["store", "rotate-key", "--new-key-file", "/a", "extra"],
        vec!["store", "rotate-key", "--yes", "/a"],
    ] {
        let error = parse(&args(&words)).expect_err(&format!("{words:?}"));
        // The usage line, not the echo of the words typed.
        assert!(
            error
                .to_string()
                .contains("wamux store rotate-key --new-key-file"),
            "{words:?}: {error}"
        );
    }
}

#[test]
fn usage_names_the_rotation() {
    assert!(usage().contains("wamux store rotate-key --new-key-file"));
}
