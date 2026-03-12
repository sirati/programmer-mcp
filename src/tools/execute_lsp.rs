//! LSP operation dispatch — symbol-based and file-based.

use crate::lsp::manager::LspManager;

use super::exec_helpers::{execute_multi_symbol, execute_on_first};
use super::json_util::{format_compact_json, strip_json_noise};
use super::operation::{Operation, OperationResult};
use super::{
    call_hierarchy, code_actions, definition, diagnostics, hover, impls, references, rename,
    symbol_info, symbol_list,
};

/// Dispatch a symbol-based LSP operation.
pub async fn execute_symbol_op(manager: &LspManager, op: Operation) -> OperationResult {
    match op {
        Operation::Definition {
            symbol_names,
            language,
            search_dir,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            let sd = search_dir.clone();
            execute_multi_symbol(
                "definition",
                clients,
                &symbol_names,
                sd.as_deref(),
                |client, name, sd| async move {
                    definition::read_definition(&client, &name, sd.as_deref()).await
                },
            )
            .await
        }
        Operation::References {
            symbol_names,
            language,
            search_dir,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "references",
                clients,
                &symbol_names,
                search_dir.as_deref(),
                |client, name, sd| async move {
                    references::find_references(&client, &name, 5, sd.as_deref()).await
                },
            )
            .await
        }
        Operation::Docstring {
            symbol_names,
            language,
            search_dir,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "docstring",
                clients,
                &symbol_names,
                search_dir.as_deref(),
                |client, name, sd| async move {
                    symbol_info::get_docstring(&client, &name, sd.as_deref()).await
                },
            )
            .await
        }
        Operation::Body {
            symbol_names,
            language,
            search_dir,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "body",
                clients,
                &symbol_names,
                search_dir.as_deref(),
                |client, name, sd| async move {
                    symbol_info::get_body(&client, &name, sd.as_deref()).await
                },
            )
            .await
        }
        Operation::Impls {
            symbol_names,
            language,
            search_dir,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "impls",
                clients,
                &symbol_names,
                search_dir.as_deref(),
                |client, name, sd| async move {
                    impls::find_impls(&client, &name, sd.as_deref()).await
                },
            )
            .await
        }
        _ => unreachable!("execute_symbol_op called with non-symbol operation"),
    }
}

/// Dispatch a file-based LSP operation.
pub async fn execute_file_op(manager: &LspManager, op: Operation) -> OperationResult {
    match op {
        Operation::Diagnostics {
            file_path,
            context_lines,
            show_line_numbers,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("diagnostics", clients, |client| {
                let path = file_path.clone();
                async move {
                    diagnostics::get_diagnostics(&client, &path, context_lines, show_line_numbers)
                        .await
                }
            })
            .await
        }
        Operation::Hover {
            file_path,
            line,
            column,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("hover", clients, |client| {
                let path = file_path.clone();
                async move { hover::get_hover_info(&client, &path, line, column).await }
            })
            .await
        }
        Operation::RenameSymbol {
            file_path,
            line,
            column,
            new_name,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("rename_symbol", clients, |client| {
                let path = file_path.clone();
                let name = new_name.clone();
                async move { rename::rename_symbol(&client, &path, line, column, &name).await }
            })
            .await
        }
        Operation::ListSymbols {
            file_path,
            max_depth,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("list_symbols", clients, |client| {
                let path = file_path.clone();
                async move { symbol_list::list_symbols(&client, &path, max_depth).await }
            })
            .await
        }
        Operation::CodeActions {
            file_path,
            line,
            column,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("code_actions", clients, |client| {
                let path = file_path.clone();
                async move { code_actions::get_code_actions(&client, &path, line, column).await }
            })
            .await
        }
        Operation::CodeAction {
            file_path,
            line,
            column,
            end_line,
            end_column,
            kinds,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("code_action", clients, |client| {
                let path = file_path.clone();
                let ks = kinds.clone();
                async move {
                    code_actions::get_code_actions_range(
                        &client, &path, line, column, end_line, end_column, &ks,
                    )
                    .await
                }
            })
            .await
        }
        Operation::ApplyCodeAction {
            file_path,
            line,
            column,
            index,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("apply_action", clients, |client| {
                let path = file_path.clone();
                async move {
                    code_actions::apply_code_action(&client, &path, line, column, index).await
                }
            }).await
        }
        Operation::Format {
            file_path,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("format", clients, |client| {
                let path = file_path.clone();
                async move { code_actions::format_file(&client, &path).await }
            })
            .await
        }
        Operation::RawLspRequest {
            method,
            params,
            language,
        } => {
            let clients = manager.resolve(Some(&language), None);
            execute_on_first("raw_lsp_request", clients, |client| {
                let m = method.clone();
                let p = params.clone();
                async move {
                    let result = client.raw_request(&m, p).await?;
                    Ok(format_compact_json(&strip_json_noise(result)))
                }
            })
            .await
        }
        _ => unreachable!("execute_file_op called with non-file operation"),
    }
}
