// connections.js: connections feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const connections = {
async execute_ssh_command(launchId) {
        try {
            const launch = this.ssh_launch_requests?.[launchId];
            if (!launch) throw new Error('SSH session details are no longer available');
            this.show_loading();
            const response = await fetch('/api/execute-ssh', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(launch)
            });
            if (!response.ok) {
                const errorData = await response.json();
                throw new Error(errorData.error || 'Failed to launch SSH command');
            }
            this.show_success('SSH command launched in your local terminal');
        } catch (error) {
            console.error('Error launching SSH command:', error);
            this.show_error('Failed to launch SSH command: ' + error.message);
        } finally {
            this.hide_loading();
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

generate_connection_id() {
        return 'conn_' + Math.random().toString(36).substr(2, 9);
    },

// Updated render_connections function to better handle connection information
render_connections: function () {
    const container = this.elements.connectionsList;
    const emptyState = document.getElementById('connectionsEmptyState');

    // Escape every backend-supplied value before it reaches innerHTML
    const h = (v) => this.escape_html(String(v ?? ''));

    // Delegated handlers: actions come from data-* attributes, never from inline JS strings
    container.onclick = (e) => {
        const el = e.target.closest('[data-action]');
        if (el) this.handle_connection_action(el);
    };
    container.onchange = (e) => {
        const el = e.target.closest('[data-action="pem-select"]');
        if (el) this.handle_pem_file_select(e, el.dataset.instanceId, el.dataset.passwordId, el.dataset.keyName || null);
    };

    container.innerHTML = '';

    if (this.connections.length === 0) {
        if (emptyState) emptyState.classList.remove('d-none');
        return;
    }

    // Show all connections regardless of selected profile/region
    const visibleConnections = this.connections;

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
                    Local Port: ${h(conn.localPort)}
                    ${conn.remotePort ? `, Remote Port: ${conn.remotePort}` : ''}
                    ${conn.remoteHost ? `, Host: ${h(conn.remoteHost)}` : ''}
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
                        <span class="font-monospace small cursor-pointer" id="${ipId}" data-action="copy" data-value="${h(info.ip)}" title="Click to copy">${h(info.ip)}</span>
                    </li>
                `;
            }
            if (info.port) {
                const portId = `${detailsId}-port`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>Port:</strong></span>
                        <span class="font-monospace small cursor-pointer" id="${portId}" data-action="copy" data-value="${h(info.port)}" title="Click to copy">${h(info.port)}</span>
                    </li>
                `;
            }
            if (info.user) {
                const userId = `${detailsId}-user`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>User:</strong></span>
                        <span class="font-monospace small cursor-pointer" id="${userId}" data-action="copy" data-value="${h(info.user)}" title="Click to copy">${h(info.user)}</span>
                    </li>
                `;
            }
            if (info.key_name) {
                const keyNameId = `${detailsId}-key-name`;
                detailsList += `
                    <li class="list-group-item d-flex justify-content-between align-items-center">
                        <span><strong>SSH Key:</strong></span>
                        <span class="font-monospace small cursor-pointer" id="${keyNameId}" data-action="copy" data-value="${h(info.key_name)}" title="Click to copy">${h(info.key_name)}</span>
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
                    // Structured launch data; the terminal command is never built from user-editable text
                    if (!this.ssh_launch_requests) this.ssh_launch_requests = {};
                    this.ssh_launch_requests[sshCommandId] = {
                        host: info.ip || '127.0.0.1',
                        port: Number(info.port),
                        user: info.user || this.preferences.linux_username || 'ec2-user',
                        key_path: info.key_path || null,
                    };
                    detailsList += `
                        <li class="list-group-item">
                            <div class="text-muted small mb-1"><strong>SSH Command:</strong></div>
                            <div class="bg-dark text-light p-2 rounded font-monospace small break-all-pointer" 
                                 id="${sshCommandId}"
                                 data-action="ssh" data-launch-id="${sshCommandId}" 
                                 title="Click to open a terminal and run this SSH command">
                                <code class="text-light">${this.escape_html(sshCommand)}</code>
                                <i class="bi bi-play-circle ms-2"></i>
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

                passwordField = `
                    <li class="list-group-item">
                        <div class="d-flex justify-content-between align-items-center mb-2">
                            <span><strong>Password:</strong></span>
                            <button class="btn btn-sm btn-outline-primary" data-action="decrypt" data-instance-id="${h(info.instance_id)}" data-password-id="${passwordValueId}" data-pem-id="${pemInputId}" title="Load PEM key to decrypt password">
                                <i class="bi bi-key"></i> Load Key
                            </button>
                        </div>
                        <input type="file" id="${pemInputId}" class="form-control form-control-sm mb-2 d-none" accept=".pem,.key" data-action="pem-select" data-instance-id="${h(info.instance_id)}" data-password-id="${passwordValueId}" data-key-name="${h(info.key_name || '')}">
                        <div id="${passwordValueId}" class="font-monospace small text-muted min-h-15">
                            <em>Click "Load Key" to decrypt password</em>
                        </div>
                    </li>
                `;

                connectButton = `
                    <li class="list-group-item">
                        <button class="btn btn-success w-100" data-action="rdp" data-ip="${h(info.ip || '127.0.0.1')}" data-port="${h(info.port || '')}" data-user="${h(info.user || 'Administrator')}" data-password-id="${passwordValueId}" title="Open RDP client and connect">
                            <i class="bi bi-display"></i> Connect with RDP Client
                        </button>
                    </li>
                `;

                if (info.key_name && this.preferences.ssh_key_folder) {
                    setTimeout(() => {
                        this.auto_decrypt_password(info.instance_id, passwordValueId, info.key_name || '');
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
                    <div class="bg-dark text-light p-2 rounded font-monospace small break-all-pointer" 
                         id="${commandId}"
                         data-command="${h(conn.command)}"
                         data-act="copy_command_from_element" data-id="${commandId}" 
                         title="Click to copy">
                        <code class="text-light">${h(conn.command)}</code>
                        <i class="bi bi-clipboard ms-2"></i>
                    </div>
                </div>
            `;
        }

        element.innerHTML = `
            <div class="d-flex justify-content-between align-items-start">
                <div class="flex-grow-1">
                    <div class="d-flex align-items-center gap-2">
                        <span class="badge ${conn.isRds ? '' : (this.get_connection_type_color(conn.type) === 'dark' ? 'bg-dark' : '')} ${conn.isRds ? '' : (this.get_connection_type_color(conn.type) === 'primary' ? 'bg-primary' : '')}"
                              ${conn.isRds || this.get_connection_type_color(conn.type) === '#9d7ce8' ? 'badge-purple' : ''}>
                            ${conn.isRds ? 'RDS' : h(conn.type)}
                        </span>
                    </div>
                    <div class="text-muted small"><b>ID: ${this.get_instance_name(conn.instanceId)}</b></div>
                    ${conn.account_alias || conn.account_id ? `<div class="text-muted small">Account: <b>${h(conn.account_alias || conn.account_id)}</b>${conn.profile ? ` · ${h(conn.profile)}` : ''}</div>` : ''}
                    ${connectionInfo}
                    ${connectionDetailsDisplay}
                    ${commandDisplay}
                    <div class="text-muted small">Started at ${timestamp}</div>
                </div>
                <button class="btn btn-sm btn-outline-danger ms-2" data-action="terminate" data-id="${h(conn.id)}" title="Terminate connection">
                    <i class="bi bi-x-lg"></i>
                </button>
            </div>
        `;

        container.appendChild(element);
    });
},

// Dispatches clicks from connection cards using only data-* attributes
handle_connection_action: function (el) {
    const d = el.dataset;
    switch (d.action) {
        case 'copy': this.copy_text(d.value); break;
        case 'ssh': this.execute_ssh_command(d.launchId); break;
        case 'terminate': this.terminate_connection(d.id); break;
        case 'decrypt': this.load_pem_key_and_decrypt(d.instanceId, d.passwordId, d.pemId); break;
        case 'rdp': this.launch_rdp_client(d.ip, d.port, d.user, d.passwordId); break;
        default: break;
    }
},

// Helper function to determine badge color based on connection type
get_connection_type_color: function (type) {
    const colors = {
        'SSH': 'dark',
        'RDP': 'primary',
        'Custom Port': '#9d7ce8',
        'Remote Host Port': '#9d7ce8'
    };
    return colors[type] || 'secondary';
},

start_connection_monitoring: function () {
    console.log('Starting connection monitoring');
    // Clear any existing monitoring interval
    if (this.monitoring_interval) {
        clearInterval(this.monitoring_interval);
    }
    this.monitoring_interval = setInterval(() => this.check_connections(), 2000);
},

check_connections: async function () {
        // Sequence number: a slow response for an older scope must not overwrite a newer one
        const seq = (this._connSeq = (this._connSeq || 0) + 1);
        try {
            const params = new URLSearchParams();
            if (this.connection_scope !== 'all') {
                if (this.current_profile) params.set('profile', this.current_profile);
                if (this.current_region) params.set('region', this.current_region);
            }
            const query = params.toString();
            const response = await fetch(`/api/active-connections${query ? `?${query}` : ''}`);
            if (!response.ok) throw new Error('Failed to check connections');
            if (seq !== this._connSeq) return;

            const activeConnections = await response.json();
            this.total_connections = activeConnections.length;
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
                    const connObj = {
                        id: connId,
                        instanceId: backendConn.instance_id,
                        localPort: backendConn.local_port,
                        remotePort: backendConn.remote_port,
                        remoteHost: backendConn.remote_host,
                        command: backendConn.command,
                        keyName: backendConn.key_name,
                        keyPath: backendConn.key_path,
                        profile: backendConn.profile,
                        account_id: backendConn.account_id,
                        account_alias: backendConn.account_alias,
                        region: backendConn.region,
                        timestamp: new Date().toISOString(),
                        connectionInfo: backendConn.connection_info
                    };
                    // Mark RDS connections (they have remote_host)
                    if (backendConn.remote_host) {
                        connObj.isRds = true;
                    }
                    this.connections.push(connObj);
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
