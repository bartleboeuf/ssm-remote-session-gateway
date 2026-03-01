use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

use crate::models::{CachedInstances, ManagedConnection, Preferences};

#[derive(Clone)]
pub struct AppState {
    pub prefs: Arc<RwLock<Preferences>>,
    pub connections: Arc<Mutex<HashMap<uuid::Uuid, ManagedConnection>>>,
    pub instance_cache: Arc<Mutex<HashMap<String, CachedInstances>>>,
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
}
