## Lets not add programming specific code into general lsp code e.g. hover.rs:
```
/// Rust keywords/primitives whose long-form docs we want to suppress.
const KEYWORD_DOCS: &[&str] = &[
    "struct", "enum", "trait", "impl", "fn", "mod", "use", "pub", "let", "mut", "const",
    "static", "type", "where", "match", "if", "else", "for", "while", "loop", "return",
    "async", "await", "move", "ref", "self", "super", "crate", "dyn", "unsafe",
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
];
```


# lets actually delete the old debug binary file
not renaming it e.g.
```
remote: Resolving deltas: 100% (10/10), done.
remote: error: Trace: ac3e2c37e5a2f1abc46c087dd5285e2b169d7b6a29daeeacbdb9bebbae35b1f2
remote: error: See https://gh.io/lfs for more information.
remote: error: File debug-mcp/programmer-mcp (deleted) is 116.56 MB; this exceeds GitHub's file size limit of 100.00 MB
remote: error: GH001: Large files detected. You may want to try Git Large File Storage - https://git-lfs.github.com.
To github.com:sirati/programmer-mcp.git
 ! [remote rejected] main -> main (pre-receive hook declined)
error: failed to push some refs to 'github.com:sirati/programmer-mcp.git'
```
