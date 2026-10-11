// api.js: API token and fetch wrapper, split from app.js (Phase 3d).
// Main application module
// app.js - SSM Remote Session Gateway UI
//
// Structure:
// - State: is_connected, instances, connections, preferences, etc.
// - Initialization: init(), cache_elements(), initialize_components()
// - Connection Management: toggle_connection(), render_connections(), etc.
// - Resource Listing: load_profiles_and_regions(), refresh_data(), etc.
// - Preferences: load_preferences(), save_preferences(), etc.
// - Utilities: show_error(), show_toast(), escape_html(), debounce(), etc.
//
// Notes:
// - All methods use trailing commas for consistency
// - Arrow functions preferred for callbacks to avoid 'this' binding issues
// - When modifying, always rebuild (cargo run) and check browser console for syntax errors

// Session token issued by the server with the page. Every /api request must carry it.
const API_TOKEN = document.querySelector('meta[name="api-token"]')?.content || '';

const nativeFetch = window.fetch.bind(window);

window.fetch = (input, init = {}) => {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    if (url.startsWith('/api/')) {
        const headers = new Headers(init.headers || {});
        headers.set('X-Api-Token', API_TOKEN);
        init = { ...init, headers };
    }
    return nativeFetch(input, init);
};
