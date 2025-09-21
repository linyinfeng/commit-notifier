use std::{
    collections::BTreeMap,
    fmt::{self},
    sync::Arc,
};

use tokio::{sync::Mutex, time::sleep};

use crate::error::Error;

#[derive(Default)]
pub struct ResourcesMap<I, R> {
    pub map: Mutex<BTreeMap<I, Arc<R>>>,
}

pub trait Resource<I>
where
    Self: Sized,
{
    async fn open(index: &I) -> Result<Self, Error>;
}

impl<I, R> ResourcesMap<I, R> {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn get(&self, index: &I) -> Result<Arc<R>, Error>
    where
        R: Resource<I>,
        I: Ord + Clone,
    {
        let mut map = self.map.lock().await;
        match map.get(index) {
            Some(resources) => Ok(resources.clone()),
            None => {
                let resources = Arc::new(R::open(index).await?);
                map.insert(index.clone(), resources.clone());
                Ok(resources)
            }
        }
    }

    pub async fn remove<C, F>(&self, index: &I, cleanup: C) -> Result<(), Error>
    where
        I: Ord + Clone + fmt::Display + fmt::Debug,
        C: FnOnce(Arc<R>) -> F,
        F: Future<Output = Result<(), Error>>,
    {
        let mut map = self.map.lock().await;
        if let Some(arc) = map.remove(index) {
            wait_for_resources_drop(index, arc.clone()).await;
            cleanup(arc).await?; // run before the map unlock
            Ok(())
        } else {
            Err(Error::UnknownResource(format!("{index}")))
        }
    }

    pub async fn clear(&self) -> Result<(), Error>
    where
        I: Ord + fmt::Display,
    {
        let mut map = self.map.lock().await;
        while let Some((task, resources)) = map.pop_first() {
            wait_for_resources_drop(&task, resources).await;
        }
        Ok(())
    }
}

pub async fn wait_for_resources_drop<I, R>(index: &I, mut arc: Arc<R>)
where
    I: fmt::Display,
{
    loop {
        match Arc::try_unwrap(arc) {
            Ok(_resource) => {
                // do nothing
                // just drop
                break;
            }
            Err(a) => {
                arc = a;
                log::info!("removing {}, waiting for existing jobs", index);
                sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}
