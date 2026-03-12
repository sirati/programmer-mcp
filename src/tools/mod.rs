pub mod call_hierarchy;
pub mod code_actions;
pub mod definition;
pub mod diagnostics;
pub mod diagnostics_cache;
pub mod doc_index;
pub mod dsl;
pub mod edit;
pub mod edit_apply;
pub mod edit_extract;
pub mod edit_types;
pub mod exec_helpers;
pub mod execute;
mod execute_lsp;
pub mod formatting;
pub mod grep;
pub mod hover;
pub mod impls;
pub mod indent;
pub mod json_util;
pub mod language_specific;
pub mod list_dir;
pub mod operation;
pub mod process_ops;
pub mod read_file;
pub mod references;
pub mod rename;
pub mod serde_helpers;
pub mod symbol_cache;
pub mod symbol_info;
pub mod symbol_list;
pub mod symbol_match;
pub mod symbol_search;
pub mod symbol_walk;
pub mod task_ops;
pub mod workspace;

pub use execute::execute_batch;
pub use operation::{Operation, OperationResult};

/// Source file extensions considered relevant across the codebase.
pub const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "py", "js", "ts", "tsx", "jsx", "c", "cc", "h", "cpp", "hpp", "java", "kt",
    "scala", "rb", "ex", "exs", "nix", "toml", "yaml", "yml", "json", "sh", "bash", "zsh", "lua",
    "zig", "swift", "cs", "fs", "ml", "mli", "hs", "el", "clj", "sql",
];
