//! DSL text parser for the `execute` tool.
//!
//! Turns a multi-line command script into a `Vec<Operation>` that can be
//! passed directly to [`crate::tools::execute_batch`].
//!
//! # Syntax overview
//!
//! Each non-empty line (after stripping `#` comments) starts with a command
//! keyword followed by optional arguments.
//!
//! ## Navigation
//! ```text
//! cd src/debug          # change current directory context
//! cd src/debug/server.rs  # change to a specific file (extension required)
//! ```
//!
//! ## File-based operations
//! ```text
//! list_symbols                         # uses current cd file
//! list_symbols [server.rs child.rs]    # explicit list
//! diagnostics [server.rs]
//! hover server.rs 42 10                # filePath line col
//! hover 42 10                          # uses current cd file
//! rename_symbol server.rs 42 10 new_name
//! ```
//!
//! ## Symbol-based operations
//! ```text
//! body [relay_command show_help]
//! definition [MyStruct MyStruct.method]
//! references [my_fn]
//! docstring [MyTrait]
//! impls [MyType]
//! ```
//!
//! ## Item list expansion
//! Lists use `[...]` with items separated by spaces, commas, or both.
//! Brace expansion works like shell: `tools/{mod.rs x.rs}` expands to
//! `tools/mod.rs` and `tools/x.rs`.
//!
//! ## Task management
//! ```text
//! set_task task-name Description text
//! update_task task-name New description
//! update_task task-name append=Additional text
//! complete_task task-name
//! list_tasks
//! list_tasks completed
//! add_subtask task-name sub-name Description
//! complete_subtask task-name sub-name
//! list_subtasks task-name
//! list_subtasks task-name completed
//! ```
//!
//! ## Background processes & triggers
//! ```text
//! start_process myproc cargo test [group=build]
//! stop_process myproc
//! search_output myproc error
//! define_trigger myTrigger "^error" [before=3] [after=5] [timeout=30000] [group=build]
//! await_trigger myTrigger
//! ```
//!
//! ## Misc
//! ```text
//! request_human_message
//! ```

pub mod ops;
pub mod parse;

use std::path::{Path, PathBuf};

use crate::tools::Operation;

use ops::{has_extension, normalize_path, resolve_cd_path, *};
use parse::{split_first_word, strip_comment};

/// Result of parsing a DSL script: operations to execute and any warnings.
pub struct ParseResult {
    pub operations: Vec<Operation>,
    pub warnings: Vec<String>,
}

/// Options for DSL parsing.
#[derive(Default, Clone)]
pub struct DslOptions {
    /// Whether `edit file` is allowed.
    pub allow_file_edit: bool,
}

/// Parse a DSL command script into a list of operations.
///
/// Lines are processed in order. `cd` commands update the current path
/// context used by subsequent file-based operations.
/// All operations are collected and returned for concurrent execution.
#[cfg(test)]
pub fn parse_dsl(commands: &str) -> ParseResult {
    parse_dsl_with_options(commands, &DslOptions::default())
}

/// Parse with explicit options (e.g. allow_file_edit flag).
pub fn parse_dsl_with_options(commands: &str, options: &DslOptions) -> ParseResult {
    let mut ops = Vec::new();
    let mut warnings = Vec::new();
    let mut ctx = DslContext {
        allow_file_edit: options.allow_file_edit,
        ..Default::default()
    };

    for raw_line in commands.lines() {
        // Support `|` as an inline command separator
        for segment in raw_line.split('|') {
            let line = strip_comment(segment).trim();
            if line.is_empty() {
                continue;
            }
            let (cmd, args) = split_first_word(line);
            ctx.dispatch(&mut ops, &mut warnings, cmd, args);
        }
    }

    ParseResult {
        operations: ops,
        warnings,
    }
}

// ── parsing context ───────────────────────────────────────────────────────────

#[derive(Default)]
struct DslContext {
    /// Current directory prefix applied to file-path arguments.
    cd_dir: PathBuf,
    /// Set when `cd` targets a file (has an extension).
    cd_file: Option<PathBuf>,
    /// Whether `edit file` is allowed.
    allow_file_edit: bool,
}

impl DslContext {
    fn dispatch(
        &mut self,
        ops: &mut Vec<Operation>,
        warnings: &mut Vec<String>,
        cmd: &str,
        args: &str,
    ) {
        match cmd {
            "cd" => self.handle_cd(args),

            // file-based
            "list_symbols" => handle_list_symbols(ops, args, &self.cd_dir, self.cd_file.as_deref()),
            "diagnostics" => handle_diagnostics(ops, args, &self.cd_dir, self.cd_file.as_deref()),
            "hover" => handle_hover(ops, args, &self.cd_dir, self.cd_file.as_deref()),
            "rename_symbol" => {
                handle_rename_symbol(ops, args, &self.cd_dir, self.cd_file.as_deref())
            }
            "code_action" => handle_code_action(ops, args, &self.cd_dir, self.cd_file.as_deref()),

            // symbol-based (with bare-arg warnings)
            "body" => handle_symbol_cmd(
                ops,
                warnings,
                "body",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "definition" => handle_symbol_cmd(
                ops,
                warnings,
                "definition",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "references" => handle_symbol_cmd(
                ops,
                warnings,
                "references",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "docstring" => handle_symbol_cmd(
                ops,
                warnings,
                "docstring",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "impls" => handle_symbol_cmd(
                ops,
                warnings,
                "impls",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "callers" => handle_symbol_cmd(
                ops,
                warnings,
                "callers",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),
            "callees" => handle_symbol_cmd(
                ops,
                warnings,
                "callees",
                args,
                &self.cd_dir,
                self.cd_file.as_deref(),
            ),

            // read file
            "read" | "cat" => handle_read(ops, args, &self.cd_dir, self.cd_file.as_deref()),

            // search
            "grep" | "search" => handle_grep(ops, args, &self.cd_dir),

            // workspace
            "workspace_info" | "workspace-info" => ops.push(Operation::WorkspaceInfo),

            // tasks
            "set_task" => handle_set_task(ops, args),
            "update_task" => handle_update_task(ops, args),
            "complete_task" => handle_complete_task(ops, args),
            "list_tasks" => handle_list_tasks(ops, args),
            "add_subtask" => handle_add_subtask(ops, args),
            "complete_subtask" => handle_complete_subtask(ops, args),
            "list_subtasks" => handle_list_subtasks(ops, args),

            // background / triggers
            "start_process" => handle_start_process(ops, args),
            "stop_process" => handle_stop_process(ops, args),
            "search_output" => handle_search_output(ops, args),
            "define_trigger" => handle_define_trigger(ops, args),
            "await_trigger" => handle_await_trigger(ops, args),

            // editing
            "edit" => handle_edit(ops, warnings, args, &self.cd_dir, self.allow_file_edit),
            "edit_range" => handle_edit_range(ops, warnings, args, &self.cd_dir),
            "apply_edit" => handle_apply_edit(ops, warnings, args, &self.cd_dir),
            "undo" => {
                let id = args.trim();
                if id.is_empty() {
                    warnings.push("undo: requires an undo ID".into());
                } else {
                    ops.push(Operation::Undo {
                        undo_id: id.to_string(),
                    });
                }
            }

            // refactoring
            "code_actions" => handle_code_actions(ops, args, &self.cd_dir, self.cd_file.as_deref()),
            "apply_action" => handle_apply_action(ops, args, &self.cd_dir, self.cd_file.as_deref()),
            "format" => handle_format(ops, args, &self.cd_dir, self.cd_file.as_deref()),

            // misc
            "request_human_message" => ops.push(Operation::RequestHumanMessage),

            // unknown
            other => {
                warnings.push(format!("unknown command: {other}"));
            }
        }
    }

    fn handle_cd(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        let raw = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            normalize_path(&self.cd_dir.join(path))
        };
        // Resolve extension-less paths against the workspace (cwd).
        let resolved = resolve_cd_path(&raw);

        if has_extension(&resolved) {
            self.cd_file = Some(resolved.clone());
            self.cd_dir = resolved.parent().unwrap_or(Path::new("")).to_path_buf();
        } else {
            self.cd_file = None;
            self.cd_dir = resolved;
        }
    }
}

#[cfg(test)]
mod tests;
