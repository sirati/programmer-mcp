/// Smart symbol-aware edit command.
///
/// Resolves symbols via LSP, extracts the relevant range (body/signature/docs),
/// applies replacement with indentation normalization, and returns a diff.
/// When exact resolution fails, suggests candidates for disambiguation.
use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::sync::Arc;

use lsp_types::SymbolInformation;
use tokio::sync::Mutex;

use crate::config::LengthLimits;
use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::uri_to_path;
use crate::tools::symbol_search::{filter_exact_matches, find_symbol_with_fallback};

use super::edit_apply::{apply_file_edit, apply_symbol_edit};
use super::edit_extract::{make_relative, word_id};

/// Which part of a symbol to edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditType {
    Body,
    Signature,
    Docs,
    File,
}

impl EditType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "body" => Some(Self::Body),
            "signature" | "sig" => Some(Self::Signature),
            "docs" | "doc" | "docstring" => Some(Self::Docs),
            "file" => Some(Self::File),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Signature => "signature",
            Self::Docs => "docs",
            Self::File => "file",
        }
    }
}

/// A pending edit waiting for disambiguation or confirmation.
#[derive(Debug, Clone)]
pub struct PendingEdit {
    pub edit_types: Vec<EditType>,
    pub new_content: String,
    pub path: String,
    pub symbol_name: String,
    pub _search_dir: Option<String>,
    pub _candidates: Vec<(String, String)>, // (path, symbol_name)
}

/// A bounded key-value store that evicts the oldest entry when at capacity.
pub struct BoundedStore<V> {
    map: HashMap<String, V>,
    order: VecDeque<String>,
    capacity: usize,
}

impl<V> BoundedStore<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    pub fn insert(&mut self, key: String, value: V) {
        // Remove old entry with same key if exists
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        }
        // Evict oldest if at capacity
        while self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            } else {
                break;
            }
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key);
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        if let Some(v) = self.map.remove(key) {
            self.order.retain(|k| k != key);
            Some(v)
        } else {
            None
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }
}

const STORE_CAPACITY: usize = 1000;

/// Storage for pending edits keyed by word ID.
pub type PendingEdits = Arc<Mutex<BoundedStore<PendingEdit>>>;

pub fn new_pending_edits() -> PendingEdits {
    Arc::new(Mutex::new(BoundedStore::new(STORE_CAPACITY)))
}

/// An undo entry: stores enough info to reverse an edit.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub file_path: String,
    /// The old content that was replaced (before normalization).
    pub old_content: String,
    /// The new content that was inserted (after normalization).
    pub new_content: String,
}

/// Storage for undo entries keyed by word ID.
pub type UndoStore = Arc<Mutex<BoundedStore<UndoEntry>>>;

pub fn new_undo_store() -> UndoStore {
    Arc::new(Mutex::new(BoundedStore::new(STORE_CAPACITY)))
}

/// Execute an undo: check if the new content still exists in the file, restore old content.
pub async fn execute_undo(undo_id: &str, undo_store: &UndoStore) -> Result<String, LspClientError> {
    let entry = {
        let mut map = undo_store.lock().await;
        map.remove(undo_id)
    };

    let Some(entry) = entry else {
        return Ok(format!("no undo entry with id '{undo_id}'"));
    };

    let file_content = std::fs::read_to_string(&entry.file_path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;

    // Normalize for comparison: strip leading/trailing whitespace per line, skip empty lines
    let normalize = |s: &str| -> Vec<String> {
        s.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };

    let _file_norm = normalize(&file_content);
    let new_norm = normalize(&entry.new_content);

    if new_norm.is_empty() {
        return Ok("undo failed: empty content to match".into());
    }

    // Find the new content in the file (normalized match)
    let needle_len = new_norm.len();
    let mut found = false;

    // Search in file lines for the matching range
    let file_lines: Vec<&str> = file_content.lines().collect();
    let mut match_start = None;
    let mut match_end = None;
    let mut norm_idx = 0;

    for (i, line) in file_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if norm_idx < needle_len && trimmed == new_norm[norm_idx] {
            if norm_idx == 0 {
                match_start = Some(i);
            }
            norm_idx += 1;
            if norm_idx == needle_len {
                match_end = Some(i);
                found = true;
                break;
            }
        } else {
            norm_idx = 0;
            match_start = None;
            // Retry current line as potential start
            if trimmed == new_norm[0] {
                match_start = Some(i);
                norm_idx = 1;
            }
        }
    }

    if !found {
        return Ok(format!(
            "undo failed: the edit content no longer exists in {}",
            entry.file_path
        ));
    }

    let start = match_start.unwrap();
    let end = match_end.unwrap();

    // Replace the matched range with old content
    let mut new_lines: Vec<String> = file_lines.iter().map(|l| l.to_string()).collect();
    let old_replacement: Vec<String> = entry.old_content.lines().map(|l| l.to_string()).collect();
    new_lines.splice(start..=end, old_replacement);

    let line_ending = crate::tools::indent::detect_line_ending(&file_content);
    let result = new_lines.join(line_ending);
    std::fs::write(&entry.file_path, &result)
        .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

    use super::edit_extract::make_relative;
    let rel = make_relative(&entry.file_path);
    Ok(format!(
        "undo applied to {rel} (lines {}-{})",
        start + 1,
        end + 1
    ))
}

/// Find symbols with exact-only matching (no fuzzy fallback).
async fn find_symbol_exact(
    client: &Arc<LspClient>,
    name: &str,
) -> Result<Vec<SymbolInformation>, LspClientError> {
    let results = client.symbol_cache().workspace_symbol(client, name).await?;
    Ok(filter_exact_matches(&results, name))
}

/// Execute a file-only edit (no LSP needed).
pub async fn execute_edit_no_lsp(
    _edit_types: &[EditType],
    path: &str,
    _symbol_name: &str,
    new_content: &str,
    _pending: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    apply_file_edit(path, new_content, undo_store, limits).await
}

/// Execute an edit operation.
///
/// Returns formatted output: either a diff on success, or a disambiguation
/// prompt with candidate suggestions.
pub async fn execute_edit(
    client: &Arc<LspClient>,
    edit_types: &[EditType],
    path: &str,
    symbol_name: &str,
    new_content: &str,
    search_dir: Option<&str>,
    pending: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    // For file-type edits, no symbol resolution needed
    if edit_types.len() == 1 && edit_types[0] == EditType::File {
        return apply_file_edit(path, new_content, undo_store, limits).await;
    }

    // Try exact resolution first
    let exact = find_symbol_exact(client, symbol_name).await?;

    // Filter by path if provided
    let filtered: Vec<_> = if path.is_empty() || path == "." {
        exact
    } else {
        exact
            .into_iter()
            .filter(|s| {
                uri_to_path(&s.location.uri)
                    .map(|p| p.contains(path))
                    .unwrap_or(false)
            })
            .collect()
    };

    if filtered.is_empty() {
        return disambiguate(
            client,
            edit_types,
            path,
            symbol_name,
            new_content,
            search_dir,
            pending,
        )
        .await;
    }

    let symbol = &filtered[0];
    apply_symbol_edit(
        client,
        symbol,
        edit_types,
        new_content,
        pending,
        undo_store,
        false,
        limits,
    )
    .await
}

/// Apply an edit that was previously stored as pending (after disambiguation).
///
/// - `path_override` / `symbol_override`: if Some, correct the stored location.
/// - `types_override`: if Some, replace stored edit types.
///
/// If nothing changed from the stored values and resolution still fails,
/// the same ID is re-inserted so the caller can try again.
pub async fn apply_pending_edit(
    client: &Arc<LspClient>,
    edit_id: &str,
    path_override: Option<&str>,
    symbol_override: Option<&str>,
    types_override: Option<&[EditType]>,
    pending: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    let entry = {
        let mut map = pending.lock().await;
        map.remove(edit_id)
    };

    let Some(mut pe) = entry else {
        return Ok(format!("no pending edit with id '{edit_id}'"));
    };

    let path = path_override.unwrap_or(&pe.path);
    let symbol_name = symbol_override.unwrap_or(&pe.symbol_name);

    if let Some(types) = types_override {
        pe.edit_types = types.to_vec();
    }

    // Detect noop: caller provided path/symbol that match what's already stored
    let path_is_noop = path_override
        .map(|p| p == pe.path || pe.path.contains(p) || p.contains(&pe.path))
        .unwrap_or(false);
    let sym_is_noop = symbol_override
        .map(|s| s == pe.symbol_name)
        .unwrap_or(false);

    // For file edits
    if pe.edit_types.len() == 1 && pe.edit_types[0] == EditType::File {
        return apply_file_edit(path, &pe.new_content, undo_store, limits).await;
    }

    // Resolve with effective path/symbol
    let exact = find_symbol_exact(client, symbol_name).await?;
    let filtered: Vec<_> = if path.is_empty() || path == "." {
        exact
    } else {
        exact
            .into_iter()
            .filter(|s| {
                uri_to_path(&s.location.uri)
                    .map(|p| p.contains(path))
                    .unwrap_or(false)
            })
            .collect()
    };

    if filtered.is_empty() {
        let mut msg = format!("symbol '{symbol_name}' not found in '{path}'");

        if path_is_noop && sym_is_noop {
            msg.push_str(
                "\nnote: these were already the stored args — \
                 the call was a noop. try correcting the path or symbol.",
            );
        }

        if let Some(p) = path_override {
            pe.path = p.to_string();
        }
        if let Some(s) = symbol_override {
            pe.symbol_name = s.to_string();
        }

        let types_label: Vec<_> = pe.edit_types.iter().map(|t| t.label()).collect();
        let _ = write!(
            msg,
            "\npending edit preserved — use:\n  apply_edit {edit_id} <correct_path> <correct_symbol>\n  \
             (stored types: [{}])",
            types_label.join(" "),
        );

        {
            let mut map = pending.lock().await;
            map.insert(edit_id.to_string(), pe);
        }

        return Ok(msg);
    }

    apply_symbol_edit(
        client,
        &filtered[0],
        &pe.edit_types,
        &pe.new_content,
        pending,
        undo_store,
        true,
        limits,
    )
    .await
}

/// Generate disambiguation response with candidates.
async fn disambiguate(
    client: &Arc<LspClient>,
    edit_types: &[EditType],
    path: &str,
    symbol_name: &str,
    new_content: &str,
    search_dir: Option<&str>,
    pending: &PendingEdits,
) -> Result<String, LspClientError> {
    let candidates = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if candidates.is_empty() {
        return Ok(format!("edit failed: symbol '{symbol_name}' not found"));
    }

    let id = word_id();
    let mut candidate_list: Vec<(String, String)> = Vec::new();
    let mut output =
        format!("edit of '{symbol_name}' failed — exact match not found\ndid you mean:\n");

    for (i, sym) in candidates.iter().take(10).enumerate() {
        let sym_path = uri_to_path(&sym.location.uri).unwrap_or_default();
        let rel = make_relative(&sym_path);
        let _ = writeln!(output, "  {}. {} {}", i + 1, rel, sym.name);
        candidate_list.push((rel, sym.name.clone()));
    }

    let _ = writeln!(
        output,
        "\nto apply, use:\n  apply_edit {id} <correct_path> <correct_symbol>"
    );

    {
        let mut map = pending.lock().await;
        map.insert(
            id,
            PendingEdit {
                edit_types: edit_types.to_vec(),
                new_content: new_content.to_string(),
                path: path.to_string(),
                symbol_name: symbol_name.to_string(),
                _search_dir: search_dir.map(|s| s.to_string()),
                _candidates: candidate_list,
            },
        );
    }

    Ok(output)
}
/// Execute a targeted range edit using before/after context anchors.
pub async fn execute_edit_range(
    client: &Arc<LspClient>,
    path: &str,
    symbol_name: &str,
    before_ctx: Option<&str>,
    after_ctx: Option<&str>,
    new_content: &str,
    search_dir: Option<&str>,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    use super::edit_apply::apply_range_edit;

    let symbols = find_symbol_exact(client, symbol_name).await?;
    let filtered: Vec<_> = symbols
        .iter()
        .filter(|s| {
            let sym_path = uri_to_path(&s.location.uri).unwrap_or_default();
            sym_path.ends_with(path)
                || search_dir.map_or(false, |d| sym_path.ends_with(&format!("{d}/{path}")))
        })
        .collect();

    if filtered.is_empty() {
        let fuzzy = find_symbol_with_fallback(client, symbol_name, search_dir).await?;
        if fuzzy.is_empty() {
            return Ok(format!(
                "edit_range failed: symbol '{symbol_name}' not found"
            ));
        }
        let mut msg = format!("edit_range: '{symbol_name}' not found at '{path}'\ndid you mean:\n");
        for (i, sym) in fuzzy.iter().take(10).enumerate() {
            let sym_path = uri_to_path(&sym.location.uri).unwrap_or_default();
            let rel = make_relative(&sym_path);
            let _ = writeln!(msg, "  {}. {} {}", i + 1, rel, sym.name);
        }
        return Ok(msg);
    }

    apply_range_edit(
        client,
        filtered[0],
        before_ctx,
        after_ctx,
        new_content,
        undo_store,
        limits,
    )
    .await
}
