always put language specific code into its own submodule in tools/language_specific/{lang}/ 




✅ please add it so we can do ls (with a set max depth) in symbol definition space (partially done)

✅ please add a function that returns or grap-searches the docstring of a symbol (or nothing if there is none)

✅ pleae add a function that returns or grap-searches the body of a symbol

✅ please do language specific stuff like listing all impl traits

✅ please do it so that all functions can take multiple symbols as to avoid you having to write the same commands over and over

✅ please in the tool usage explicitly require that always multiple commands are passed unless its absolutely unnessessary

✅ please add a tool that can starts a background program, other argumetns are the name of the background process, and the group of the background process. add a grap-search function that can search the background program based on named and/or group named, further add a trigger function that defines/loads and runs a named trigger (it will be saved to .programmer-mcp/triggers/{name}.json). a trigger will print the lines as and configured N lines before and M lines after the trigger was called, the trigger also has a trigger_then_wait timeout, just so that waiting for the M lines after doesnt stall us for long. if a trigger is not awaited the result of the trigger will be attached to the next tool-call. a trigger can also be attached to a group, in that case it will always be on when a background program is started with that group, further there is a trigger-await function that takes a trigger name. if since the last start of background program the trigger triggered it returns immiediately, otherwise it doesnt yield till the trigger is triggered or the defined timeout is reached.

✅ detect if we are starting in an environment where nix is available. if so detech if nix flakes are on. in nix if a lang server is missing we can use nix to run it!

✅ add a new function task it can add a named task (saved in .programmer-mcp/tasks/{name}.json) it should always be set, subtask can be added to task and are saved too, task can be updated, appened to and completed, lsit-task will unless explicitly requested only list uncompleted tasks, list-subtasks does the same based on a task name.

✅ highest priority feature: add a remote feature: when started create a new fd socket inside of ~/.share/programmer-mcp/ 
if started with --debug it should be named debug-mcp.sock otherwise the name of the current project with path  .sock
now we have a new --remote {user@host:port} (user and port part are optional), if called it should use ssh to connect to the remote host and check ~/.share/programmer-mcp/ if started with --debug it takes the debug one, otherwise it if there are multiple (excluding debug) it should ask on the first command, remember the command, and after connection execute the queued command 
the connection works by asking to establish a session by sending a random string to the socket. the server will then create a new fd socket inside of ~/.share/programmer-mcp/{project_name with path/debug}.session-{rnd sessionstr}-in/out.sock, the client should now forward these sockets via ssh, and connect to them, all input / output is forwarded to and from the socket. for this forwarding logic look at /src/debug/proxy.rs for reference, shared code should be extracted into a separate module. the bigger change this requires is that now the code has to be able to handle multiple sessions simultaneously: the regular stdio one and the one established via ssh. 


{
  "method": "tools/call",
  "params": {
    "name": "execute",
    "arguments": {
      "filePath": "src/debug/relay.rs",
      "operations": [
        {
          "operation": "body",
          "symbolNames": [
            "RelayChannel",
            "RelayChannel.relay",
            "RelayChannel.ensure_initialized"
          ]
        }
      ]
    }
  }
}
---
RelayChannel.relay not found
---
Channel.ensure_initialized not found

we need to be able to deal with such confusing ai input, first look for the parent symbol and of it the child, if not these the child fuzzy, if not there again with parent fuzzy, if not there only the child, if not there only the child fuzzy

another thing I have noticed is that ssh connections are not closed then the program shuts down, so they are left behind :(



✅ it happens quite often that you get confused on how to call a function through the relay:
```
{
  "method": "tools/call",
  "params": {
    "arguments": {
      "filePath": "src/remote/listener.rs",
      "operations": [
        {
          "filePath": "src/remote/listener.rs",
          "operation": "list_symbols"
        }
      ]
    },
    "name": "execute"
  }
}
```
e.g. here filePath is included twice, the one at the arguments level is definitely wrong
lets do the following to fix the issue:
lets split the debug relay functions into two parts:
show-help, and execute
show-help will call tools/list of the child
execute will call execute of the child through tools/call

the second suggestion is to radiacally change the syntax of execute, lets have all arguments be a single text, with commands being the first word in a line and them separated by new lines
e.g.
```
cd src  #cd is only available for folders and also files cd /src/main.rs (in case of cd the extension is mandatory)
list_symbols [main, tools/{mod.rs symbol_info.rs definition} .] #for files the extension is optional, list is defined by [] separated by either space or comma or both or multiple, one can use {} to define variants e.g. [main, tools/{mod.rs symbol_info.rs}] is equivalent to [main, tools{/mod.rs /symbol_info.rs}] or [main, tools/mod.rs tools/symbol_info.rs]
cd tools/mod.rs
body [execute_one ]
cd ../symbol_search.rs
bosy [filter_exact_matches,find_{symbol_with_fallback ,similar_in_document} collect_nested_matches] #inside of {} space or comma are also the separators
```
# a singular . inside of [] e.g. [main,.] or [main . ] refers to the current file as set by cd
cd does not fold between executes! it must be set anew each time
further this is allowed to: tools/{*} to mean all subsymbols tools{*} this however would not be allowed {*} is special and requires a separator like / or . beforehand. 
make sure that if a part of the argument is invalid, it should be interpreted on a best effort basis, and the rest of the arguments should be still be valid. now should a error make a command cancel or crash. if something is wrong the other commands will still be executed, but the syntax error will be reported
btw as you are now impl a dsl consider using an existing rust crate that makes it easier, especially one that allows fuzzy parsing and rich error messages, that can be shortened
remember that file extensions for recognised languages are always optional


lets filewatch and when a file was changes automatially call diagnostics on it, inside of our ./.programmer-mcp/.cache/ folder in a parallel filestructure to the project we save the diagnostic results as well as a file hash. if a file was changed we compare  and any new diagnostics will be reported in the output of the next execute. when diagnostics command is called we always check anew!


i just saw these three commands being executed seperately:
{
  "commands": "cd src/remote/client.rs\nlist_symbols"
}
{
  "commands": "cd src/tools/mod.rs\nlist_symbols"
}
{
  "commands": "cd src/lsp/client.rs\nlist_symbols"
}
please make clear from example that the correct usage is 
{
  "commands": "list_symbol src/{{lsp remote}/client tools/mod}"
}


i just saw the following incorrect usage:
{
  "commands": "cd src/tools/mod.rs\nbody execute_batch execute_one execute_on_clients execute_on_first execute_multi_symbol strip_json_noise format_compact_json run_trigger_scanner"
}
because body just takes one argument, we should support it, but still warn! like this
"command body [list] was used wrongly correct call would be body [execute_batch execute_one execute_on_clients execute_on_first execute_multi_symbol strip_json_noise format_compact_json run_trigger_scanner]"



✅ please add support so that subproject / workspaces inside of folders of the main directory are automatically discovered. so are standalone files. add a command workspace-info that shows all workspaces and standalone files (for files: by number in folder not name, unless its <=3 in the folder)


✅ we should expose refactor commands that lsp support 

test with other language servers than just rust.

list_symbols when done on a folder should act like ls



✅ fix compiler warnings (dead code removed, incomplete features annotated with #[allow(dead_code)] TODO)





{
  "commands": "body [TriggerResult TriggerResult.fmt]"
}
no results for body

--- Auto-diagnostics: /home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs ---
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:119:13: warning: use of deprecated field `lsp_types::InitializeParams::root_uri`: Use `workspace_folders` instead when possible
`#[warn(deprecated)]` on by default
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:120:13: warning: use of deprecated field `lsp_types::InitializeParams::root_path`: Use `root_uri` instead when possible
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:164:25: warning: unused import: `futures::StreamExt`
`#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

--- Auto-diagnostics: /home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs ---
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:119:13: warning: use of deprecated field `lsp_types::InitializeParams::root_uri`: Use `workspace_folders` instead when possible
`#[warn(deprecated)]` on by default
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:120:13: warning: use of deprecated field `lsp_types::InitializeParams::root_path`: Use `root_uri` instead when possible
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:164:25: warning: unused import: `futures::StreamExt`
`#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

--- Auto-diagnostics: /home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs ---
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:119:13: warning: use of deprecated field `lsp_types::InitializeParams::root_uri`: Use `workspace_folders` instead when possible
`#[warn(deprecated)]` on by default
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:120:13: warning: use of deprecated field `lsp_types::InitializeParams::root_path`: Use `root_uri` instead when possible
/home/sirati/devel/rust/programmer-mcp/src/lsp/client/mod.rs:164:25: warning: unused import: `futures::StreamExt`
`#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

There are multiple problems here:
1. the diagnostics are attached to a message, even though the edits made since the last command did not change the diagnostics, and only new ones should be attached
2. the path is very verbose instead if should be like this:
```
New diagnosstics based on recent edits:
cd src/lsp/client  
2 new warning for mod.rs:
use of deprecated field:
L119:13 `lsp_types::InitializeParams::root_uri`: Use `workspace_folders` instead when possible
L120:13: `lsp_types::InitializeParams::root_path`: Use `root_uri` instead when possible
{another type of warning}:
L159:13 {case specific submessage}

1 new warning for helper.rs:
{yet another type of warning}:
L59:23 {case specific submessage}

1 new error for helper.rs:
{yet another type of error}:
L224:43 {case specific submessage}

cd ../server
{more}

```
  - absolute path always converted to relative path based on project root
  - cd used to allow for shorter paths which multiple in a location
  - general goal is to keep everything short, concise and without repeats
  - grouping by error/warning type
  - noise from message has been removed: "`#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default" 

3. if the same warning happens multiple times the locations inside a file should be grouped
4. within each group, the locations should be sorted by line number and column number
5. groups should be sorted by the first line number 

further

{
  "commands": "body [TriggerResult TriggerResult.fmt]"
}
no results for body
did not cd into the location of TriggerResult if the fuzzy logic completely fails based on path restricted logic, the path should one step at a time be traversed upwards (e.g. cd ..) and at that location ALL sublocation should be searched again using the fuzzy logic. this stops at the project root of course cd cannot leave the project root. if this path traversal happens, it should say found symbol {symbol} as {fuzzy resolved} at unexpected location {location with e.g. file extensions removed, reletative path to where the command was issued}. if multiple symbols are resolved at the same path it should be like {symbol1 symbal2.{. subsymbol}}} as {fuzzy1 fuzzy2.{. fuzzysubsymbol}} respectively, so avoid repeating.



const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "py", "js", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "java", "kt", "scala",
    "rb", "ex", "exs", "nix", "toml", "yaml", "yml", "json", "sh", "bash", "zsh", "lua",
    "zig", "swift", "cs", "fs", "ml", "mli", "hs", "el", "clj", "sql",
];
lists like those exist i think more than four times in the codebase. extract such duplicated code to its own module. in this case it should be a module about identifying and working with source files based on their extensions.


i have noticed that .cache has both files named after something resembling the file structure, (i think its better if we actually build a parallel folder structure) further there are also files outside of diagnostics that have hex as their name. neither files contain the actual diagnostic, just the hash. idk why there are two types of files. anyway for it to be a cache we need to store the warnings and errors in a rich format.



if you have already added general file editing functionality please put behind a config flag that default to false (when off even the help does not include this option)

what you please can include instead is a new edit command that works like this:
edit [operation type = file/body/signature/docs this can be a list e.g. signature body] path_to_file symbol_quantifier new_content
for this its important that if the path and symbol cannot be resolved, we do NOT fallback on fuzzy search, instead we use fuzzy search to find candidates and answer with
edit to path_to_file symbol_quantifier failed
did you mean:
1. path_option1 symbol_option1
2. path_option2 symbol_option2
to apply edit do
apply_edit {short unique identifier} correct_path correct_symbol
apply_edit fails the same way if again incorrect

on a correct edit the reply should be
applied edit [types] correct_path correct_symbol -> diff:
```diff
{diff of edit}
```
editing should be robust against \n\r stuff and indention i.e. take the indention if the first line of the new content and change indention of whole section as if the indention was at 0 there. also when counting indention identify if it is tab / 1 space / 2 spaces / 3 / 4 ...
you do the same for the to be edited content
then you apply the new-line and indention style to the to be inserted content, you indent the whole thing to the same level as the first line of the to be edited conent and then you insert it



whenever the command includes only on command excluding cd from count, the last like of the response should be: Please always batch multiple commands together.



```rust
impl IndentStyle {
    /// Produce a single indent level in this style.
    pub fn unit(&self) -> &str {
        match self {
            IndentStyle::Tabs => "\t",
            IndentStyle::Spaces(2) => "  ",
            IndentStyle::Spaces(4) => "    ",
            IndentStyle::Spaces(8) => "        ",
            _ => "    ", // fallback
        }
    }
}
```
we must support all possible IndentStyle even insane ones like 17 spaces


{
  "commands": "edit body src/tools/indent.rs gcd_u32 fn gcd_u32(mut a: u32, mut b: u32) -> u32 {\\n    while b != 0 {\\n        let t = b;\\n        b = a % b;\\n        a = t;\\n    }\\n    a\\n}"
}
- fn gcd_u32(a: u32, b: u32) -> u32 {
-     if b == 0 {
-         a
-     } else {
-         gcd_u32(b, a % b)
-     }
- }
+ fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
+     while b != 0 {
+         let t = b;
+         b = a % b;
+         a = t;
+     }
+     a
+ }
can we try to use a better diff algorithm? like this would be nicer:
- fn gcd_u32(a: u32, b: u32) -> u32 {
+ fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
-     if b == 0 {
-         a
-     } else {
-         gcd_u32(b, a % b)
+     while b != 0 {
+         let t = b;
+         b = a % b;
+         a = t;
     }
+     a
}
another issue here is the edit was called with body but was able to edit the signature it should have been called with [body signature], we can have heuristics to detect this mistake, but in that care we should fail like this:
detected a signature at the start of a body only edit! please use `edit [body signature] ...` in the future. to apply edit:
apply_edit {short unique identifier e.g. three random words separated by _} [body signature] 



/// Check if a line looks like a doc comment or attribute (not code).
fn is_doc_or_attr(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("/**")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("*/")
        || trimmed == "*"
        || trimmed.starts_with("\"\"\"")
        || trimmed.starts_with("'''")
        || (trimmed.starts_with('#') && trimmed.contains('['))
}
please put programming specific code into language_specific/{lang}/

lets also allow more targetted edits by providing a 3-6 line snipped before it starts, and after it ends. here we need to support a way to indicate START end END of the file/body to allow for targetted edits that end at the end of starts just after the signature. if multiple matches were found all options are presented. and one can do apply_edit {short unique identifier e} [list of the indecies of the ranges presented must be none-overlapping] otherwise we fail again. 
when doing this the segment before and after may be separated to our edits by empty / only whitespace lines. we ignore these but keep them as is. further for finding the indention we must also ignore such empty / only whitespace lines.
because some languages like python use indentation for meaning this command should be provided with these section in a multiline friendly way, if provided like rn without multilines, we try our best to apply it, if its a language like python we fail by printing the diff, and ask if the indendation is correct with edit_apply {identifier} if correct, or telling them to do edit_apply {identifier} "correct\n  multiline\ncode"
 s

lets create a configurable max file length and max function length (ignoring empty lines). the config should have a suggestion and a hard limit for both. if an edit results in either of these four limits to be execeeded, it does not stop the edit from applying, it is only printed if the edit was applied!  s


{
  "commands": "apply_edit warm_sky_dips src/tools/indent.rs gcd_u32"
}
detected a signature change at the start of a body-only edit!
please use `edit body,signature` in the future.
to apply this edit anyway:
  apply_edit bold_ash_arcs src/tools/indent.rs gcd_u32
  
if apply_edit does not change the edit content and targetting and it fails again, lets 



i have noticed these kinds of usage errors often because this is how edit works, where you specify the path first
{
  "commands": "body src/tools/dsl/ops/edit.rs handle_apply_edit"
}
⚠ command `body src/tools/dsl/ops/edit.rs handle_apply_edit` was used without brackets — correct usage: `body [src/tools/dsl/ops/edit.rs handle_apply_edit]` 
if our fuzze detects that the first argument is exactly a path OR the second argument is fuzzily resolved to be located in the first argument we should warn differently:
incorrect arguments, corrected to `body src/tools/dsl/ops/edit.rs.{handle_apply_edit, .}`


when an edit was applied lets allow undo {new unique word identifier} -> this checks if the inserted edit string still exists (ignoring indention trailing whitespaces and empty lines), and if it does, we undo it.
the successful edit response should end is Undo with: undo {new unique word identifier}


i have noticed the when listing symbols the result is more verbose than it has to be:
rn ifs: method a method b field c using d trait f trait g
it should me usings: d, methods: a, b, fields: c, traits: f, g
listing symbols currently does not work for directories, in that case it should list the directory content like
dirs: a, b, files: c, d, e


lets make sure that stores like undo or for apply_edit are per connection, and cleared if the connection is closed. also they should have a max capacity of 1000, after that the longest unused entry is dropped in favour if thenew.


i have seen a single cd command issues, lets detech this and cds at the end of a command chain, and warn that cd does not change path, and only applies to the multi-command chains commands after it.



{
  "commands": "cd src\ngrep \"lsp.*init\\|start_lsp\\|spawn_lsp\\|LspClient::new\\|lsp_clients\\|fn.*lsp\""
}
⚠ unknown command: start_lsp\
⚠ unknown command: spawn_lsp\
⚠ unknown command: LspClient::new\
⚠ unknown command: lsp_clients\
⚠ unknown command: fn.*lsp"

no matches for `"lsp.*init\`
Please always batch multiple commands together.

BUG: we need to support dealing with quotes in commands, we need to deal with escaping in commands



when a language server times out, 


target/flycheck0/stdout` (this is a cargo tmp file)

what is that anyway??? lets make it so that filewatcher ignores files not tracked by git, also on a per file bases add a backoff timeout starting with 1s ending at 15s, if for x4 that time no update was received, its reset. 
if possible lets use some rust crate that can parse .gitignore and not rely on git directly. 

next make sure that lsp only get file change notifications for files of their language, or that a known configuration files for that langauage












{
  "commands": "cd src/tools/dsl/ops\nbody lsp.rs.{handle_symbol_cmd, push_symbol_op}"
}

⚠ command `body lsp.rs.{handle_symbol_cmd, push_symbol_op}` was used without brackets — correct usage: `body [lsp.rs.{handle_symbol_cmd, push_symbol_op}]



{
  "commands": "cd src/main.rs | definition [execute_one]"
}

Symbol: execute_one
File: /home/sirati/devel/rust/programmer-mcp/src/tools/execute.rs
Kind: Function
Range: L61:C1 - L186:C2
problem 1: file is not relative
problem 2: definition and body are doing the same, lets think about what definition should do?




todos: symbol index should also be saved to .cache, .cache files maybe can just contain last changed so we dont need to hash?  when starting .cache should be loaded and only changed files reindexed




BUG BUG
{
  "commands": "body [execute_body] | references [read_range_from_file]"
}
No references found for symbol: read_range_from_file

---

No references found for symbol: read_range_from_file

---

No references found for symbol: read_range_from_file

---

Some requests found nothing
"The symbol cache may still be seeding. Let me search directly."
we always print if we are seeding, so that we did not find the symbol but grep did is a major bug!!
