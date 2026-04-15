//! Browser automation tool — headless Chrome via CDP (chromiumoxide).
//!
//! Single `browser` tool with an `action` parameter. Discoverable via `search_tools`,
//! NOT always-on. Actions: navigate, snapshot, click, type, select, evaluate, screenshot, close.
//!
//! Core pattern (same as proven accessibility-tree approach):
//!   1. `navigate` → open a URL
//!   2. `snapshot` → get accessibility tree with refs (e1, e2, ...)
//!   3. `click`/`type`/`select` → interact using refs
//!   4. Repeat: snapshot again after each interaction

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chromiumoxide::browser::{Browser, BrowserConfig as ChromeBrowserConfig};
use chromiumoxide::cdp::browser_protocol::accessibility::{
    AxNode, AxProperty, AxPropertyName, GetFullAxTreeParams,
};
use chromiumoxide::cdp::browser_protocol::dom::{BackendNodeId, ResolveNodeParams};
use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
use chromiumoxide::page::Page;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::config::BrowserConfig;
use crate::tools::registry::ToolDef;

// =============================================================================
// Constants
// =============================================================================

/// Roles that are noise — skip when building the accessibility tree.
const NOISE_ROLES: &[&str] = &[
    "none",
    "generic",
    "presentation",
    "InlineTextBox",
    "LineBreak",
];

/// Roles that get interactive refs (e1, e2, ...).
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "tab",
    "menuitem",
    "switch",
    "slider",
    "searchbox",
    "spinbutton",
];

/// Max text length before truncation in accessibility tree output.
const MAX_TEXT_LEN: usize = 80;

/// URL schemes we allow.
const ALLOWED_SCHEMES: &[&str] = &["http", "https"];

// =============================================================================
// Accessibility tree formatting (pure functions — testable without Chrome)
// =============================================================================

/// A node in our simplified accessibility tree.
#[derive(Debug, Clone)]
pub struct AXTreeNode {
    pub role: String,
    pub name: String,
    pub value: String,
    pub level: Option<i64>,
    pub children: Vec<AXTreeNode>,
    /// CDP backend node ID for interaction (only set for interactive roles).
    pub backend_node_id: Option<i64>,
}

/// Result of building the accessibility tree — the formatted text + ref map.
#[derive(Debug)]
pub struct AXTreeResult {
    pub text: String,
    pub refs: HashMap<String, i64>,
}

/// Check if a role is interactive (gets a ref).
pub fn is_interactive_role(role: &str) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

/// Check if a role is noise (skipped entirely).
pub fn is_noise_role(role: &str) -> bool {
    NOISE_ROLES.contains(&role)
}

/// Truncate text to MAX_TEXT_LEN chars.
pub fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_len).collect();
        format!("{truncated}...")
    }
}

/// Format an accessibility tree into text with refs.
///
/// Returns the formatted text and a map of ref → backend_node_id.
pub fn format_ax_tree(nodes: &[AXTreeNode], title: &str, url: &str) -> AXTreeResult {
    let mut refs = HashMap::new();
    let mut ref_counter: u32 = 0;
    let mut lines = vec![format!("[page] {title} - {url}")];

    fn walk(
        node: &AXTreeNode,
        depth: usize,
        lines: &mut Vec<String>,
        refs: &mut HashMap<String, i64>,
        ref_counter: &mut u32,
    ) {
        let role = node.role.as_str();

        // Skip noise roles entirely
        if is_noise_role(role) {
            // But still walk children — they may be meaningful
            for child in &node.children {
                walk(child, depth, lines, refs, ref_counter);
            }
            return;
        }

        let indent = "  ".repeat(depth);
        let is_interactive = is_interactive_role(role);

        let ref_prefix = if is_interactive {
            if let Some(backend_id) = node.backend_node_id {
                *ref_counter += 1;
                let ref_name = format!("e{ref_counter}");
                refs.insert(ref_name.clone(), backend_id);
                format!("[{ref_name}] ")
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Build the line
        let name_part = if node.name.is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", truncate_text(&node.name, MAX_TEXT_LEN))
        };

        let value_part = if node.value.is_empty() {
            String::new()
        } else {
            format!(" value=\"{}\"", truncate_text(&node.value, MAX_TEXT_LEN))
        };

        let level_part = if let Some(lvl) = node.level {
            format!(" (level={lvl})")
        } else {
            String::new()
        };

        let line = format!("{indent}{ref_prefix}{role}{name_part}{value_part}{level_part}");

        // Skip empty structural roles with no name and only one child
        if !is_interactive
            && node.name.is_empty()
            && node.value.is_empty()
            && node.level.is_none()
            && node.children.len() == 1
        {
            // Collapse: just walk the single child at current depth
            for child in &node.children {
                walk(child, depth, lines, refs, ref_counter);
            }
            return;
        }

        lines.push(line);

        for child in &node.children {
            walk(child, depth + 1, lines, refs, ref_counter);
        }
    }

    for node in nodes {
        walk(node, 1, &mut lines, &mut refs, &mut ref_counter);
    }

    AXTreeResult {
        text: lines.join("\n"),
        refs,
    }
}

/// Validate a URL — only http:// and https:// allowed.
pub fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    let scheme = parsed.scheme();
    if !ALLOWED_SCHEMES.contains(&scheme) {
        return Err(format!(
            "URL scheme '{scheme}' not allowed. Only http:// and https:// URLs are supported."
        ));
    }
    Ok(())
}

// =============================================================================
// CDP accessibility tree → AXTreeNode conversion
// =============================================================================

/// Extract a string property value from an AX node.
fn ax_property_str(node: &AxNode, prop: AxPropertyName) -> String {
    node.properties
        .as_ref()
        .and_then(|props: &Vec<AxProperty>| {
            props.iter().find(|p| p.name == prop).and_then(|p| {
                p.value.value.as_ref().and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
            })
        })
        .unwrap_or_default()
}

/// Extract a string from an AxValue's inner serde_json::Value.
fn ax_value_string(val: &chromiumoxide::cdp::browser_protocol::accessibility::AxValue) -> String {
    val.value
        .as_ref()
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Convert CDP AxNode flat list into our tree structure.
pub fn build_ax_tree_from_cdp(nodes: &[AxNode]) -> Vec<AXTreeNode> {
    if nodes.is_empty() {
        return Vec::new();
    }

    // Build a map of node_id → index
    let mut id_to_idx: HashMap<String, usize> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        let id_str: &str = node.node_id.as_ref();
        id_to_idx.insert(id_str.to_string(), i);
    }

    // Build children map
    let mut children_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        if let Some(ref child_ids) = node.child_ids {
            let child_indices: Vec<usize> = child_ids
                .iter()
                .filter_map(|cid| {
                    let s: &str = cid.as_ref();
                    id_to_idx.get(s).copied()
                })
                .collect();
            children_map.insert(i, child_indices);
        }
    }

    // Recursive builder
    fn build_node(
        idx: usize,
        nodes: &[AxNode],
        children_map: &HashMap<usize, Vec<usize>>,
    ) -> AXTreeNode {
        let node = &nodes[idx];

        let role = node
            .role
            .as_ref()
            .map(ax_value_string)
            .unwrap_or_else(|| "unknown".to_string());

        let name_val = node.name.as_ref().map(ax_value_string).unwrap_or_default();

        let value_val = node.value.as_ref().map(ax_value_string).unwrap_or_default();

        let level = ax_property_str(node, AxPropertyName::Level)
            .parse::<i64>()
            .ok();

        let backend_node_id = node.backend_dom_node_id.as_ref().map(|id| *id.inner());

        let children = children_map
            .get(&idx)
            .map(|indices| {
                indices
                    .iter()
                    .map(|&ci| build_node(ci, nodes, children_map))
                    .collect()
            })
            .unwrap_or_default();

        AXTreeNode {
            role,
            name: name_val,
            value: value_val,
            level,
            children,
            backend_node_id,
        }
    }

    // Find root nodes (those not referenced as children)
    let mut is_child = vec![false; nodes.len()];
    for indices in children_map.values() {
        for &idx in indices {
            if idx < is_child.len() {
                is_child[idx] = true;
            }
        }
    }

    let roots: Vec<AXTreeNode> = (0..nodes.len())
        .filter(|&i| !is_child[i])
        .map(|i| build_node(i, nodes, &children_map))
        .collect();

    roots
}

// =============================================================================
// BrowserManager
// =============================================================================

/// Manages a headless Chrome instance and its pages.
pub struct BrowserManager {
    browser: Option<Browser>,
    pages: HashMap<String, Page>,
    /// Ref → CDP backend node ID for the current snapshot.
    pub refs: HashMap<String, i64>,
    workspace: PathBuf,
    config: BrowserConfig,
}

impl BrowserManager {
    pub fn new(workspace: PathBuf, config: BrowserConfig) -> Self {
        Self {
            browser: None,
            pages: HashMap::new(),
            refs: HashMap::new(),
            workspace,
            config,
        }
    }

    /// Launch Chrome if not already running.
    async fn ensure_browser(&mut self) -> Result<(), String> {
        if self.browser.is_some() {
            return Ok(());
        }

        let mut builder = if self.config.chrome_path.is_empty() {
            ChromeBrowserConfig::builder()
        } else {
            ChromeBrowserConfig::builder().chrome_executable(self.config.chrome_path.clone())
        };

        if self.config.headless {
            builder = builder.arg("--headless=new");
        }

        builder = builder
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg(format!(
                "--window-size={},{}",
                self.config.viewport_width, self.config.viewport_height
            ))
            .enable_cache();

        let browser_config = builder
            .build()
            .map_err(|e| format!("Failed to build browser config: {e}"))?;

        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|e| {
                format!(
                    "Failed to launch Chrome. Ensure Chrome/Chromium is installed or set DOLPHIN_MILK_BROWSER_CHROME_PATH. Error: {e}"
                )
            })?;

        // Spawn the CDP handler — drives the WebSocket event loop
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        self.browser = Some(browser);
        Ok(())
    }

    /// Get or create a named page.
    async fn get_page(&mut self, page_name: &str) -> Result<&Page, String> {
        if !self.pages.contains_key(page_name) {
            let browser = self.browser.as_ref().ok_or("Browser not launched")?;

            // Enforce max pages — close oldest non-main page
            if self.pages.len() >= self.config.max_pages {
                let to_remove: Option<String> = self
                    .pages
                    .keys()
                    .find(|k| *k != "main" && *k != page_name)
                    .cloned();
                if let Some(key) = to_remove {
                    if let Some(page) = self.pages.remove(&key) {
                        let _ = page.close().await;
                    }
                }
            }

            let page = browser
                .new_page("about:blank")
                .await
                .map_err(|e| format!("Failed to create page: {e}"))?;

            self.pages.insert(page_name.to_string(), page);
        }

        Ok(self.pages.get(page_name).unwrap())
    }

    /// Invalidate refs (after navigation or interaction).
    fn invalidate_refs(&mut self) {
        self.refs.clear();
    }

    /// Execute a browser action.
    pub async fn execute(&mut self, params: Value) -> String {
        let action = match params.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => {
                return "Error: 'action' is required. Available actions: navigate, snapshot, click, type, select, evaluate, screenshot, close".to_string();
            }
        };

        match action.as_str() {
            "navigate" => self.action_navigate(&params).await,
            "snapshot" => self.action_snapshot(&params).await,
            "click" => self.action_click(&params).await,
            "type" => self.action_type(&params).await,
            "select" => self.action_select(&params).await,
            "evaluate" => self.action_evaluate(&params).await,
            "screenshot" => self.action_screenshot(&params).await,
            "close" => self.action_close(&params).await,
            _ => format!(
                "Error: unknown action '{action}'. Available: navigate, snapshot, click, type, select, evaluate, screenshot, close"
            ),
        }
    }

    // -------------------------------------------------------------------------
    // Actions
    // -------------------------------------------------------------------------

    async fn action_navigate(&mut self, params: &Value) -> String {
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return "Error: 'url' is required for navigate action".to_string(),
        };

        if let Err(e) = validate_url(url) {
            return format!("Error: {e}");
        }

        let page_name = params
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();

        // Ensure browser is running
        if let Err(e) = self.ensure_browser().await {
            return format!("Error: {e}");
        }

        // Get or create page
        if let Err(e) = self.get_page(&page_name).await {
            return format!("Error: {e}");
        }

        let page = self.pages.get(&page_name).unwrap();

        // Navigate with timeout
        let timeout = std::time::Duration::from_secs(self.config.page_load_timeout);
        match tokio::time::timeout(timeout, page.goto(url)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return format!("Error navigating to {url}: {e}"),
            Err(_) => {
                return format!(
                    "Error: page load timed out after {}s",
                    self.config.page_load_timeout
                )
            }
        }

        // Wait for page to settle
        if self.config.default_wait_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.config.default_wait_ms,
            ))
            .await;
        }

        // Get page title before dropping the immutable borrow
        let title = page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_else(|| "(no title)".to_string());

        self.invalidate_refs();

        format!("Navigated to {url} — title: \"{title}\". Use snapshot to see page content.")
    }

    async fn action_snapshot(&mut self, params: &Value) -> String {
        let page_name = params
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        let page = match self.pages.get(page_name) {
            Some(p) => p,
            None => return format!("Error: page '{page_name}' not found. Navigate first."),
        };

        // Get the full accessibility tree via CDP
        let ax_params = GetFullAxTreeParams::builder().build();
        let ax_tree = match page.execute(ax_params).await {
            Ok(result) => result.result.nodes,
            Err(e) => return format!("Error getting accessibility tree: {e}"),
        };

        // Get page info
        let title = page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_else(|| "(no title)".to_string());
        let url = page
            .url()
            .await
            .unwrap_or_default()
            .unwrap_or_else(|| "about:blank".to_string());

        // Convert CDP nodes to our tree
        let tree_nodes = build_ax_tree_from_cdp(&ax_tree);

        // Format and build refs
        let result = format_ax_tree(&tree_nodes, &title, &url);
        self.refs = result.refs.iter().map(|(k, v)| (k.clone(), *v)).collect();

        result.text
    }

    async fn action_click(&mut self, params: &Value) -> String {
        let ref_name = match params.get("ref").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return "Error: 'ref' is required for click action (e.g., \"e1\")".to_string(),
        };

        let backend_id = match self.refs.get(ref_name) {
            Some(id) => *id,
            None => {
                return format!(
                    "Error: ref '{ref_name}' not found. Take a new snapshot to get current refs."
                )
            }
        };

        // Find the page with this node — default to "main"
        let page = match self.pages.get("main") {
            Some(p) => p,
            None => return "Error: no page open. Navigate first.".to_string(),
        };

        // Use CDP to click the node by backend ID
        let resolve = ResolveNodeParams::builder()
            .backend_node_id(BackendNodeId::new(backend_id))
            .build();

        let node_result = match page.execute(resolve).await {
            Ok(r) => r,
            Err(e) => return format!("Error resolving node: {e}"),
        };

        let object_id = match node_result.result.object.object_id {
            Some(id) => id,
            None => return "Error: could not get remote object for node".to_string(),
        };

        // Scroll into view and click
        let click_js = match CallFunctionOnParams::builder()
            .object_id(object_id)
            .function_declaration("function() { this.scrollIntoViewIfNeeded(); this.click(); }")
            .build()
        {
            Ok(cmd) => cmd,
            Err(e) => return format!("Error building click command: {e}"),
        };

        match page.execute(click_js).await {
            Ok(_) => {}
            Err(e) => return format!("Error clicking element: {e}"),
        }

        // Wait briefly for any updates
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        self.invalidate_refs();

        format!("Clicked [{ref_name}]. Take a new snapshot to see the updated page.")
    }

    async fn action_type(&mut self, params: &Value) -> String {
        let ref_name = match params.get("ref").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return "Error: 'ref' is required for type action (e.g., \"e4\")".to_string(),
        };

        let text = match params.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return "Error: 'text' is required for type action".to_string(),
        };

        let backend_id = match self.refs.get(ref_name) {
            Some(id) => *id,
            None => {
                return format!(
                    "Error: ref '{ref_name}' not found. Take a new snapshot to get current refs."
                )
            }
        };

        let page = match self.pages.get("main") {
            Some(p) => p,
            None => return "Error: no page open. Navigate first.".to_string(),
        };

        // Focus the element and type
        let resolve = ResolveNodeParams::builder()
            .backend_node_id(BackendNodeId::new(backend_id))
            .build();

        let node_result = match page.execute(resolve).await {
            Ok(r) => r,
            Err(e) => return format!("Error resolving node: {e}"),
        };

        let object_id = match node_result.result.object.object_id {
            Some(id) => id,
            None => return "Error: could not get remote object for node".to_string(),
        };

        // Focus, clear, and set value
        let escaped_text = text.replace('\\', "\\\\").replace('\'', "\\'");
        let type_js = match CallFunctionOnParams::builder()
            .object_id(object_id)
            .function_declaration(format!(
                "function() {{ this.focus(); this.value = ''; this.value = '{escaped_text}'; this.dispatchEvent(new Event('input', {{bubbles: true}})); }}"
            ))
            .build()
        {
            Ok(cmd) => cmd,
            Err(e) => return format!("Error building type command: {e}"),
        };

        match page.execute(type_js).await {
            Ok(_) => {}
            Err(e) => return format!("Error typing text: {e}"),
        }

        self.invalidate_refs();

        format!(
            "Typed \"{}\" into [{ref_name}]. Take a new snapshot to see the result.",
            truncate_text(text, 40)
        )
    }

    async fn action_select(&mut self, params: &Value) -> String {
        let ref_name = match params.get("ref").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return "Error: 'ref' is required for select action".to_string(),
        };

        let value = match params.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return "Error: 'value' is required for select action".to_string(),
        };

        let backend_id = match self.refs.get(ref_name) {
            Some(id) => *id,
            None => {
                return format!(
                    "Error: ref '{ref_name}' not found. Take a new snapshot to get current refs."
                )
            }
        };

        let page = match self.pages.get("main") {
            Some(p) => p,
            None => return "Error: no page open. Navigate first.".to_string(),
        };

        let resolve = ResolveNodeParams::builder()
            .backend_node_id(BackendNodeId::new(backend_id))
            .build();

        let node_result = match page.execute(resolve).await {
            Ok(r) => r,
            Err(e) => return format!("Error resolving node: {e}"),
        };

        let object_id = match node_result.result.object.object_id {
            Some(id) => id,
            None => return "Error: could not get remote object for node".to_string(),
        };
        let escaped_val = value.replace('\\', "\\\\").replace('\'', "\\'");
        let select_js = match CallFunctionOnParams::builder()
            .object_id(object_id)
            .function_declaration(format!(
                "function() {{ \
                    for (let o of this.options) {{ \
                        if (o.text === '{escaped_val}' || o.value === '{escaped_val}') {{ \
                            o.selected = true; \
                            this.dispatchEvent(new Event('change', {{bubbles: true}})); \
                            return 'selected: ' + o.text; \
                        }} \
                    }} \
                    return 'Error: option not found'; \
                }}"
            ))
            .build()
        {
            Ok(cmd) => cmd,
            Err(e) => return format!("Error building select command: {e}"),
        };

        let result = match page.execute(select_js).await {
            Ok(r) => r
                .result
                .result
                .value
                .and_then(|v: serde_json::Value| v.as_str().map(String::from))
                .unwrap_or_else(|| "selected".to_string()),
            Err(e) => return format!("Error selecting option: {e}"),
        };

        self.invalidate_refs();

        format!("{result} in [{ref_name}]. Take a new snapshot to see the result.")
    }

    async fn action_evaluate(&mut self, params: &Value) -> String {
        let expression = match params.get("expression").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return "Error: 'expression' is required for evaluate action".to_string(),
        };

        let page = match self.pages.get("main") {
            Some(p) => p,
            None => return "Error: no page open. Navigate first.".to_string(),
        };

        match page.evaluate(expression).await {
            Ok(val) => {
                self.invalidate_refs();
                let result_str = format!("{:?}", val.value().unwrap_or(&Value::Null));
                truncate_text(&result_str, 2000)
            }
            Err(e) => format!("Error evaluating JavaScript: {e}"),
        }
    }

    async fn action_screenshot(&mut self, params: &Value) -> String {
        let page_name = params
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        let page = match self.pages.get(page_name) {
            Some(p) => p,
            None => return format!("Error: page '{page_name}' not found. Navigate first."),
        };

        // Create screenshots directory
        let screenshots_dir = self.workspace.join("screenshots");
        let _ = std::fs::create_dir_all(&screenshots_dir);

        let filename = format!("{}.png", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
        let filepath = screenshots_dir.join(&filename);

        match page
            .save_screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .full_page(false)
                    .build(),
                &filepath,
            )
            .await
        {
            Ok(_) => format!("Screenshot saved to {}", filepath.display()),
            Err(e) => format!("Error taking screenshot: {e}"),
        }
    }

    /// Shut down Chrome and close all pages. Called automatically at task teardown.
    pub async fn shutdown(&mut self) {
        if self.browser.is_none() {
            return;
        }
        for (_, page) in self.pages.drain() {
            let _ = page.close().await;
        }
        self.browser = None;
        self.refs.clear();
        tracing::info!("Browser shut down (auto-cleanup)");
    }

    async fn action_close(&mut self, params: &Value) -> String {
        let page_name = params
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        if page_name == "all" {
            // Close all pages and shut down browser
            for (_, page) in self.pages.drain() {
                let _ = page.close().await;
            }
            self.browser = None;
            self.invalidate_refs();
            return "All pages closed, browser shut down.".to_string();
        }

        match self.pages.remove(page_name) {
            Some(page) => {
                let _ = page.close().await;
                self.invalidate_refs();

                // If last page, shut down browser
                if self.pages.is_empty() {
                    self.browser = None;
                    format!("Page '{page_name}' closed. Browser shut down (no pages remaining).")
                } else {
                    format!("Page '{page_name}' closed.")
                }
            }
            None => format!("Error: page '{page_name}' not found."),
        }
    }
}

// =============================================================================
// Tool factory
// =============================================================================

/// Create browser tools. Returns empty vec if browser is disabled in config.
pub fn all_browser_tools(workspace: PathBuf, config: BrowserConfig) -> Vec<ToolDef> {
    if !config.enabled {
        return Vec::new();
    }

    let manager = Arc::new(tokio::sync::Mutex::new(BrowserManager::new(
        workspace, config,
    )));
    let cleanup_mgr = manager.clone();

    vec![ToolDef {
        name: "browser".to_string(),
        description: "Control a headless Chrome browser. Actions: navigate (open URL), snapshot (get page content as accessibility tree with refs), click (click ref), type (type text into ref), select (select option in dropdown), evaluate (run JavaScript), screenshot (save PNG), close (close page/browser). Pattern: navigate → snapshot → interact using refs → snapshot again.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: navigate, snapshot, click, type, select, evaluate, screenshot, close",
                    "enum": ["navigate", "snapshot", "click", "type", "select", "evaluate", "screenshot", "close"]
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (navigate action only)"
                },
                "ref": {
                    "type": "string",
                    "description": "Element reference from snapshot (e.g. 'e1') for click/type/select"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (type action only)"
                },
                "value": {
                    "type": "string",
                    "description": "Option text or value to select (select action only)"
                },
                "expression": {
                    "type": "string",
                    "description": "JavaScript expression to evaluate (evaluate action only)"
                },
                "page": {
                    "type": "string",
                    "description": "Named page (default 'main'). Use 'all' with close to shut down."
                }
            },
            "required": ["action"]
        }),
        execute: Box::new(move |params| {
            let mgr = manager.clone();
            Box::pin(async move {
                let mut browser = mgr.lock().await;
                browser.execute(params).await
            })
        }),
        category: "browser".to_string(),
        cleanup: Some(Box::new(move || {
            let mgr = cleanup_mgr.clone();
            Box::pin(async move {
                mgr.lock().await.shutdown().await;
            })
        })),
        deferred: true,
        always_load: false,
        search_hint: Some("Headless Chrome browser automation (navigate, click, type, screenshot)".to_string()),
    }]
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_interactive_role() {
        assert!(is_interactive_role("button"));
        assert!(is_interactive_role("link"));
        assert!(is_interactive_role("textbox"));
        assert!(!is_interactive_role("heading"));
        assert!(!is_interactive_role("paragraph"));
        assert!(!is_interactive_role("generic"));
    }

    #[test]
    fn test_is_noise_role() {
        assert!(is_noise_role("none"));
        assert!(is_noise_role("generic"));
        assert!(is_noise_role("presentation"));
        assert!(is_noise_role("InlineTextBox"));
        assert!(!is_noise_role("button"));
        assert!(!is_noise_role("heading"));
    }

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("hello", 10), "hello");
        assert_eq!(truncate_text("hello world", 5), "hello...");
        assert_eq!(truncate_text("", 10), "");
    }

    #[test]
    fn test_validate_url_allowed() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://localhost:8080").is_ok());
        assert!(validate_url("https://example.com/path?q=1").is_ok());
    }

    #[test]
    fn test_validate_url_rejected() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("data:text/html,<h1>hi</h1>").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn test_format_ax_tree_empty() {
        let result = format_ax_tree(&[], "Test", "https://example.com");
        assert_eq!(result.text, "[page] Test - https://example.com");
        assert!(result.refs.is_empty());
    }

    #[test]
    fn test_format_ax_tree_single_button() {
        let nodes = vec![AXTreeNode {
            role: "button".to_string(),
            name: "Submit".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(42),
        }];
        let result = format_ax_tree(&nodes, "Form", "https://example.com");
        assert!(result.text.contains("[e1] button \"Submit\""));
        assert_eq!(result.refs.get("e1"), Some(&42));
    }

    #[test]
    fn test_format_ax_tree_non_interactive_no_ref() {
        let nodes = vec![AXTreeNode {
            role: "heading".to_string(),
            name: "Welcome".to_string(),
            value: String::new(),
            level: Some(1),
            children: vec![],
            backend_node_id: Some(10),
        }];
        let result = format_ax_tree(&nodes, "Page", "https://example.com");
        assert!(result.text.contains("heading \"Welcome\" (level=1)"));
        // No ref for non-interactive
        assert!(result.refs.is_empty());
    }

    #[test]
    fn test_format_ax_tree_noise_skipped() {
        let nodes = vec![AXTreeNode {
            role: "generic".to_string(),
            name: String::new(),
            value: String::new(),
            level: None,
            children: vec![AXTreeNode {
                role: "button".to_string(),
                name: "OK".to_string(),
                value: String::new(),
                level: None,
                children: vec![],
                backend_node_id: Some(5),
            }],
            backend_node_id: None,
        }];
        let result = format_ax_tree(&nodes, "Page", "https://example.com");
        // generic is skipped but child button appears
        assert!(!result.text.contains("generic"));
        assert!(result.text.contains("[e1] button \"OK\""));
    }

    #[test]
    fn test_format_ax_tree_long_text_truncated() {
        let long_name = "A".repeat(200);
        let nodes = vec![AXTreeNode {
            role: "link".to_string(),
            name: long_name,
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(1),
        }];
        let result = format_ax_tree(&nodes, "Page", "https://example.com");
        // Should be truncated at MAX_TEXT_LEN
        assert!(result.text.contains("..."));
        assert!(result.refs.contains_key("e1"));
    }

    #[test]
    fn test_format_ax_tree_textbox_with_value() {
        let nodes = vec![AXTreeNode {
            role: "textbox".to_string(),
            name: "Email".to_string(),
            value: "user@test.com".to_string(),
            level: None,
            children: vec![],
            backend_node_id: Some(7),
        }];
        let result = format_ax_tree(&nodes, "Form", "https://example.com");
        assert!(result
            .text
            .contains("textbox \"Email\" value=\"user@test.com\""));
    }

    #[test]
    fn test_format_ax_tree_multiple_refs() {
        let nodes = vec![
            AXTreeNode {
                role: "button".to_string(),
                name: "OK".to_string(),
                value: String::new(),
                level: None,
                children: vec![],
                backend_node_id: Some(1),
            },
            AXTreeNode {
                role: "link".to_string(),
                name: "Help".to_string(),
                value: String::new(),
                level: None,
                children: vec![],
                backend_node_id: Some(2),
            },
        ];
        let result = format_ax_tree(&nodes, "Page", "https://example.com");
        assert!(result.refs.contains_key("e1"));
        assert!(result.refs.contains_key("e2"));
        assert_eq!(result.refs.len(), 2);
    }

    #[test]
    fn test_format_ax_tree_header_line() {
        let result = format_ax_tree(&[], "My App", "https://myapp.com/page");
        assert!(result
            .text
            .starts_with("[page] My App - https://myapp.com/page"));
    }

    #[test]
    fn test_all_browser_tools_disabled() {
        let config = BrowserConfig {
            enabled: false,
            ..Default::default()
        };
        let tools = all_browser_tools(PathBuf::from("/tmp"), config);
        assert!(tools.is_empty());
    }

    #[test]
    fn test_all_browser_tools_enabled() {
        let config = BrowserConfig::default();
        let tools = all_browser_tools(PathBuf::from("/tmp"), config);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "browser");
        assert_eq!(tools[0].category, "browser");
    }

    #[test]
    fn test_browser_config_defaults() {
        let config = BrowserConfig::default();
        assert!(config.enabled);
        assert!(config.headless);
        assert!(config.chrome_path.is_empty());
        assert_eq!(config.page_load_timeout, 30);
        assert_eq!(config.default_wait_ms, 1000);
        assert_eq!(config.max_pages, 3);
        assert_eq!(config.viewport_width, 1280);
        assert_eq!(config.viewport_height, 720);
    }

    #[tokio::test]
    async fn test_execute_missing_action() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({})).await;
        assert!(result.starts_with("Error: 'action' is required"));
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "fly"})).await;
        assert!(result.starts_with("Error: unknown action 'fly'"));
    }

    #[tokio::test]
    async fn test_navigate_missing_url() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "navigate"})).await;
        assert!(result.contains("'url' is required"));
    }

    #[tokio::test]
    async fn test_navigate_bad_scheme() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr
            .execute(json!({"action": "navigate", "url": "file:///etc/passwd"}))
            .await;
        assert!(result.contains("not allowed"));
    }

    #[tokio::test]
    async fn test_click_missing_ref() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "click"})).await;
        assert!(result.contains("'ref' is required"));
    }

    #[tokio::test]
    async fn test_click_stale_ref() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "click", "ref": "e1"})).await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_type_missing_text() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        // Set a fake ref so we get past the ref check
        mgr.refs.insert("e1".to_string(), 1);
        let result = mgr.execute(json!({"action": "type", "ref": "e1"})).await;
        assert!(result.contains("'text' is required"));
    }

    #[tokio::test]
    async fn test_evaluate_missing_expression() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "evaluate"})).await;
        assert!(result.contains("'expression' is required"));
    }

    #[tokio::test]
    async fn test_snapshot_no_page() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr.execute(json!({"action": "snapshot"})).await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_close_no_page() {
        let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
        let result = mgr
            .execute(json!({"action": "close", "page": "nonexistent"}))
            .await;
        assert!(result.contains("not found"));
    }
}
