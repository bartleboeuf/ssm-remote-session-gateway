// database.js: database feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const database = {
// Load RDS databases
    async load_rds_databases() {
        const rdsLoadingState = document.getElementById('rdsLoadingState');
        const rdsEmptyState = document.getElementById('rdsEmptyState');
        try {
            console.log('[RDS] Loading databases from /api/rds...');
            if (rdsLoadingState) rdsLoadingState.classList.remove('d-none');
            if (rdsEmptyState) rdsEmptyState.classList.add('d-none');

            const response = await fetch('/api/rds');
            console.log('[RDS] Response status:', response.status);

            if (!response.ok) {
                const errorText = await response.text();
                throw new Error(`HTTP ${response.status}: ${errorText}`);
            }

            const databases = await response.json();
            console.log('[RDS] Loaded databases:', databases);
            this.rds_databases = databases;
            this.render_rds_databases();

            if (rdsLoadingState) rdsLoadingState.classList.add('d-none');
        } catch (error) {
            console.error('[RDS] Error loading RDS databases:', error);
            this.show_error('Failed to load RDS databases: ' + error.message);
            if (rdsLoadingState) rdsLoadingState.classList.add('d-none');
        }
    },

// Render RDS databases
    render_rds_databases() {
        const container = document.getElementById('rdsList');
        const emptyState = document.getElementById('rdsEmptyState');
        const countBadge = document.getElementById('rdsCount');
        const filterInput = document.getElementById('rdsFilter');

        if (!this.rds_databases || this.rds_databases.length === 0) {
            container.innerHTML = '';
            emptyState.classList.remove('d-none');
            countBadge.textContent = '0 databases';
            return;
        }

        emptyState.classList.add('d-none');

        const filterValue = filterInput?.value || '';
        let filtered = this.rds_databases;

        if (filterValue) {
            try {
                const regex = new RegExp(filterValue, 'i');
                filtered = this.rds_databases.filter(db => regex.test(db.db_instance_identifier));
            } catch (e) {
                console.warn('Invalid regex filter:', e);
            }
        }

        container.innerHTML = filtered.map(db => `
            <div class="col-md-12">
                <div class="card instance-card">
                    <div class="card-body">
                        <div class="d-flex justify-content-between align-items-start mb-3">
                            <div>
                                <h6 class="mb-1">${this.escape_html(db.db_instance_identifier)}</h6>
                                <small class="text-muted">Engine: ${this.escape_html(db.engine.toUpperCase())}</small>
                            </div>
                            <span class="badge ${db.status === 'available' ? 'bg-success' : 'bg-warning'}">${this.escape_html(db.status)}</span>
                        </div>
                        <div class="instance-info-grid mb-3">
                            <div class="info-item">
                                <span class="info-label">Endpoint</span>
                                <span class="info-value text-break">${this.escape_html(db.endpoint)}</span>
                            </div>
                            <div class="d-flex justify-content-between align-items-center gap-2">
                                <span><span class="info-label">Port</span> <span class="info-value">${db.port}</span></span>
                                <button class="btn btn-primary btn-sm" data-act="show_rds_port_modal" data-db-id="${this.escape_html(String(db.db_instance_identifier))}" data-endpoint="${this.escape_html(String(db.endpoint))}" data-port="${this.escape_html(String(db.port))}" data-engine="${this.escape_html(String(db.engine))}" title="Set up port forwarding">
                                    <i class="bi bi-arrow-left-right me-1"></i> Port Forward
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        `).join('');

        countBadge.textContent = `${filtered.length} database${filtered.length !== 1 ? 's' : ''}`;
    },

// Show RDS port modal
    show_rds_port_modal(db) {
        this.current_rds_db = db;
        document.getElementById('rdsEndpoint').value = db.endpoint;
        document.getElementById('rdsRemotePort').value = db.port;
        document.getElementById('rdsLocalPort').value = '';

        // Populate EC2 instances with SSM support
        const instanceSelect = document.getElementById('rdsInstanceSelect');
        instanceSelect.innerHTML = '<option value="">Select an EC2 instance...</option>';

        if (this.instances && this.instances.length > 0) {
            const ssmInstances = this.instances.filter(i => i.has_ssm);
            if (ssmInstances.length > 0) {
                ssmInstances.forEach(instance => {
                    const option = document.createElement('option');
                    option.value = instance.id;
                    option.textContent = `${instance.name || instance.id} (${instance.id})`;
                    instanceSelect.appendChild(option);
                });
            } else {
                const option = document.createElement('option');
                option.value = '';
                option.textContent = 'No instances with SSM support available';
                option.disabled = true;
                instanceSelect.appendChild(option);
            }
        }

        // Set preset buttons
        const isMysql = db.engine.toLowerCase().includes('mysql');
        const isPostgres = db.engine.toLowerCase().includes('postgres');
        const isAurora = db.engine.toLowerCase().includes('aurora');

        document.getElementById('portMysql').onclick = () => {
            document.getElementById('rdsRemotePort').value = '3306';
        };
        document.getElementById('portPostgres').onclick = () => {
            document.getElementById('rdsRemotePort').value = '5432';
        };
        document.getElementById('portAurora').onclick = () => {
            document.getElementById('rdsRemotePort').value = '3306';
        };

        this.modals.rdsPort.show();
    },

// Load ECS tasks
    async load_ecs_tasks() {
        const ecsLoadingState = document.getElementById('ecsLoadingState');
        const ecsEmptyState = document.getElementById('ecsEmptyState');
        try {
            console.log('[ECS] Loading tasks from /api/ecs...');
            if (ecsLoadingState) ecsLoadingState.classList.remove('d-none');
            if (ecsEmptyState) ecsEmptyState.classList.add('d-none');

            const response = await fetch('/api/ecs');
            console.log('[ECS] Response status:', response.status);

            if (!response.ok) {
                const errorText = await response.text();
                throw new Error(`HTTP ${response.status}: ${errorText}`);
            }

            const tasks = await response.json();
            console.log('[ECS] Loaded tasks:', tasks);
            this.ecs_tasks = tasks;
            this.render_ecs_tasks();

            if (ecsLoadingState) ecsLoadingState.classList.add('d-none');
        } catch (error) {
            console.error('[ECS] Error loading ECS tasks:', error);
            this.show_error('Failed to load ECS tasks: ' + error.message);
            if (ecsLoadingState) ecsLoadingState.classList.add('d-none');
        }
    },

// Render ECS tasks
    render_ecs_tasks() {
        const container = document.getElementById('ecsList');
        const emptyState = document.getElementById('ecsEmptyState');
        const countBadge = document.getElementById('ecsCount');
        const filterInput = document.getElementById('ecsFilter');

        if (!this.ecs_tasks || this.ecs_tasks.length === 0) {
            container.innerHTML = '';
            emptyState.classList.remove('d-none');
            countBadge.textContent = '0 tasks';
            return;
        }

        emptyState.classList.add('d-none');

        const filterValue = filterInput?.value || '';
        let filtered = this.ecs_tasks;

        if (filterValue) {
            try {
                const regex = new RegExp(filterValue, 'i');
                filtered = this.ecs_tasks.filter(t => regex.test(t.task_arn));
            } catch (e) {
                console.warn('Invalid regex filter:', e);
            }
        }

        container.innerHTML = filtered.map(task => `
            <div class="col-md-12">
                <div class="card instance-card">
                    <div class="card-body">
                        <div class="d-flex justify-content-between align-items-start mb-3">
                            <div>
                                <h6 class="mb-1">${this.escape_html(task.task_arn.split('/').pop())}</h6>
                                <small class="text-muted">Cluster: ${this.escape_html(task.cluster_name)}</small>
                            </div>
                            <span class="badge ${task.status === 'RUNNING' ? 'bg-success' : 'bg-warning'}">${this.escape_html(task.status)}</span>
                        </div>
                        <div class="instance-info-grid mb-3">
                            <div class="info-item">
                                <span class="info-label">Task Definition</span>
                                <span class="info-value text-break">${this.escape_html(task.task_definition)}</span>
                            </div>
                            <div class="info-item">
                                <span class="info-label">Launch Type</span>
                                <span class="info-value">${this.escape_html(task.launch_type)}</span>
                            </div>
                            <div class="info-item">
                                <span class="info-label">Container</span>
                                <span class="info-value">${this.escape_html(task.container_name)}</span>
                            </div>
                        </div>
                        <button class="btn btn-primary btn-sm" data-act="show_ecs_port_modal" data-task-arn="${this.escape_html(String(task.task_arn))}" data-container-name="${this.escape_html(String(task.container_name))}" data-cluster-name="${this.escape_html(String(task.cluster_name))}">
                            <i class="bi bi-arrow-left-right me-1"></i> Port Forward
                        </button>
                    </div>
                </div>
            </div>
        `).join('');

        countBadge.textContent = `${filtered.length} task${filtered.length !== 1 ? 's' : ''}`;
    },

// Show ECS port modal
    show_ecs_port_modal(task) {
        this.current_ecs_task = task;
        document.getElementById('ecsTaskArn').value = task.task_arn;
        document.getElementById('ecsRemotePort').value = '80';
        document.getElementById('ecsLocalPort').value = '';

        // Populate EC2 instances with SSM support
        const instanceSelect = document.getElementById('ecsInstanceSelect');
        instanceSelect.innerHTML = '<option value="">Select an EC2 instance...</option>';

        if (this.instances && this.instances.length > 0) {
            const ssmInstances = this.instances.filter(i => i.has_ssm);
            if (ssmInstances.length > 0) {
                ssmInstances.forEach(instance => {
                    const option = document.createElement('option');
                    option.value = instance.id;
                    option.textContent = `${instance.name || instance.id} (${instance.id})`;
                    instanceSelect.appendChild(option);
                });
            } else {
                const option = document.createElement('option');
                option.value = '';
                option.textContent = 'No instances with SSM support available';
                option.disabled = true;
                instanceSelect.appendChild(option);
            }
        }

        this.modals.ecsPort.show();
    },

// Start RDS port forwarding
    async start_rds_port_forwarding() {
        if (!this.current_rds_db) {
            this.show_error('No RDS database selected');
            return;
        }

        const instanceId = document.getElementById('rdsInstanceSelect').value;
        if (!instanceId) {
            this.show_error('Please select an EC2 instance to route through');
            return;
        }

        const remotePort = parseInt(document.getElementById('rdsRemotePort').value);
        const localPort = document.getElementById('rdsLocalPort').value ? parseInt(document.getElementById('rdsLocalPort').value) : null;

        if (!remotePort || remotePort < 1 || remotePort > 65535) {
            this.show_error('Invalid remote port');
            return;
        }

        try {
            const response = await fetch(`/api/custom-port/${instanceId}`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    remote_port: remotePort,
                    local_port: localPort || null,
                    remote_host: this.current_rds_db.endpoint
                })
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(error.error || 'Failed to start port forwarding');
            }

            const connection = await response.json();
            connection.id = connection.connection_id || connection.id;  // Normalize ID field
            connection.type = 'Custom Port';  // RDS uses custom port forwarding
            connection.localPort = connection.local_port;  // Map snake_case to camelCase
            connection.remotePort = connection.remote_port;
            connection.connectionInfo = connection.connection_info || null;
            connection.timestamp = new Date().toISOString();
            connection.instanceId = this.current_rds_db.db_instance_identifier;
            connection.remoteHost = this.current_rds_db.endpoint;
            connection.isRds = true;
            this.connections.push(connection);
            this.render_connections();
            this.update_counters();
            this.modals.rdsPort.hide();
            this.show_toast(`RDS port forwarding started on localhost:${connection.local_port}`, 'success');
        } catch (error) {
            console.error('Error starting RDS port forwarding:', error);
            this.show_error('Failed to start RDS port forwarding: ' + error.message);
        }
    },

// Start ECS port forwarding
    async start_ecs_port_forwarding() {
        if (!this.current_ecs_task) {
            this.show_error('No ECS task selected');
            return;
        }

        const instanceId = document.getElementById('ecsInstanceSelect').value;
        if (!instanceId) {
            this.show_error('Please select an EC2 instance to route through');
            return;
        }

        const remotePort = parseInt(document.getElementById('ecsRemotePort').value);
        const localPort = document.getElementById('ecsLocalPort').value ? parseInt(document.getElementById('ecsLocalPort').value) : null;

        if (!remotePort || remotePort < 1 || remotePort > 65535) {
            this.show_error('Invalid remote port');
            return;
        }

        try {
            const response = await fetch(`/api/custom-port/${instanceId}`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    remote_port: remotePort,
                    local_port: localPort || null,
                    remote_host: this.current_ecs_task.task_arn
                })
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(error.error || 'Failed to start port forwarding');
            }

            const connection = await response.json();
            connection.id = connection.connection_id || connection.id;  // Normalize ID field
            connection.type = 'Custom Port';  // ECS uses custom port forwarding
            connection.localPort = connection.local_port;  // Map snake_case to camelCase
            connection.remotePort = connection.remote_port;
            connection.connectionInfo = connection.connection_info || null;
            connection.timestamp = new Date().toISOString();
            connection.instanceId = this.current_ecs_task.task_arn;
            this.connections.push(connection);
            this.render_connections();
            this.update_counters();
            this.modals.ecsPort.hide();
            this.show_toast(`ECS port forwarding started on localhost:${connection.local_port}`, 'success');
        } catch (error) {
            console.error('Error starting ECS port forwarding:', error);
            this.show_error('Failed to start ECS port forwarding: ' + error.message);
        }
    }
};
