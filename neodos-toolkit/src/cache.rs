use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::RwLock;

use crate::indexer::ParsedFile;

pub struct DocumentCache {
    files: RwLock<HashMap<PathBuf, CachedDocument>>,
    capacity: usize,
}

struct CachedDocument {
    source: String,
    last_accessed: std::time::Instant,
}

impl DocumentCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            capacity: capacity.max(2),
        }
    }

    pub fn get_source(&self, path: &PathBuf) -> Option<String> {
        let cache = self.files.read();
        cache.get(path).map(|cd| cd.source.clone())
    }

    pub fn insert(&self, path: PathBuf, source: String, _version: i64, _parsed: ParsedFile) {
        let mut cache = self.files.write();

        if cache.len() >= self.capacity && !cache.contains_key(&path)
            && let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.last_accessed)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }

        cache.insert(
            path,
            CachedDocument {
                source,
                last_accessed: std::time::Instant::now(),
            },
        );
    }

    pub fn remove(&self, path: &PathBuf) {
        self.files.write().remove(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::ParsedFile;

    #[test]
    fn test_cache_insert_and_get() {
        let cache = DocumentCache::new(10);
        let p = PathBuf::from("test.rs");

        cache.insert(p.clone(), "fn foo() {}".to_string(), 1, ParsedFile {
            symbols: vec![], references: vec![], neodos_items: vec![],
        });
        let src = cache.get_source(&p).expect("should be cached");
        assert_eq!(src, "fn foo() {}");
    }

    #[test]
    fn test_cache_update() {
        let cache = DocumentCache::new(10);
        let p = PathBuf::from("v.rs");
        cache.insert(p.clone(), "old".to_string(), 1, ParsedFile {
            symbols: vec![], references: vec![], neodos_items: vec![],
        });
        assert_eq!(cache.get_source(&p), Some("old".to_string()));
        cache.insert(p.clone(), "new".to_string(), 2, ParsedFile {
            symbols: vec![], references: vec![], neodos_items: vec![],
        });
        assert_eq!(cache.get_source(&p), Some("new".to_string()));
    }
}
