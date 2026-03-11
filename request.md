Please write in idomatic async rust an mcp server akin in functionality to the one in ./ref-srcs/mcp-language-server
as a reference impl for an lsp client look at ./ref-srcs/lsp-client

just for reference I have cloned the sources of ./ref-srcs/lsp-types and ./ref-srcs/rust-sdk, do not directly depend on these folders, but import from crates.io as usual


the main difference in functionality should be that our mcp server can connect to multiple lsp clients simultaneously and offer a unified interface that allows specifying the language but falls back on checking with all lsps, further it only offers one commands, that allows including multiple commands as offered by the reference (excluding edit-files), so that all of them can be executed in parallel. if e.g. a symbol is not found the code will:
1. try all variations of symbol name is different developer cases
2. try to find a file with a similar name, and the check for the most similar symbols in that file
