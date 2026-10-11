// prefs.js: prefs feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const prefs = {
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
                    data-act="remove_ssh_key_folder_input" data-id="${folderId}"
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

apply_dark_mode(enabled) {
        document.documentElement.setAttribute('data-bs-theme', enabled ? 'dark' : 'light');
    },

// Update the show_preferences method in app.js
show_preferences: async function () {
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

        // Load username preferences
        const linuxUsernameField = document.getElementById('linuxUsername');
        if (linuxUsernameField) {
            linuxUsernameField.value = prefs.linux_username || 'ec2-user';
        }
        const windowsUsernameField = document.getElementById('windowsUsername');
        if (windowsUsernameField) {
            windowsUsernameField.value = prefs.windows_username || 'Administrator';
        }

        // Load dark mode preference
        const prefDarkMode = document.getElementById('prefDarkMode');
        if (prefDarkMode) {
            prefDarkMode.checked = prefs.dark_mode || false;
            prefDarkMode.onchange = (e) => this.apply_dark_mode(e.target.checked);
        }
        const prefStrictHostKey = document.getElementById('prefStrictHostKey');
        if (prefStrictHostKey) {
            prefStrictHostKey.checked = prefs.strict_host_key_checking !== false;
        }
        await this.fill_preferred_selects(prefs.preferred_aws || {});

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
},

// Update the save_preferences method to ensure we're using the correct structure
save_preferences: async function () {
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
        const linuxUsername = (document.getElementById('linuxUsername')?.value || '').trim() || 'ec2-user';
        const windowsUsername = (document.getElementById('windowsUsername')?.value || '').trim() || 'Administrator';

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
            cache_ttl_secs: cacheTtl,
            linux_username: linuxUsername,
            windows_username: windowsUsername
        };

        // Add SSH key folder if provided (newline-separated)
        if (sshKeyFolder) {
            newPreferences.ssh_key_folder = sshKeyFolder;
        }

        // Dark mode
        const prefDarkMode = document.getElementById('prefDarkMode');
        newPreferences.dark_mode = prefDarkMode ? prefDarkMode.checked : false;

        // Strict host key checking (used by SSH launches and SCP transfers)
        const prefStrictHostKey = document.getElementById('prefStrictHostKey');
        newPreferences.strict_host_key_checking = prefStrictHostKey ? prefStrictHostKey.checked : true;

        // Preferred profile and region (empty means "last used")
        newPreferences.preferred_aws = {
            profile: document.getElementById('prefProfile')?.value || null,
            region: document.getElementById('prefRegion')?.value || null,
        };

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
        this.preferences.linux_username = linuxUsername;
        this.preferences.windows_username = windowsUsername;

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
},

// Update load_preferences to match backend structure
load_preferences: async function () {
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
        this.preferences.linux_username = prefs.linux_username || 'ec2-user';
        this.preferences.windows_username = prefs.windows_username || 'Administrator';
        this.preferences.dark_mode = prefs.dark_mode || false;
        this.apply_dark_mode(this.preferences.dark_mode);

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
},

// Setup keyboard shortcuts
setup_keyboard_shortcuts: function () {
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
},
    // Fills the Settings profile and region selects. Profiles come from the backend; regions use the
    // standard list so the field works before a profile is connected.
    async fill_preferred_selects(preferred) {
        const profileSel = document.getElementById('prefProfile');
        const regionSel = document.getElementById('prefRegion');
        if (profileSel && profileSel.options.length <= 1) {
            try {
                const res = await fetch('/api/profiles');
                const profiles = res.ok ? await res.json() : [];
                profileSel.innerHTML = '<option value="">Last used</option>' +
                    profiles.map(p => `<option value="${this.escape_html(p)}">${this.escape_html(p)}</option>`).join('');
            } catch (_) { /* keep "Last used" only */ }
        }
        if (profileSel) profileSel.value = preferred.profile || '';
        if (regionSel && regionSel.options.length <= 1) {
            const regions = ['us-east-1', 'us-east-2', 'us-west-1', 'us-west-2', 'eu-west-1', 'eu-west-2',
                'eu-west-3', 'eu-central-1', 'eu-north-1', 'ap-south-1', 'ap-southeast-1', 'ap-southeast-2',
                'ap-northeast-1', 'ap-northeast-2', 'ca-central-1', 'sa-east-1'];
            regionSel.innerHTML = '<option value="">Last used</option>' +
                regions.map(r => `<option value="${r}">${r}</option>`).join('');
        }
        if (regionSel) regionSel.value = preferred.region || '';
    },
};
