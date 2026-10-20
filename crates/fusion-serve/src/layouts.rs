//! Where layouts are stored.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Stores layouts by name. The server calls it on its own task for each
/// request.
pub trait LayoutStore: Clone + Send + Sync + 'static {
    /// Stores a layout, replacing one of the same name.
    fn save(&self, name: String, json: String) -> impl Future<Output = Result<(), String>> + Send;
    /// The layout stored under a name, if any.
    fn load(&self, name: String) -> impl Future<Output = Result<Option<String>, String>> + Send;
}

/// Layouts in memory: gone when the server stops.
#[derive(Debug, Clone, Default)]
pub struct MemoryLayouts {
    layouts: Arc<Mutex<BTreeMap<String, String>>>,
}

impl LayoutStore for MemoryLayouts {
    fn save(&self, name: String, json: String) -> impl Future<Output = Result<(), String>> + Send {
        let saved = self
            .layouts
            .lock()
            .map(|mut layouts| {
                layouts.insert(name, json);
            })
            .map_err(|_| "the layout store failed".to_owned());
        async move { saved }
    }

    fn load(&self, name: String) -> impl Future<Output = Result<Option<String>, String>> + Send {
        let loaded = self
            .layouts
            .lock()
            .map(|layouts| layouts.get(&name).cloned())
            .map_err(|_| "the layout store failed".to_owned());
        async move { loaded }
    }
}
