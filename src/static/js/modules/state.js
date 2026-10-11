// state.js: state feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const state = {
// State
    is_connected: false,

refresh_interval: null,

// 
    refresh_countdown: 30,

current_profile: '',

current_region: '',
    connection_scope: 'scope', // 'scope' shows the selected profile/region; 'all' shows every connection
    total_connections: 0,

instances: [],

connections: [],

aws_account_id: null,

// Add AWS account ID state
    aws_account_alias: null,

// Add AWS account alias state
    filter_active_only: false,

current_transfer_id: null,

transfer_poll_interval: null,

file_transfer_instance_id: null,

// Cached DOM elements
    elements: {},

// Bootstrap components
    modals: {},

toasts: {},

preferences: {
        startPort: null,  // Will be loaded from backend (OS-specific defaults)
        endPort: null,    // Will be loaded from backend (OS-specific defaults)
        logLevel: 'INFO',
        ssh_key_folder: null,
        ssh_options: '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null',
        cache_ttl_secs: 3600,
        linux_username: 'ec2-user',
        windows_username: 'Administrator'
    }
};
