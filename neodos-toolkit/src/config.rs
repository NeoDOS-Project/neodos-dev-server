use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct NeodosConfig {
    pub workspace: WorkspaceConfig,
    pub cache: CacheConfig,
    pub indexing: IndexingConfig,
}

#[derive(Debug)]
pub struct WorkspaceConfig {
    pub roots: parking_lot::RwLock<Vec<PathBuf>>,
    pub max_files: usize,
    pub exclude_patterns: Vec<String>,
    pub watch_enabled: bool,
}

impl Clone for WorkspaceConfig {
    fn clone(&self) -> Self {
        Self {
            roots: parking_lot::RwLock::new(self.roots.read().clone()),
            max_files: self.max_files,
            exclude_patterns: self.exclude_patterns.clone(),
            watch_enabled: self.watch_enabled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub documents: usize,
}

#[derive(Debug, Clone)]
pub struct IndexingConfig {
    pub strategy: IndexingStrategy,
    pub threads: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingStrategy {
    Incremental,
}

impl Default for NeodosConfig {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig {
                roots: parking_lot::RwLock::new(vec![PathBuf::from(".")]),
                max_files: 10000,
                exclude_patterns: vec![
                    "target/**".into(),
                    ".git/**".into(),
                    "**/*.bin".into(),
                    "**/*.elf".into(),
                    "**/*.efi".into(),
                    "**/*.img".into(),
                    "**/*.vdi".into(),
                    "**/*.log".into(),
                ],
                watch_enabled: true,
            },
            cache: CacheConfig {
                documents: 256,
            },
            indexing: IndexingConfig {
                strategy: IndexingStrategy::Incremental,
                threads: 0,
            },
        }
    }
}

impl NeodosConfig {
    pub fn detection() -> Self {
        let mut cfg = Self::default();

        if let Ok(val) = std::env::var("NEODOS_LSP_ROOT") {
            *cfg.workspace.roots.write() = vec![PathBuf::from(val)];
        }
        if let Ok(val) = std::env::var("NEODOS_TOOLKIT_MAX_FILES")
            && let Ok(n) = val.parse() {
                cfg.workspace.max_files = n;
            }
        if let Ok(val) = std::env::var("NEODOS_TOOLKIT_CACHE_SIZE")
            && let Ok(n) = val.parse() {
                cfg.cache.documents = n;
            }
        if let Ok(val) = std::env::var("NEODOS_TOOLKIT_THREADS")
            && let Ok(n) = val.parse() {
                cfg.indexing.threads = n;
            }
        if let Ok(val) = std::env::var("NEODOS_TOOLKIT_WATCH") {
            cfg.workspace.watch_enabled = val == "1" || val.eq_ignore_ascii_case("true");
        }

        cfg
    }
}
