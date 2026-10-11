// session.js: session feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const session = {
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
                        // Open installation page if session-manager-plugin is not installed
                        if (!health.session_manager_plugin) {
                            window.open('https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html', '_blank');
                        }
                    } else if (health.session_manager_plugin) {
                        // Check for latest version on GitHub
                        try {
                            const tagsRes = await fetch('https://api.github.com/repos/aws/session-manager-plugin/tags');
                            if (tagsRes.ok) {
                                const tags = await tagsRes.json();
                                if (Array.isArray(tags) && tags.length > 0) {
                                    const latestVersion = tags[0].name;
                                    const installedVersion = health.session_manager_plugin_version;
                                    if (latestVersion && installedVersion && latestVersion !== installedVersion) {
                                        const upgradeBanner = document.createElement('div');
                                        upgradeBanner.id = 'upgradePluginBanner';
                                        upgradeBanner.style.cssText = 'background:#f0ad4e;color:#fff;padding:10px 20px;text-align:center;font-weight:600;position:fixed;top:0;left:0;right:0;z-index:9999;';
                                        upgradeBanner.innerHTML =
                                            `⚠ A newer version of Session Manager Plugin is available ` +
                                            `(installed: ${this.escape_html(installedVersion)}, latest: ${this.escape_html(latestVersion)}). ` +
                                            `<a href="#" id="upgradePluginLink" class="upgrade-link">Download update</a>` +
                                            ` <button data-act="dismiss_upgrade_banner" ` +
                                            `class="upgrade-close">✕</button>`;
                                        document.body.prepend(upgradeBanner);
                                        document.body.style.paddingTop = `${upgradeBanner.offsetHeight}px`;
                                        document.getElementById('upgradePluginLink').onclick = (e) => {
                                            e.preventDefault();
                                            window.open('https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html', '_blank');
                                        };
                                    }
                                }
                            }
                        } catch (e) {
                            console.warn('Failed to check latest session-manager-plugin version:', e);
                        }
                    }
                    if (!health.aws_credentials) app.show_toast('No AWS credentials detected. Configure your profile.', 'warning');
                    // Initialize About button popover with session-manager-plugin version
                    const aboutBtn = document.getElementById('aboutBtn');
                    if (aboutBtn) {
                        const smpVersion = health.session_manager_plugin_version || 'unknown';
                        new bootstrap.Popover(aboutBtn, {
                            content: `Session Manager Plugin: ${smpVersion}`,
                            trigger: 'hover focus',
                            placement: 'bottom',
                            title: 'Version Info',
                        });
                    }
                    console.log('Health:', health);
                }
            } catch (e) {
                console.warn('Health check failed:', e);
            }

            this.start_connection_monitoring();
            this.setup_shutdown_handlers();
            this.check_for_update(); // fire-and-forget
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

        // Initialize RDS port modal
        const rdsPortModal = document.getElementById('rdsPortModal');
        if (rdsPortModal) {
            this.modals.rdsPort = new bootstrap.Modal(rdsPortModal);
        } else {
            console.warn('RDS port modal element not found');
        }

        // Initialize ECS port modal
        const ecsPortModal = document.getElementById('ecsPortModal');
        if (ecsPortModal) {
            this.modals.ecsPort = new bootstrap.Modal(ecsPortModal);
        } else {
            console.warn('ECS port modal element not found');
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
        const fileTransferModal = document.getElementById('fileTransferModal');
        if (fileTransferModal) {
            this.modals.fileTransfer = new bootstrap.Modal(fileTransferModal);
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

        // Resource type selector
        const resourceTypeSelect = document.getElementById('resourceTypeSelect');
        if (resourceTypeSelect) {
            resourceTypeSelect.onchange = (e) => {
                const type = e.target.value;
                console.log(`[Resource Selector] Changed to: ${type}, Profile: ${this.current_profile}, Region: ${this.current_region}`);

                document.querySelectorAll('.resource-section').forEach(s => s.classList.add('d-none'));
                const sectionId = `${type}Section`;
                const section = document.getElementById(sectionId);
                if (section) {
                    section.classList.remove('d-none');
                } else {
                    console.error(`Section ${sectionId} not found`);
                }

                if (type === 'rds') {
                    console.log('Loading RDS databases...');
                    this.load_rds_databases();
                } else if (type === 'ecs') {
                    console.log('Loading ECS tasks...');
                    this.load_ecs_tasks();
                } else {
                    console.log('Loading EC2 instances...');
                    this.load_instances();
                }
            };
        } else {
            console.warn('resourceTypeSelect element not found');
        }

        // RDS filter (regex)
        const rdsFilter = document.getElementById('rdsFilter');
        if (rdsFilter) {
            rdsFilter.oninput = this.debounce(() => {
                this.render_rds_databases();
            }, 200);
        }

        // ECS filter (regex)
        const ecsFilter = document.getElementById('ecsFilter');
        if (ecsFilter) {
            ecsFilter.oninput = this.debounce(() => {
                this.render_ecs_tasks();
            }, 200);
        }

        // RDS port forwarding button
        const startRdsPortForwardingBtn = document.getElementById('startRdsPortForwardingBtn');
        if (startRdsPortForwardingBtn) {
            startRdsPortForwardingBtn.onclick = () => this.start_rds_port_forwarding();
        }

        // ECS port forwarding button
        const startEcsPortForwardingBtn = document.getElementById('startEcsPortForwardingBtn');
        if (startEcsPortForwardingBtn) {
            startEcsPortForwardingBtn.onclick = () => this.start_ecs_port_forwarding();
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
        this.setup_sidebar_positioning();
    },

// Setup sidebar positioning to align with main content section
    setup_sidebar_positioning() {
        const calculateSidebarPosition = () => {
            const mainContent = document.getElementById('main-content');
            const sidebar = document.getElementById('connectionsPanel');

            if (mainContent && sidebar) {
                // Get the viewport position of where the main content section starts
                const rect = mainContent.getBoundingClientRect();
                const sidebarTop = rect.top;
                document.documentElement.style.setProperty('--sidebar-top', `${sidebarTop}px`);
                console.log(`[Sidebar] Positioned at top: ${sidebarTop}px (aligned with main content section)`);
            }
        };

        // Calculate on init
        setTimeout(calculateSidebarPosition, 100);

        // Recalculate on window resize
        window.addEventListener('resize', calculateSidebarPosition);
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
    async check_for_update() {
        try {
            const resp = await fetch('/api/check-update');
            if (!resp.ok) return;
            const data = await resp.json();
            if (data.has_update) {
                const banner = document.getElementById('updateBanner');
                const versionSpan = document.getElementById('updateLatestVersion');
                const releaseLink = document.getElementById('updateReleaseLink');
                if (versionSpan) versionSpan.textContent = data.latest_version;
                if (releaseLink && data.release_url) releaseLink.href = data.release_url;
                if (banner) banner.classList.remove('d-none');
            }
        } catch (_) { /* update check is non-critical, ignore errors */ }
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
                    // A preferred profile/region set in Settings wins over the last used connection
                    const pref = lastConn.preferred_aws || {};
                    if (pref.profile) savedProfile = pref.profile;
                    if (pref.region) savedRegion = pref.region;
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

                // Switching profile re-scopes the view; live sessions on other profiles keep running
                this.current_profile = profile;
                await this.reload_for_selection();
                if (this.check_connections) await this.check_connections();
            });

            // Force disconnect when region changes
            this.elements.regionSelect.addEventListener('change', async () => {
                // Switching scope must not end live sessions on other profiles or regions.
                // Only the scope-dependent lists are cleared; connections re-scope on the next poll.
                this.current_region = this.elements.regionSelect.value || '';
                await this.reload_for_selection();
                if (this.check_connections) await this.check_connections();
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
        this.filter_active_only = false;
        const filterActiveBtn = document.getElementById('filterActiveBtn');
        if (filterActiveBtn) {
            filterActiveBtn.classList.remove('text-bg-dark');
            filterActiveBtn.classList.add('text-bg-warning');
        }
        this.rds_databases = [];
        this.ecs_tasks = [];

        this.update_aws_account_display();
        this.elements.connectBtn.innerHTML = '<i class="bi bi-plug"></i> Connect';
        this.elements.connectBtn.classList.replace('btn-danger', 'btn-success');
        this.elements.instancesList.innerHTML = '';
        this.elements.connectionsList.innerHTML = '';
        this.render_rds_databases();
        this.render_ecs_tasks();
        this.update_counters();

        if (this.auto_refresh_interval) {
            this.toggle_auto_refresh({ target: { checked: false } });
        }

        this.show_success('Disconnected successfully');
    },

refresh_data: async function () {
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
},

toggle_auto_refresh: function (enabled) {
    if (enabled && enabled.target) {
        enabled = enabled.target.checked;
    }

    console.log('Toggle auto-refresh:', enabled);

    if (enabled) {
        this.start_auto_refresh();
    } else {
        this.stop_auto_refresh();
    }
},

start_auto_refresh: function () {
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
},

stop_auto_refresh: function () {
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
},

setup_event_listeners: function () {
    console.log('Setting up event listeners...');
    this.elements.connectBtn.onclick = () => this.toggle_connection();
    this.elements.refreshBtn.onclick = () => this.refresh_data();

    // Modifica la gestione dell'evento autoRefreshSwitch
    this.elements.autoRefreshSwitch.onchange = (e) => {
        this.toggle_auto_refresh(e.target.checked);
    };
},

update_refresh_timer: function () {
    // Only show countdown if auto-refresh is active
    if (this.elements.autoRefreshSwitch.checked && this.refresh_countdown > 0) {
        this.elements.refreshTimer.textContent = `(${this.refresh_countdown}s)`;
    } else {
        this.elements.refreshTimer.textContent = '';
    }
},
    // Disconnects only the connections for the currently selected profile and region, after confirmation.
    async disconnect_scope() {
        const profile = this.elements.profileSelect?.value || '';
        const region = this.elements.regionSelect?.value || '';
        const label = profile && region ? `${profile} / ${region}` : (profile || region || 'all profiles');
        if (!window.confirm(`Disconnect all active connections for ${label}?`)) return;
        try {
            const res = await fetch('/api/disconnect', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ profile: profile || null, region: region || null }),
            });
            if (!res.ok) throw new Error(await res.text());
            const { disconnected } = await res.json();
            this.show_success(`Disconnected ${disconnected ?? 0} connection(s) for ${label}.`);
        } catch (err) {
            this.show_error(`Could not disconnect: ${err.message}`);
        }
        await this.check_connections();
    },
    // Switches the sidebar between the selected profile/region ("scope") and every connection ("all").
    // Only the view changes; no connection is ended.
    set_connection_scope(scope) {
        this.connection_scope = scope === 'all' ? 'all' : 'scope';
        document.getElementById('scopeThisBtn')?.classList.toggle('active', this.connection_scope === 'scope');
        document.getElementById('scopeAllBtn')?.classList.toggle('active', this.connection_scope === 'all');
        this.check_connections();
    },
    // Reloads resource lists after the profile or region changes while connected.
    // Validates and saves the new selection through /api/connect (the backend lists resources
    // for the saved profile/region), then loads instances, RDS and ECS. Live sessions are untouched.
    async reload_for_selection() {
        if (!this.is_connected) return;
        const profile = this.elements.profileSelect?.value || '';
        const region = this.elements.regionSelect?.value || '';
        if (!profile || !region) return;

        this.show_loading();
        try {
            const response = await fetch('/api/connect', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ profile, region })
            });
            const result = await response.json();
            if (!response.ok || result.status !== 'success') {
                throw new Error(result.error || 'Connection failed');
            }
            this.current_profile = profile;
            this.current_region = region;
            this.aws_account_id = result.account_id;
            this.aws_account_alias = result.account_alias || null;
            this.update_aws_account_display();

            this.instances = [];
            this.rds_databases = [];
            this.ecs_tasks = [];
            const state = this.elements.instanceStateFilter?.value || null;
            const os = this.elements.instanceOsFilter?.value || '';
            await this.load_instances(state, os);
            if (this.load_rds_databases) await this.load_rds_databases();
            if (this.load_ecs_tasks) await this.load_ecs_tasks();
        } catch (err) {
            this.instances = [];
            this.rds_databases = [];
            this.ecs_tasks = [];
            this.render_instances();
            this.render_rds_databases();
            this.render_ecs_tasks();
            this.show_error(`Could not load resources for ${profile} / ${region}: ${err.message}`);
        } finally {
            this.hide_loading();
        }
    },
};
