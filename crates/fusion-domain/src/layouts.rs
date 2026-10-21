//! Where layouts are stored: a repository trait (a port), and a store in
//! memory. `fusion-persistence-pg` keeps them in PostgreSQL.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Stores layouts by name, as JSON. The view server calls it on its own
/// task for each request.
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

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use super::*;

    /// The value of a future that is ready at once, as the memory store's are.
    fn ready<T>(future: impl Future<Output = T>) -> T {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the memory store never waits"),
        }
    }

    #[test]
    fn a_saved_layout_loads_and_a_new_one_replaces_it() {
        let store = MemoryLayouts::default();
        assert_eq!(ready(store.load("main".into())), Ok(None));
        assert_eq!(ready(store.save("main".into(), "{}".into())), Ok(()));
        let copy = store.clone();
        assert_eq!(ready(copy.save("main".into(), "{\"a\":1}".into())), Ok(()));
        assert_eq!(
            ready(store.load("main".into())),
            Ok(Some("{\"a\":1}".to_owned())),
            "clones share the layouts"
        );
        assert_eq!(ready(store.load("other".into())), Ok(None));
    }
}
