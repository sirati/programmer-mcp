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



please add support so that subproject / workspaces inside of folders of the main directory are automatically discovered. so are standalone files. add a command workspace-info that shows all workspaces and standalone files (for files: by number in folder not name, unless its <=3 in the folder)


we should expose refactor commands that lsp support 

test with other language servers than just rust.
