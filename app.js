import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
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
const settingsModal = document.getElementById("settings-modal");
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

// Settings modal & Header Nav
let isModalClosing = false;

function openSettings() {
  if (!settingsModal) return;
  if (settingsToggle) settingsToggle.classList.add("active");
  settingsModal.classList.remove("closing");
  settingsModal.classList.add("open");
  loadOfflineModelStatus();
  log("Settings opened");
}

function closeSettings() {
  if (!settingsModal || isModalClosing) return;

  isModalClosing = true;
  if (settingsToggle) settingsToggle.classList.remove("active");
  settingsModal.classList.remove("open");
  settingsModal.classList.add("closing");

  setTimeout(() => {
    settingsModal.classList.remove("closing");
    isModalClosing = false;
  }, 200);
}

window.openSettings = openSettings;
window.closeSettings = closeSettings;

if (settingsToggle) settingsToggle.addEventListener("click", openSettings);
if (settingsClose) settingsClose.addEventListener("click", closeSettings);
if (settingsModal) {
  settingsModal.addEventListener("click", (e) => {
    if (e.target === settingsModal) closeSettings();
  });
}

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
    if (offlineModeToggle) offlineModeToggle.checked = !!config.offline_mode;
    if (offlineSttSelect) {
      offlineSttSelect.value = ["v2", "v3"].includes(config.stt_version) ? config.stt_version : "v2";
      currentConfig.stt_version = offlineSttSelect.value;
    }
    if (offlineLlmSelect) {
      offlineLlmSelect.value = ["2b", "4b", "9b"].includes(config.llm_model_size) ? config.llm_model_size : "4b";
      currentConfig.llm_model_size = offlineLlmSelect.value;
    }
    updateOfflineModelControls();
    if (hotkeyInput) hotkeyInput.value = config.hotkey || "Ctrl+Space";
    log("Config loaded");
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
    api_base_url: apiUrlInput.value,
    model: modelSelect && modelSelect.value ? modelSelect.value : modelInput.value,
    cleanup_model: cleanupModelSelect && cleanupModelSelect.value ? cleanupModelSelect.value : cleanupModelInput.value,
    api_key: apiKeyInput.value,
    auto_punctuation: punctuationToggle.checked,
    remove_fillers: fillersToggle.checked,
    auto_paste: autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
    dictionary: currentConfig.dictionary || [],
    hotkey: hotkeyInput ? hotkeyInput.value : "Ctrl+Space",
    overlay_position: currentConfig.overlay_position || "bottom",
    offline_mode: currentConfig.offline_mode,
    stt_version: currentConfig.stt_version,
    llm_model_size: currentConfig.llm_model_size,
    models_dir: currentConfig.models_dir,
    auto_download_models: currentConfig.auto_download_models,
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
  if (cleanupModelSelect) cleanupModelSelect.addEventListener("change", saveConfig);
  cleanupModelInput.addEventListener("change", saveConfig);
  apiKeyInput.addEventListener("change", saveConfig);
  punctuationToggle.addEventListener("change", saveConfig);
  fillersToggle.addEventListener("change", saveConfig);
  autopasteToggle.addEventListener("change", saveConfig);
  startupToggle.addEventListener("change", saveConfig);
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
}

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

function updateOfflineConfig() {
  if (!currentConfig) return;
  currentConfig.offline_mode = !!offlineModeToggle?.checked;
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
    setModelOperationRunning(true, "Starting model download...");
    try {
      const status = await invoke("download_models", {
        sttVersion: offlineSttSelect.value,
        llmSize: offlineLlmSelect.value,
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

// History & stats
const statTodayWords = document.getElementById("stat-today-words");
const statTodayDictations = document.getElementById("stat-today-dictations");
const statTotalWords = document.getElementById("stat-total-words");
const statTotalDictations = document.getElementById("stat-total-dictations");
const recentsList = document.getElementById("recents-list");
const recentsEmpty = document.getElementById("recents-empty");
const bulkDeleteBar = document.getElementById("bulk-delete-bar");
const bulkDeleteCount = document.getElementById("bulk-delete-count");
const bulkDeleteBtn = document.getElementById("bulk-delete-btn");

// Track checked indexes
let checkedIndexes = new Set();

function updateBulkBar() {
  const count = checkedIndexes.size;
  if (count > 0) {
    bulkDeleteBar.classList.add("visible");
    bulkDeleteCount.textContent = `${count} selected`;
  } else {
    bulkDeleteBar.classList.remove("visible");
  }
}

if (bulkDeleteBtn) {
  bulkDeleteBtn.addEventListener("click", async () => {
    // Delete highest indexes first to avoid index shifting
    const sorted = [...checkedIndexes].sort((a, b) => b - a);
    for (const idx of sorted) {
      try { await invoke("delete_history_entry", { index: idx }); }
      catch (err) { console.error("bulk delete failed for index", idx, err); }
    }
    checkedIndexes.clear();
    updateBulkBar();
    loadHistory();
  });
}

const popover = document.createElement("div");
popover.className = "recent-popover";
popover.style.display = "none";
document.body.appendChild(popover);

function closePopover() { popover.style.display = "none"; }

document.addEventListener("click", (e) => {
  if (!popover.contains(e.target)) closePopover();
});

function showPopover(anchor, entry) {
  popover.innerHTML = "";

  // ── Copy ──────────────────────────────────────────
  const copyBtn = document.createElement("button");
  copyBtn.className = "popover-btn";
  copyBtn.innerHTML = `Copy`;
  copyBtn.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(entry.text); }
    catch (err) { console.error("copy failed:", err); }
    closePopover();
  });

  // ── Re-inject ─────────────────────────────────────
  const reinjectBtn = document.createElement("button");
  reinjectBtn.className = "popover-btn";
  reinjectBtn.innerHTML = `Re-inject`;
  reinjectBtn.addEventListener("click", async () => {
    try { await invoke("inject_text", { text: entry.text }); }
    catch (err) { console.error("re-inject failed:", err); }
    closePopover();
  });

  // ── Delete ────────────────────────────────────────
  const deleteBtn = document.createElement("button");
  deleteBtn.className = "popover-btn popover-btn-danger";
  deleteBtn.innerHTML = `Delete`;
  deleteBtn.addEventListener("click", async () => {
    try { await invoke("delete_history_entry", { index: entry.index }); }
    catch (err) { console.error("delete failed:", err); }
    closePopover();
    loadHistory();
  });

  popover.appendChild(copyBtn);
  popover.appendChild(reinjectBtn);
  popover.appendChild(deleteBtn);

  // ── Smart positioning ─────────────────────────────
  // 1. Paint invisibly so we can measure real dimensions
  popover.style.visibility = "hidden";
  popover.style.display = "flex";

  const rect = anchor.getBoundingClientRect();
  const pw = popover.offsetWidth;
  const ph = popover.offsetHeight;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const gap = 4;

  // Align right edge of popover to right edge of the ⋮ button
  let left = rect.right - pw;
  // If it goes off the left edge, clamp to 8px
  if (left < 8) left = 8;
  // If it goes off the right edge, clamp
  if (left + pw > vw - 8) left = vw - pw - 8;

  // Prefer opening below the button; flip above if not enough space
  let top;
  if (rect.bottom + gap + ph <= vh - 8) {
    top = rect.bottom + gap;          // below
  } else {
    top = rect.top - ph - gap;        // above
    if (top < 8) top = 8;            // last resort: clamp to top
  }

  popover.style.left = left + "px";
  popover.style.top = top + "px";
  popover.style.visibility = "visible";
}

function padZero(num, size) {
  let s = num + "";
  while (s.length < size) s = "0" + s;
  return s;
}

function renderRecents(todayEntries) {
  if (!recentsList) return;
  recentsList.innerHTML = "";

  // Hide static placeholder slots
  for (let i = 1; i <= 3; i++) {
    const slot = document.getElementById(`slot-placeholder-${i}`);
    if (slot) slot.style.display = "none";
  }

  if (!todayEntries || todayEntries.length === 0) {
    if (recentsEmpty) recentsEmpty.style.display = "flex";
    return;
  }

  if (recentsEmpty) recentsEmpty.style.display = "none";

  // Prune stale checked indexes (entries that were deleted individually via popover)
  const validIndexes = new Set(todayEntries.map(e => e.index));
  for (const idx of checkedIndexes) {
    if (!validIndexes.has(idx)) checkedIndexes.delete(idx);
  }
  updateBulkBar();

  for (const entry of todayEntries) {
    const li = document.createElement("li");
    li.className = "dictation-item";

    // Checkbox
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.className = "dictation-checkbox";
    checkbox.checked = checkedIndexes.has(entry.index);
    checkbox.addEventListener("change", () => {
      if (checkbox.checked) {
        checkedIndexes.add(entry.index);
      } else {
        checkedIndexes.delete(entry.index);
      }
      updateBulkBar();
    });

    const textDiv = document.createElement("div");
    textDiv.className = "dictation-text";
    textDiv.textContent = entry.text;
    textDiv.title = entry.text; // full text on hover

    const moreBtn = document.createElement("button");
    moreBtn.className = "dictation-menu-btn";
    moreBtn.textContent = "⋮";
    moreBtn.setAttribute("aria-label", "Entry options");
    moreBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      showPopover(moreBtn, entry);
    });

    li.appendChild(checkbox);
    li.appendChild(textDiv);
    li.appendChild(moreBtn);
    recentsList.appendChild(li);
  }
}

async function loadHistory() {
  let data = { total_words: 0, total_dictations: 0, entries: [] };
  try { data = await invoke("get_history"); }
  catch (err) { console.error("get_history failed:", err); }

  const entries = data.entries || [];
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
  const todayEntries = entries
    .map((e, i) => ({ text: e.text, timestamp: e.timestamp, words: e.words, index: i }))
    .filter(e => e.timestamp * 1000 >= midnight.getTime())
    .reverse();

  const todayWords = todayEntries.reduce((sum, e) => sum + (e.words || 0), 0);
  if (statTodayWords) statTodayWords.textContent = todayWords;
  if (statTodayDictations) statTodayDictations.textContent = todayEntries.length;
  if (statTotalWords) statTotalWords.textContent = padZero(data.total_words ?? 0, 3);
  if (statTotalDictations) statTotalDictations.textContent = padZero(data.total_dictations ?? 0, 2);

  renderRecents(todayEntries);
}

loadHistory();

// Refresh recents whenever a dictation completes (main window stays stale
// otherwise — loadHistory() only runs once at startup)
try {
  listen("transcript-result", () => loadHistory());
} catch (err) {
  console.warn("transcript-result listener setup failed:", err);
}

const statsResetBtn = document.getElementById("stats-reset-btn");
const resetModal = document.getElementById("reset-modal");
const resetModalClose = document.getElementById("reset-modal-close");
const resetCountersCheck = document.getElementById("reset-counters-check");
const resetHistoryCheck = document.getElementById("reset-history-check");
const resetConfirmBtn = document.getElementById("reset-confirm-btn");
const resetCancelBtn = document.getElementById("reset-cancel-btn");

function updateResetConfirmState() {
  if (resetConfirmBtn) {
    resetConfirmBtn.disabled = !((resetCountersCheck && resetCountersCheck.checked) || (resetHistoryCheck && resetHistoryCheck.checked));
  }
}

function openResetModal() {
  if (!resetModal || isModalClosing) return;
  if (resetCountersCheck) resetCountersCheck.checked = false;
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
  const counters = resetCountersCheck && resetCountersCheck.checked;
  const history = resetHistoryCheck && resetHistoryCheck.checked;
  if (resetConfirmBtn) resetConfirmBtn.disabled = true;
  try {
    if (counters) await invoke("reset_stats");
    if (history) await invoke("clear_history");
  } catch (err) {
    log(`Reset failed: ${err}`);
    updateResetConfirmState();
    return;
  }
  closeResetModal();
  await loadHistory();
}

if (statsResetBtn) statsResetBtn.addEventListener("click", openResetModal);
if (resetCancelBtn) resetCancelBtn.addEventListener("click", closeResetModal);
if (resetModalClose) resetModalClose.addEventListener("click", closeResetModal);
if (resetModal) {
  resetModal.addEventListener("click", (e) => {
    if (e.target === resetModal) closeResetModal();
  });
}
if (resetCountersCheck) resetCountersCheck.addEventListener("change", updateResetConfirmState);
if (resetHistoryCheck) resetHistoryCheck.addEventListener("change", updateResetConfirmState);
if (resetConfirmBtn) resetConfirmBtn.addEventListener("click", confirmReset);

loadConfig();

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
