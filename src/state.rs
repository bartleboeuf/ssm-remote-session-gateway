use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::models::{
    AccountInfo, CachedEcsTasks, CachedInstances, CachedRdsInstances, ManagedConnection,
    ManagedTransfer, Preferences,
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
    pub transfers: Arc<Mutex<HashMap<String, ManagedTransfer>>>,
    /// Account ID and alias per profile, recorded at connect time and shown on each connection.
    pub accounts: Arc<Mutex<HashMap<String, AccountInfo>>>,
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
}
