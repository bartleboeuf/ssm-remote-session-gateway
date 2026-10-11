// transfer.js: transfer feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const transfer = {
/** Open the file-transfer modal for a given instance */
    async show_file_transfer_modal(instanceId) {
        // Check SCP availability
        try {
            const resp = await fetch('/api/check-scp');
            const data = await resp.json();
            const warning = document.getElementById('scpWarning');
            const startBtn = document.getElementById('startTransferBtn');
            if (!data.available) {
                if (warning) warning.classList.remove('d-none');
                if (startBtn) startBtn.disabled = true;
            } else {
                if (warning) warning.classList.add('d-none');
                if (startBtn) startBtn.disabled = false;
            }
        } catch (_) { /* ignore */ }

        // Pre-fill username from preferences
        const usernameField = document.getElementById('transferUsername');
        if (usernameField) usernameField.value = this.preferences?.linux_username || 'ec2-user';

        // A transfer may still be running on the server; resume its view instead of resetting it
        this._stop_transfer_poll();
        const resuming = Boolean(this.current_transfer_id);
        if (resuming) {
            document.getElementById('transferProgress')?.classList.remove('d-none');
            document.getElementById('startTransferBtn')?.classList.add('d-none');
            document.getElementById('cancelTransferBtn')?.classList.remove('d-none');
        } else {
            document.getElementById('transferProgress')?.classList.add('d-none');
            document.getElementById('cancelTransferBtn')?.classList.add('d-none');
            document.getElementById('startTransferBtn')?.classList.remove('d-none');
            this._reset_transfer_progress_ui();
        }

        // Reset direction to upload
        const dirUpload = document.getElementById('dirUpload');
        if (dirUpload) dirUpload.checked = true;
        this._toggle_transfer_direction();

        // Wire up direction radio buttons
        document.querySelectorAll('input[name="transferDir"]').forEach(radio => {
            radio.onchange = () => this._toggle_transfer_direction();
        });

        this.file_transfer_instance_id = instanceId;
        this._bind_transfer_modal_events();
        if (resuming) this._start_transfer_poll();
        if (this.modals.fileTransfer) this.modals.fileTransfer.show();
    },

// Stops polling when the modal closes; the transfer keeps running on the server
    _bind_transfer_modal_events() {
        const modalEl = document.getElementById('fileTransferModal');
        if (!modalEl || modalEl.dataset.transferBound) return;
        modalEl.dataset.transferBound = '1';
        modalEl.addEventListener('hidden.bs.modal', () => {
            this._stop_transfer_poll();
            if (this.current_transfer_id) {
                this.show_toast('Transfer continues in the background. Reopen File Transfer to view progress.', 'info');
            }
        });
    },

_toggle_transfer_direction() {
        const isUpload = document.getElementById('dirUpload')?.checked;
        const uploadFields = document.getElementById('uploadFields');
        const downloadFields = document.getElementById('downloadFields');
        if (uploadFields) uploadFields.classList.toggle('d-none', !isUpload);
        if (downloadFields) downloadFields.classList.toggle('d-none', isUpload);
    },

async start_transfer() {
        const instanceId = this.file_transfer_instance_id;
        if (!instanceId) return;
        if (this.current_transfer_id) {
            this.show_error('A transfer is already in progress.');
            return;
        }

        const isUpload = document.getElementById('dirUpload')?.checked;
        const username = document.getElementById('transferUsername')?.value.trim() || '';
        const key_path = document.getElementById('transferKeyPath')?.value.trim() || '';

        let local_path, remote_path;
        if (isUpload) {
            local_path = document.getElementById('transferLocalFile')?.value.trim() || '';
            remote_path = document.getElementById('transferRemotePath')?.value.trim() || '';
        } else {
            remote_path = document.getElementById('transferRemoteFile')?.value.trim() || '';
            local_path = document.getElementById('transferLocalFolder')?.value.trim() || '';
        }

        if (!local_path || !remote_path) {
            this.show_error('Please fill in both local and remote paths.');
            return;
        }

        const body = {
            direction: isUpload ? 'upload' : 'download',
            local_path,
            remote_path,
            username: username || undefined,
            key_path: key_path || undefined,
        };

        document.getElementById('transferProgress')?.classList.remove('d-none');
        document.getElementById('startTransferBtn')?.classList.add('d-none');
        document.getElementById('cancelTransferBtn')?.classList.remove('d-none');
        const statusLabel = document.getElementById('transferStatusLabel');
        if (statusLabel) statusLabel.textContent = 'Connecting…';

        try {
            const resp = await fetch(`/api/transfer/${instanceId}`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            });
            if (!resp.ok) {
                const errText = await resp.text();
                this.show_error(`Transfer failed: ${errText}`);
                this._reset_transfer_ui();
                return;
            }
            const { id } = await resp.json();
            this.current_transfer_id = id;
            this._start_transfer_poll();
        } catch (err) {
            this.show_error(`Transfer error: ${err.message}`);
            this._reset_transfer_ui();
        }
    },

_start_transfer_poll() {
        this._stop_transfer_poll();
        const tick = async () => {
            this.transfer_poll_interval = null;
            if (!this.current_transfer_id) return;
            try {
                const resp = await fetch(`/api/transfer-progress/${this.current_transfer_id}`);
                if (resp.status === 404) {
                    // Removed on the server (finished long ago or lost); stop polling for it
                    this._reset_transfer_ui();
                    return;
                }
                if (resp.ok) {
                    this._render_transfer_progress(await resp.json());
                }
            } catch (_) { /* transient network error; retry on the next tick */ }
            if (this.current_transfer_id) {
                this.transfer_poll_interval = setTimeout(tick, 800);
            }
        };
        tick();
    },

_render_transfer_progress(p) {
        const bar = document.getElementById('transferProgressBar');
        const label = document.getElementById('transferStatusLabel');
        const eta = document.getElementById('transferSpeedEta');

        if (bar) {
            bar.style.width = `${p.percent}%`;
            bar.textContent = `${p.percent}%`;
            bar.setAttribute('aria-valuenow', p.percent);
        }

        if (p.status === 'connecting') {
            if (label) label.textContent = 'Connecting via SSM tunnel…';
        } else if (p.status === 'transferring') {
            if (label) label.textContent = 'Transferring…';
            if (eta) eta.textContent = p.speed || '';
        } else if (p.status === 'completed') {
            if (bar) { bar.classList.remove('progress-bar-animated'); bar.classList.add('bg-success'); }
            if (label) label.textContent = 'Transfer complete!';
            this.show_success('File transfer completed successfully.');
            this._reset_transfer_ui();
        } else if (p.status === 'failed') {
            if (bar) { bar.classList.remove('progress-bar-animated'); bar.classList.add('bg-danger'); }
            if (label) label.textContent = `Failed: ${p.error || 'Unknown error'}`;
            this.show_error(`Transfer failed: ${p.error || 'Unknown error'}`);
            this._reset_transfer_ui();
        } else if (p.status === 'cancelled') {
            if (label) label.textContent = 'Transfer cancelled.';
            this._reset_transfer_ui();
        }
    },

async cancel_transfer() {
        if (!this.current_transfer_id) return;
        this._stop_transfer_poll();
        try {
            const resp = await fetch(`/api/transfer/${this.current_transfer_id}`, { method: 'DELETE' });
            if (!resp.ok && resp.status !== 404) {
                throw new Error(await resp.text());
            }
        } catch (err) {
            this.show_error(`Could not cancel transfer: ${err.message}`);
            this._start_transfer_poll(); // keep the UI in sync with the real state
            return;
        }
        this._reset_transfer_ui();
        this._reset_transfer_progress_ui();
        const label = document.getElementById('transferStatusLabel');
        if (label) label.textContent = 'Transfer cancelled.';
    },

_stop_transfer_poll() {
        if (this.transfer_poll_interval) {
            clearTimeout(this.transfer_poll_interval);
            this.transfer_poll_interval = null;
        }
    },

_reset_transfer_progress_ui() {
        const bar = document.getElementById('transferProgressBar');
        if (bar) {
            bar.style.width = '0%';
            bar.textContent = '0%';
            bar.className = 'progress-bar progress-bar-striped progress-bar-animated';
        }
        const label = document.getElementById('transferStatusLabel');
        if (label) label.textContent = '';
        const eta = document.getElementById('transferSpeedEta');
        if (eta) eta.textContent = '';
    },

_reset_transfer_ui() {
        this._stop_transfer_poll();
        this.current_transfer_id = null;
        document.getElementById('startTransferBtn')?.classList.remove('d-none');
        document.getElementById('cancelTransferBtn')?.classList.add('d-none');
    }
};
