// instances.js: instances feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const instances = {
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

            if (this.filter_active_only) {
                const activeIds = new Set(this.connections.map(c => c.instanceId || c.instance_id));
                filtered = filtered.filter(i => activeIds.has(i.id));
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
                            <div class="info-item min-w-220">
                                <span class="info-label text-primary">Instance</span>
                                <span class="info-value fw-bold text-truncate" title="${safeName}">${safeName}</span>
                                <span class="text-xs text-muted text-tiny">${safeId}</span>
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
        const isWindows = instance && instance.os && instance.os.toLowerCase().includes('win');

        return `
            <div class="btn-group shadow-sm">
                ${hasSsm ? `
                    <button class="btn btn-sm btn-dark" data-act="start_ssh" data-id="${safeInstanceId}" title="SSH Terminal">
                        <i class="bi bi-terminal"></i>
                    </button>
                    <button class="btn btn-sm btn-primary" data-act="start_rdp" data-id="${safeInstanceId}" title="RDP Desktop">
                        <i class="bi bi-display"></i>
                    </button>
                    <button class="btn btn-sm btn-purple text-white" data-act="show_custom_port_modal" data-id="${safeInstanceId}" title="Port Forwarding">
                        <i class="bi bi-arrow-left-right"></i>
                    </button>
                    ${!isWindows ? `
                        <button class="btn btn-sm btn-outline-secondary" data-act="show_file_transfer_modal" data-id="${safeInstanceId}" title="File Transfer (SCP)">
                            <i class="bi bi-file-earmark-arrow-up"></i>
                        </button>
                    ` : ''}
                ` : ''}
                <button class="btn btn-sm btn-ottanio text-white" data-act="show_instance_details" data-id="${safeInstanceId}" title="Instance Info">
                    <i class="bi bi-info-circle"></i>
                </button>
            </div>
        `;
    },

toggle_active_filter() {
        this.filter_active_only = !this.filter_active_only;
        const btn = document.getElementById('filterActiveBtn');
        if (btn) {
            if (this.filter_active_only) {
                btn.classList.remove('text-bg-warning');
                btn.classList.add('text-bg-dark');
            } else {
                btn.classList.remove('text-bg-dark');
                btn.classList.add('text-bg-warning');
            }
        }
        this.render_instances();
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
                            ${details.created_at ? this.create_detail_row('Created at', details.created_at) : ''}
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

        // Build DOM elements programmatically to avoid XSS
        const container = document.createElement('div');
        container.className = 'd-flex justify-content-between align-items-center';

        const span = document.createElement('span');
        span.id = passwordId;
        span.className = 'font-monospace';
        span.style.cursor = 'pointer';
        span.title = 'Click to copy';
        span.textContent = this.mask_password(password);
        span.dataset.password = password;
        span.onclick = () => this.copy_to_clipboard(password);

        const button = document.createElement('button');
        button.className = 'btn btn-sm btn-link p-0 ms-2';
        button.title = 'Show/Hide password';
        button.dataset.passwordId = passwordId;
        button.dataset.password = password;
        button.onclick = () => this.toggle_password_visibility(passwordId, password);

        const icon = document.createElement('i');
        icon.id = `${passwordId}-icon`;
        icon.className = 'bi bi-eye';

        button.appendChild(icon);
        container.appendChild(span);
        container.appendChild(button);
        passwordElement.replaceChildren(container);
    },

// Utility methods for connections
    get_instance_name(instanceId) {
        const instance = this.instances.find(i => i.id === instanceId);
        return instance ? instance.name : instanceId;
    },

// Custom port forwarding
start_custom_port_forwarding: async function () {
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
}
};
