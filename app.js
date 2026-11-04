import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { open } from "@tauri-apps/plugin-dialog";
const debug = document.getElementById("debug-text");
const debugIcon = document.getElementById("debug-info-icon");
const debugPopover = document.getElementById("debug-popover");
let lastDebugMessage = "App loaded";

function log(msg) {
  if (!debug) return;
  lastDebugMessage = msg;

  // Color the text based on severity
  const lower = msg.toLowerCase();
  const isError = lower.includes("error") || lower.includes("fail") || lower.includes("invalid") || lower.includes("not found");

  if (isError) {
    debug.textContent = "Error";
    debug.className = "status-error";
    if (debugIcon) debugIcon.classList.add("has-error");
  } else {
    debug.textContent = msg;
    debug.className = "status-ok";
    if (debugIcon) debugIcon.classList.remove("has-error");
  }

  // Sync popover content
  if (debugPopover) debugPopover.textContent = msg;
}

// Info icon hover — show/hide popover with full message
if (debugIcon && debugPopover) {
  debugIcon.addEventListener("mouseenter", () => {
    debugPopover.textContent = lastDebugMessage;
    debugPopover.classList.add("visible");
  });
  debugIcon.addEventListener("mouseleave", () => {
    debugPopover.classList.remove("visible");
  });
  // Keep popover open when hovering over the popover itself
  debugPopover.addEventListener("mouseenter", () => {
    debugPopover.classList.add("visible");
  });
  debugPopover.addEventListener("mouseleave", () => {
    debugPopover.classList.remove("visible");
  });
}

log("App loaded");

// DOM Elements
const recordBtn = document.getElementById("record-btn");
const btnLabel = document.getElementById("btn-label");
const statusText = document.getElementById("status-text");
const transcriptEl = document.getElementById("transcript");
const copyTranscriptBtn = document.getElementById("copy-transcript-btn");

if (copyTranscriptBtn && transcriptEl) {
  copyTranscriptBtn.addEventListener("click", async () => {
    const text = transcriptEl.textContent;
    if (!text || text === "Your dictated text will appear here...") return;
    try {
      await navigator.clipboard.writeText(text);
      const span = copyTranscriptBtn.querySelector("span");
      if (span) {
        const old = span.textContent;
        span.textContent = "Copied!";
        setTimeout(() => { span.textContent = old; }, 1500);
      }
    } catch (e) { console.error("Copy error:", e); }
  });
}

const settingsToggle = document.getElementById("settings-toggle");
const settingsClose = document.getElementById("settings-close");
const themeSelect = document.getElementById("theme-select");

const apiUrlInput = document.getElementById("api-url-input");
const modelSelect = document.getElementById("model-select");
const modelInput = document.getElementById("model-input");
const cleanupModelSelect = document.getElementById("cleanup-model-select");
const cleanupModelInput = document.getElementById("cleanup-model-input");
const apiKeyInput = document.getElementById("api-key-input");
const toggleKeyVisibility = document.getElementById("toggle-key-visibility");
const punctuationToggle = document.getElementById("punctuation-toggle");
const fillersToggle = document.getElementById("fillers-toggle");
const autopasteToggle = document.getElementById("autopaste-toggle");
const startupToggle = document.getElementById("startup-toggle");
const keepClipsToggle = document.getElementById("keep-clips-toggle");
const hotkeyInput = document.getElementById("hotkey-input");
const settingsStatus = document.getElementById("settings-status");
const offlineModeToggle = document.getElementById("offline-mode-toggle");
const offlineModelsControls = document.getElementById("offline-models-controls");
const offlineSttSelect = document.getElementById("offline-stt-select");
const offlineLlmSelect = document.getElementById("offline-llm-select");
const downloadModelsBtn = document.getElementById("download-models-btn");
const deleteModelsBtn = document.getElementById("delete-models-btn");
const refreshModelsBtn = document.getElementById("refresh-models-btn");
const offlineModelStatus = document.getElementById("offline-model-status");
const offlineModelFiles = document.getElementById("offline-model-files");
const modelsDirInput = document.getElementById("models-dir-input");
const browseModelsDirBtn = document.getElementById("browse-models-dir-btn");
const modelsDirDefaultBtn = document.getElementById("models-dir-default-btn");
const modelsDirHint = document.getElementById("models-dir-hint");
const customLlmPathInput = document.getElementById("custom-llm-path-input");
const browseCustomLlmBtn = document.getElementById("browse-custom-llm-btn");
const findLocalModelsBtn = document.getElementById("find-local-models-btn");
const localModelsList = document.getElementById("local-models-list");
const localModelsEmpty = document.getElementById("local-models-empty");
const localModelsEmptyText = localModelsEmpty ? localModelsEmpty.textContent : "";
const customLlmBypassNote = document.getElementById("custom-llm-bypass-note");
const cleanupEnabledToggle = document.getElementById("cleanup-enabled-toggle");
const cleanupModelRows = document.getElementById("cleanup-model-rows");
const customSttDirInput = document.getElementById("custom-stt-dir-input");
const browseCustomSttBtn = document.getElementById("browse-custom-stt-btn");
const customSttStatus = document.getElementById("custom-stt-status");
const customSttStatusText = customSttStatus ? customSttStatus.textContent : "";

// ── Click-to-expand "?" help panels ──────────────────────────────────
// One delegated listener drives every `.help-dot` in the app: the button names
// its panel with aria-controls, and the panel is a `.help-panel[hidden]` div.
// New panels need markup only, no extra JS.
function setHelpPanelOpen(button, open) {
  if (!button) return;
  const panelId = button.getAttribute("aria-controls");
  const panel = panelId ? document.getElementById(panelId) : null;
  if (!panel) return;
  panel.hidden = !open;
  button.setAttribute("aria-expanded", open ? "true" : "false");
}

// Escape closes every open panel. Clicking another `?` deliberately leaves
// sibling panels open — collapsing them would hide whatever the reader
// already opened.
document.addEventListener("click", (e) => {
  const dot = e.target instanceof Element ? e.target.closest(".help-dot") : null;
  if (!dot) return;
  setHelpPanelOpen(dot, dot.getAttribute("aria-expanded") !== "true");
});

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  document.querySelectorAll('.help-dot[aria-expanded="true"]').forEach((dot) => setHelpPanelOpen(dot, false));
});

// Surface hotkey registration/parse errors from the backend at startup
try {
  listen("hotkey-error", (e) => {
    settingsStatus.textContent = "Hotkey: " + e.payload;
    log("Hotkey error: " + e.payload);
  });
} catch (err) {
  console.warn("hotkey-error listener setup failed:", err);
}

const dictionaryTags = document.getElementById("dictionary-tags");
const dictionaryInput = document.getElementById("dictionary-input");
const dictionaryAddBtn = document.getElementById("dictionary-add-btn");

let isRecording = false;
let currentConfig = null;
let offlineModelStatusData = null;
let offlineModelProgress = new Map();
let modelOperationRunning = false;

// Theme Switcher & Persistence
function applyTheme(themeName) {
  const finalTheme = themeName === "classic" ? "classic" : "retro";
  document.body.className = `theme-${finalTheme}`;
  if (themeSelect) themeSelect.value = finalTheme;
  localStorage.setItem("smoothflow_theme", finalTheme);
}

const savedTheme = localStorage.getItem("smoothflow_theme") || "retro";
applyTheme(savedTheme);

if (themeSelect) {
  themeSelect.addEventListener("change", (e) => {
    applyTheme(e.target.value);
  });
}

// ── Settings shell ───────────────────────────────────────────────────
// The main window is a shell: a sidebar of sections plus one scrollable
// column. Switching is client-side only and the last section is restored on
// load. `#settings-toggle` (header) jumps to General, `#settings-close`
// (sidebar) goes back to the section that was open before.
const SECTION_STORAGE_KEY = "smoothflow:section";
const DEFAULT_SECTION = "general";
const sectionButtons = Array.from(document.querySelectorAll(".sidebar-item"));
const sectionIds = sectionButtons.map((btn) => btn.dataset.section).filter(Boolean);

let currentSection = DEFAULT_SECTION;
let previousSection = DEFAULT_SECTION;

function showSection(name) {
  const target = sectionIds.includes(name) ? name : DEFAULT_SECTION;
  for (const id of sectionIds) {
    const section = document.getElementById(`section-${id}`);
    if (section) section.hidden = id !== target;
  }
  for (const btn of sectionButtons) {
    const active = btn.dataset.section === target;
    btn.classList.toggle("active", active);
    if (active) btn.setAttribute("aria-current", "page");
    else btn.removeAttribute("aria-current");
  }
  const content = document.querySelector(".settings-content");
  if (content) content.scrollTop = 0;
  // History rows fold from real layout, and the initial load happens while this
  // section is still hidden (clientHeight 0), so folds can only be measured
  // once it is actually visible. `measureHistoryFolds` is a hoisted declaration.
  if (target === "history") measureHistoryFolds();
  return target;
}

function setSection(name) {
  const target = showSection(name);
  if (target !== currentSection) previousSection = currentSection;
  currentSection = target;
  try {
    localStorage.setItem(SECTION_STORAGE_KEY, target);
  } catch (err) {
    // Storage blocked: the section simply does not survive a restart.
  }
  return target;
}

function storedSection() {
  try {
    return localStorage.getItem(SECTION_STORAGE_KEY) || DEFAULT_SECTION;
  } catch (err) {
    return DEFAULT_SECTION;
  }
}

// Shared by the reset modal, which still animates open/closed.
let isModalClosing = false;

function openSettings() {
  if (settingsToggle) settingsToggle.classList.add("active");
  if (currentSection !== DEFAULT_SECTION) previousSection = currentSection;
  setSection(DEFAULT_SECTION);
  loadOfflineModelStatus();
  log("Settings opened");
}

function closeSettings() {
  if (settingsToggle) settingsToggle.classList.remove("active");
  setSection(previousSection || DEFAULT_SECTION);
  log("Settings closed");
}

window.openSettings = openSettings;
window.closeSettings = closeSettings;

if (settingsToggle) settingsToggle.addEventListener("click", openSettings);
if (settingsClose) settingsClose.addEventListener("click", closeSettings);

for (const btn of sectionButtons) {
  btn.addEventListener("click", () => {
    setSection(btn.dataset.section);
    // Rows measured while the section was hidden cannot report their real
    // height, so the first page is re-rendered when History comes into view.
    if (btn.dataset.section === "history") loadHistoryPage({ reset: true });
    if (settingsToggle) settingsToggle.classList.remove("active");
  });
}

// Restore the last section the user looked at.
currentSection = setSection(storedSection());

// Model dropdown handlers - show custom input when "Custom" selected
function setupModelSelect(selectEl, inputEl) {
  if (!selectEl || !inputEl) return;
  selectEl.addEventListener("change", () => {
    const isCustom = selectEl.value === "";
    inputEl.style.display = isCustom ? "block" : "none";
    if (!isCustom) {
      inputEl.value = selectEl.value;
    }
    saveConfig();
  });
  // Also sync when custom input changes
  inputEl.addEventListener("change", () => {
    if (selectEl.value === "") {
      saveConfig();
    }
  });
}

setupModelSelect(modelSelect, modelInput);
setupModelSelect(cleanupModelSelect, cleanupModelInput);

// Provider detection & model filtering
const GROQ_MODELS = [
  { value: "whisper-large-v3-turbo", label: "whisper-large-v3-turbo (fastest)" },
  { value: "whisper-large-v3", label: "whisper-large-v3 (default)" },
  { value: "distil-whisper-large-v3-en", label: "distil-whisper-large-v3-en (English only)" },
];

const GROQ_CLEANUP_MODELS = [
  { value: "", label: "Disabled (no LLM cleanup)" },
  // Groq shut llama-3.1-8b-instant and llama-3.3-70b-versatile down on
  // 2026-08-16. Offering them 404'd on every dictation and, because cleanup
  // failures are silent, looked like the transcript was simply not cleaned.
  { value: "openai/gpt-oss-20b", label: "openai/gpt-oss-20b (Groq, fast — recommended)" },
  { value: "qwen/qwen3.6-27b", label: "qwen/qwen3.6-27b (Groq, best at self-corrections)" },
  { value: "openai/gpt-oss-120b", label: "openai/gpt-oss-120b (Groq, highest quality)" },
];

const OPENAI_MODELS = [
  { value: "whisper-1", label: "whisper-1 (OpenAI)" },
  { value: "gpt-4o-transcribe", label: "gpt-4o-transcribe (OpenAI)" },
  { value: "gpt-4o-mini-transcribe", label: "gpt-4o-mini-transcribe (OpenAI)" },
];

const OPENAI_CLEANUP_MODELS = [
  { value: "", label: "Disabled (no LLM cleanup)" },
  { value: "gpt-4o-mini", label: "gpt-4o-mini (OpenAI)" },
  { value: "gpt-4o", label: "gpt-4o (OpenAI)" },
];

function detectProvider(baseUrl) {
  const url = (baseUrl || "").toLowerCase();
  if (url.includes("groq")) return "groq";
  if (url.includes("openai")) return "openai";
  return "custom";
}

function populateSelect(selectEl, options, currentValue) {
  if (!selectEl) return;
  const customShown = selectEl.value === "" && selectEl.querySelector('option[value=""]');
  // Preserve the "Custom..." option at the end
  const customOption = selectEl.querySelector('option[value=""]');
  selectEl.innerHTML = "";
  options.forEach(opt => {
    const option = document.createElement("option");
    option.value = opt.value;
    option.textContent = opt.label;
    selectEl.appendChild(option);
  });
  // Re-add Custom option
  const customOpt = document.createElement("option");
  customOpt.value = "";
  customOpt.textContent = "Custom...";
  selectEl.appendChild(customOpt);
  // Restore value
  if (currentValue && [...selectEl.options].some(o => o.value === currentValue)) {
    selectEl.value = currentValue;
  } else {
    selectEl.value = "";
  }
}

function updateModelDropdowns(baseUrl) {
  const provider = detectProvider(baseUrl);
  let sttModels, cleanupModels;
  if (provider === "groq") {
    sttModels = GROQ_MODELS;
    cleanupModels = GROQ_CLEANUP_MODELS;
  } else if (provider === "openai") {
    sttModels = OPENAI_MODELS;
    cleanupModels = OPENAI_CLEANUP_MODELS;
  } else {
    // Custom provider - keep current options, just ensure Custom is available
    return;
  }
  const currentModel = modelSelect ? modelSelect.value : "";
  const currentCleanup = cleanupModelSelect ? cleanupModelSelect.value : "";
  populateSelect(modelSelect, sttModels, currentModel);
  populateSelect(cleanupModelSelect, cleanupModels, currentCleanup);
  // Sync custom inputs visibility
  if (modelSelect && modelInput) {
    modelInput.style.display = modelSelect.value === "" ? "block" : "none";
  }
  if (cleanupModelSelect && cleanupModelInput) {
    cleanupModelInput.style.display = cleanupModelSelect.value === "" ? "block" : "none";
  }
}

// Auto-update dropdowns when API URL changes
if (apiUrlInput) {
  apiUrlInput.addEventListener("input", () => {
    updateModelDropdowns(apiUrlInput.value);
  });
  apiUrlInput.addEventListener("paste", () => {
    setTimeout(() => updateModelDropdowns(apiUrlInput.value), 0);
  });
}

// Toggle API key visibility
if (!toggleKeyVisibility) console.warn("toggle-key-visibility not found");
else toggleKeyVisibility.addEventListener("click", () => {
  const type = apiKeyInput.getAttribute("type") === "password" ? "text" : "password";
  apiKeyInput.setAttribute("type", type);
});

// Auto-validate API connection when key or URL is changed/pasted
const testApiStatus = document.getElementById("test-api-status");
let apiValidationTimer = null;
// ponytail - at autostart the app can boot before the network is up, so a
// failed check is retried with backoff instead of failing forever.
const API_RETRY_ATTEMPTS = 8;
const API_RETRY_BASE_MS = 1500;

async function validateApiKeyAuto(attempt = 0) {
  if (!testApiStatus) return;
  const apiKey = apiKeyInput ? apiKeyInput.value.trim() : "";
  const baseUrl = apiUrlInput ? apiUrlInput.value.trim() : "";

  if (!apiKey) {
    testApiStatus.textContent = "";
    testApiStatus.className = "";
    return;
  }

  testApiStatus.textContent = "Testing connection...";
  testApiStatus.className = "testing";

  try {
    await invoke("test_api_connection", { baseUrl, apiKey });
    testApiStatus.textContent = "Valid API Key";
    testApiStatus.className = "valid";
    if (currentConfig && apiKey !== currentConfig.api_key) {
      saveConfig();
    }
  } catch (err) {
    if (attempt < API_RETRY_ATTEMPTS) {
      testApiStatus.textContent = "Offline — retrying...";
      testApiStatus.className = "testing";
      setTimeout(
        () => validateApiKeyAuto(attempt + 1),
        API_RETRY_BASE_MS * (attempt + 1)
      );
    } else {
      testApiStatus.textContent = "Invalid API Key";
      testApiStatus.className = "invalid";
    }
  }
}

function triggerAutoApiValidation() {
  if (apiValidationTimer) clearTimeout(apiValidationTimer);
  apiValidationTimer = setTimeout(validateApiKeyAuto, 400);
}

if (apiKeyInput) {
  apiKeyInput.addEventListener("input", triggerAutoApiValidation);
  apiKeyInput.addEventListener("paste", triggerAutoApiValidation);
}
if (apiUrlInput) {
  apiUrlInput.addEventListener("input", triggerAutoApiValidation);
  apiUrlInput.addEventListener("paste", triggerAutoApiValidation);
}

// Config
async function loadConfig() {
  try {
    const config = await invoke("get_config");
    currentConfig = config;
    apiUrlInput.value = config.api_base_url;
    
    // Model select
    const modelVal = config.model || "";
    if (modelSelect) {
      const hasOption = [...modelSelect.options].some(o => o.value === modelVal);
      if (hasOption && modelVal) {
        modelSelect.value = modelVal;
        modelInput.style.display = "none";
      } else {
        modelSelect.value = "";
        modelInput.value = modelVal;
        modelInput.style.display = "block";
      }
    }
    
    // Cleanup model select
    const cleanupVal = config.cleanup_model || "";
    if (cleanupModelSelect) {
      const hasOption = [...cleanupModelSelect.options].some(o => o.value === cleanupVal);
      if (hasOption || cleanupVal === "") {
        cleanupModelSelect.value = cleanupVal;
        cleanupModelInput.style.display = "none";
      } else {
        cleanupModelSelect.value = "";
        cleanupModelInput.value = cleanupVal;
        cleanupModelInput.style.display = "block";
      }
    }
    
    apiKeyInput.value = config.api_key;
    renderDictionary(config.dictionary);
    punctuationToggle.checked = config.auto_punctuation;
    fillersToggle.checked = config.remove_fillers;
    autopasteToggle.checked = config.auto_paste;
    startupToggle.checked = config.launch_on_startup;
    if (keepClipsToggle) keepClipsToggle.checked = !!config.keep_audio_clips;
    // The leg fields are the real switches; `offline_mode` is the retired
    // legacy mirror (still written so old configs keep round-tripping).
    offlineModeToggle.checked = !!config.stt_local;
    currentConfig.stt_local = !!config.stt_local;
    currentConfig.offline_mode = !!config.offline_mode;
    if (offlineSttSelect) {
      offlineSttSelect.value = ["v2", "v3"].includes(config.stt_version) ? config.stt_version : "v2";
      currentConfig.stt_version = offlineSttSelect.value;
    }
    if (offlineLlmSelect) {
      offlineLlmSelect.value = ["2b", "4b", "9b"].includes(config.llm_model_size) ? config.llm_model_size : "4b";
      currentConfig.llm_model_size = offlineLlmSelect.value;
    }
    currentConfig.models_dir = config.models_dir || "";
    currentConfig.custom_llm_path = config.custom_llm_path || "";
    if (customLlmPathInput) customLlmPathInput.value = currentConfig.custom_llm_path;
    // Folder of Parakeet files, when the user brought their own. Saved by the
    // Rust side as `custom_stt_dir`; absent from older configs -> "".
    currentConfig.custom_stt_dir = config.custom_stt_dir || "";
    if (customSttDirInput) customSttDirInput.value = currentConfig.custom_stt_dir;
    renderModelsDir();
    updateOfflineModelControls();
    renderCleanupToggle();
    // The offline model status used to be loaded when SETTINGS was clicked;
    // the shell is now the whole window and the button is gone, so it loads
    // with the config instead.
    loadOfflineModelStatus();
    // The hotkey lives in a hidden input; the visible control is the
    // click-to-capture chip in the General section.
    if (hotkeyInput) {
      hotkeyInput.value = config.hotkey || "Ctrl+Space";
      renderHotkeyChip(hotkeyInput.value);
    }
    if (hotkeyModeSelect) {
      hotkeyModeSelect.value = config.hotkey_mode === "toggle" ? "toggle" : "hold";
      if (hotkeyModeHint) hotkeyModeHint.textContent = describeHotkeyMode(hotkeyModeSelect.value);
    }
    if (apiKeyInput.value) {
      validateApiKeyAuto();
    }
  } catch (err) {
    log(`Config load error: ${err}`);
  }
}

async function saveConfig() {
  if (!currentConfig) return;
  const updatedConfig = {
    ...currentConfig,
    api_base_url: apiUrlInput.value,
    model: modelSelect && modelSelect.value ? modelSelect.value : modelInput.value,
    cleanup_model: cleanupModelSelect && cleanupModelSelect.value ? cleanupModelSelect.value : cleanupModelInput.value,
    api_key: apiKeyInput.value,
    auto_punctuation: punctuationToggle.checked,
    remove_fillers: fillersToggle.checked,
    auto_paste: autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
    keep_audio_clips: !!keepClipsToggle?.checked,
    dictionary: currentConfig.dictionary || [],
    hotkey: hotkeyInput ? hotkeyInput.value : "Ctrl+Space",
    hotkey_mode: hotkeyModeSelect ? hotkeyModeSelect.value : "hold",
    offline_mode: !!offlineModeToggle?.checked,
    // `offline_mode` is retired (the Rust side reads `stt_local` /
    // `cleanup_engine` instead); it is still written so older configs keep
    // round-tripping. `cleanup_engine` carries the leg the AI-cleanup toggle
    // drives: local engine when this machine does the cleanup, cloud engine
    // when cleanup runs through the API provider.
    stt_local: !!offlineModeToggle?.checked,
    cleanup_engine: readCleanupModel()
      ? (offlineModeToggle?.checked ? "local" : "cloud")
      : "off",
    stt_version: offlineSttSelect ? offlineSttSelect.value : currentConfig.stt_version,
    llm_model_size: offlineLlmSelect ? offlineLlmSelect.value : currentConfig.llm_model_size,
    models_dir: currentConfig.models_dir || "",
    custom_llm_path: customLlmPathInput ? customLlmPathInput.value.trim() : (currentConfig.custom_llm_path || ""),
    custom_stt_dir: (currentConfig.custom_stt_dir || "").trim(),
  };
  try {
    await invoke("update_config", { newConfig: updatedConfig });
    currentConfig = updatedConfig;
    if (settingsStatus) {
      settingsStatus.className = "";
      settingsStatus.textContent = "Settings saved successfully";
      setTimeout(() => {
        if (settingsStatus.textContent === "Settings saved successfully") {
          settingsStatus.textContent = "";
        }
      }, 3000);
    }
    log("Config saved");
  } catch (err) {
    log(`Config save error: ${err}`);
    if (settingsStatus) {
      settingsStatus.className = "error";
      settingsStatus.replaceChildren();
      const lbl = document.createElement("span");
      lbl.className = "error-label";
      lbl.textContent = "config error";
      const info = document.createElement("span");
      info.className = "info-dot";
      info.textContent = "\u24D8";
      info.setAttribute("data-tip", String(err));
      settingsStatus.append(lbl, info);
    }
  }
}

const saveSettingsBtn = document.getElementById("save-settings-btn");
if (saveSettingsBtn) {
  saveSettingsBtn.addEventListener("click", async () => {
    saveSettingsBtn.disabled = true;
    saveSettingsBtn.textContent = "SAVING...";
    await saveConfig();
    saveSettingsBtn.textContent = "SAVE";
    saveSettingsBtn.disabled = false;
  });
}

try {
  apiUrlInput.addEventListener("change", saveConfig);
  if (modelSelect) modelSelect.addEventListener("change", saveConfig);
  modelInput.addEventListener("change", saveConfig);
  if (cleanupModelSelect) cleanupModelSelect.addEventListener("change", () => { renderCleanupToggle(); saveConfig(); });
  cleanupModelInput.addEventListener("change", () => { renderCleanupToggle(); saveConfig(); });
  apiKeyInput.addEventListener("change", saveConfig);
  punctuationToggle.addEventListener("change", saveConfig);
  fillersToggle.addEventListener("change", saveConfig);
  autopasteToggle.addEventListener("change", saveConfig);
  startupToggle.addEventListener("change", saveConfig);
  if (keepClipsToggle) keepClipsToggle.addEventListener("change", saveConfig);
  if (hotkeyInput) hotkeyInput.addEventListener("change", saveConfig);
} catch (e) { console.warn("Config bind error:", e); }

function formatModelSize(bytes) {
  const size = Math.max(0, Number(bytes) || 0);
  if (size >= 1_000_000_000) return `${(size / 1_000_000_000).toFixed(2)} GB`;
  return `${(size / 1_000_000).toFixed(2)} MB`;
}

function updateOfflineModelControls() {
  if (offlineModelsControls && offlineModeToggle) {
    offlineModelsControls.style.display = offlineModeToggle.checked ? "flex" : "none";
  }
  if (customLlmBypassNote && customLlmPathInput) {
    customLlmBypassNote.style.display = customLlmPathInput.value.trim() ? "block" : "none";
  }
}

// ── Optional: AI cleanup ─────────────────────────────────────────────
// The Rust side needs BOTH fields for a model to run: `cleanup_engine`
// ("local" | "cloud" — which machine does the cleanup) and a non-empty
// `cleanup_model` ("off" engine means no model at all). The toggle writes the
// `cleanup_model` side and saveConfig derives `cleanup_engine` from it, so the
// pair always satisfies gate in `postprocess::prepare`. The value the toggle
// restores lives in localStorage so turning it back on reuses the last choice.
const CLEANUP_MODEL_STORAGE_KEY = "smoothflow_cleanup_model";
const DEFAULT_CLEANUP_MODEL = "4b"; // matches the "Qwen 4B" option of #offline-llm-select

function readCleanupModel() {
  if (cleanupModelSelect && cleanupModelSelect.value) return cleanupModelSelect.value;
  // Only a *visible* custom box counts — a hidden stale value must not read as "enabled".
  if (cleanupModelInput && cleanupModelInput.style.display !== "none") {
    return (cleanupModelInput.value || "").trim();
  }
  return "";
}

function writeCleanupModel(value) {
  const next = (value || "").trim();
  if (!cleanupModelSelect) {
    if (cleanupModelInput) cleanupModelInput.value = next;
    return;
  }
  const hasOption = [...cleanupModelSelect.options].some((option) => option.value === next);
  if (hasOption) {
    cleanupModelSelect.value = next;
    if (cleanupModelInput) {
      cleanupModelInput.value = "";
      cleanupModelInput.style.display = "none";
    }
    return;
  }
  // Not one of the listed models (e.g. "4b" restored in cloud mode) -> custom box.
  cleanupModelSelect.value = "";
  if (cleanupModelInput) {
    cleanupModelInput.value = next;
    cleanupModelInput.style.display = next ? "block" : "none";
  }
}

function storedCleanupModel() {
  try {
    return (localStorage.getItem(CLEANUP_MODEL_STORAGE_KEY) || "").trim();
  } catch (err) {
    return "";
  }
}

function renderCleanupToggle() {
  const enabled = !!readCleanupModel();
  if (cleanupEnabledToggle) cleanupEnabledToggle.checked = enabled;
  if (cleanupModelRows) cleanupModelRows.style.display = enabled ? "flex" : "none";
}

if (cleanupEnabledToggle) {
  cleanupEnabledToggle.addEventListener("change", () => {
    if (!currentConfig) return;
    if (cleanupEnabledToggle.checked) {
      writeCleanupModel(storedCleanupModel() || DEFAULT_CLEANUP_MODEL);
    } else {
      const previous = readCleanupModel();
      if (previous) {
        try { localStorage.setItem(CLEANUP_MODEL_STORAGE_KEY, previous); } catch (err) { /* storage blocked: the default still restores */ }
      }
      writeCleanupModel("");
    }
    currentConfig.cleanup_model = readCleanupModel();
    renderCleanupToggle();
    saveConfig();
  });
}

let modelsDirResolved = "";
let modelsDirIsCustom = false;

function renderModelsDir() {
  if (modelsDirInput) {
    modelsDirInput.value = (currentConfig?.models_dir || "").trim() || modelsDirResolved;
  }
  if (modelsDirHint) {
    modelsDirHint.textContent = modelsDirIsCustom
      ? `Custom location — models are read from ${modelsDirResolved || "the path above"}. Clear it to go back to the default.`
      : `By default models are stored in ${modelsDirResolved || "the app data folder"}. You can move them to another drive if C: is short on space.`;
  }
}

async function loadModelsDir() {
  try {
    const location = await invoke("get_models_dir");
    modelsDirResolved = location?.resolved || "";
    modelsDirIsCustom = !!location?.is_custom;
    renderModelsDir();
  } catch (err) {
    if (modelsDirHint) modelsDirHint.textContent = `Could not read models folder: ${err}`;
    log(`Models folder error: ${err}`);
  }
}

function modelsDirFromInput() {
  const value = (modelsDirInput?.value || "").trim();
  if (!value) return "";
  if (modelsDirIsCustom) return value;
  return value === modelsDirResolved ? "" : value;
}

async function applyModelsDir() {
  if (!currentConfig) return;
  currentConfig.models_dir = modelsDirFromInput();
  await saveConfig();
  await loadModelsDir();
  await loadOfflineModelStatus();
}

function applyCustomLlmPath() {
  if (!currentConfig) return;
  currentConfig.custom_llm_path = (customLlmPathInput?.value || "").trim();
  updateOfflineModelControls();
  saveConfig();
}

if (browseModelsDirBtn) {
  browseModelsDirBtn.addEventListener("click", async () => {
    const picked = await open({ directory: true, multiple: false, defaultPath: modelsDirInput?.value || undefined });
    if (!picked) return;
    if (modelsDirInput) modelsDirInput.value = picked;
    await applyModelsDir();
  });
}

if (modelsDirDefaultBtn) {
  modelsDirDefaultBtn.addEventListener("click", async () => {
    if (modelsDirInput) modelsDirInput.value = modelsDirResolved;
    currentConfig.models_dir = "";
    await saveConfig();
    await loadModelsDir();
    await loadOfflineModelStatus();
  });
}

if (modelsDirInput) modelsDirInput.addEventListener("change", applyModelsDir);

if (browseCustomLlmBtn) {
  browseCustomLlmBtn.addEventListener("click", async () => {
    const picked = await open({ multiple: false, filters: [{ name: "GGUF model", extensions: ["gguf"] }, { name: "All files", extensions: ["*"] }] });
    if (!picked) return;
    if (customLlmPathInput) customLlmPathInput.value = picked;
    applyCustomLlmPath();
  });
}

if (customLlmPathInput) customLlmPathInput.addEventListener("change", applyCustomLlmPath);

// ── Speech model folder (bring your own Parakeet files) ──────────────
function setCustomSttStatus(message, isError) {
  if (!customSttStatus) return;
  // textContent only: this can carry a path or an error string from Rust.
  customSttStatus.textContent = message;
  customSttStatus.style.color = isError ? "#ff6b6b" : "";
}

async function saveCustomSttDir(dir) {
  if (!currentConfig) return;
  const next = (dir || "").trim();
  currentConfig.custom_stt_dir = next;
  if (customSttDirInput) customSttDirInput.value = next;
  await saveConfig();
}

// Validates before saving: an incomplete folder is reported and never stored.
async function checkCustomSttDir(dir) {
  const next = (dir || "").trim();
  if (!next) {
    await saveCustomSttDir("");
    setCustomSttStatus(customSttStatusText, false);
    return;
  }
  try {
    await invoke("inspect_stt_dir", { path: next });
    await saveCustomSttDir(next);
    setCustomSttStatus("All 4 files found.", false);
  } catch (err) {
    // The app answers with the exact files it could not find.
    setCustomSttStatus(String(err), true);
  }
}

if (browseCustomSttBtn) {
  browseCustomSttBtn.addEventListener("click", async () => {
    let picked = null;
    try {
      picked = await open({
        directory: true,
        multiple: false,
        defaultPath: (customSttDirInput?.value || "").trim() || undefined,
      });
    } catch (err) {
      setCustomSttStatus(`Could not open the folder picker: ${err}`, true);
      return;
    }
    if (!picked) return;
    browseCustomSttBtn.disabled = true;
    setCustomSttStatus("Checking the folder...", false);
    try {
      await checkCustomSttDir(picked);
    } finally {
      browseCustomSttBtn.disabled = false;
    }
  });
}

if (customSttDirInput) {
  customSttDirInput.addEventListener("change", async () => {
    if (!currentConfig) return;
    await checkCustomSttDir(customSttDirInput.value);
  });
}

function hideLocalModelsList() {
  if (localModelsList) {
    localModelsList.replaceChildren();
    localModelsList.style.display = "none";
  }
  if (localModelsEmpty) localModelsEmpty.style.display = "none";
}

function pickLocalModel(model) {
  if (!model || !model.path) return;
  if (customLlmPathInput) customLlmPathInput.value = model.path;
  hideLocalModelsList();
  // Runs the same update + save path the BROWSE button uses.
  applyCustomLlmPath();
}

function renderLocalModels(models) {
  if (!localModelsList || !localModelsEmpty) return;
  localModelsList.replaceChildren();
  if (!models.length) {
    localModelsList.style.display = "none";
    localModelsEmpty.textContent = localModelsEmptyText;
    localModelsEmpty.style.display = "block";
    return;
  }
  for (const model of models) {
    const item = document.createElement("li");
    item.style.cssText = "display: flex; align-items: baseline; justify-content: space-between; gap: 12px; padding: 8px 10px; cursor: pointer;";
    item.tabIndex = 0;
    item.setAttribute("role", "button");
    item.addEventListener("mouseenter", () => { item.style.background = "rgba(255, 255, 255, 0.06)"; });
    item.addEventListener("mouseleave", () => { item.style.background = ""; });

    const label = document.createElement("span");
    label.textContent = model.label || model.path;
    label.style.cssText = "font-family: var(--font-mono); font-size: 11px; color: #e8d6c8; word-break: break-all;";
    label.title = model.path || "";

    const meta = document.createElement("span");
    const parts = [];
    if (Number(model.size_bytes) > 0) parts.push(formatModelSize(model.size_bytes));
    if (model.parameters) parts.push(String(model.parameters));
    if (model.quant) parts.push(String(model.quant));
    meta.textContent = parts.join(" · ");
    meta.style.cssText = "color: #8a7a70; font-size: 11px; white-space: nowrap;";

    item.append(label, meta);
    item.addEventListener("click", () => pickLocalModel(model));
    item.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        pickLocalModel(model);
      }
    });
    localModelsList.appendChild(item);
  }
  localModelsEmpty.style.display = "none";
  localModelsList.style.display = "block";
}

async function findLocalModels() {
  if (!findLocalModelsBtn) return;
  const idleLabel = findLocalModelsBtn.textContent;
  findLocalModelsBtn.disabled = true;
  findLocalModelsBtn.textContent = "Looking...";
  hideLocalModelsList();
  try {
    const found = await invoke("list_local_models");
    renderLocalModels(Array.isArray(found) ? found : []);
  } catch (err) {
    if (localModelsEmpty) {
      localModelsEmpty.textContent = `Could not look for local models: ${err}. Browse for a .gguf file instead.`;
      localModelsEmpty.style.display = "block";
    }
    console.warn("list_local_models failed:", err);
    log(`Local models lookup failed: ${err}`);
  } finally {
    findLocalModelsBtn.disabled = false;
    findLocalModelsBtn.textContent = idleLabel || "FIND MODELS";
  }
}

if (findLocalModelsBtn) findLocalModelsBtn.addEventListener("click", findLocalModels);

function offlineStatusSummary() {
  if (!offlineModelStatusData) return "";
  const files = offlineModelStatusData.files || [];
  const selectedFiles = files.filter((file) =>
    (file.category === "stt" && file.directory === offlineSttSelect?.value)
    || (file.category === "llm" && file.directory === offlineLlmSelect?.value)
  );
  const selectedBytes = selectedFiles.reduce((sum, file) => sum + (Number(file.expected_size) || 0), 0);
  return `${offlineModelStatusData.installed_files || 0} of ${files.length} files installed · ${formatModelSize(offlineModelStatusData.total_bytes)} on disk · ${formatModelSize(selectedBytes)} selected`;
}

function setOfflineModelError(message) {
  if (!offlineModelStatus) return;
  offlineModelStatus.textContent = message;
  offlineModelStatus.style.color = "#ff6b6b";
}

function renderOfflineModelStatus() {
  if (offlineModelStatus) {
    offlineModelStatus.textContent = modelOperationRunning
      ? `${offlineStatusSummary()} · Model operation in progress`
      : offlineStatusSummary() || "No local model status available.";
    offlineModelStatus.style.color = modelOperationRunning ? "#f4d1ba" : "#cdafa0";
  }
  if (!offlineModelFiles) return;
  offlineModelFiles.replaceChildren();
  const files = offlineModelStatusData?.files || [];
  for (const file of files) {
    const item = document.createElement("li");
    const size = formatModelSize(file.expected_size);
    const progress = offlineModelProgress.get(file.id);
    if (progress?.status === "starting") {
      item.textContent = `${file.id} — Downloading (${size})`;
    } else if (progress?.status === "complete") {
      item.textContent = `${file.id} — Download complete (${size})`;
    } else if (progress?.status === "failed") {
      item.textContent = `${file.id} — Failed: ${progress.message || "Unknown error"}`;
      item.style.color = "#ff6b6b";
    } else if (file.installed) {
      item.textContent = `${file.id} — ${file.verified ? "Installed" : "Installed, verification failed"} (${size})`;
    } else {
      item.textContent = `${file.id} — Missing (${size})`;
    }
    offlineModelFiles.appendChild(item);
  }
}

function setModelOperationRunning(running, message = "") {
  modelOperationRunning = running;
  if (downloadModelsBtn) {
    downloadModelsBtn.disabled = running;
    downloadModelsBtn.textContent = running ? "WORKING..." : "DOWNLOAD";
  }
  if (deleteModelsBtn) deleteModelsBtn.disabled = running;
  if (refreshModelsBtn) refreshModelsBtn.disabled = running;
  if (offlineModeToggle) offlineModeToggle.disabled = running;
  if (offlineSttSelect) offlineSttSelect.disabled = running;
  if (offlineLlmSelect) offlineLlmSelect.disabled = running;
  if (running && message) {
    offlineModelStatus.textContent = message;
    offlineModelStatus.style.color = "#f4d1ba";
  } else {
    renderOfflineModelStatus();
  }
}

async function loadOfflineModelStatus(operationError = "") {
  if (!offlineModelStatus || !offlineModelFiles) return;
  offlineModelStatus.textContent = "Checking local models...";
  offlineModelStatus.style.color = "#cdafa0";
  try {
    offlineModelStatusData = await invoke("get_offline_status");
    offlineModelProgress.clear();
    renderOfflineModelStatus();
    if (operationError) setOfflineModelError(operationError);
  } catch (err) {
    setOfflineModelError(operationError || `Could not load local model status: ${err}`);
  }
}

// ── Activation shortcut: choose keys, don't type them ────────────────
// Handy-style recorder. Click the chip, press the combo, done; Escape or
// clicking away cancels. The hidden #hotkey-input keeps the saved string for
// saveConfig, which the Rust side re-registers immediately.
const hotkeyCapture = document.getElementById("hotkey-capture");
const hotkeyChip = document.getElementById("hotkey-chip");
const hotkeyReset = document.getElementById("hotkey-reset");
const hotkeyModeSelect = document.getElementById("hotkey-mode-select");
const hotkeyModeHint = document.getElementById("hotkey-mode-hint");
// The glow lives on the field wrapper, so the whole box lights up while
// the recorder is listening.
const hotkeyField = hotkeyCapture ? hotkeyCapture.closest(".hotkey-field") : null;

function emitChipCapturing() {
  if (hotkeyCapture) hotkeyCapture.classList.add("capturing");
  if (hotkeyField) hotkeyField.classList.add("capturing");
}

function emitChipDone() {
  if (hotkeyCapture) hotkeyCapture.classList.remove("capturing");
  if (hotkeyField) hotkeyField.classList.remove("capturing");
}

function comboFromEvent(e) {
  // Canonical order the Rust parser recognises: Ctrl, Alt, Shift, Win, key.
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  return parts;
}

function plainKeyToken(key) {
  const named = {
    " ": "Space",
    Escape: "Escape",
    Enter: "Enter",
    Tab: "Tab",
    Backspace: "Backspace",
    Delete: "Delete",
    Insert: "Insert",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
  };
  if (key in named) return named[key];
  if (/^[a-zA-Z0-9]$/.test(key)) return key.toUpperCase();
  if (key.length === 1) {
    const punct = {
      "`": "Backquote", "-": "Minus", "=": "Equal", ";": "Semicolon",
      "'": "Quote", ",": "Comma", ".": "Period", "/": "Slash",
      "\\": "Backslash", "[": "BracketLeft", "]": "BracketRight",
    };
    if (key in punct) return punct[key];
  }
  return "";
}

// Plain text, not keycap pills: the box just reads "Ctrl + Space".
function renderHotkeyChip(value) {
  if (!hotkeyChip) return;
  const combo = (value || "Ctrl+Space").split("+").filter(Boolean).join(" + ");
  hotkeyChip.textContent = combo || "Ctrl + Space";
}

let hotkeyCapturing = false;

function hotkeyCaptureKeydown(e) {
  if (!hotkeyCapturing) return;
  e.preventDefault();
  e.stopPropagation();
  // Escape with no modifiers cancels, like Handy.
  if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
    endHotkeyCapture(false);
    return;
  }
  const mods = comboFromEvent(e);
  const plain = hotkeyCapturing ? plainKeyToken(e.key) : "";
  if (!plain) {
    // Modifier alone: show a live partial so the user sees progress.
    if (hotkeyChip) {
      hotkeyChip.textContent = mods.length ? mods.join(" + ") + " …" : "Press keys…";
    }
    return;
  }
  const combo = [...mods, plain].join("+");
  endHotkeyCapture(false);
  if (hotkeyInput) hotkeyInput.value = combo;
  renderHotkeyChip(combo);
  saveConfig();
}

function endHotkeyCapture(restore = true) {
  hotkeyCapturing = false;
  emitChipDone();
  window.removeEventListener("keydown", hotkeyCaptureKeydown, true);
  document.removeEventListener("click", hotkeyCancelAway, true);
  renderHotkeyChip(hotkeyInput ? hotkeyInput.value : "");
  if (restore) return;
}

function hotkeyCancelAway(e) {
  if (hotkeyCapturing && hotkeyCapture && !hotkeyCapture.contains(e.target)) {
    endHotkeyCapture(false);
  }
}

if (hotkeyCapture) {
  hotkeyCapture.addEventListener("click", () => {
    hotkeyCapturing = true;
    emitChipCapturing();
    if (hotkeyChip) hotkeyChip.textContent = "Press keys…";
    window.addEventListener("keydown", hotkeyCaptureKeydown, true);
    document.addEventListener("click", hotkeyCancelAway, true);
  });
}

// ── Activation style: choose-only dropdown, description follows it ──
function describeHotkeyMode(mode) {
  return mode === "toggle"
    ? "Press once to start, press again to stop. No holding."
    : "Hold the shortcut while you speak, release to transcribe.";
}

if (hotkeyModeSelect) {
  hotkeyModeSelect.addEventListener("change", () => {
    if (hotkeyModeHint) hotkeyModeHint.textContent = describeHotkeyMode(hotkeyModeSelect.value);
    saveConfig();
  });
}

// ── Reload: put the default Ctrl+Space back ─────────────────────────
if (hotkeyReset) {
  hotkeyReset.addEventListener("click", () => {
    if (hotkeyCapturing) endHotkeyCapture(false);
    if (hotkeyInput) hotkeyInput.value = "Ctrl+Space";
    renderHotkeyChip("Ctrl+Space");
    saveConfig();
  });
}

// ── "!" circle: one delegated listener drops every row's context
// open, and the same click drops it closed again. ────────────────────
document.addEventListener("click", (e) => {
  const btn = e.target && e.target.closest ? e.target.closest(".ctx-toggle") : null;
  if (!btn) return;
  const panel = document.getElementById(btn.getAttribute("aria-controls") || "");
  if (!panel) return;
  const open = btn.getAttribute("aria-expanded") === "true";
  btn.setAttribute("aria-expanded", String(!open));
  btn.setAttribute("aria-label", open ? "Show details" : "Hide details");
  panel.hidden = open;
});

function updateOfflineConfig() {
  if (!currentConfig) return;
  currentConfig.offline_mode = !!offlineModeToggle?.checked;
  currentConfig.stt_local = !!offlineModeToggle?.checked;
  if (offlineSttSelect) currentConfig.stt_version = offlineSttSelect.value;
  if (offlineLlmSelect) currentConfig.llm_model_size = offlineLlmSelect.value;
  updateOfflineModelControls();
  saveConfig();
}

if (offlineModeToggle) offlineModeToggle.addEventListener("change", updateOfflineConfig);
if (offlineSttSelect) offlineSttSelect.addEventListener("change", updateOfflineConfig);
if (offlineLlmSelect) offlineLlmSelect.addEventListener("change", updateOfflineConfig);
if (refreshModelsBtn) refreshModelsBtn.addEventListener("click", () => loadOfflineModelStatus());

if (downloadModelsBtn) {
  downloadModelsBtn.addEventListener("click", async () => {
    if (modelOperationRunning) return;
    // Download only what the user asked for. An empty string tells the backend
    // to skip that bundle (offline::bundle_ids), so turning AI cleanup off never
    // hands someone a 1.3-5.7 GB Qwen download, and a custom speech folder
    // means the Parakeet files are already on disk.
    const wantStt = !(customSttDirInput?.value || "").trim();
    const wantLlm = cleanupEnabledToggle ? cleanupEnabledToggle.checked : true;
    if (!wantStt && !wantLlm) {
      if (offlineModelStatus) {
        offlineModelStatus.textContent =
          "Nothing to download — you are already using your own models.";
      }
      return;
    }
    setModelOperationRunning(true, "Starting model download...");
    try {
      const status = await invoke("download_models", {
        sttVersion: wantStt ? offlineSttSelect.value : "",
        llmSize: wantLlm ? offlineLlmSelect.value : "",
      });
      offlineModelStatusData = status;
      offlineModelProgress.clear();
      setModelOperationRunning(false);
      renderOfflineModelStatus();
      if (offlineModelStatus) offlineModelStatus.textContent = `Download complete. ${offlineStatusSummary()}`;
    } catch (err) {
      setModelOperationRunning(false);
      await loadOfflineModelStatus(`Model download failed: ${err}`);
    }
  });
}

if (deleteModelsBtn) {
  deleteModelsBtn.addEventListener("click", async () => {
    if (modelOperationRunning || !window.confirm("Delete all downloaded SmoothFlow models?")) return;
    setModelOperationRunning(true, "Deleting downloaded models...");
    try {
      offlineModelStatusData = await invoke("delete_models");
      offlineModelProgress.clear();
      setModelOperationRunning(false);
      renderOfflineModelStatus();
      if (offlineModelStatus) offlineModelStatus.textContent = `Downloaded models deleted. ${offlineStatusSummary()}`;
    } catch (err) {
      setModelOperationRunning(false);
      await loadOfflineModelStatus(`Model deletion failed: ${err}`);
    }
  });
}

try {
  listen("model-download-progress", ({ payload }) => {
    if (!payload?.id) return;
    offlineModelProgress.set(payload.id, { status: payload.status, message: payload.message });
    renderOfflineModelStatus();
    if (payload.status === "failed") setOfflineModelError(`Model download failed: ${payload.message || "Unknown error"}`);
  }).catch((err) => setOfflineModelError(`Could not listen for model download progress: ${err}`));
} catch (err) {
  setOfflineModelError(`Could not listen for model download progress: ${err}`);
}

// Personal Dictionary
function renderDictionary(words) {
  dictionaryTags.innerHTML = "";
  if (!words || words.length === 0) {
    dictionaryTags.innerHTML = '<span style="font-size:12px;opacity:0.5;font-family:var(--font-mono)">no words added</span>';
    return;
  }
  for (const word of words) {
    const tag = document.createElement("span");
    tag.className = "dictionary-tag";
    const wordText = document.createTextNode(word + " ");
    const removeBtn = document.createElement("span");
    removeBtn.className = "dictionary-tag-remove";
    removeBtn.dataset.word = word;
    removeBtn.textContent = "\u00d7";
    tag.appendChild(wordText);
    tag.appendChild(removeBtn);
    removeBtn.addEventListener("click", () => removeDictionaryWord(word));
    dictionaryTags.appendChild(tag);
  }
}

function removeDictionaryWord(word) {
  if (!currentConfig) return;
  currentConfig.dictionary = currentConfig.dictionary.filter(w => w !== word);
  renderDictionary(currentConfig.dictionary);
  saveConfig();
}

function addDictionaryWord() {
  if (!currentConfig || !dictionaryInput) return;
  const word = dictionaryInput.value.trim();
  if (!word) return;
  if (currentConfig.dictionary.includes(word)) {
    dictionaryInput.value = "";
    return;
  }
  currentConfig.dictionary.push(word);
  renderDictionary(currentConfig.dictionary);
  dictionaryInput.value = "";
  saveConfig();
}

if (dictionaryAddBtn) dictionaryAddBtn.addEventListener("click", addDictionaryWord);
if (dictionaryInput) dictionaryInput.addEventListener("keydown", (e) => { if (e.key === "Enter") addDictionaryWord(); });

// ── History list ────────────────────────────────────────────────────
// Handy-style reverse-chronological rows, paged 30 at a time out of SQLite.
// Every piece of dictation text lands via textContent — never innerHTML — so a
// transcript can never inject markup. Stats are derived from these rows on the
// Rust side, so the UI has no counters of its own.
const HISTORY_PAGE_SIZE = 30;
const historyList = document.getElementById("history-list");
const historyEmpty = document.getElementById("history-empty");
const historyLoadingEl = document.getElementById("history-loading");
const historySentinel = document.getElementById("history-sentinel");
const transcribeFileBtn = document.getElementById("transcribe-file-btn");
const transcribeFileStatus = document.getElementById("transcribe-file-status");

// Static icon markup only — no user data ever goes through innerHTML.
const ICON_COPY = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"></rect><path d="M5 15V5a2 2 0 0 1 2-2h10"></path></svg>`;
const ICON_CHECK = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"></path></svg>`;
const ICON_STAR = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 3 2.9 5.9 6.5.9-4.7 4.6 1.1 6.5L12 17.8 6.2 20.9l1.1-6.5L2.6 9.8l6.5-.9Z"></path></svg>`;
const ICON_TRASH = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18"></path><path d="M8 6V4h8v2"></path><path d="M19 6l-1 14H6L5 6"></path></svg>`;
const ICON_PLAY = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="none" aria-hidden="true"><polygon points="7 4 20 12 7 20 7 4"></polygon></svg>`;
const ICON_PAUSE = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="none" aria-hidden="true"><rect x="6" y="4" width="4" height="16" rx="1"></rect><rect x="14" y="4" width="4" height="16" rx="1"></rect></svg>`;
const ICON_REDICTATE = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-2.6-6.4"></path><path d="M21 3v6h-6"></path></svg>`;

// Only one clip plays at a time: starting a row stops whatever was playing.
// `{ audio, btn }` for the active row, or null.
let currentClipAudio = null;

function stopCurrentClip() {
  if (!currentClipAudio) return;
  currentClipAudio.audio.pause();
  currentClipAudio.btn.innerHTML = ICON_PLAY;
  currentClipAudio.btn.setAttribute("aria-label", "Play recording");
  currentClipAudio.btn.title = "Play recording";
  currentClipAudio = null;
}

let historyEntries = [];
let historyHasMore = false;
let historyLoading = false;
let historyCursor = null;

// Re-dictations in flight, by history id.
//
// A row is rebuilt from scratch when Rust reports its refresh, so the button's
// own `disabled` flag dies with the old element and the replacement starts
// enabled — which let a second run of the same clip start while the first was
// still being handled. This set outlives the rebuild instead.
const redictatingIds = new Set();

function formatHistoryDate(timestampSeconds) {
  const date = new Date((timestampSeconds || 0) * 1000);
  if (Number.isNaN(date.getTime())) return "";
  const day = date.toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  const time = date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${day} - ${time}`;
}

function updateHistoryEmptyState() {
  if (!historyEmpty) return;
  historyEmpty.style.display = historyEntries.length === 0 ? "flex" : "none";
}

function historyIconButton(act, label, icon) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "history-icon-btn";
  btn.dataset.act = act;
  btn.title = label;
  btn.setAttribute("aria-label", label);
  btn.innerHTML = icon;
  return btn;
}

function setStarSaved(btn, saved) {
  btn.classList.toggle("is-saved", saved);
  const svg = btn.querySelector("svg");
  if (svg) svg.setAttribute("fill", saved ? "currentColor" : "none");
}

// The fold is pure CSS (-webkit-line-clamp). Only measure once the row is
// actually laid out — inside a hidden section clientHeight is 0 and every row
// would look truncated.
function measureHistoryRow(row) {
  const text = row.querySelector(".history-text");
  const more = row.querySelector(".history-more");
  if (!text || !more) return;
  if (text.clientHeight === 0) return;            // section hidden — measure later
  if (!text.classList.contains("is-folded")) return; // expanded: height tells us nothing
  more.hidden = text.scrollHeight <= text.clientHeight + 1;
}

function measureHistoryFolds() {
  if (!historyList) return;
  for (const row of historyList.querySelectorAll(".history-row")) measureHistoryRow(row);
}

// A row's text changed in place (a re-dictation from its saved clip). Rebuild
// the element rather than patching pieces: the text, the fold measurement and
// the word count all move together.
function replaceHistoryRow(entry) {
  const index = historyEntries.findIndex((e) => e.id === entry.id);
  if (index !== -1) historyEntries[index] = entry;
  if (!historyList) return;
  const existing = historyList.querySelector(`.history-row[data-id="${entry.id}"]`);
  if (!existing) return;
  // The row is about to be swapped out: an <audio> owned by the old element
  // would keep playing from a detached button, so stop it first.
  if (currentClipAudio && existing.contains(currentClipAudio.btn)) stopCurrentClip();
  const row = buildHistoryRow(entry);
  existing.replaceWith(row);
  measureHistoryRow(row);
}

function buildHistoryRow(entry) {
  const text = entry.post_processed_text ?? entry.transcription_text ?? "";

  // A failed dictation is the only row written with an empty transcript: Rust
  // keeps the WAV and stores the row with no text but a non-null audio_file so
  // the clip can be retried. A successful dictation never yields an empty
  // transcript, and a failure with clips disabled writes no row at all — so an
  // empty text plus a clip is unambiguously a failure.
  const failed = !text && Boolean(entry.audio_file);

  const li = document.createElement("li");
  li.className = "history-row";
  li.dataset.id = String(entry.id);

  const head = document.createElement("div");
  head.className = "history-row-head";

  const date = document.createElement("span");
  date.className = "history-date";
  date.textContent = formatHistoryDate(entry.timestamp);
  head.appendChild(date);

  if (entry.source_name) {
    const source = document.createElement("span");
    source.className = "history-source";
    source.textContent = entry.source_name;
    head.appendChild(source);
  }

  const actions = document.createElement("div");
  actions.className = "history-row-actions";

  // Nothing to copy on a failed row — the clipboard would get an empty string.
  const copyBtn = failed ? null : historyIconButton("copy", "Copy", ICON_COPY);
  const starBtn = historyIconButton("star", "Save", ICON_STAR);
  const deleteBtn = historyIconButton("delete", "Delete", ICON_TRASH);
  setStarSaved(starBtn, Boolean(entry.saved));

  // Playback and re-dictation both need the saved clip. Clips are opt-in, so a
  // row dictated with the setting off has no audio and shows neither control.
  let playBtn = null;
  let redictateBtn = null;
  let audioEl = null;

  const errorEl = document.createElement("p");
  errorEl.className = "history-row-error";
  errorEl.hidden = true;
  function showRowError(message) {
    errorEl.textContent = message;
    errorEl.hidden = false;
  }

  if (entry.audio_file) {
    playBtn = historyIconButton("play", "Play recording", ICON_PLAY);
    playBtn.addEventListener("click", async () => {
      // Resolve the asset URL lazily: most rows are never played, and this
      // keeps one invoke per row instead of one per page.
      if (!audioEl) {
        try {
          const abs = await invoke("get_audio_file_path", { fileName: entry.audio_file });
          audioEl = new Audio(convertFileSrc(abs));
          audioEl.addEventListener("ended", () => {
            if (currentClipAudio && currentClipAudio.audio === audioEl) stopCurrentClip();
          });
          audioEl.addEventListener("error", () => {
            if (currentClipAudio && currentClipAudio.audio === audioEl) stopCurrentClip();
            showRowError("This recording could not be played.");
          });
        } catch (err) {
          console.error("get_audio_file_path failed:", err);
          showRowError(String(err));
          return;
        }
      }
      // Second click is a pause; a different row being active is handled by
      // stopCurrentClip() below.
      if (!audioEl.paused) {
        stopCurrentClip();
        return;
      }
      stopCurrentClip();
      errorEl.hidden = true;
      currentClipAudio = { audio: audioEl, btn: playBtn };
      playBtn.innerHTML = ICON_PAUSE;
      playBtn.title = "Pause recording";
      playBtn.setAttribute("aria-label", "Pause recording");
      try {
        await audioEl.play();
      } catch (err) {
        console.error("clip playback failed:", err);
        stopCurrentClip();
        showRowError("This recording could not be played.");
      }
    });

    redictateBtn = historyIconButton(
      "redictate",
      // On a failed row this is the primary action, so name it for what it does.
      failed ? "Retry transcription" : "Re-dictate from the saved recording",
      ICON_REDICTATE
    );
    // A row rebuilt mid-run has to come back disabled.
    redictateBtn.disabled = redictatingIds.has(entry.id);
    redictateBtn.classList.toggle("is-busy", redictatingIds.has(entry.id));
    redictateBtn.addEventListener("click", async () => {
      if (redictateBtn.disabled || redictatingIds.has(entry.id)) return;
      redictatingIds.add(entry.id);
      redictateBtn.disabled = true;
      redictateBtn.classList.add("is-busy");
      errorEl.hidden = true;
      try {
        // The row itself is rebuilt by the `history-updated` listener when
        // Rust reports the refreshed entry — nothing to patch here.
        await invoke("redictate_from_clip", { id: entry.id });
      } catch (err) {
        console.error("redictate_from_clip failed:", err);
        showRowError(String(err));
      } finally {
        redictatingIds.delete(entry.id);
        // `history-updated` has usually replaced this row already, so re-enable
        // whichever button is live now — not just this (possibly detached) one.
        const live =
          historyList &&
          historyList.querySelector(
            `.history-row[data-id="${entry.id}"] .history-icon-btn[data-act="redictate"]`
          );
        for (const btn of [redictateBtn, live]) {
          if (!btn) continue;
          btn.disabled = false;
          btn.classList.remove("is-busy");
        }
      }
    });
  }

  if (copyBtn) {
    copyBtn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(text);
        copyBtn.innerHTML = ICON_CHECK;
        copyBtn.classList.add("is-copied");
        setTimeout(() => {
          copyBtn.innerHTML = ICON_COPY;
          copyBtn.classList.remove("is-copied");
        }, 2000);
      } catch (err) {
        console.error("copy failed:", err);
      }
    });
  }

  // Optimistic: flip the star now, revert if the write fails.
  starBtn.addEventListener("click", async () => {
    const next = !starBtn.classList.contains("is-saved");
    setStarSaved(starBtn, next);
    try {
      await invoke("toggle_history_saved", { id: entry.id });
      entry.saved = next;
    } catch (err) {
      console.error("toggle_history_saved failed:", err);
      setStarSaved(starBtn, !next);
    }
  });

  // Optimistic: drop the row now, reload page one if the delete fails.
  deleteBtn.addEventListener("click", async () => {
    if (currentClipAudio && li.contains(currentClipAudio.btn)) stopCurrentClip();
    li.remove();
    historyEntries = historyEntries.filter((e) => e.id !== entry.id);
    updateHistoryEmptyState();
    try {
      await invoke("delete_history_entry", { id: entry.id });
    } catch (err) {
      console.error("delete_history_entry failed:", err);
      await loadHistoryPage({ reset: true });
    }
  });

  if (playBtn) actions.appendChild(playBtn);
  if (copyBtn) actions.appendChild(copyBtn);
  actions.appendChild(starBtn);
  if (redictateBtn) actions.appendChild(redictateBtn);
  actions.appendChild(deleteBtn);
  head.appendChild(actions);

  const body = document.createElement("p");
  // No fold on a failed row: the status line is one short sentence, and there
  // is no transcript to clamp.
  body.className = failed ? "history-text history-text-failed" : "history-text is-folded";
  body.textContent = failed
    ? "Transcription failed — press ↻ to retry from the saved recording"
    : text;

  let moreBtn = null;
  if (!failed) {
    moreBtn = document.createElement("button");
    moreBtn.type = "button";
    moreBtn.className = "history-more";
    moreBtn.textContent = "Show more";
    moreBtn.hidden = true;
    moreBtn.addEventListener("click", () => {
      const folded = body.classList.toggle("is-folded");
      moreBtn.textContent = folded ? "Show more" : "Show less";
    });
  }

  li.appendChild(head);
  li.appendChild(body);
  // No toggle on a failed row — measureHistoryRow() already bails when either
  // element is missing, so it needs no extra guard.
  if (moreBtn) li.appendChild(moreBtn);
  li.appendChild(errorEl);
  return li;
}

// Infinite scroll: the sentinel is observed only while a next page exists and
// no load is in flight, so a single intersection cannot queue two pages.
const historyObserver = new IntersectionObserver((entries) => {
  for (const item of entries) {
    if (item.isIntersecting && historyHasMore && !historyLoading) {
      loadHistoryPage({ reset: false });
      return;
    }
  }
}, { root: null, threshold: 0 });

function resetHistoryView() {
  historyEntries = [];
  historyHasMore = false;
  historyCursor = null;
  if (historyList) historyList.replaceChildren();
  if (historyObserver) historyObserver.unobserve(historySentinel);
  updateHistoryEmptyState();
}

async function loadHistoryPage({ reset = false } = {}) {
  if (!historyList) return;
  if (historyLoading) return;
  if (!reset && !historyHasMore) return;

  historyLoading = true;
  historyObserver.unobserve(historySentinel);
  if (reset) historyLoadingEl.style.display = "block";

  try {
    const page = await invoke("get_history", {
      cursor: reset ? null : historyCursor,
      limit: HISTORY_PAGE_SIZE,
    });
    const entries = Array.isArray(page && page.entries) ? page.entries : [];

    if (reset) {
      historyEntries = [];
      historyList.replaceChildren();
    }

    const seen = new Set(historyEntries.map((e) => e.id));
    for (const entry of entries) {
      if (seen.has(entry.id)) continue;
      seen.add(entry.id);
      historyEntries.push(entry);
      historyList.appendChild(buildHistoryRow(entry));
    }

    // An empty page means the cursor cannot advance — stop paging instead of
    // hammering the same query.
    historyHasMore = Boolean(page && page.has_more) && entries.length > 0;
    historyCursor = historyEntries.length ? historyEntries[historyEntries.length - 1].id : null;

    updateHistoryEmptyState();
    measureHistoryFolds();
  } catch (err) {
    console.error("get_history failed:", err);
  } finally {
    historyLoading = false;
    if (historyLoadingEl) historyLoadingEl.style.display = "none";
    if (historyHasMore) historyObserver.observe(historySentinel);
  }
}

// Folding is measured from real layout, so re-measure when the window reflows.
let historyResizeTimer = null;
window.addEventListener("resize", () => {
  clearTimeout(historyResizeTimer);
  historyResizeTimer = setTimeout(measureHistoryFolds, 150);
});

loadHistoryPage({ reset: true });

// Live updates from Rust. `deleted` and `toggled` are already applied
// optimistically by the row itself — handling them here would double-apply.
try {
  listen("history-updated", ({ payload }) => {
    if (!payload) return;
    if (payload.action === "added" && payload.entry) {
      if (historyEntries.some((e) => e.id === payload.entry.id)) return;
      historyEntries.unshift(payload.entry);
      historyList.prepend(buildHistoryRow(payload.entry));
      updateHistoryEmptyState();
      measureHistoryFolds();
    } else if (payload.action === "updated" && payload.entry) {
      replaceHistoryRow(payload.entry);
    } else if (payload.action === "cleared") {
      resetHistoryView();
    }
  });
} catch (err) {
  console.warn("history-updated listener setup failed:", err);
}

// ── Transcribe an existing WAV file into History ─────────────────────
// Rust does all the work (decode, engine, cleanup, History row) and emits
// `history-updated` itself, so this never inserts a row or pastes text.
let transcribeFileStatusTimer = null;

function setTranscribeFileStatus(message, isError) {
  if (!transcribeFileStatus) return;
  // Any new message (or a clear) cancels the previous auto-hide timer so a
  // second run's result can't be wiped by the first run's timeout.
  if (transcribeFileStatusTimer) {
    clearTimeout(transcribeFileStatusTimer);
    transcribeFileStatusTimer = null;
  }
  transcribeFileStatus.classList.toggle("is-error", !!isError);
  // textContent only: this carries file names and Rust error strings.
  transcribeFileStatus.textContent = message || "";
  transcribeFileStatus.hidden = !message;
  if (!message) return;
  transcribeFileStatusTimer = setTimeout(() => {
    transcribeFileStatusTimer = null;
    if (!transcribeFileStatus) return;
    transcribeFileStatus.textContent = "";
    transcribeFileStatus.hidden = true;
    transcribeFileStatus.classList.remove("is-error");
  }, 6000);
}

if (transcribeFileBtn) {
  transcribeFileBtn.addEventListener("click", async () => {
    let picked = null;
    try {
      picked = await open({ multiple: false, filters: [{ name: "Audio (WAV)", extensions: ["wav"] }] });
    } catch (err) {
      console.error("transcribe_audio_file: file dialog failed:", err);
      return;
    }
    // Cancelled: no status text, no error, nothing to clean up.
    if (!picked) return;

    transcribeFileBtn.disabled = true;
    transcribeFileBtn.textContent = "Transcribing…";
    setTranscribeFileStatus("", false);
    try {
      const entry = await invoke("transcribe_audio_file", { path: picked });
      const words = entry && Number.isFinite(entry.words) ? entry.words : 0;
      setTranscribeFileStatus(`Added ${words} ${words === 1 ? "word" : "words"} to History`, false);
    } catch (err) {
      console.error("transcribe_audio_file failed:", err);
      // Rust errors lead with a `[FILE-001] `-style code; show the sentence only.
      const message = String(err).replace(/^\[[A-Z]+-\d+\]\s*/, "");
      setTranscribeFileStatus(message || "Could not transcribe that file.", true);
    } finally {
      transcribeFileBtn.disabled = false;
      transcribeFileBtn.textContent = "+ Transcribe file";
    }
  });
}

const statsResetBtn = document.getElementById("stats-reset-btn");
const resetModal = document.getElementById("reset-modal");
const resetModalClose = document.getElementById("reset-modal-close");
const resetHistoryCheck = document.getElementById("reset-history-check");
const resetConfirmBtn = document.getElementById("reset-confirm-btn");
const resetCancelBtn = document.getElementById("reset-cancel-btn");

function updateResetConfirmState() {
  if (resetConfirmBtn) {
    resetConfirmBtn.disabled = !(resetHistoryCheck && resetHistoryCheck.checked);
  }
}

function openResetModal() {
  if (!resetModal || isModalClosing) return;
  if (resetHistoryCheck) resetHistoryCheck.checked = false;
  if (resetConfirmBtn) resetConfirmBtn.disabled = true;
  resetModal.classList.remove("closing");
  resetModal.classList.add("open");
  log("Reset popup opened");
}

function closeResetModal() {
  if (!resetModal || isModalClosing) return;

  isModalClosing = true;
  resetModal.classList.remove("open");
  resetModal.classList.add("closing");

  setTimeout(() => {
    resetModal.classList.remove("closing");
    isModalClosing = false;
  }, 200);
}

async function confirmReset() {
  if (resetConfirmBtn && resetConfirmBtn.disabled) return;
  const history = resetHistoryCheck && resetHistoryCheck.checked;
  if (resetConfirmBtn) resetConfirmBtn.disabled = true;
  try {
    if (history) await invoke("clear_history");
  } catch (err) {
    log(`Reset failed: ${err}`);
    updateResetConfirmState();
    return;
  }
  closeResetModal();
  resetHistoryView();
}

if (statsResetBtn) statsResetBtn.addEventListener("click", openResetModal);
if (resetCancelBtn) resetCancelBtn.addEventListener("click", closeResetModal);
if (resetModalClose) resetModalClose.addEventListener("click", closeResetModal);
if (resetModal) {
  resetModal.addEventListener("click", (e) => {
    if (e.target === resetModal) closeResetModal();
  });
}
if (resetHistoryCheck) resetHistoryCheck.addEventListener("change", updateResetConfirmState);
if (resetConfirmBtn) resetConfirmBtn.addEventListener("click", confirmReset);

loadConfig();
loadModelsDir();

// Auto-updater
// TESTING: dummy update - set true only to retest the update UI without a real release
const USE_DUMMY_UPDATE = false;

const updateBell = document.getElementById("update-bell");
const updateBellDot = document.getElementById("update-bell-dot");
const updateMarquee = document.getElementById("update-marquee");
const updatePopover = document.getElementById("update-popover");

function toggleUpdatePopover(show) {
  if (!updatePopover) return;
  const open = show !== undefined ? show : updatePopover.style.display === "none";
  updatePopover.style.display = open ? "block" : "none";
}

if (updateBell) {
  updateBell.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleUpdatePopover();
  });
}
if (updateMarquee) {
  updateMarquee.addEventListener("click", () => toggleUpdatePopover(true));
}

// Close popover when clicking anywhere outside bell + popover
document.addEventListener("click", (e) => {
  if (!updatePopover || updatePopover.style.display === "none") return;
  if (!updatePopover.contains(e.target) && !updateBell?.contains(e.target)) {
    toggleUpdatePopover(false);
  }
});

function showUpdateAvailable(version) {
  if (updateBellDot) updateBellDot.classList.add("visible");
  if (updateMarquee) {
    updateMarquee.textContent = `UPDATE v${version} AVAILABLE — CLICK THE BELL TO VIEW`;
    updateMarquee.style.display = "block";
  }
  const info = document.getElementById("update-info");
  if (info) info.textContent = `SMOOTHFLOW v${version} AVAILABLE`;
}

async function checkForUpdates() {
  let update = null;
  if (USE_DUMMY_UPDATE) {
    // TESTING: dummy update - flip USE_DUMMY_UPDATE back to true to retest the UI
    update = { version: "0.1.3", body: "", downloadAndInstall: async () => { await new Promise(r => setTimeout(r, 1000)); } };
  } else {
    try {
      update = await check();
    } catch (err) {
      console.warn("Update check failed:", err); // offline / endpoint not configured — non-fatal
      return;
    }
  }
  if (!update) return; // no update available

  showUpdateAvailable(update.version);

  const updateBtn = document.getElementById("update-btn");
  const laterBtn = document.getElementById("update-later-btn");

  if (updateBtn) {
    updateBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      updateBtn.disabled = true;
      updateBtn.textContent = "UPDATING...";
      try {
        await update.downloadAndInstall();
        log("Update installed — restarting");
        await invoke("restart_app");
      } catch (err) {
        log(`Update failed: ${err}`);
        updateBtn.disabled = false;
        updateBtn.textContent = "CONFIRM";
      }
    });
  }
  if (laterBtn) {
    laterBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleUpdatePopover(false); // marquee + dot stay until restart/successful update
    });
  }
}

checkForUpdates();
