use super::*;
use crate::tools::Operation;

#[test]
fn test_cd_and_list_symbols() {
    let parsed = parse_dsl("cd src/debug\nlist_symbols [server.rs]");
    assert_eq!(parsed.operations.len(), 1);
    match &parsed.operations[0] {
        Operation::ListSymbols { file_path, .. } => {
            assert_eq!(file_path, "src/debug/server.rs");
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn test_cd_file_then_list_symbols() {
    let parsed = parse_dsl("cd src/debug/server.rs\nlist_symbols");
    assert_eq!(parsed.operations.len(), 1);
    match &parsed.operations[0] {
        Operation::ListSymbols { file_path, .. } => {
            assert_eq!(file_path, "src/debug/server.rs");
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn test_body() {
    let parsed = parse_dsl("body [relay_command show_help]");
    assert_eq!(parsed.operations.len(), 1);
    match &parsed.operations[0] {
        Operation::Body { symbol_names, .. } => {
            assert_eq!(symbol_names, &["relay_command", "show_help"]);
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn test_body_bare_args_warns() {
    let parsed = parse_dsl("body foo bar baz");
    assert_eq!(parsed.operations.len(), 1);
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].contains("without brackets"));
}

#[test]
fn test_body_path_first_corrects() {
    let parsed = parse_dsl("body src/foo.rs my_fn");
    assert_eq!(parsed.operations.len(), 1);
    assert_eq!(parsed.warnings.len(), 1);
    assert!(
        parsed.warnings[0].contains("src/foo.rs.{my_fn, .}"),
        "expected path.{{sym, .}} format, got: {}",
        parsed.warnings[0]
    );
    match &parsed.operations[0] {
        Operation::Body { symbol_names, .. } => {
            assert_eq!(symbol_names, &["my_fn"]);
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn test_comments_stripped() {
    let parsed = parse_dsl("# this is a comment\nbody [foo] # inline");
    assert_eq!(parsed.operations.len(), 1);
}

#[test]
fn test_brace_expansion_in_list_symbols() {
    let parsed = parse_dsl("cd src\nlist_symbols [tools/{mod.rs symbol_list.rs}]");
    assert_eq!(parsed.operations.len(), 2);
    let paths: Vec<_> = parsed
        .operations
        .iter()
        .filter_map(|op| {
            if let Operation::ListSymbols { file_path, .. } = op {
                Some(file_path.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(paths.contains(&"src/tools/mod.rs"));
    assert!(paths.contains(&"src/tools/symbol_list.rs"));
}

#[test]
fn test_unknown_command_warns() {
    let parsed = parse_dsl("foobar something");
    assert!(parsed.operations.is_empty());
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].contains("unknown command: foobar"));
}
