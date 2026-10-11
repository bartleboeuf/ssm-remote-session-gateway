// app.js: entry point. Assembles the app object from feature modules (Phase 3d).
import './modules/api.js';
import { state } from './modules/state.js';
import { session } from './modules/session.js';
import { ui } from './modules/ui.js';
import { prefs } from './modules/prefs.js';
import { transfer } from './modules/transfer.js';
import { database } from './modules/database.js';
import { connections } from './modules/connections.js';
import { instances } from './modules/instances.js';

const app = Object.assign({}, state, session, ui, prefs, transfer, database, connections, instances);
window.app = app; // used by the ACTIONS dispatcher and the browser console

// Initialize app when document is ready
document.addEventListener('DOMContentLoaded', () => app.init());

// Apply saved theme very early to avoid flash
(function () {
    document.documentElement.setAttribute('data-theme', 'light');
})();

// Delegated click dispatcher. Replaces inline onclick handlers: elements declare
// data-act="<action>" and pass arguments through data-* attributes.
const ACTIONS = {
    dismiss_upgrade_banner: () => {
        document.getElementById('upgradePluginBanner')?.remove();
        document.body.style.paddingTop = '';
    },
    remove_ssh_key_folder_input: (el) => app.remove_ssh_key_folder_input(el.dataset.id),
    start_ssh: (el) => app.start_ssh(el.dataset.id),
    start_rdp: (el) => app.start_rdp(el.dataset.id),
    show_custom_port_modal: (el) => app.show_custom_port_modal(el.dataset.id),
    show_file_transfer_modal: (el) => app.show_file_transfer_modal(el.dataset.id),
    show_instance_details: (el) => app.show_instance_details(el.dataset.id),
    show_rds_port_modal: (el) => app.show_rds_port_modal({
        db_instance_identifier: el.dataset.dbId,
        endpoint: el.dataset.endpoint,
        port: Number(el.dataset.port),
        engine: el.dataset.engine,
    }),
    show_ecs_port_modal: (el) => app.show_ecs_port_modal({
        task_arn: el.dataset.taskArn,
        container_name: el.dataset.containerName,
        cluster_name: el.dataset.clusterName,
    }),
    copy_command_from_element: (el) => app.copy_command_from_element(el.dataset.id),
    toggle_active_filter: () => app.toggle_active_filter(),
    start_transfer: () => app.start_transfer(),
    cancel_transfer: () => app.cancel_transfer(),
    refresh_connections: () => app.check_connections(),
    disconnect_scope: () => app.disconnect_scope(),
    set_connection_scope: (el) => app.set_connection_scope(el.dataset.scope),
};

document.addEventListener('click', (event) => {
    const el = event.target.closest('[data-act]');
    const handler = el && ACTIONS[el.dataset.act];
    if (handler) handler(el);
});

// Apply saved theme
(function () {
    document.documentElement.setAttribute('data-theme', 'light');
})();
