use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::models::{
    CachedEcsTasks, CachedInstances, CachedRdsInstances, ManagedConnection, Preferences,
};

#[derive(Clone)]
pub struct AppState {
    pub prefs: Arc<RwLock<Preferences>>,
    pub connections: Arc<Mutex<HashMap<uuid::Uuid, ManagedConnection>>>,
    pub instance_cache: Arc<Mutex<HashMap<String, CachedInstances>>>,
    pub instance_refreshes: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    pub rds_cache: Arc<Mutex<HashMap<String, CachedRdsInstances>>>,
    pub rds_refreshes: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    pub ecs_cache: Arc<Mutex<HashMap<String, CachedEcsTasks>>>,
    pub ecs_refreshes: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
}
