// Main application module
//app.js
const app = {
    // State
    is_connected: false,
    refresh_interval: null,  // 
    refresh_countdown: 30,
    current_profile: '',
    current_region: '',
    instances: [],
    connections: [],
    aws_account_id: null,  // Add AWS account ID state
    aws_account_alias: null,  // Add AWS account alias state 
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
        cache_ttl_secs: 3600
    },
    //theme: 'light',
    // Initialize application
    async init() {
        console.log('Initializing application...');
        try {
            this.cache_elements();
            this.initialize_components();
            this.setup_event_listeners();
            await this.load_profiles_and_regions();
            // Health check on init
            try {
                const healthRes = await fetch('/api/health');
                if (healthRes.ok) {
                    const health = await healthRes.json();
                    const missing = [];
                    // if (!health.aws_cli) missing.push('AWS CLI'); // AWS CLI is not required for now
                    if (!health.session_manager_plugin) missing.push('Session Manager Plugin');
                    if (missing.length > 0) {
                        // Show persistent, non-dismissible warning banner
                        const banner = document.createElement('div');
                        banner.id = 'prerequisiteBanner';
                        banner.style.cssText = 'background:#d9534f;color:#fff;padding:10px 20px;text-align:center;font-weight:600;position:fixed;top:0;left:0;right:0;z-index:9999;';
                        banner.textContent = `⚠ Missing required tools: ${missing.join(', ')}. SSM sessions will not work.`;
                        document.body.prepend(banner);
                        // Also add padding to body so content is not hidden behind the banner
                        document.body.style.paddingTop = `${banner.offsetHeight}px`;
                    }
                    if (!health.aws_credentials) app.show_toast('No AWS credentials detected. Configure your profile.', 'warning');
                    console.log('Health:', health);
                }
            } catch (e) {
                console.warn('Health check failed:', e);
            }

            this.start_connection_monitoring();
            this.setup_shutdown_handlers();
            console.log('Application initialized successfully');
        } catch (error) {
            console.error('Error initializing application:', error);
        }

    },

    // Cache DOM elements for better performance
    cache_elements() {
        console.log('Caching DOM elements...');
        this.elements = {
            profileSelect: document.getElementById('profileSelect'),
            refreshProfilesBtn: document.getElementById('refreshProfilesBtn'),
            regionSelect: document.getElementById('regionSelect'),
            connectBtn: document.getElementById('connectBtn'),
            refreshBtn: document.getElementById('refreshBtn'),
            autoRefreshSwitch: document.getElementById('autoRefreshSwitch'),
            refreshTimer: document.getElementById('refreshTimer'),
            instancesList: document.getElementById('instancesList'),
            instanceStateFilter: document.getElementById('instanceStateFilter'),
            instanceOsFilter: document.getElementById('instanceOsFilter'),
            connectionsList: document.getElementById('connectionsList'),
            instanceCount: document.getElementById('instanceCount'),
            connectionCount: document.getElementById('connectionCount'),
            loadingOverlay: document.getElementById('loadingOverlay')  // 
        };
    },


    initialize_components() {
        // Initialize modals
        console.log('Initializing Bootstrap components...');
        if (typeof bootstrap === 'undefined') {
            throw new Error('Bootstrap is not loaded. Please check your dependencies.');
        }

        // Check for required modal elements
        const instanceDetailsModal = document.getElementById('instanceDetailsModal');
        const customPortModal = document.getElementById('customPortModal');
        const preferencesModal = document.getElementById('preferencesModal');
        const quitModal = document.getElementById('quitModal');

        // Initialize instance details modal
        if (instanceDetailsModal) {
            this.modals.instanceDetails = new bootstrap.Modal(instanceDetailsModal);
        } else {
            console.warn('Instance details modal element not found');
        }

        // Initialize custom port modal
        if (customPortModal) {
            this.modals.customPort = new bootstrap.Modal(customPortModal);
        } else {
            console.warn('Custom port modal element not found');
        }

        // Setup keyboard shortcuts
        this.setup_keyboard_shortcuts();

        // Initialize preferences modal
        if (preferencesModal) {
            this.modals.preferences = new bootstrap.Modal(preferencesModal);
        } else {
            console.warn('Preferences modal element not found');
        }
        if (quitModal) {
            this.modals.quit = new bootstrap.Modal(quitModal);
        }

        // Setup preferences button
        const preferencesBtn = document.getElementById('preferencesBtn');
        if (preferencesBtn) {
            preferencesBtn.onclick = () => this.show_preferences();
        } else {
            console.warn('Preferences button not found');
        }

        // Filters change handlers (state + OS)
        const stateFilter = this.elements.instanceStateFilter;
        const osFilter = this.elements.instanceOsFilter;
        const handleFilters = async () => {
            const stateVal = stateFilter ? stateFilter.value : '';
            const osVal = osFilter ? osFilter.value : '';
            console.log(`[Filters] State: ${stateVal || 'all'}, OS: ${osVal || 'all'}`);
            await this.load_instances(stateVal || null, osVal || '');
        };
        if (stateFilter) stateFilter.onchange = handleFilters;
        if (osFilter) osFilter.onchange = handleFilters;

        // Name filter (regex)
        const nameFilter = document.getElementById('instanceFilter');
        if (nameFilter) {
            nameFilter.oninput = this.debounce(() => {
                this.render_instances();
            }, 200);
        }

        // Quit button
        const quitBtn = document.getElementById('quitBtn');
        if (quitBtn) {
            quitBtn.onclick = () => this.show_quit_confirm();
        }
        const confirmQuitBtn = document.getElementById('confirmQuitBtn');
        if (confirmQuitBtn) {
            confirmQuitBtn.onclick = () => this.quit_application();
        }

        // Theme toggle
        // Theme toggle disabled

        const aboutModal = document.getElementById('aboutModal');
        if (aboutModal) {
            this.modals.about = new bootstrap.Modal(aboutModal);
        } else {
            console.warn('About modal element not found');
        }

        // Setup about button
        const aboutBtn = document.getElementById('aboutBtn');
        if (aboutBtn) {
            aboutBtn.onclick = () => this.show_about();
        } else {
            console.warn('About button not found');
        }


        // Initialize tooltips
        const tooltips = document.querySelectorAll('[data-bs-toggle="tooltip"]');
        tooltips.forEach(tooltip => {
            try {
                new bootstrap.Tooltip(tooltip);
            } catch (error) {
                console.warn('Failed to initialize tooltip:', error);
            }
        });

        // Setup port forwarding button
        const startPortForwardingBtn = document.getElementById('startPortForwardingBtn');
        if (startPortForwardingBtn) {
            startPortForwardingBtn.onclick = () => this.start_custom_port_forwarding();
        }

        // Setup preferences menu items
        const preferencesMenuItem = document.getElementById('preferencesMenuItem');
        if (preferencesMenuItem) {
            preferencesMenuItem.onclick = () => this.show_preferences();
        } else {
            console.warn('Preferences menu item not found');
        }

        // Setup preferences save button
        const savePreferencesBtn = document.getElementById('savePreferencesBtn');
        if (savePreferencesBtn) {
            savePreferencesBtn.onclick = () => this.save_preferences();
        } else {
            console.warn('Save preferences button not found');
        }

        // Load saved preferences
        this.load_preferences();

        // Setup SSH key folders management
        this.setup_ssh_key_folders();
    },


    // Setup SSH key folders management
    setup_ssh_key_folders() {
        const container = document.getElementById('sshKeyFoldersContainer');
        const addBtn = document.getElementById('addSshKeyFolderBtn');
        const browseBtn = document.getElementById('browseSshKeyFolderBtn');
        const fileInput = document.getElementById('sshKeyFolderFileInput');

        if (!container) return;

        // Add folder button handler
        if (addBtn) {
            addBtn.onclick = () => this.add_ssh_key_folder_input('');
        }

        // Browse folder button handler
        if (browseBtn && fileInput) {
            browseBtn.onclick = () => fileInput.click();

            fileInput.onchange = (event) => {
                const files = event.target.files;
                if (files && files.length > 0) {
                    const firstFile = files[0];
                    let folderPath = null;

                    // Try to get folder path from file
                    if (firstFile.webkitRelativePath) {
                        const pathParts = firstFile.webkitRelativePath.split('/');
                        if (pathParts.length > 1) {
                            pathParts.pop();
                            const folderName = pathParts[0];
                            if (folderName === '.ssh' || folderName.includes('ssh') || folderName.includes('key')) {
                                folderPath = '~/.ssh';
                            } else {
                                folderPath = `~/${folderName}`;
                            }
                        }
                    }

                    if (firstFile.path) {
                        const path = firstFile.path;
                        const lastSlashIndex = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
                        if (lastSlashIndex > 0) {
                            folderPath = path.substring(0, lastSlashIndex).replace(/\\/g, '/');
                        }
                    }

                    if (!folderPath) {
                        folderPath = '~/.ssh';
                        this.show_toast('Folder selected. Please verify the path and adjust if needed.', 'info');
                    }

                    // Add new folder input with the selected path
                    this.add_ssh_key_folder_input(folderPath);
                }

                // Reset the input
                event.target.value = '';
            };
        }

        // Initialize with at least one empty folder input if container is empty
        if (container.children.length === 0) {
            this.add_ssh_key_folder_input('');
        }
    },

    show_quit_confirm() {
        if (this.modals.quit) {
            this.modals.quit.show();
        } else {
            if (confirm('Quit EC2 Session Gate? This will terminate connections and close the tab.')) {
                this.quit_application();
            }
        }
    },

    async quit_application() {
        try {
            // Ask backend to shutdown gracefully (will also terminate connections)
            await fetch('/api/quit', { method: 'POST' });
        } catch (e) {
            console.warn('Failed to terminate connections on quit:', e);
        }
        // Attempt to close the tab; if blocked, navigate to about:blank
        window.close();
        setTimeout(() => { window.location.href = 'about:blank'; }, 300);
    },

    // Add a new SSH key folder input
    add_ssh_key_folder_input(value = '') {
        const container = document.getElementById('sshKeyFoldersContainer');
        if (!container) return;

        const folderId = `sshKeyFolder_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
        const folderDiv = document.createElement('div');
        folderDiv.className = 'input-group mb-2';
        folderDiv.id = `folderGroup_${folderId}`;

        folderDiv.innerHTML = `
            <input type="text" 
                   class="form-control" 
                   id="${folderId}"
                   placeholder="e.g., ~/.ssh or /path/to/keys"
                   value="${value.replace(/"/g, '&quot;')}">
            <button type="button" 
                    class="btn btn-outline-danger" 
                    onclick="app.remove_ssh_key_folder_input('${folderId}')"
                    title="Remove this folder">
                <i class="bi bi-trash"></i>
            </button>
        `;

        container.appendChild(folderDiv);

        // Focus on the new input
        const input = document.getElementById(folderId);
        if (input) {
            input.focus();
        }
    },

    // Remove an SSH key folder input
    remove_ssh_key_folder_input(folderId) {
        const input = document.getElementById(folderId);
        if (!input) return;

        const folderGroup = input.closest('.input-group');
        if (folderGroup) {
            folderGroup.remove();
        }

        // Ensure at least one folder input remains
        const container = document.getElementById('sshKeyFoldersContainer');
        if (container && container.children.length === 0) {
            this.add_ssh_key_folder_input('');
        }
    },

    // Get all SSH key folder values
    get_ssh_key_folders() {
        const container = document.getElementById('sshKeyFoldersContainer');
        if (!container) return [];

        const folders = [];
        const inputs = container.querySelectorAll('input[type="text"]');
        inputs.forEach(input => {
            const value = input.value.trim();
            if (value) {
                folders.push(value);
            }
        });

        return folders;
    },

    // Load SSH key folders into the UI
    load_ssh_key_folders(folders) {
        const container = document.getElementById('sshKeyFoldersContainer');
        if (!container) return;

        // Clear existing inputs
        container.innerHTML = '';

        if (folders && folders.length > 0) {
            // Parse folders (could be comma-separated or newline-separated)
            const folderList = typeof folders === 'string'
                ? folders.split(/[,\n]/).map(f => f.trim()).filter(f => f)
                : folders;

            folderList.forEach(folder => {
                this.add_ssh_key_folder_input(folder);
            });
        } else {
            // Add one empty input
            this.add_ssh_key_folder_input('');
        }
    },

    // Setup event listeners
    setup_event_listeners() {
        console.log('Setting up event listeners...');
        if (this.elements.refreshProfilesBtn) {
            this.elements.refreshProfilesBtn.onclick = async () => {
                const currentProfile = this.elements.profileSelect.value;
                const currentRegion = this.elements.regionSelect.value;
                this.elements.refreshProfilesBtn.disabled = true;
                try {
                    await this.load_profiles_and_regions();
                    if (currentProfile) this.elements.profileSelect.value = currentProfile;
                    if (currentRegion) this.elements.regionSelect.value = currentRegion;
                    if (this.show_toast) this.show_toast('Profiles list refreshed', 'success');
                } finally {
                    this.elements.refreshProfilesBtn.disabled = false;
                }
            };
        }
        this.elements.connectBtn.onclick = () => this.toggle_connection();
        this.elements.refreshBtn.onclick = () => this.refresh_data();
        this.elements.autoRefreshSwitch.onchange = (e) => this.toggle_auto_refresh(e);
    },

    // Setup shutdown handlers to cleanup connections when app closes
    setup_shutdown_handlers() {
        console.log('Setting up shutdown handlers...');

        let cleanupInProgress = false;

        const terminateAllConnections = () => {
            if (cleanupInProgress || this.connections.length === 0) {
                return;
            }
            cleanupInProgress = true;

            console.log(`Terminating ${this.connections.length} connections...`);

            // Use sendBeacon for reliable delivery during page unload
            // sendBeacon is more reliable than fetch during unload
            const terminateAllUrl = '/api/terminate-all-connections';
            if (navigator.sendBeacon) {
                try {
                    const blob = new Blob([JSON.stringify({})], { type: 'application/json' });
                    navigator.sendBeacon(terminateAllUrl, blob);
                    console.log('Sent terminate-all request via sendBeacon');
                } catch (e) {
                    console.warn('sendBeacon failed, trying fetch:', e);
                }
            }

            // Also try fetch with keepalive as fallback
            try {
                fetch(terminateAllUrl, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({}),
                    keepalive: true
                }).catch(() => {
                    // Ignore errors during unload
                });
            } catch (e) {
                // Ignore errors during unload
            }
        };

        // Handle browser/tab close-use both beforeunload and unload
        window.addEventListener('beforeunload', (event) => {
            terminateAllConnections();
        });

        window.addEventListener('unload', (event) => {
            terminateAllConnections();
        });

        // Handle pagehide (more reliable than unload in some browsers)
        window.addEventListener('pagehide', (event) => {
            terminateAllConnections();
        });

        // Handle visibility change (when tab becomes hidden)
        document.addEventListener('visibilitychange', () => {
            if (document.hidden && this.connections.length > 0) {
                // Tab is hidden, but don't terminate connections yet
                // Connections will be cleaned up on actual page unload
            }
        });
    },

    // Load profiles and regions

    // Load profiles and regions dynamically
    async load_profiles_and_regions() {
        console.log('[Profile Loading] Starting to load profiles and regions...');
        try {
            // Load saved profile and region from backend preferences
            let savedProfile = '';
            let savedRegion = '';
            try {
                const lastConnRes = await fetch('/api/last-connection');
                if (lastConnRes.ok) {
                    const lastConn = await lastConnRes.json();
                    savedProfile = lastConn.profile || '';
                    savedRegion = lastConn.region || '';
                    console.log('[Profile Loading] Loaded saved preferences from backend:', { profile: savedProfile, region: savedRegion });
                }
            } catch (e) {
                console.warn('[Profile Loading] Failed to load last connection from backend:', e);
            }

            const profilesRes = await fetch('/api/profiles');
            if (!profilesRes.ok) throw new Error(`Failed to load profiles: ${profilesRes.status}`);
            const profiles = await profilesRes.json();
            console.log('[Profile Loading] Loaded profiles:', profiles);

            // Update profile dropdown
            if (Array.isArray(profiles)) {
                this.updateSelect(this.elements.profileSelect, profiles);

                // Restore saved profile if it exists and is valid
                if (savedProfile && profiles.includes(savedProfile)) {
                    this.elements.profileSelect.value = savedProfile;
                    this.current_profile = savedProfile;
                    console.log('[Profile Loading] Restored profile from backend:', savedProfile);
                }
            } else {
                console.error('[Profile Loading] Invalid profiles data:', profiles);
            }

            // Load regions for the selected profile (saved or default)
            const profileToUse = savedProfile && Array.isArray(profiles) && profiles.includes(savedProfile)
                ? savedProfile
                : (Array.isArray(profiles) && profiles.length > 0 ? profiles[0] : 'default');

            try {
                const regionsRes = await fetch(`/api/regions?profile=${encodeURIComponent(profileToUse)}`);
                const regionsData = await regionsRes.json();

                if (regionsRes.ok && regionsData.ok && Array.isArray(regionsData.data)) {
                    this.updateSelect(this.elements.regionSelect, regionsData.data);

                    // Restore saved region if it exists and is valid
                    if (savedRegion && regionsData.data.includes(savedRegion)) {
                        this.elements.regionSelect.value = savedRegion;
                        this.current_region = savedRegion;
                        console.log('[Profile Loading] Restored region from backend:', savedRegion);
                    }
                } else {
                    // Fallback to default regions
                    const defaultRegionsRes = await fetch('/api/regions');
                    if (defaultRegionsRes.ok) {
                        const defaultRegionsJson = await defaultRegionsRes.json();
                        if (defaultRegionsJson.ok && Array.isArray(defaultRegionsJson.data)) {
                            this.updateSelect(this.elements.regionSelect, defaultRegionsJson.data);
                            if (savedRegion && defaultRegionsJson.data.includes(savedRegion)) {
                                this.elements.regionSelect.value = savedRegion;
                                this.current_region = savedRegion;
                            }
                        }
                    }
                }
            } catch (error) {
                console.error('[Profile Loading] Error loading regions:', error);
                this.show_toast('Could not load regions. You may need to configure AWS credentials.', 'warning');
            }

            //  Listen for profile change and dynamically fetch enabled regions
            this.elements.profileSelect.addEventListener('change', async (e) => {
                const profile = e.target.value;
                if (!profile) return;

                // Disable region dropdown while loading
                if (this.elements.regionSelect) {
                    this.elements.regionSelect.disabled = true;
                }

                try {
                    console.log(`[Profile] Selected: ${profile}`);
                    const res = await fetch(`/api/regions?profile=${encodeURIComponent(profile)}`);
                    if (!res.ok) throw new Error(`Failed to fetch regions for profile ${profile}`);
                    const data = await res.json();

                    if (data.ok) {
                        const regionList = data.data;
                        console.log(`[Regions] Enabled regions for ${profile}:`, regionList);
                        this.updateSelect(this.elements.regionSelect, regionList);

                        // Try to restore saved region from backend if it's valid for this profile
                        try {
                            const lastConnRes = await fetch('/api/last-connection');
                            if (lastConnRes.ok) {
                                const lastConn = await lastConnRes.json();
                                const savedRegionForProfile = lastConn.region || '';
                                if (savedRegionForProfile && regionList.includes(savedRegionForProfile)) {
                                    this.elements.regionSelect.value = savedRegionForProfile;
                                    this.current_region = savedRegionForProfile;
                                    console.log(`[Profile Change] Restored saved region from backend: ${savedRegionForProfile}`);
                                }
                            }
                        } catch (e) {
                            console.warn('[Profile Change] Failed to load last connection:', e);
                        }

                        this.show_toast(`Regions updated for profile ${profile}`, 'info');
                    } else {
                        throw new Error(data.error || 'Unknown error fetching regions');
                    }
                } catch (err) {
                    console.error(`[Regions] Error for profile ${profile}:`, err);
                    this.show_error(`Could not load regions for profile ${profile}`);
                } finally {
                    // Re-enable region dropdown
                    if (this.elements.regionSelect) {
                        this.elements.regionSelect.disabled = false;
                    }
                }

                // Refresh active connections when profile changes
                if (typeof this.render_connections === 'function') {
                    this.render_connections();
                    this.update_counters();
                }
            });

            // Refresh active connections when region changes
            this.elements.regionSelect.addEventListener('change', async () => {
                if (this.is_connected) {
                    await this.disconnect();
                    await this.toggle_connection();
                } else {
                    if (typeof this.render_connections === 'function') {
                        this.render_connections();
                        this.update_counters();
                    }
                }
            });

        } catch (error) {
            console.error('[Profile Loading] Error loading profiles and regions:', error);
            this.show_error('Failed to load profiles and regions: ' + error.message);
        }
    },


    // Update select element with options
    updateSelect(select, options) {
        if (!select || !options) return;
        console.log(`Updating select ${select.id} with options:`, options);

        select.innerHTML = `<option value="">Select ${select.id.replace('Select', '')}</option>`;
        options.forEach(option => {
            const opt = document.createElement('option');
            opt.value = option;
            opt.textContent = option;
            select.appendChild(opt);
        });
    },

    // Toggle connection state
    async toggle_connection() {
        if (this.is_connected) {
            if (!confirm('Are you sure you want to disconnect?')) return;
            this.disconnect();
            return;
        }

        const profile = this.elements.profileSelect.value;
        const region = this.elements.regionSelect.value;

        if (!profile || !region) {
            this.show_error('Please select both profile and region');
            return;
        }

        try {
            this.show_loading();
            const response = await fetch('/api/connect', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ profile, region })
            });

            // Get the response data first to check for errors
            const result = await response.json();

            if (!response.ok) {
                // Extract error message from response
                const errorMsg = result.error || result.status === 'error' ? result.error : 'Connection failed';
                throw new Error(errorMsg);
            }

            if (result.status === 'success') {
                this.is_connected = true;
                this.current_profile = profile;
                this.current_region = region;
                this.aws_account_id = result.account_id;  // Store account ID
                this.aws_account_alias = result.account_alias || null;  // Store account alias if available
                // Update UI with account ID and alias
                this.update_aws_account_display();

                this.elements.connectBtn.innerHTML = '<i class="bi bi-plug fs-5"></i> Disconnect';
                this.elements.connectBtn.classList.replace('btn-success', 'btn-danger');

                // Note: Profile and region are saved to backend preferences by the /api/connect endpoint

                const initialState = this.elements.instanceStateFilter?.value || null;
                const initialOs = this.elements.instanceOsFilter?.value || '';
                await this.load_instances(initialState, initialOs);
                this.show_success('Connected successfully');
            } else {
                throw new Error(result.error || 'Connection failed');
            }
        } catch (error) {
            this.show_error('Connection error: ' + error.message);
        } finally {
            this.hide_loading();
        }
    },

    // function to update AWS account display
    update_aws_account_display() {
        const accountContainer = document.getElementById('accountIdContainer');
        const accountId = document.getElementById('awsAccountId');

        if (this.is_connected && this.aws_account_id && accountContainer && accountId) {
            // Build display text with account ID and alias (if available)
            let displayText = this.aws_account_id;
            if (this.aws_account_alias) {
                displayText = `${this.aws_account_alias} (${this.aws_account_id})`;
            }
            accountId.textContent = displayText;
            // Show the container
            accountContainer.classList.remove('d-none');
        } else if (accountContainer && accountId) {
            // Hide when disconnected
            accountContainer.classList.add('d-none');
            accountId.textContent = '';
        }
    },

    // Disconnect from AWS
    async disconnect() {
        // Terminate all active SSM sessions before resetting UI
        try {
            await fetch('/api/terminate-all-connections', { method: 'POST' });
        } catch (e) {
            console.warn('Failed to terminate connections:', e);
        }

        this.is_connected = false;
        this.current_profile = '';
        this.current_region = '';
        this.aws_account_id = null; // Clear account ID
        this.aws_account_alias = null; // Clear account alias
        this.instances = [];
        this.connections = [];

        this.update_aws_account_display();
        this.elements.connectBtn.innerHTML = '<i class="bi bi-plug"></i> Connect';
        this.elements.connectBtn.classList.replace('btn-danger', 'btn-success');
        this.elements.instancesList.innerHTML = '';
        this.elements.connectionsList.innerHTML = '';
        this.update_counters();

        if (this.auto_refresh_interval) {
            this.toggle_auto_refresh({ target: { checked: false } });
        }

        this.show_success('Disconnected successfully');
    },

    // Load instances
    async load_instances(filterState = 'running', filterOs = '') {
        if (!this.is_connected) return;
        this.current_filters = { state: filterState || '', os: filterOs || '' };

        const loadingState = document.getElementById('instancesLoadingState');
        const emptyState = document.getElementById('instancesEmptyState');

        try {
            // Show loading state
            if (loadingState) {
                loadingState.classList.remove('d-none');
                // Update loading message if filtering
                const loadingText = loadingState.querySelector('p');
                if (loadingText) {
                    loadingText.textContent = filterState
                        ? `Loading ${filterState} instances...`
                        : 'Loading instances...';
                }
            }
            if (emptyState) emptyState.classList.add('d-none');

            // Build URL with optional filters
            let url = '/api/instances';
            const params = new URLSearchParams();
            if (filterState && filterState !== 'all') params.set('filter_state', filterState);
            if (filterOs && filterOs !== 'all') params.set('filter_os', filterOs);
            const qs = params.toString();
            if (qs) url += `?${qs}`;

            const response = await fetch(url);
            if (!response.ok) throw new Error('Failed to load instances');

            this.instances = await response.json();
            // Client-side OS filter as a safeguard (backend may also filter)
            if (filterOs) {
                this.instances = this.instances.filter(i => i.os === filterOs);
            }
            this.render_instances();
            this.update_counters();
        } catch (error) {
            this.show_error('Failed to load instances: ' + error.message);
            if (loadingState) loadingState.classList.add('d-none');
        }
    },

    // Render instances list
    render_instances() {
        const instancesList = this.elements.instancesList;
        const emptyState = document.getElementById('instancesEmptyState');
        const loadingState = document.getElementById('instancesLoadingState');

        instancesList.innerHTML = '';

        // Hide loading state
        if (loadingState) loadingState.classList.add('d-none');

        if (this.instances.length === 0) {
            // Show empty state
            if (emptyState) emptyState.classList.remove('d-none');
        } else {
            // Hide empty state and render instances
            if (emptyState) emptyState.classList.add('d-none');

            // Apply name filter (regex) locally on the already filtered/loaded instances
            const nameFilter = document.getElementById('instanceFilter');
            const query = nameFilter ? nameFilter.value.trim() : '';
            let filtered = this.instances;

            if (query) {
                try {
                    const regex = new RegExp(query, 'i');
                    filtered = this.instances.filter(inst => {
                        const safeName = inst.name || inst.id || '';
                        return regex.test(safeName);
                    });
                } catch (e) {
                    console.warn('Invalid regex:', e.message);
                }
            }

            if (filtered.length === 0) {
                if (instancesList) {
                    instancesList.innerHTML = '<div class="text-center text-muted p-4"><i class="bi bi-search d-block mb-3"></i>No matching instances found</div>';
                }
            } else {
                filtered.forEach(instance => {
                    const card = this.create_instance_card(instance);
                    instancesList.appendChild(card);
                });
            }
        }
    },

    // Create instance card
    // Create instance card
    create_instance_card(instance) {
        const card = document.createElement('div');
        card.className = `col-md-12 ${instance.has_ssm ? '' : 'non-ssm'}`;

        const glowClass = instance.state === 'running' ? 'status-glow-running' : 'status-glow-stopped';

        // Sanitize instance data to prevent XSS
        const safeName = this.escape_html(this.sanitize_string(instance.name || instance.id || 'No Name'));
        const safeId = this.escape_html(this.sanitize_string(instance.id || ''));
        const safeState = this.escape_html(this.sanitize_string(instance.state || ''));
        const safeType = this.escape_html(this.sanitize_string(instance.type || ''));
        const safeOs = this.escape_html(this.sanitize_string(instance.os || ''));

        card.innerHTML = `
                <div class="card instance-card shadow-sm mb-3">
                    <div class="card-header border-0">
                        <div class="instance-info-grid">
                            <!-- Name & ID -->
                            <div class="info-item" style="min-width: 220px;">
                                <span class="info-label text-primary">Instance</span>
                                <span class="info-value fw-bold text-truncate" title="${safeName}">${safeName}</span>
                                <span class="text-xs text-muted" style="font-size: 0.7rem;">${safeId}</span>
                            </div>

                            <!-- Status -->
                            <div class="info-item">
                                <span class="info-label">Status</span>
                                <span class="info-value d-flex align-items-center">
                                    <span class="status-glow ${glowClass}"></span>
                                    ${safeState}
                                </span>
                            </div>

                            <!-- Specs -->
                            <div class="info-item">
                                <span class="info-label">Specs & OS</span>
                                <span class="info-value">
                                    <i class="bi bi-cpu me-1"></i>${safeType}
                                    <span class="mx-1">|</span>
                                    <i class="bi bi-${safeOs.toLowerCase().includes('win') ? 'windows' : 'ubuntu'} me-1"></i>${safeOs}
                                </span>
                            </div>

                            <!-- Gateway Status -->
                            <div class="info-item">
                                <span class="info-label">Gateway</span>
                                <span class="info-value">
                                    ${instance.has_ssm ?
                '<span class="badge badge-fucsia"><i class="bi bi-shield-check me-1"></i>SSM Active</span>' :
                '<span class="badge bg-secondary">Offline</span>'}
                                </span>
                            </div>

                            <!-- Actions -->
                            <div class="info-item text-end ms-auto">
                                ${this.create_action_buttons(instance.id)}
                            </div>
                        </div>
                    </div>
                </div>
        `;

        return card;
    },

    // Create action buttons for instance
    create_action_buttons(instanceId) {
        const safeInstanceId = this.escape_html(this.sanitize_string(instanceId || ''));
        const instance = this.instances.find(i => i.id === instanceId);
        const hasSsm = instance && instance.has_ssm;

        return `
            <div class="btn-group shadow-sm">
                ${hasSsm ? `
                    <button class="btn btn-sm btn-dark" onclick="app.start_ssh('${safeInstanceId}')" title="SSH Terminal">
                        <i class="bi bi-terminal"></i>
                    </button>
                    <button class="btn btn-sm btn-primary" onclick="app.start_rdp('${safeInstanceId}')" title="RDP Desktop">
                        <i class="bi bi-display"></i>
                    </button>
                    <button class="btn btn-sm btn-purple text-white" onclick="app.show_custom_port_modal('${safeInstanceId}')" title="Port Forwarding">
                        <i class="bi bi-arrow-left-right"></i>
                    </button>
                ` : ''}
                <button class="btn btn-sm btn-ottanio text-white" onclick="app.show_instance_details('${safeInstanceId}')" title="Instance Info">
                    <i class="bi bi-info-circle"></i>
                </button>
            </div>
        `;
    },

    // Utility functions
    show_loading() {
        if (this.elements.loadingOverlay) {
            this.elements.loadingOverlay.classList.remove('d-none');
        }
    },

    // Hide loading overlay
    hide_loading() {
        if (this.elements.loadingOverlay) {
            this.elements.loadingOverlay.classList.add('d-none');
        }
    },

    // Show error message
    show_error(message) {
        this.show_toast(message, 'danger');
    },

    // Show success message
    show_success(message) {
        this.show_toast(message, 'success');
    },

    // General method to show toast notifications
    show_toast(message, type = 'info') {
        const toastContainer = document.querySelector('.toast-container');
        const toast = document.createElement('div');
        toast.className = `toast align-items-center border-0 bg-${type} text-white`;
        toast.setAttribute('role', 'alert');
        // Sanitize message to prevent XSS
        const safeMessage = this.escape_html(this.sanitize_string(message || ''));
        toast.innerHTML = `
            <div class="d-flex">
                <div class="toast-body">${safeMessage}</div>
                <button type="button" class="btn-close btn-close-white me-2 m-auto" data-bs-dismiss="toast"></button>
            </div>
        `;
        toastContainer.appendChild(toast);
        const bsToast = new bootstrap.Toast(toast);
        bsToast.show();
        toast.addEventListener('hidden.bs.toast', () => toast.remove());
    },

    // Debounce utility to limit how often a function can be called
    debounce(func, wait = 300) {  // Default 300ms debounce delay
        let timeout;
        return function executedFunction(...args) {
            const later = () => {
                clearTimeout(timeout);
                func(...args);
            };
            clearTimeout(timeout);
            timeout = setTimeout(later, wait);
        };
    },

    // Input sanitization utilities
    sanitize_string(value, maxLength = 100) {  // Default max length 100 chars
        if (typeof value !== 'string') {
            return '';
        }
        // Remove null bytes and control characters except newline and tab
        let sanitized = value.replace(/[\x00-\x1F\x7F]/g, '').replace(/\x00/g, '');
        // Remove potential script tags
        sanitized = sanitized.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '');
        // Limit length if specified
        if (maxLength && sanitized.length > maxLength) {
            sanitized = sanitized.substring(0, maxLength);
        }
        return sanitized;
    },

    // Escape HTML to prevent XSS when inserting into DOM
    escape_html(text) {
        if (typeof text !== 'string') {
            return '';
        }
        const map = {
            '&': '&amp;',
            '<': '&lt;',
            '>': '&gt;',
            '"': '&quot;',
            "'": '&#039;'
        };
        return text.replace(/[&<>"']/g, m => map[m]);
    },

    // Update instance and connection counters
    update_counters() {
        this.elements.instanceCount.textContent = `${this.instances.length} instances`;
        this.elements.connectionCount.textContent = `${this.connections.length} active`;
    },

    // Connection Management Methods
    async start_ssh(instanceId) {
        try {
            this.show_loading();
            const response = await fetch(`/api/ssh/${instanceId}`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    profile: this.current_profile,
                    region: this.current_region
                })
            });

            if (!response.ok) throw new Error('Failed to start SSH session');

            const result = await response.json();

            if (result.status === 'success') {
                this.add_connection({
                    id: result.connection_id,
                    instanceId: instanceId,
                    type: 'SSH',
                    localPort: result.local_port,
                    remotePort: result.remote_port || 22,
                    command: result.command || '',
                    connectionInfo: result.connection_info || null,
                    timestamp: new Date(),
                    status: 'active'
                });
                const msg = result.connection_info
                    ? `SSH port forwarding active! ${result.connection_info.instruction} `
                    : `SSH port forward started on local port ${result.local_port} `;
                this.show_success(msg);
            } else {
                throw new Error(result.error || 'Failed to start SSH session');
            }
        } catch (error) {
            this.show_error('SSH connection error: ' + error.message);
        } finally {
            this.hide_loading();
        }
    },

    // Start RDP session
    async start_rdp(instanceId) {
        try {
            this.show_loading();
            const response = await fetch(`/api/rdp/${instanceId}`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    profile: this.current_profile,
                    region: this.current_region
                })
            });

            if (!response.ok) throw new Error('Failed to start RDP session');

            const result = await response.json();

            if (result.status === 'success') {
                this.add_connection({
                    id: result.connection_id,
                    instanceId: instanceId,
                    type: 'RDP',
                    localPort: result.local_port,
                    remotePort: result.remote_port || 3389,
                    command: result.command || '',
                    connectionInfo: result.connection_info || null,
                    timestamp: new Date(),
                    status: 'active'
                });
                const msg = result.connection_info
                    ? `RDP port forwarding active! ${result.connection_info.instruction} `
                    : `RDP port forward started on local port ${result.local_port} `;
                this.show_success(msg);
            } else {
                throw new Error(result.error || 'Failed to start RDP session');
            }
        } catch (error) {
            this.show_error('RDP connection error: ' + error.message);
        } finally {
            this.hide_loading();
        }
    },

    show_custom_port_modal(instanceId) {
        console.log(`Showing custom port modal for instance ${instanceId}`);
        // Store the instance ID for use when starting the connection
        this.selectedInstanceId = instanceId;

        // Reset the form
        document.getElementById('remotePort').value = '80';
        document.getElementById('localPort').value = '';

        // Show the modal
        this.modals.customPort.show();
    },

    // Start custom port forwarding
    async show_instance_details(instanceId) {
        console.log(`Showing instance details for ${instanceId}`);

        try {
            this.show_loading();

            // Fetch instance details from backend
            const response = await fetch(`/api/instance-details/${instanceId}`);
            if (!response.ok) throw new Error('Failed to fetch instance details');

            const details = await response.json();
            if (!details) throw new Error('No instance details found');

            // Update modal content
            const contentDiv = document.getElementById('instanceDetailsContent');
            contentDiv.innerHTML = `
                <div class="table-responsive">
                    <table class="table table-hover mb-0">
                        <tbody>
                            ${this.create_detail_row('Instance ID', details.id)}
                            ${this.create_detail_row('Name', details.name)}
                            ${this.create_detail_row('Platform', details.platform)}
                            ${this.create_detail_row('Public IPv4', details.public_ip)}
                            ${this.create_detail_row('Private IPv4', details.private_ip)}
                            ${this.create_detail_row('VPC ID', details.vpc_id)}
                            ${this.create_detail_row('Subnet ID', details.subnet_id)}
                            ${this.create_detail_row('IAM Role', details.iam_role)}
                            ${this.create_detail_row('AMI ID', details.ami_id)}
                            ${this.create_detail_row('SSH Key', details.key_name)}
                            ${this.create_detail_row('Security Groups', details.security_groups)}
                        </tbody>
                    </table>
                    <div class="text-muted small text-center mt-3">
                        <i class="bi bi-info-circle me-1"></i>Click on any value to copy to clipboard
                    </div>
                </div>
            `;

            // Add click handlers for copying
            contentDiv.querySelectorAll('.copy-value').forEach(element => {
                element.addEventListener('click', () => this.copy_to_clipboard(element.dataset.value));
            });

            // Show the modal
            this.modals.instanceDetails.show();

        } catch (error) {
            console.error('Error showing instance details:', error);
            this.show_error('Failed to load instance details');
        } finally {
            this.hide_loading();
        }
    },

    // Helper method to create detail rows
    create_detail_row(label, value) {
        // Handle undefined, null, or empty values
        const displayValue = (value === undefined || value === null || value === '') ? 'N/A' : String(value);
        const copyValue = displayValue === 'N/A' ? '' : displayValue;
        const cursorStyle = displayValue === 'N/A' ? 'cursor: default' : 'cursor: pointer';
        const title = displayValue === 'N/A' ? '' : 'Click to copy';

        return `
            <tr>
                <td class="fw-bold" style="width: 35%">${label}:</td>
                <td>
                    <span class="copy-value" role="button" data-value="${copyValue}" 
                          style="${cursorStyle}" title="${title}">
                        ${displayValue}
                    </span>
                </td>
            </tr>
        `;
    },

    // Helper method to copy to clipboard
    async copy_to_clipboard(text) {
        try {
            await navigator.clipboard.writeText(text);
            this.show_success('Copied to clipboard');
        } catch (error) {
            console.error('Failed to copy:', error);
            this.show_error('Failed to copy to clipboard');
        }
    },

    // Helper method to copy text (copy_to_clipboard already shows success message)
    copy_text(text) {
        this.copy_to_clipboard(text).catch(err => {
            this.show_error('Failed to copy: ' + err.message);
        });
    },

    async copy_command_from_element(elementId) {
        const element = document.getElementById(elementId);
        if (element && element.dataset.command) {
            await this.copy_to_clipboard(element.dataset.command);
        }
    },

    // Load PEM key and decrypt Windows password
    load_pem_key_and_decrypt(instanceId, passwordElementId, pemInputId) {
        const pemInput = document.getElementById(pemInputId);
        if (pemInput) {
            pemInput.click();
        }
    },

    // Handle PEM file selection and decrypt password
    async handle_pem_file_select(event, instanceId, passwordElementId, keyName = null) {
        const file = event.target.files[0];
        if (!file) return;

        const passwordElement = document.getElementById(passwordElementId);
        if (!passwordElement) return;

        // Show loading state
        passwordElement.innerHTML = '<em class="text-muted">Decrypting password...</em>';

        try {
            // Read file content
            const pemKeyContent = await this.read_file_as_text(file);

            // Call API to decrypt password
            await this.decrypt_password_with_key(instanceId, passwordElementId, pemKeyContent, keyName);
        } catch (error) {
            console.error('Error decrypting password:', error);
            passwordElement.innerHTML = `<em class="text-danger"> Error: ${error.message}</em> `;
            this.show_error('Failed to decrypt password: ' + error.message);
        }
    },

    // Auto-decrypt password using key from configured folder
    async auto_decrypt_password(instanceId, passwordElementId, keyName) {
        if (!this.preferences.ssh_key_folder || !keyName) {
            return;
        }

        const passwordElement = document.getElementById(passwordElementId);
        if (!passwordElement) return;

        // Show loading state
        passwordElement.innerHTML = '<em class="text-muted">Auto-decrypting password...</em>';

        try {
            // Call API to decrypt password (backend will auto-lookup key)
            await this.decrypt_password_with_key(instanceId, passwordElementId, null, keyName);
        } catch (error) {
            console.error('Error auto-decrypting password:', error);
            // Don't show error for auto-decrypt failures, just show the manual option
            passwordElement.innerHTML = '<em class="text-muted">Click "Load Key" to decrypt password</em>';
        }
    },

    // Decrypt password with key (either provided or auto-looked up)
    async decrypt_password_with_key(instanceId, passwordElementId, pemKeyContent, keyName) {
        const passwordElement = document.getElementById(passwordElementId);
        if (!passwordElement) return;

        const requestBody = {};
        if (pemKeyContent) {
            requestBody.pem_key = pemKeyContent;
        }
        if (keyName) {
            requestBody.key_name = keyName;
        }

        const response = await fetch(`/api/windows-password/${instanceId}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(requestBody)
        });

        if (!response.ok) {
            const error = await response.json();
            throw new Error(error.error || 'Failed to decrypt password');
        }

        const result = await response.json();
        const password = result.password;

        // Display password with copy functionality
        const passwordId = `${passwordElementId}-value`;
        const escapedPassword = password.replace(/'/g, "\\'").replace(/"/g, '&quot;');
        passwordElement.innerHTML = `
    <div class="d-flex justify-content-between align-items-center">
                <span class="font-monospace" id="${passwordId}" style="cursor: pointer;" onclick="app.copy_text('${escapedPassword}')" title="Click to copy">${this.mask_password(password)}</span>
                <button class="btn btn-sm btn-link p-0 ms-2" onclick="app.toggle_password_visibility('${passwordId}', '${escapedPassword}')" title="Show/Hide password">
                    <i class="bi bi-eye" id="${passwordId}-icon"></i>
                </button>
            </div>
    `;
    },

    // Read file as text
    read_file_as_text(file) {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = (e) => resolve(e.target.result);
            reader.onerror = (e) => reject(new Error('Failed to read file'));
            reader.readAsText(file);
        });
    },

    // Mask password for display
    mask_password(password) {
        if (!password) return '';
        return '•'.repeat(Math.min(password.length, 20));
    },

    // Toggle password visibility
    toggle_password_visibility(elementId, password) {
        const element = document.getElementById(elementId);
        const icon = document.getElementById(`${elementId}-icon`);
        if (!element || !icon) return;

        if (element.textContent.includes('•')) {
            element.textContent = password;
            icon.classList.remove('bi-eye');
            icon.classList.add('bi-eye-slash');
        } else {
            element.textContent = this.mask_password(password);
            icon.classList.remove('bi-eye-slash');
            icon.classList.add('bi-eye');
        }
    },

    // Launch RDP client with connection parameters
    async launch_rdp_client(ip, port, username, passwordElementId) {
        // Try to get password from the password element if it's been decrypted
        let password = null;
        if (passwordElementId) {
            const passwordElement = document.getElementById(passwordElementId);
            if (passwordElement) {
                const passwordValueElement = document.getElementById(`${passwordElementId} -value`);
                if (passwordValueElement) {
                    const passwordText = passwordValueElement.textContent;
                    // Check if password is visible (not masked)
                    if (passwordText && !passwordText.includes('•') && passwordText.trim() !== '') {
                        password = passwordText.trim();
                    }
                }
            }
        }

        try {
            this.show_loading();

            const response = await fetch('/api/rdp-client/connect', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    ip: ip,
                    port: parseInt(port),
                    username: username,
                    password: password
                })
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(error.error || 'Failed to launch RDP client');
            }

            const result = await response.json();
            this.show_success(result.message || 'RDP client launched successfully');
        } catch (error) {
            console.error('Error launching RDP client:', error);
            this.show_error('Failed to launch RDP client: ' + error.message);
        } finally {
            this.hide_loading();
        }
    },

    // Copy command from element's data attribute
    async copy_command_from_element(elementId) {
        const element = document.getElementById(elementId);
        if (element && element.dataset.command) {
            await this.copy_to_clipboard(element.dataset.command);
        }
    },

    // Connection Management
    add_connection(connection) {
        this.connections.push(connection);
        this.render_connections();
        this.update_counters();
    },

    async terminate_connection(connectionId) {
        try {
            this.show_loading();
            const response = await fetch(`/api/terminate-connection/${connectionId}`, {
                method: 'POST'
            });

            if (!response.ok) {
                const errorData = await response.json();
                throw new Error(errorData.error || 'Failed to terminate connection');
            }

            this.connections = this.connections.filter(c => c.id !== connectionId);
            this.render_connections();
            this.update_counters();
            this.show_success('Connection terminated successfully');
        } catch (error) {
            this.show_error('Failed to terminate connection: ' + error.message);
        } finally {
            this.hide_loading();
        }
    },

    render_connections() {
        const container = this.elements.connectionsList;
        const emptyState = document.getElementById('connectionsEmptyState');

        container.innerHTML = '';

        if (this.connections.length === 0) {
            // Show empty state
            if (emptyState) emptyState.classList.remove('d-none');
            return;
        }

        const selectedProfile = this.elements.profileSelect.value;
        const selectedRegion = this.elements.regionSelect.value;

        const visibleConnections = this.connections.filter(c =>
            (!selectedProfile || c.profile === selectedProfile) &&
            (!selectedRegion || c.region === selectedRegion)
        );

        if (visibleConnections.length === 0) {
            // Show empty state
            if (emptyState) emptyState.classList.remove('d-none');
            return;
        }

        // Hide empty state
        if (emptyState) emptyState.classList.add('d-none');

        visibleConnections.forEach(conn => {
            const element = document.createElement('div');
            element.className = 'connection-item';

            // Format timestamp
            const timestamp = new Date(conn.timestamp).toLocaleTimeString();

            // Create connection info based on type
            let connectionInfo = '';
            if (conn.type === 'RDP' || conn.type === 'Custom Port') {
                connectionInfo = `
    <div class="text-muted small">
        Local Port: ${conn.localPort}
                        ${conn.remotePort ? `, Remote Port: ${conn.remotePort}` : ''}
                    </div>
    `;
            }

            element.innerHTML = `
    < !--
                <div class="d-flex justify-content-between align-items-start">
                    <div>
                        <div class="d-flex align-items-center gap-2">
                            
                            <span class="badge bg-${this.get_connection_type_color(conn.type)}">
                                ${conn.type}
                            </span>
                          
                            
                            <div class="text-muted small"><b>ID: ${this.get_instance_name(conn.instanceId)}</b></div>
                            ${connectionInfo}
                            <div class="text-muted small">Started at ${timestamp}</div>
                        </div>
                        
                    </div>
                    <button class="btn btn-sm btn-outline-danger" 
                            onclick="app.terminate_connection('${conn.id}')">
                        <i class="bi bi-x-lg"></i>
                    </button>
                </div>
    -->
    <div class="d-flex justify-content-between align-items-start">
        <div>
            <div class="d-flex align-items-center gap-2">
                <span class="badge 
                                ${this.get_connection_type_color(conn.type) === 'dark' ? 'bg-dark' : ''} 
                                ${this.get_connection_type_color(conn.type) === 'primary' ? 'bg-primary' : ''}"
                    style="${this.get_connection_type_color(conn.type) === '#800080' ? `background-color: #800080;` : ''}">
                    ${conn.type}
                </span>
            </div>
            <div class="text-muted small"><b>ID: ${this.get_instance_name(conn.instanceId)}</b></div>
            ${connectionInfo}
            <div class="text-muted small">Started at ${timestamp}</div>
        </div>
        <button class="btn btn-sm btn-outline-danger"
            onclick="app.terminate_connection('${conn.id}')">
            <i class="bi bi-x-lg"></i>
        </button>
    </div>

`;

            container.appendChild(element);
        });
    },

    // Utility methods for connections
    get_connection_type_color(type) {
        const colors = {
            'SSH': 'dark',
            'RDP': 'primary',
            'Custom Port': '#800080'
        };
        return colors[type] || 'secondary';
    },

    get_instance_name(instanceId) {
        const instance = this.instances.find(i => i.id === instanceId);
        return instance ? instance.name : instanceId;
    },

    generate_connection_id() {
        return 'conn_' + Math.random().toString(36).substr(2, 9);
    },

    // Connection monitoring
    start_connection_monitoring() {
        setInterval(() => this.check_connections(), 5000);
    },

    async check_connections() {
        try {
            const response = await fetch('/api/active-connections');
            if (!response.ok) throw new Error('Failed to check connections');

            const activeConnections = await response.json();
            const activeIds = new Set(activeConnections.map(c => c.connection_id || c.id));
            const activeConnectionsMap = new Map(activeConnections.map(c => [c.connection_id || c.id, c]));

            let needsUpdate = false;
            const previousCount = this.connections.length;

            this.connections = this.connections.filter(conn => {
                const isActive = activeIds.has(conn.id);
                if (!isActive) {
                    console.log(`Connection ${conn.id} is no longer active`);
                    this.show_toast(`Connection to ${this.get_instance_name(conn.instanceId)} was terminated`, 'warning');
                    return false;
                }

                // Update connection info from backend if available
                const backendConn = activeConnectionsMap.get(conn.id);
                if (backendConn && backendConn.connection_info && !conn.connectionInfo) {
                    conn.connectionInfo = backendConn.connection_info;
                    needsUpdate = true;
                }

                return true;
            });

            // Add new ones from backend that aren't in this.connections
            const localIds = new Set(this.connections.map(c => c.id));
            activeConnections.forEach(backendConn => {
                const connId = backendConn.connection_id || backendConn.id;
                if (!localIds.has(connId)) {
                    this.connections.push({
                        id: connId,
                        instanceId: backendConn.instance_id,
                        localPort: backendConn.local_port,
                        remotePort: backendConn.remote_port,
                        remoteHost: backendConn.remote_host,
                        command: backendConn.command,
                        keyName: backendConn.key_name,
                        keyPath: backendConn.key_path,
                        profile: backendConn.profile,
                        region: backendConn.region,
                        timestamp: new Date().toISOString(),
                        connectionInfo: backendConn.connection_info
                    });
                    needsUpdate = true;
                } else {
                    // Update profile/region if missing in local
                    const localConn = this.connections.find(c => c.id === connId);
                    if (localConn) {
                        if (backendConn.profile && !localConn.profile) {
                            localConn.profile = backendConn.profile;
                            needsUpdate = true;
                        }
                        if (backendConn.region && !localConn.region) {
                            localConn.region = backendConn.region;
                            needsUpdate = true;
                        }
                    }
                }
            });

            if (previousCount !== this.connections.length || needsUpdate) {
                this.render_connections();
                this.update_counters();
            }
        } catch (error) {
            console.error('Error checking connections:', error);
        }
    }


};


app.refresh_data = async function () {
    if (!this.is_connected) return;

    try {
        this.show_loading();

        //  Save current selections before refreshing
        const selectedProfile = this.elements.profileSelect.value;
        const selectedRegion = this.elements.regionSelect.value;

        // (Optional) You can skip this call entirely if you don’t need to reload profiles each time
        // await this.loadProfilesAndRegions();

        // Refresh instance list respecting current filters
        const state = this.current_filters?.state || (document.getElementById('instanceStateFilter')?.value || '');
        const os = this.current_filters?.os || (document.getElementById('instanceOsFilter')?.value || '');
        await this.load_instances(state || null, os || '');
        this.update_counters();

        // Restore selections
        if (selectedProfile) {
            this.elements.profileSelect.value = selectedProfile;
            this.current_profile = selectedProfile;
        }
        if (selectedRegion) {
            this.elements.regionSelect.value = selectedRegion;
            this.current_region = selectedRegion;
        }

    } catch (error) {
        this.show_error('Failed to refresh data: ' + error.message);
        this.toggle_auto_refresh(false);
    } finally {
        this.hide_loading();
    }
};

app.toggle_auto_refresh = function (enabled) {
    if (enabled && enabled.target) {
        enabled = enabled.target.checked;
    }

    console.log('Toggle auto-refresh:', enabled);

    if (enabled) {
        this.start_auto_refresh();
    } else {
        this.stop_auto_refresh();
    }
};

app.start_auto_refresh = function () {
    console.log('Starting auto-refresh');
    this.refresh_countdown = 30;
    this.update_refresh_timer();

    // Clear any existing interval
    if (this.refresh_interval) {
        clearInterval(this.refresh_interval);
    }

    // Set new interval for countdown and refresh
    this.refresh_interval = setInterval(() => {
        this.refresh_countdown--;
        this.update_refresh_timer();

        if (this.refresh_countdown <= 0) {
            this.refresh_data();
            this.refresh_countdown = 30;  // Reset countdown
        }
    }, 1000);
};

app.stop_auto_refresh = function () {
    console.log('Stopping auto-refresh');
    // Clear the interval
    if (this.refresh_interval) {
        clearInterval(this.refresh_interval);
        this.refresh_interval = null;
    }

    // Reset countdown and clear display
    this.refresh_countdown = 0;
    this.elements.refreshTimer.textContent = '';
    this.elements.autoRefreshSwitch.checked = false;
};

app.setup_event_listeners = function () {
    console.log('Setting up event listeners...');
    this.elements.connectBtn.onclick = () => this.toggle_connection();
    this.elements.refreshBtn.onclick = () => this.refresh_data();

    // Modifica la gestione dell'evento autoRefreshSwitch
    this.elements.autoRefreshSwitch.onchange = (e) => {
        this.toggle_auto_refresh(e.target.checked);
    };
};

app.update_refresh_timer = function () {
    // Only show countdown if auto-refresh is active
    if (this.elements.autoRefreshSwitch.checked && this.refresh_countdown > 0) {
        this.elements.refreshTimer.textContent = `(${this.refresh_countdown}s)`;
    } else {
        this.elements.refreshTimer.textContent = '';
    }
};
// Update the show_preferences method in app.js

app.show_preferences = async function () {
    console.log('Showing preferences dialog');

    try {
        // Fetch latest preferences from server
        const response = await fetch('/api/preferences');
        if (!response.ok) throw new Error('Failed to load preferences');

        const prefs = await response.json();

        // Update the form fields with current values
        document.getElementById('startPort').value = prefs.port_range.start;
        document.getElementById('endPort').value = prefs.port_range.end;
        document.getElementById('logLevel').value = prefs.logging.level;
        const cacheTtlField = document.getElementById('cacheTtl');
        if (cacheTtlField) {
            cacheTtlField.value = prefs.cache_ttl_secs || 3600;
        }

        // Load SSH options
        const sshOptionsField = document.getElementById('sshOptions');
        if (sshOptionsField) {
            sshOptionsField.value = prefs.ssh_options || '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null';
        }

        // Load SSH key folders
        this.load_ssh_key_folders(prefs.ssh_key_folder || '');

        // Show the modal
        if (this.modals.preferences) {
            this.modals.preferences.show();
        } else {
            console.error('Preferences modal not initialized');
        }
    } catch (error) {
        console.error('Error loading preferences:', error);
        this.show_error('Failed to load current preferences');
    }
};

// Update the save_preferences method to ensure we're using the correct structure
app.save_preferences = async function () {
    console.log('Saving preferences');
    const form = document.getElementById('preferencesForm');
    const saveBtn = document.getElementById('savePreferencesBtn');
    const spinner = saveBtn?.querySelector('.spinner-border-sm');

    // Validate form
    if (form && !form.checkValidity()) {
        form.reportValidity();
        return;
    }

    try {
        const startPort = parseInt(document.getElementById('startPort').value);
        const endPort = parseInt(document.getElementById('endPort').value);
        const logLevel = document.getElementById('logLevel').value;
        const sshOptions = document.getElementById('sshOptions').value.trim() || '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null';
        const cacheTtlField = document.getElementById('cacheTtl');
        const cacheTtl = cacheTtlField ? parseInt(cacheTtlField.value) : 3600;

        // Get SSH key folders (one per line)
        const sshKeyFolders = this.get_ssh_key_folders();
        const sshKeyFolder = sshKeyFolders.length > 0 ? sshKeyFolders.join('\n') : null;

        // Validate values
        if (startPort >= endPort) {
            this.show_error('Start port must be less than end port');
            return;
        }

        if (startPort < 1024 || endPort > 65535) {
            this.show_error('Ports must be between 1024 and 65535');
            return;
        }
        if (isNaN(cacheTtl) || cacheTtl < 60) {
            this.show_error('Cache TTL must be at least 60 seconds');
            return;
        }

        // Show loading state on button
        if (saveBtn) saveBtn.disabled = true;
        if (spinner) spinner.classList.remove('d-none');

        // Create new preferences object matching backend structure
        const newPreferences = {
            port_range: {
                start: startPort,
                end: endPort
            },
            logging: {
                level: logLevel,
                format: "%(asctime)s-%(name)s-%(levelname)s-[%(filename)s:%(lineno)d]-%(message)s"
            },
            ssh_options: sshOptions,
            cache_ttl_secs: cacheTtl
        };

        // Add SSH key folder if provided (newline-separated)
        if (sshKeyFolder) {
            newPreferences.ssh_key_folder = sshKeyFolder;
        }

        const response = await fetch('/api/preferences', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(newPreferences)
        });

        if (!response.ok) throw new Error('Failed to save preferences');

        // Update local preferences (preserve structure)
        this.preferences.startPort = newPreferences.port_range.start;
        this.preferences.endPort = newPreferences.port_range.end;
        this.preferences.logLevel = newPreferences.logging.level;
        this.preferences.ssh_options = newPreferences.ssh_options;
        if (sshKeyFolder) {
            this.preferences.ssh_key_folder = sshKeyFolder;
        }
        this.preferences.cache_ttl_secs = cacheTtl;

        // Hide modal and show success message
        if (this.modals.preferences) {
            this.modals.preferences.hide();
        }
        this.show_success('Preferences saved successfully');

    } catch (error) {
        console.error('Error saving preferences:', error);
        this.show_error('Failed to save preferences');
    } finally {
        // Reset button state
        if (saveBtn) saveBtn.disabled = false;
        if (spinner) spinner.classList.add('d-none');
    }
};

// Update load_preferences to match backend structure
app.load_preferences = async function () {
    console.log('Loading initial preferences');
    try {
        const response = await fetch('/api/preferences');
        if (!response.ok) throw new Error('Failed to load preferences');

        const prefs = await response.json();
        // Map backend structure to frontend preferences object
        this.preferences.startPort = prefs.port_range?.start || null;
        this.preferences.endPort = prefs.port_range?.end || null;
        this.preferences.logLevel = prefs.logging?.level || 'INFO';
        this.preferences.ssh_options = prefs.ssh_options || '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null';
        this.preferences.ssh_key_folder = prefs.ssh_key_folder || null;
        this.preferences.cache_ttl_secs = prefs.cache_ttl_secs || 3600;

        // Also store full structure for compatibility
        this.preferences.port_range = prefs.port_range;
        this.preferences.logging = prefs.logging;

        console.log('Loaded preferences:', this.preferences);

    } catch (error) {
        console.error('Error loading initial preferences:', error);
        // Use safe default values if loading fails (will be overridden by backend on next load)
        // These are generic safe ranges that work across platforms
        this.preferences.startPort = 40000;
        this.preferences.endPort = 40100;
        this.preferences.logLevel = 'INFO';
        this.preferences.ssh_options = '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null';
    }
};


// Setup keyboard shortcuts
app.setup_keyboard_shortcuts = function () {
    document.addEventListener('keydown', (e) => {
        // Ctrl+Enter or Cmd+Enter to connect
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault();
            const connectBtn = document.getElementById('connectBtn');
            if (connectBtn && !connectBtn.disabled) {
                connectBtn.click();
            }
        }

        // F5 to refresh
        if (e.key === 'F5') {
            e.preventDefault();
            const refreshBtn = document.getElementById('refreshBtn');
            if (refreshBtn && !refreshBtn.disabled) {
                refreshBtn.click();
            }
        }

        // Escape to close modals
        if (e.key === 'Escape') {
            const openModal = document.querySelector('.modal.show');
            if (openModal) {
                const closeBtn = openModal.querySelector('[data-bs-dismiss="modal"]');
                if (closeBtn) closeBtn.click();
            }
        }
    });
};

// Custom port forwarding
app.start_custom_port_forwarding = async function () {
    const instanceId = this.selectedInstanceId;
    const form = document.getElementById('customPortForm');
    const startBtn = document.getElementById('startPortForwardingBtn');
    const spinner = startBtn?.querySelector('.spinner-border-sm');

    if (!instanceId) {
        this.show_error('No instance selected');
        return;
    }

    // Validate form
    if (form && !form.checkValidity()) {
        form.reportValidity();
        return;
    }

    // Get remote port (required)
    const remotePort = document.getElementById('remotePort').value;
    if (!remotePort) {
        this.show_error('Please specify a remote port');
        return;
    }

    // Get local port (optional)
    const localPort = document.getElementById('localPort').value;

    // Show loading state on button
    if (startBtn) startBtn.disabled = true;
    if (spinner) spinner.classList.remove('d-none');

    // Show loading BEFORE preparing request data
    this.show_loading();

    try {
        let requestData = {
            profile: this.current_profile,
            region: this.current_region,
            remote_port: parseInt(remotePort)
        };

        // Add local port if provided
        if (localPort) {
            requestData.local_port = parseInt(localPort);
        }

        const response = await fetch(`/api/custom-port/${instanceId}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(requestData)
        });

        if (!response.ok) throw new Error('Failed to start port forwarding');
        const result = await response.json();

        if (result.status === 'success') {
            // Create connection object
            const connectionData = {
                id: result.connection_id,
                instanceId: instanceId,
                type: 'Custom Port',
                localPort: result.local_port,
                remotePort: result.remote_port,
                command: result.command || '',
                connectionInfo: result.connection_info || null,
                timestamp: new Date(),
                status: 'active'
            };

            this.add_connection(connectionData);

            // Show success message
            const successMessage = result.connection_info
                ? `Port forwarding active! ${result.connection_info.instruction} `
                : `Port forwarding started(Local: ${result.local_port}, Remote: ${result.remote_port})`;

            this.show_success(successMessage);
            this.modals.customPort.hide();
            // Reset form
            if (form) form.reset();
        } else {
            throw new Error(result.error || 'Failed to start port forwarding');
        }
    } catch (error) {
        this.show_error('Port forwarding error: ' + error.message);
    } finally {
        this.hide_loading();
        // Reset button state
        if (startBtn) startBtn.disabled = false;
        if (spinner) spinner.classList.add('d-none');
    }
};

// Updated render_connections function to better handle connection information
app.render_connections = function () {
    const container = this.elements.connectionsList;
    const emptyState = document.getElementById('connectionsEmptyState');

    container.innerHTML = '';

    if (this.connections.length === 0) {
        if (emptyState) emptyState.classList.remove('d-none');
        return;
    }

    const selectedProfile = this.elements.profileSelect.value;
    const selectedRegion = this.elements.regionSelect.value;

    const visibleConnections = this.connections.filter(c =>
        (!selectedProfile || c.profile === selectedProfile) &&
        (!selectedRegion || c.region === selectedRegion)
    );

    if (visibleConnections.length === 0) {
        if (emptyState) emptyState.classList.remove('d-none');
        return;
    }

    if (emptyState) emptyState.classList.add('d-none');

    visibleConnections.forEach(conn => {
        const element = document.createElement('div');
        element.className = 'connection-item';

        const timestamp = new Date(conn.timestamp).toLocaleTimeString();

        let connectionInfo = '';
        if (conn.localPort) {
            connectionInfo = `
                <div class="text-muted small">
                    Local Port: ${conn.localPort}
                    ${conn.remotePort ? `, Remote Port: ${conn.remotePort}` : ''}
                    ${conn.remoteHost ? `, Host: ${conn.remoteHost}` : ''}
                </div>
            `;
        }

        let connectionDetailsDisplay = '';
        if (conn.connectionInfo) {
            const info = conn.connectionInfo;
            const detailsId = `conn-details-${conn.id.replace(/[^a-zA-Z0-9]/g, '-')}`;

            let detailsList = '';
            if (info.ip) {
                const ipId = `${detailsId}-ip`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>IP:</strong></span>
                        <span class="font-monospace small" id="${ipId}" style="cursor: pointer;" onclick="app.copy_text('${info.ip}')" title="Click to copy">${info.ip}</span>
                    </li>
                `;
            }
            if (info.port) {
                const portId = `${detailsId}-port`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>Port:</strong></span>
                        <span class="font-monospace small" id="${portId}" style="cursor: pointer;" onclick="app.copy_text('${info.port}')" title="Click to copy">${info.port}</span>
                    </li>
                `;
            }
            if (info.user) {
                const userId = `${detailsId}-user`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>User:</strong></span>
                        <span class="font-monospace small" id="${userId}" style="cursor: pointer;" onclick="app.copy_text('${info.user}')" title="Click to copy">${info.user}</span>
                    </li>
                `;
            }
            if (info.key_name) {
                const keyNameId = `${detailsId}-key-name`;
                const escapedKeyName = info.key_name.replace(/'/g, "\\'").replace(/"/g, '&quot;');
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>SSH Key:</strong></span>
                        <span class="font-monospace small" id="${keyNameId}" style="cursor: pointer;" onclick="app.copy_text('${escapedKeyName}')" title="Click to copy">${info.key_name}</span>
                    </li>
                `;
            }

            if (info.type === 'ssh') {
                let sshCommand = '';
                const sshOptions = this.preferences.ssh_options || '-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null';
                if (info.ssh_command_with_key) {
                    sshCommand = info.ssh_command_with_key;
                } else if (info.command) {
                    sshCommand = info.command;
                    if (!sshCommand.includes('StrictHostKeyChecking') && !sshCommand.includes('-o')) {
                        sshCommand = sshCommand.replace(/^ssh /, `ssh ${sshOptions} `);
                    }
                    if (info.key_name && info.key_name !== 'N/A') {
                        const sshKeyFolders = this.preferences.ssh_key_folder;
                        let firstFolder = null;
                        if (sshKeyFolders) {
                            const folders = sshKeyFolders.split('\n').flatMap(f => f.split(','));
                            firstFolder = folders.find(f => f.trim())?.trim();
                        }
                        let keyPath = firstFolder ? `${firstFolder}/${info.key_name}` : `~/.ssh/${info.key_name}`;
                        keyPath = keyPath.replace(/\\/g, '/');
                        const quotedKeyPath = `"${keyPath}"`;
                        sshCommand = `ssh ${sshOptions} -i ${quotedKeyPath} -p ${info.port} ${info.user}@${info.ip}`;
                    }
                }

                if (sshCommand) {
                    const sshCommandId = `${detailsId}-ssh-command`;
                    const escapedSshCommand = sshCommand.replace(/'/g, "\\'").replace(/"/g, '&quot;').replace(/\$/g, '\\$');
                    detailsList += `
                        <li class="list-group-item">
                            <div class="text-muted small mb-1"><strong>SSH Command:</strong></div>
                            <div class="bg-dark text-light p-2 rounded font-monospace small" style="word-break: break-all; cursor: pointer;" 
                                 id="${sshCommandId}"
                                 onclick="app.copy_text('${escapedSshCommand}')" 
                                 title="Click to copy">
                                <code class="text-light">${sshCommand}</code>
                                <i class="bi bi-clipboard ms-2"></i>
                            </div>
                        </li>
                    `;
                }
            }

            let passwordField = '';
            let connectButton = '';
            if (info.type === 'rdp' && info.instance_id) {
                const passwordId = `${detailsId}-password`;
                const pemInputId = `${detailsId}-pem-input`;
                const passwordValueId = `${detailsId}-password-value`;
                const escapedInstanceId = info.instance_id.replace(/'/g, "\\'").replace(/"/g, '&quot;');
                const escapedKeyName = (info.key_name || '').replace(/'/g, "\\'").replace(/"/g, '&quot;');
                const escapedIp = (info.ip || '127.0.0.1').replace(/'/g, "\\'").replace(/"/g, '&quot;');
                const escapedPort = (info.port || '').replace(/'/g, "\\'").replace(/"/g, '&quot;');
                const escapedUser = (info.user || 'Administrator').replace(/'/g, "\\'").replace(/"/g, '&quot;');

                passwordField = `
                    <li class="list-group-item">
                        <div class="d-flex justify-content-between align-items-center mb-2">
                            <span><strong>Password:</strong></span>
                            <button class="btn btn-sm btn-outline-primary" onclick="app.load_pem_key_and_decrypt('${escapedInstanceId}', '${passwordValueId}', '${pemInputId}')" title="Load PEM key to decrypt password">
                                <i class="bi bi-key"></i> Load Key
                            </button>
                        </div>
                        <input type="file" id="${pemInputId}" class="form-control form-control-sm mb-2" accept=".pem,.key" style="display: none;" onchange="app.handle_pem_file_select(event, '${escapedInstanceId}', '${passwordValueId}', '${escapedKeyName}')">
                        <div id="${passwordValueId}" class="font-monospace small text-muted" style="min-height: 1.5em;">
                            <em>Click "Load Key" to decrypt password</em>
                        </div>
                    </li>
                `;

                connectButton = `
                    <li class="list-group-item">
                        <button class="btn btn-success w-100" onclick="app.launch_rdp_client('${escapedIp}', '${escapedPort}', '${escapedUser}', '${passwordValueId}')" title="Open RDP client and connect">
                            <i class="bi bi-display"></i> Connect with RDP Client
                        </button>
                    </li>
                `;

                if (info.key_name && this.preferences.ssh_key_folder) {
                    setTimeout(() => {
                        this.auto_decrypt_password(escapedInstanceId, passwordValueId, escapedKeyName);
                    }, 500);
                }
            }

            if (detailsList || passwordField || connectButton) {
                connectionDetailsDisplay = `
                    <div class="mt-2">
                        <div class="text-muted small mb-2">
                            <strong>Connection Details:</strong>
                        </div>
                        <ul class="list-group list-group-flush">
                            ${detailsList}
                            ${passwordField}
                            ${connectButton}
                        </ul>
                    </div>
                `;
            }
        }

        let commandDisplay = '';
        if (conn.command && !conn.connectionInfo) {
            const commandId = `cmd-${conn.id.replace(/[^a-zA-Z0-9]/g, '-')}`;
            commandDisplay = `
                <div class="mt-2">
                    <div class="text-muted small mb-1">
                        <strong>Port forwarding command:</strong>
                    </div>
                    <div class="bg-dark text-light p-2 rounded font-monospace small" style="word-break: break-all; cursor: pointer;" 
                         id="${commandId}"
                         data-command="${conn.command.replace(/"/g, '&quot;').replace(/'/g, '&#39;')}"
                         onclick="app.copy_command_from_element('${commandId}')" 
                         title="Click to copy">
                        <code class="text-light">${conn.command}</code>
                        <i class="bi bi-clipboard ms-2"></i>
                    </div>
                </div>
            `;
        }

        element.innerHTML = `
            <div class="d-flex justify-content-between align-items-start">
                <div class="flex-grow-1">
                    <div class="d-flex align-items-center gap-2">
                        <span class="badge ${this.get_connection_type_color(conn.type) === 'dark' ? 'bg-dark' : ''} ${this.get_connection_type_color(conn.type) === 'primary' ? 'bg-primary' : ''}" 
                              style="${this.get_connection_type_color(conn.type) === '#800080' ? `background-color: #800080;` : ''}">
                            ${conn.type}
                        </span>
                    </div>
                    <div class="text-muted small"><b>ID: ${this.get_instance_name(conn.instanceId)}</b></div>
                    ${connectionInfo}
                    ${connectionDetailsDisplay}
                    ${commandDisplay}
                    <div class="text-muted small">Started at ${timestamp}</div>
                </div>
                <button class="btn btn-sm btn-outline-danger ms-2" onclick="app.terminate_connection('${conn.id}')" title="Terminate connection">
                    <i class="bi bi-x-lg"></i>
                </button>
            </div>
        `;

        container.appendChild(element);
    });
};

// Helper function to determine badge color based on connection type
app.get_connection_type_color = function (type) {
    const colors = {
        'SSH': 'dark',
        'RDP': 'primary',
        'Custom Port': '#800080',
        'Remote Host Port': '#800080'
    };
    return colors[type] || 'secondary';
};

app.start_connection_monitoring = function () {
    console.log('Starting connection monitoring');
    // Clear any existing monitoring interval
    if (this.monitoring_interval) {
        clearInterval(this.monitoring_interval);
    }
    this.monitoring_interval = setInterval(() => this.check_connections(), 2000);
},

    app.check_connections = async function () {
        try {
            const response = await fetch('/api/active-connections');
            if (!response.ok) throw new Error('Failed to check connections');

            const activeConnections = await response.json();
            const activeIds = new Set(activeConnections.map(c => c.connection_id || c.id));
            const activeConnectionsMap = new Map(activeConnections.map(c => [c.connection_id || c.id, c]));

            let needsUpdate = false;
            const previousCount = this.connections.length;

            this.connections = this.connections.filter(conn => {
                const isActive = activeIds.has(conn.id);
                if (!isActive) {
                    console.log(`Connection ${conn.id} is no longer active`);
                    this.show_toast(`Connection to ${this.get_instance_name(conn.instanceId)} was terminated`, 'warning');
                    return false;
                }

                // Update connection info from backend if available
                const backendConn = activeConnectionsMap.get(conn.id);
                if (backendConn && backendConn.connection_info && !conn.connectionInfo) {
                    conn.connectionInfo = backendConn.connection_info;
                    needsUpdate = true;
                }

                return true;
            });

            // Add new ones from backend that aren't in this.connections
            const localIds = new Set(this.connections.map(c => c.id));
            activeConnections.forEach(backendConn => {
                const connId = backendConn.connection_id || backendConn.id;
                if (!localIds.has(connId)) {
                    this.connections.push({
                        id: connId,
                        instanceId: backendConn.instance_id,
                        localPort: backendConn.local_port,
                        remotePort: backendConn.remote_port,
                        remoteHost: backendConn.remote_host,
                        command: backendConn.command,
                        keyName: backendConn.key_name,
                        keyPath: backendConn.key_path,
                        profile: backendConn.profile,
                        region: backendConn.region,
                        timestamp: new Date().toISOString(),
                        connectionInfo: backendConn.connection_info
                    });
                    needsUpdate = true;
                } else {
                    // Update profile/region if missing in local
                    const localConn = this.connections.find(c => c.id === connId);
                    if (localConn) {
                        if (backendConn.profile && !localConn.profile) {
                            localConn.profile = backendConn.profile;
                            needsUpdate = true;
                        }
                        if (backendConn.region && !localConn.region) {
                            localConn.region = backendConn.region;
                            needsUpdate = true;
                        }
                    }
                }
            });

            if (previousCount !== this.connections.length || needsUpdate) {
                this.render_connections();
                this.update_counters();
            }
        } catch (error) {
            console.error('Error checking connections:', error);
        }
    },

    app.show_about = async function () {
        if (this.modals.about) {
            // Load version information
            try {
                const versionRes = await fetch('/api/version');
                if (versionRes.ok) {
                    const versionData = await versionRes.json();
                    const versionElement = document.getElementById('versionNumber');
                    if (versionElement && versionData.version) {
                        versionElement.textContent = versionData.version;
                    }
                }
            } catch (e) {
                console.warn('Could not load version:', e);
                const versionElement = document.getElementById('versionNumber');
                if (versionElement) {
                    versionElement.textContent = 'Unknown';
                }
            }

            this.modals.about.show();
        } else {
            console.error('About modal not initialized');
        }
    };
app.show_loading = function () {
    if (this.elements.loadingOverlay) {
        // Add a class to dim the background if a modal is open
        const openModals = document.querySelectorAll('.modal.show');
        openModals.forEach(modal => {
            modal.classList.add('modal-loading');
        });

        this.elements.loadingOverlay.classList.remove('d-none');
    }
};

app.hide_loading = function () {
    if (this.elements.loadingOverlay) {
        // Remove the class that dims the background from all modals
        const openModals = document.querySelectorAll('.modal.show');
        openModals.forEach(modal => {
            modal.classList.remove('modal-loading');
        });

        this.elements.loadingOverlay.classList.add('d-none');
    }
};



// Initialize app when document is ready
document.addEventListener('DOMContentLoaded', () => app.init());
// Apply saved theme very early to avoid flash
(function () {
    document.documentElement.setAttribute('data-theme', 'light');
})();


// App initialized
document.addEventListener('DOMContentLoaded', () => app.init());
// Apply saved theme
(function () {
    document.documentElement.setAttribute('data-theme', 'light');
})();
