use super::*;

#[test]
fn test_strip_comment() {
    assert_eq!(strip_comment("cd src  # navigate"), "cd src  ");
    assert_eq!(
        strip_comment("body [foo{a,b}] # comment"),
        "body [foo{a,b}] "
    );
    assert_eq!(strip_comment("no comment"), "no comment");
}

#[test]
fn test_expand_braces_simple() {
    let mut r = expand_braces("tools/{mod.rs x.rs}");
    r.sort();
    assert_eq!(r, vec!["tools/mod.rs", "tools/x.rs"]);
}

#[test]
fn test_expand_braces_empty() {
    assert_eq!(expand_braces(".{}"), vec!["."]);
}

#[test]
fn test_expand_braces_no_brace() {
    assert_eq!(expand_braces("main.rs"), vec!["main.rs"]);
}

#[test]
fn test_parse_item_list() {
    let items = parse_item_list("[main, tools/{mod.rs x.rs}]");
    assert_eq!(items, vec!["main", "tools/mod.rs", "tools/x.rs"]);
}

#[test]
fn test_parse_item_list_bare() {
    let items = parse_item_list("a b c");
    assert_eq!(items, vec!["a", "b", "c"]);
}

#[test]
fn test_unquote() {
    assert_eq!(unquote("\"hello world\""), "hello world");
    assert_eq!(unquote("'single'"), "single");
    assert_eq!(unquote("bare"), "bare");
    assert_eq!(unquote(r#""escaped \" quote""#), "escaped \" quote");
    assert_eq!(unquote(r#""backslash \\""#), "backslash \\");
}

#[test]
fn test_strip_comment_with_quotes() {
    // # inside quotes should not be treated as comment
    assert_eq!(strip_comment(r#"grep "foo # bar""#), r#"grep "foo # bar""#);
    assert_eq!(strip_comment("grep 'pattern' # comment"), "grep 'pattern' ");
}

#[test]
fn test_split_words_with_quotes() {
    let words = split_words(r#"a "b c" d"#);
    assert_eq!(words, vec!["a", "\"b c\"", "d"]);
}

#[test]
fn test_parse_item_list_quoted() {
    let items = parse_item_list(r#"["hello world" simple]"#);
    assert_eq!(items, vec!["hello world", "simple"]);
}
