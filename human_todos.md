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


LSP method+params and returns pretty-printed JSON response
pretty-printed JSON is too verbose, need something less verbose

lets add a new subcommand to execute: request_human_message (should be used instead of ending the session)
programmer-mcp on launch created a unix fd socket inside of the project root. if programmer-mcp is launches with -- everything afterwards is the message. via the socket this is communicated to the main programmer-mcp process, next time execute is called, the human request will be appended at the end. if request_human_message was called the mcp does not yield till a request is send via this method


please add it so we can do ls (with a set max depth) in symbol definition space

please add a function that returns or grap-searches the docstring of a symbol (or nothing if there is none)

pleae add a function that returns or grap-searches the body of a symbol

please do language specific stuff like listing all impl traits

please do it so that all functions can take multiple symbols as to avoid you having to write the same commands over and over

please in the tool usage explicitly require that always multiple commands are passed unless its absolutely unnessessary

please add a tool that can starts a background program, other argumetns are the name of the background process, and the group of the background process. add a grap-search function that can search the background program based on named and/or group named, further add a trigger function that defines/loads and runs a named trigger (it will be saved to .programmer-mcp/triggers/{name}.json). a trigger will print the lines as and configured N lines before and M lines after the trigger was called, the trigger also has a trigger_then_wait timeout, just so that waiting for the M lines after doesnt stall us for long. if a trigger is not awaited the result of the trigger will be attached to the next tool-call. a trigger can also be attached to a group, in that case it will always be on when a background program is started with that group, further there is a trigger-await function that takes a trigger name. if since the last start of background program the trigger triggered it returns immiediately, otherwise it doesnt yield till the trigger is triggered or the defined timeout is reached.

detect if we are starting in an environment where nix is available. if so detech if nix flakes are on. in nix if a lang server is missing we can use nix to run it!
