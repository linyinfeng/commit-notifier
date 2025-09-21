use std::sync::LazyLock;

use deadpool_sqlite::Pool;
use git2::Repository;
use tokio::sync::{Mutex, RwLock};

use crate::{
    error::Error,
    repo::{cache, paths::RepoPaths, settings::RepoSettings},
    resources::{Resource, ResourcesMap},
    utils::{read_json, write_json},
};

pub static RESOURCES_MAP: LazyLock<ResourcesMap<String, RepoResources>> =
    LazyLock::new(ResourcesMap::new);

pub struct RepoResources {
    pub name: String,
    pub paths: RepoPaths,
    pub repo: Mutex<Repository>,
    pub cache: Pool,
    pub settings: RwLock<RepoSettings>,
}

impl Resource<String> for RepoResources {
    async fn open(name: &String) -> Result<Self, Error> {
        let paths = RepoPaths::new(name);
        if !paths.outer.is_dir() {
            return Err(Error::UnknownRepository(name.to_string()));
        }
        // load repo
        let repo = Mutex::new(Repository::open(&paths.repo)?);
        // load cache
        let cache_exists = paths.cache.is_file();
        let cache_cfg = deadpool_sqlite::Config::new(&paths.cache);
        let cache = cache_cfg.create_pool(deadpool_sqlite::Runtime::Tokio1)?;
        if !cache_exists {
            log::debug!("initializing cache for {name}...");
            let conn = cache.get().await?;
            conn.interact(|c| cache::initialize(c))
                .await
                .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
        }
        // load settings
        let settings = RwLock::new(read_json(&paths.settings)?);

        Ok(Self {
            name: name.clone(),
            paths,
            repo,
            cache,
            settings,
        })
    }
}

impl RepoResources {
    pub async fn save_settings(&self) -> Result<(), Error> {
        let in_mem = self.settings.read().await;
        write_json(&self.paths.settings, &*in_mem)
    }

    pub async fn cache(&self) -> Result<deadpool_sqlite::Object, Error> {
        Ok(self.cache.get().await?)
    }
}
