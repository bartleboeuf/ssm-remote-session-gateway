// ui.js: ui feature methods, split from app.js (Phase 3d).
// Methods use `this` and are merged into the app object by js/app.js.
export const ui = {
show_quit_confirm() {
        if (this.modals.quit) {
            this.modals.quit.show();
        } else {
            if (confirm('Quit EC2 Session Gate? This will terminate connections and close the tab.')) {
                this.quit_application();
            }
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
        const total = this.total_connections ?? this.connections.length;
        const visible = this.connections.length;
        this.elements.connectionCount.textContent = `${visible} active`;
        const totalEl = document.getElementById('connectionTotal');
        if (totalEl) {
            totalEl.textContent = total > visible ? `(${total} total)` : '';
        }
        const activeInstanceIds = new Set(this.connections.map(c => c.instanceId || c.instance_id));
        const badge = document.getElementById('activeConnInstanceCount');
        if (badge) badge.textContent = activeInstanceIds.size;
    },

// Helper method to create detail rows
    create_detail_row(label, value) {
        // Handle undefined, null, or empty values
        const displayValue = (value === undefined || value === null || value === '') ? 'N/A' : String(value);
        const copyValue = displayValue === 'N/A' ? '' : displayValue;
        const cursorClass = displayValue === 'N/A' ? 'cursor-default' : 'cursor-pointer';
        const title = displayValue === 'N/A' ? '' : 'Click to copy';

        return `
            <tr>
                <td class="fw-bold col-35">${label}:</td>
                <td>
                    <span class="copy-value" role="button" data-value="${copyValue}" 
                          class="${cursorClass}" title="${title}">
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

// Copy command from element's data attribute
    async copy_command_from_element(elementId) {
        const element = document.getElementById(elementId);
        if (element && element.dataset.command) {
            await this.copy_to_clipboard(element.dataset.command);
        }
    },

show_about: async function () {
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
    },

show_loading: function () {
    if (this.elements.loadingOverlay) {
        // Add a class to dim the background if a modal is open
        const openModals = document.querySelectorAll('.modal.show');
        openModals.forEach(modal => {
            modal.classList.add('modal-loading');
        });

        this.elements.loadingOverlay.classList.remove('d-none');
    }
},

hide_loading: function () {
    if (this.elements.loadingOverlay) {
        // Remove the class that dims the background from all modals
        const openModals = document.querySelectorAll('.modal.show');
        openModals.forEach(modal => {
            modal.classList.remove('modal-loading');
        });

        this.elements.loadingOverlay.classList.add('d-none');
    }
}
};
