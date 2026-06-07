//! WASM extension provider: wraps one Extism plugin instance as a `Provider`.
//!
//! Isolation guarantees, in order of importance:
//! - a trapping/hanging extension never panics or stalls the host — every call
//!   is bounded by a cancel-handle watchdog and returns `Result`;
//! - after any failed call the instance is rebuilt from disk before reuse
//!   (post-trap module state is not trustworthy);
//! - three consecutive failures auto-disable the extension for the session,
//!   with the error surfaced to the Settings UI.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use extism::{Manifest as WasmManifest, Plugin, Wasm};
use portunus_ext_sdk::{
    ActivateInput, ExtensionResult, PreviewContent, PreviewInput, SearchInput, SearchOutput,
};

use super::{Provider, SearchResult, EXTENSION_BAND, SCORE_EXTENSION};
use crate::extensions::hostfns::{self, ExtensionCtx};
use crate::extensions::kv::ExtensionKv;
use crate::extensions::manifest::{ExtensionManifest, Limits};
use crate::util;

/// 64 MB linear-memory cap, in 64 KiB wasm pages.
const MEMORY_MAX_PAGES: u32 = 1024;
/// Raw JSON returned by a call is rejected past this size before parsing.
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
/// Per-query result cap (host truncates before scoring).
const MAX_RESULTS: usize = 200;
/// Title/subtitle clamp — UI sanity, not a wire error.
const MAX_FIELD_BYTES: usize = 2048;
/// Base64 payload cap for image previews (~1 MB decoded).
const MAX_IMAGE_B64_BYTES: usize = 1_400_000;
/// Consecutive failures before the extension is benched for the session.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

const PREVIEW_TIMEOUT_MS: u64 = 500;

pub struct WasmProvider {
    /// Registry id: `ext:<name>`.
    reg_id: String,
    name: String,
    kind: String,
    manifest: ExtensionManifest,
    wasm_path: PathBuf,
    limits: Limits,
    kv: Arc<ExtensionKv>,
    /// None = needs (re)instantiation before the next call.
    instance: Mutex<Option<Plugin>>,
    last_error: Mutex<Option<String>>,
    fail_count: AtomicU32,
    benched: AtomicBool,
}

impl WasmProvider {
    /// Loads a validated manifest + wasm file into a live instance.
    /// Compiles the module — call from a background thread, never under the
    /// registry write lock.
    pub fn load(
        manifest: ExtensionManifest,
        wasm_path: PathBuf,
        kv: Arc<ExtensionKv>,
    ) -> Result<Self, String> {
        let limits = manifest.limits.clamped();
        let provider = Self {
            reg_id: format!("ext:{}", manifest.name),
            name: manifest.name.clone(),
            kind: manifest.default_kind(),
            manifest,
            wasm_path,
            limits,
            kv,
            instance: Mutex::new(None),
            last_error: Mutex::new(None),
            fail_count: AtomicU32::new(0),
            benched: AtomicBool::new(false),
        };
        let plugin = provider.instantiate()?;
        *util::lock(&provider.instance) = Some(plugin);
        Ok(provider)
    }

    /// Search wall-clock budget — the registry sizes its fan-out deadline off
    /// the largest budget among loaded extensions.
    pub fn search_budget_ms(&self) -> u64 {
        self.limits.search_timeout_ms
    }

    pub fn last_error(&self) -> Option<String> {
        util::lock(&self.last_error).clone()
    }

    pub fn is_benched(&self) -> bool {
        self.benched.load(Ordering::Relaxed)
    }

    fn instantiate(&self) -> Result<Plugin, String> {
        let mut wasm_manifest = WasmManifest::new([Wasm::file(&self.wasm_path)])
            .with_memory_max(MEMORY_MAX_PAGES);
        if !self.manifest.permissions.network.is_empty() {
            wasm_manifest = wasm_manifest
                .with_allowed_hosts(self.manifest.permissions.network.iter().cloned());
        }
        let ctx = ExtensionCtx::new(self.name.clone(), &self.manifest.permissions, self.kv.clone());
        Plugin::new(&wasm_manifest, hostfns::build(ctx), false)
            .map_err(|e| format!("failed to load extension: {e}"))
    }

    /// Runs one exported function with a JSON payload under a wall-clock
    /// budget. The instance is lazily rebuilt if the previous call failed.
    fn call_with_budget(
        &self,
        function: &str,
        input: String,
        budget: Duration,
    ) -> Result<Option<String>, String> {
        if self.benched.load(Ordering::Relaxed) {
            return Err("extension disabled after repeated failures".to_string());
        }
        let mut guard = util::lock(&self.instance);
        if guard.is_none() {
            match self.instantiate() {
                Ok(p) => *guard = Some(p),
                Err(e) => {
                    self.record_failure(&e);
                    return Err(e);
                }
            }
        }
        let plugin = guard.as_mut().expect("instance populated above");
        if !plugin.function_exists(function) {
            return Ok(None);
        }

        // Watchdog: cancel the in-flight call if it outlives its budget.
        // recv_timeout keeps the thread cheap — it exits the moment the call
        // finishes, and epoch interruption makes cancel() safe mid-execution.
        let cancel = plugin.cancel_handle();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            if done_rx.recv_timeout(budget).is_err() {
                let _ = cancel.cancel();
            }
        });
        let outcome = plugin.call::<&str, String>(function, &input);
        let _ = done_tx.send(());
        let _ = watchdog.join();

        match outcome {
            Ok(json) => {
                // The module ran cleanly — reset the failure streak regardless.
                self.fail_count.store(0, Ordering::Relaxed);
                if json.len() > MAX_OUTPUT_BYTES {
                    // Author bug (too much data), not a runtime failure: the
                    // instance is healthy, so keep it and don't bench —
                    // surface the error in Settings and drop the response.
                    let e = format!(
                        "{function} returned {} bytes (cap {MAX_OUTPUT_BYTES}) — return less data per call",
                        json.len()
                    );
                    drop(guard);
                    self.note_error(&e);
                    return Err(e);
                }
                Ok(Some(json))
            }
            Err(e) => {
                let e = format!("{function}: {}", e.root_cause());
                // Post-trap instance state is unreliable — rebuild next call.
                *guard = None;
                drop(guard);
                self.record_failure(&e);
                Err(e)
            }
        }
    }

    /// Logs and surfaces an error in Settings without counting it toward the
    /// auto-bench failure streak.
    fn note_error(&self, error: &str) {
        eprintln!("[ext:{}] {error}", self.name);
        *util::lock(&self.last_error) = Some(error.to_string());
    }

    fn record_failure(&self, error: &str) {
        self.note_error(error);
        let fails = self.fail_count.fetch_add(1, Ordering::Relaxed) + 1;
        if fails >= MAX_CONSECUTIVE_FAILURES {
            self.benched.store(true, Ordering::Relaxed);
            eprintln!(
                "[ext:{}] disabled for this session after {fails} consecutive failures",
                self.name
            );
        }
    }

    /// Maps one extension DTO into an internal result: namespaced id, fixed
    /// kind, host-private score band, clamped fields, no exec.
    fn to_search_result(&self, dto: ExtensionResult) -> SearchResult {
        let relevance = if dto.relevance.is_finite() { dto.relevance } else { 0.0 };
        SearchResult {
            id: format!("ext:{}:{}", self.name, dto.id),
            title: clamp_field(dto.title.clone()),
            subtitle: dto.subtitle.clone().map(clamp_field),
            kind: self.kind.clone(),
            score: SCORE_EXTENSION + relevance.clamp(0.0, 100.0) / 100.0 * EXTENSION_BAND,
            // Round-tripped back to the extension on activate/preview.
            ext: Some(dto),
            ..Default::default()
        }
    }

    /// Runs the extension's default (or named) action for a result.
    pub fn activate(&self, result: ExtensionResult, action: Option<String>) -> Result<(), String> {
        let input = ActivateInput { result, action };
        let json = serde_json::to_string(&input).map_err(|e| e.to_string())?;
        match self.call_with_budget(
            "activate",
            json,
            Duration::from_millis(self.limits.activate_timeout_ms),
        )? {
            Some(_) => Ok(()),
            None => Err("extension has no activate function".to_string()),
        }
    }

    /// Fetches declarative preview content, or None if the extension doesn't
    /// export `preview`.
    pub fn preview(&self, result: ExtensionResult) -> Result<Option<PreviewContent>, String> {
        let input = PreviewInput { result };
        let json = serde_json::to_string(&input).map_err(|e| e.to_string())?;
        let Some(raw) = self.call_with_budget(
            "preview",
            json,
            Duration::from_millis(PREVIEW_TIMEOUT_MS),
        )?
        else {
            return Ok(None);
        };
        let content: PreviewContent =
            serde_json::from_str(&raw).map_err(|e| format!("preview: invalid response: {e}"))?;
        if let PreviewContent::Image { mime, data_base64 } = &content {
            // Mainstream formats only — an arbitrary mime would hand
            // extension-controlled bytes to whatever obscure WebKitGTK codec
            // matches it (codec bugs are a sandbox-independent attack surface).
            const ALLOWED_MIME: [&str; 4] =
                ["image/png", "image/jpeg", "image/gif", "image/webp"];
            if !ALLOWED_MIME.contains(&mime.as_str()) {
                return Err(format!(
                    "preview: image mime \"{mime}\" not allowed (png/jpeg/gif/webp only)"
                ));
            }
            if data_base64.len() > MAX_IMAGE_B64_BYTES {
                return Err("preview: image exceeds the 1 MB cap".to_string());
            }
        }
        Ok(Some(content))
    }
}

fn clamp_field(mut s: String) -> String {
    if s.len() > MAX_FIELD_BYTES {
        let mut cut = MAX_FIELD_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

impl Provider for WasmProvider {
    fn id(&self) -> &str {
        &self.reg_id
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let input = match serde_json::to_string(&SearchInput { query: query.to_string() }) {
            Ok(j) => j,
            Err(_) => return Vec::new(),
        };
        let raw = match self.call_with_budget(
            "search",
            input,
            Duration::from_millis(self.limits.search_timeout_ms),
        ) {
            Ok(Some(raw)) => raw,
            // Missing search export or a failed call both mean "no results";
            // failures are already recorded for the Settings UI.
            Ok(None) | Err(_) => return Vec::new(),
        };
        let output: SearchOutput = match serde_json::from_str(&raw) {
            Ok(o) => o,
            Err(e) => {
                self.record_failure(&format!("search: invalid response: {e}"));
                return Vec::new();
            }
        };
        output
            .results
            .into_iter()
            .take(MAX_RESULTS)
            .map(|dto| self.to_search_result(dto))
            .collect()
    }
}
