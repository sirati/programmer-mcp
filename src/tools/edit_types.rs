//! Types and storage for the edit system.
//!
//! Contains `EditType`, `PendingEdit`, `UndoEntry`, and `BoundedStore`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::Mutex;

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
    pub _candidates: Vec<(String, String)>,
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
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        }
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
    pub old_content: String,
    pub new_content: String,
}

/// Storage for undo entries keyed by word ID.
pub type UndoStore = Arc<Mutex<BoundedStore<UndoEntry>>>;

pub fn new_undo_store() -> UndoStore {
    Arc::new(Mutex::new(BoundedStore::new(STORE_CAPACITY)))
}
