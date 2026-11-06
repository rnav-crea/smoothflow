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

// Info icon hover â€” show/hide popover with full message
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
// ── Models panel ──────────────────────────────────────────────────────────
// Two cards (Speech, AI cleanup), each with its own source picker. There is no
// stored field for "the source": `saveConfig()` derives `stt_local`,
// `cleanup_engine` and `cleanup_model` from the two radios below.
const sttProviderSelect = document.getElementById("stt-provider-select");
const sttUrlRow = document.getElementById("stt-url-row");
const offlineSpeechControls = document.getElementById("offline-speech-controls");
const sttCloudControls = document.getElementById("stt-cloud-controls");
const cleanupLocalControls = document.getElementById("cleanup-local-controls");
const cleanupCloudControls = document.getElementById("cleanup-cloud-controls");
const summarySpeech = document.getElementById("summary-speech");
const summaryCleanup = document.getElementById("summary-cleanup");
const cleanupProviderNote = document.getElementById("cleanup-provider-note");
const cleanupFallbackNote = document.getElementById("cleanup-fallback-note");
const deleteModelsBtn = document.getElementById("delete-models-btn");
const refreshModelsBtn = document.getElementById("refresh-models-btn");
const offlineModelStatus = document.getElementById("offline-model-status");
const engineStatusEl = document.getElementById("engine-status");
// One dialog owns every download: it is the only place that knows the folder
// the bytes land in, so it also owns the folder picker.
const downloadModal = document.getElementById("download-modal");
const downloadModalTitle = document.getElementById("download-modal-title");
const downloadModalClose = document.getElementById("download-modal-close");
const downloadModalSize = document.getElementById("download-modal-size");
const downloadModalDest = document.getElementById("download-modal-dest");
const downloadModalChangeBtn = document.getElementById("download-modal-change-btn");
const downloadModalEngine = document.getElementById("download-modal-engine");
const downloadModalProgress = document.getElementById("download-modal-progress");
const downloadModalPercent = document.getElementById("download-modal-percent");
const downloadModalMessage = document.getElementById("download-modal-message");
const downloadModalCancel = document.getElementById("download-modal-cancel");
const downloadModalContinue = document.getElementById("download-modal-continue");
// Shared destructive confirm (removing models), replacing window.confirm().
const confirmModal = document.getElementById("confirm-modal");
const confirmModalTitle = document.getElementById("confirm-modal-title");
const confirmModalClose = document.getElementById("confirm-modal-close");
const confirmModalMessage = document.getElementById("confirm-modal-message");
const confirmModalCancel = document.getElementById("confirm-modal-cancel");
const confirmModalConfirm = document.getElementById("confirm-modal-confirm");
// Model table (rendered from get_model_catalog — no model names or sizes live
// in the HTML any more).
const modelTableEl = document.getElementById("model-table");
const modelTableCleanupEl = document.getElementById("cleanup-model-rows");
const modelSearchInput = document.getElementById("model-search");

function speechSource() {
  // Anything other than an explicit "local" is cloud, matching the Rust default
  // (`stt_local: false`): a missing radio must not silently turn on local STT.
  return document.getElementById("speech-source-local")?.checked ? "local" : "cloud";
}

function cleanupSource() {
  if (document.getElementById("cleanup-source-local")?.checked) return "local";
  if (document.getElementById("cleanup-source-cloud")?.checked) return "cloud";
  return "off";
}

function setSpeechSource(source) {
  const radio = document.getElementById(source === "local" ? "speech-source-local" : "speech-source-cloud");
  if (radio) radio.checked = true;
}

function setCleanupSource(source) {
  const id = source === "local"
    ? "cleanup-source-local"
    : source === "cloud" ? "cleanup-source-cloud" : "cleanup-source-off";
  const radio = document.getElementById(id);
  if (radio) radio.checked = true;
}

// The local model rows live in a table that renders after `get_model_catalog`
// returns, so at startup there is a window where no radio is checked yet. The
// fallback keeps a save made in that window from rewriting a v3 / 9B user back
// to the v2 / 4B default.
function getSttChoice(fallback = "v2") {
  return document.querySelector('input[name="offline-stt-choice"]:checked')?.value || fallback;
}
function getLlmChoice(fallback = "4b") {
  return document.querySelector('input[name="offline-llm-choice"]:checked')?.value || fallback;
}
function setSttChoice(value) {
  for (const r of document.querySelectorAll('input[name="offline-stt-choice"]')) {
    r.checked = r.value === value;
    r.closest(".model-card")?.classList.toggle("is-selected", r.checked);
  }
}
function setLlmChoice(value) {
  for (const r of document.querySelectorAll('input[name="offline-llm-choice"]')) {
    r.checked = r.value === value;
    r.closest(".model-card")?.classList.toggle("is-selected", r.checked);
  }
}

// â”€â”€ Click-to-expand "?" help panels â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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
// sibling panels open â€” collapsing them would hide whatever the reader
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
let engineStatusData = null;

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

// â”€â”€ Settings shell â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

// Model dropdown handlers - show custom input when "Custom" selected.
// `afterChange` lets the Models panel repaint its summary chip before saving;
// it defaults to a plain save so a third dropdown need not touch this.
function setupModelSelect(selectEl, inputEl, afterChange = () => saveConfig()) {
  if (!selectEl || !inputEl) return;
  selectEl.addEventListener("change", () => {
    const isCustom = selectEl.value === "";
    inputEl.style.display = isCustom ? "block" : "none";
    if (!isCustom) {
      inputEl.value = selectEl.value;
    }
    afterChange();
  });
  // Also sync when custom input changes
  inputEl.addEventListener("change", () => {
    if (selectEl.value === "") {
      afterChange();
    }
  });
}

setupModelSelect(modelSelect, modelInput, afterModelChoiceChange);
setupModelSelect(cleanupModelSelect, cleanupModelInput, afterCleanupCloudModelChange);

// Provider detection & model filtering
const GROQ_MODELS = [
  { value: "whisper-large-v3-turbo", label: "whisper-large-v3-turbo (fastest)" },
  { value: "whisper-large-v3", label: "whisper-large-v3 (default)" },
  { value: "distil-whisper-large-v3-en", label: "distil-whisper-large-v3-en (English only)" },
];

const GROQ_CLEANUP_MODELS = [
  // Groq shut llama-3.1-8b-instant and llama-3.3-70b-versatile down on
  // 2026-08-16. Offering them 404'd on every dictation and, because cleanup
  // failures are silent, looked like the transcript was simply not cleaned.
  { value: "openai/gpt-oss-20b", label: "openai/gpt-oss-20b (Groq, fast â€” recommended)" },
  { value: "qwen/qwen3.6-27b", label: "qwen/qwen3.6-27b (Groq, best at self-corrections)" },
  { value: "openai/gpt-oss-120b", label: "openai/gpt-oss-120b (Groq, highest quality)" },
];

const OPENAI_MODELS = [
  { value: "whisper-1", label: "whisper-1 (OpenAI)" },
  { value: "gpt-4o-transcribe", label: "gpt-4o-transcribe (OpenAI)" },
  { value: "gpt-4o-mini-transcribe", label: "gpt-4o-mini-transcribe (OpenAI)" },
];

const OPENAI_CLEANUP_MODELS = [
  { value: "gpt-4o-mini", label: "gpt-4o-mini (OpenAI)" },
  { value: "gpt-4o", label: "gpt-4o (OpenAI)" },
];

// Which endpoint a stored URL points at. Only used to initialise the provider
// select from an existing config — the dropdowns follow the select, not the URL,
// so a custom base URL never silently rewrites the model ids.
function providerFromUrl(baseUrl) {
  const url = (baseUrl || "").toLowerCase();
  if (url.includes("groq")) return "groq";
  if (url.includes("openai")) return "openai";
  return "custom";
}

// The two endpoints whose model ids are known up front.
const PROVIDER_URLS = {
  groq: "https://api.groq.com/openai/v1",
  openai: "https://api.openai.com/v1",
};

function defaultCleanupCloudModel(provider) {
  return provider === "openai" ? "gpt-4o-mini" : "openai/gpt-oss-20b";
}

function populateSelect(selectEl, options, currentValue) {
  if (!selectEl) return;
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

// Repopulate both cloud dropdowns from a provider id. The provider select is
// the source of truth, not the URL: typing a URL must not swap model ids under
// the user's cursor.
function updateModelDropdowns(provider) {
  let sttModels, cleanupModels;
  if (provider === "groq") {
    sttModels = GROQ_MODELS;
    cleanupModels = GROQ_CLEANUP_MODELS;
  } else if (provider === "openai") {
    sttModels = OPENAI_MODELS;
    cleanupModels = OPENAI_CLEANUP_MODELS;
  } else {
    // Custom endpoint - keep the current ids, just ensure Custom stays available
    return;
  }
  const currentModel = modelSelect ? modelSelect.value : "";
  const currentCleanup = cleanupModelSelect ? cleanupModelSelect.value : "";
  populateSelect(modelSelect, sttModels, currentModel);
  populateSelect(cleanupModelSelect, cleanupModels, currentCleanup);
  // A model id from the other provider matches nothing, so land on the
  // provider's default instead of an empty "Custom..." box.
  if (modelSelect && !modelSelect.value) {
    modelSelect.value = sttModels[0].value;
    if (modelInput) {
      modelInput.value = modelSelect.value;
      modelInput.style.display = "none";
    }
  }
  if (cleanupModelSelect && !cleanupModelSelect.value) {
    writeCleanupCloudModel(defaultCleanupCloudModel(provider));
  }
  // Sync custom inputs visibility
  if (modelSelect && modelInput) {
    modelInput.style.display = modelSelect.value === "" ? "block" : "none";
  }
  if (cleanupModelSelect && cleanupModelInput) {
    cleanupModelInput.style.display = cleanupModelSelect.value === "" ? "block" : "none";
  }
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
      testApiStatus.textContent = "Offline â€” retrying...";
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
    if (apiUrlInput) apiUrlInput.value = config.api_base_url || "";
    if (apiKeyInput) apiKeyInput.value = config.api_key || "";
    if (sttProviderSelect) sttProviderSelect.value = providerFromUrl(config.api_base_url);

    // Cloud speech model
    const modelVal = config.model || "";
    if (modelSelect) {
      const hasOption = [...modelSelect.options].some(o => o.value === modelVal);
      if (hasOption && modelVal) {
        modelSelect.value = modelVal;
        if (modelInput) modelInput.style.display = "none";
      } else {
        modelSelect.value = "";
        if (modelInput) {
          modelInput.value = modelVal;
          modelInput.style.display = "block";
        }
      }
    }

    // Cloud cleanup model. The stored model wins while cleanup actually points
    // at the cloud; otherwise the last cloud choice is restored so a trip
    // through Local/Off does not lose it.
    const cleanupEngine = config.cleanup_engine || "off";
    const cloudModel = cleanupEngine === "cloud" && (config.cleanup_model || "").trim()
      ? config.cleanup_model.trim()
      : rememberedCleanupCloudModel();
    writeCleanupCloudModel(cloudModel);
    rememberCleanupCloudModel();

    renderDictionary(config.dictionary);
    punctuationToggle.checked = config.auto_punctuation;
    fillersToggle.checked = config.remove_fillers;
    autopasteToggle.checked = config.auto_paste;
    startupToggle.checked = config.launch_on_startup;
    if (keepClipsToggle) keepClipsToggle.checked = !!config.keep_audio_clips;
    // `stt_local` and `cleanup_engine` are the real switches; `offline_mode` is
    // the retired legacy mirror (still read here so old configs round-trip).
    setSpeechSource(config.stt_local ? "local" : "cloud");
    setCleanupSource(cleanupEngine);
    currentConfig.stt_local = !!config.stt_local;
    currentConfig.offline_mode = !!config.offline_mode;
    setSttChoice(["v2", "v3", "sv"].includes(config.stt_version) ? config.stt_version : "v2");
    currentConfig.stt_version = getSttChoice();
    setLlmChoice(["2b", "4b", "9b"].includes(config.llm_model_size) ? config.llm_model_size : "4b");
    currentConfig.llm_model_size = getLlmChoice();
    currentConfig.models_dir = config.models_dir || "";
    currentConfig.custom_llm_path = config.custom_llm_path || "";
    currentConfig.custom_stt_dir = config.custom_stt_dir || "";
    renderModelPanel();
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
    if (apiKeyInput && apiKeyInput.value) {
      validateApiKeyAuto();
    }
  } catch (err) {
    log(`Config load error: ${err}`);
  }
}

async function saveConfig() {
  if (!currentConfig) return;
  const speech = speechSource();
  const cleanup = cleanupSource();
  // The two source radios are the only thing the panel stores. Everything the
  // Rust side reads is derived here, so the pair can never disagree:
  //   speech  local -> stt_local true,  cloud -> stt_local false
  //   cleanup off   -> cleanup_engine "off"   + cleanup_model ""
  //   cleanup local -> cleanup_engine "local" + cleanup_model "2b"|"4b"|"9b"
  //                    (the local engine needs a non-empty model id to run;
  //                    the Qwen row id is the marker it has always used)
  //   cleanup cloud -> cleanup_engine "cloud" + cleanup_model <provider id>
  // An empty model id is not a neutral value here: `cleanup_engine: "cloud"`
  // with `cleanup_model: ""` passes Rust's gate as "no model at all", so a cloud
  // user would silently get rules-only output. Land on the provider's default
  // and paint it, so what runs is exactly what the card shows.
  const provider = sttProviderSelect ? sttProviderSelect.value : "groq";
  const cleanupModel = cleanup === "off"
    ? ""
    : cleanup === "local"
      ? getLlmChoice(currentConfig.llm_model_size || "4b")
      : cleanupCloudModel() || defaultCleanupCloudModel(provider);
  if (cleanup === "cloud" && cleanupModel && !cleanupCloudModel()) {
    writeCleanupCloudModel(cleanupModel);
  }
  const updatedConfig = {
    ...currentConfig,
    api_base_url: apiUrlInput ? apiUrlInput.value : currentConfig.api_base_url,
    model: modelSelect && modelSelect.value
      ? modelSelect.value
      : (modelInput ? modelInput.value : currentConfig.model),
    cleanup_model: cleanupModel,
    api_key: apiKeyInput ? apiKeyInput.value : currentConfig.api_key,
    auto_punctuation: punctuationToggle.checked,
    remove_fillers: fillersToggle.checked,
    auto_paste: autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
    keep_audio_clips: !!keepClipsToggle?.checked,
    dictionary: currentConfig.dictionary || [],
    hotkey: hotkeyInput ? hotkeyInput.value : "Ctrl+Space",
    hotkey_mode: hotkeyModeSelect ? hotkeyModeSelect.value : "hold",
    // `stt_local` and `cleanup_engine` are the fields Rust actually reads;
    // `offline_mode` is the retired legacy mirror, still written so older
    // configs keep round-tripping.
    stt_local: speech === "local",
    cleanup_engine: cleanup,
    offline_mode: speech === "local" || cleanup === "local",
    stt_version: getSttChoice(currentConfig.stt_version || "v2"),
    llm_model_size: getLlmChoice(currentConfig.llm_model_size || "4b"),
    models_dir: currentConfig.models_dir || "",
    custom_llm_path: currentConfig.custom_llm_path || "",
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
    // `update_config` rejects the whole save when a cloud leg is incomplete
    // ("[CFG-002] API key cannot be empty…", "[CFG-005] Cloud AI cleanup needs an
    // API key…"). The About section is not where the user just clicked, so the
    // reason also lands in the Models footer — otherwise the radios look saved
    // and silently revert on the next launch.
    if (String(err).includes("[CFG-") && offlineModelStatus) {
      offlineModelStatus.textContent = `Not saved — ${err}`;
      offlineModelStatus.style.color = "#ff6b6b";
    }
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
  if (apiUrlInput) apiUrlInput.addEventListener("change", saveConfig);
  if (apiKeyInput) apiKeyInput.addEventListener("change", saveConfig);
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

// Fill the transfer bar with the real percent the Rust loop reports per event.
// No percent (starting/failed events) leaves the indeterminate slide running.
function setDownloadDialogBarPercent(percent) {
  if (!downloadModalProgress) return;
  const bar = downloadModalProgress.querySelector(".model-card-bar");
  if (!bar) return;
  if (typeof percent !== "number" || Number.isNaN(percent)) return;
  const clamped = Math.max(0, Math.min(100, percent));
  bar.classList.add("is-determinate");
  const fill = bar.querySelector("i");
  if (fill) fill.style.setProperty("width", `${clamped.toFixed(0)}%`);
  if (downloadModalPercent) downloadModalPercent.textContent = `${clamped.toFixed(0)}%`;
}

let pendingDownloadGroup = null;

function engineDialogText() {
  if (!engineStatusData) return "Checking…";
  const sttInstalled = !!engineStatusData.stt_installed;
  const llamaInstalled = !!engineStatusData.llama_installed;
  if (sttInstalled && llamaInstalled) {
    const megabytes = Math.round((Number(engineStatusData.installed_bytes) || 0) / (1024 * 1024));
    return `Engine ready — ${megabytes} MB on this computer`;
  }
  return "Engine missing — it downloads first with this model";
}

function openDownloadDialog(group) {
  if (!downloadModal || !downloadModalTitle) return;
  pendingDownloadGroup = group;
  if (downloadModalTitle) downloadModalTitle.textContent = `Download ${group.name}`;
  if (downloadModalSize) downloadModalSize.textContent = formatCatalogBytes(group.size_bytes);
  if (downloadModalDest) downloadModalDest.textContent = modelsDirResolved
    ? `${modelsDirResolved}${modelsDirIsCustom ? " (custom)" : ""}`
    : "Not chosen yet — you'll pick a folder";
  if (downloadModalEngine) downloadModalEngine.textContent = engineDialogText();
  if (downloadModalMessage) {
    downloadModalMessage.textContent = "Press CONTINUE to start.";
    downloadModalMessage.className = "settings-hint";
  }
  if (downloadModalProgress) downloadModalProgress.hidden = true;
  if (downloadModalPercent) downloadModalPercent.textContent = "0%";
  if (downloadModalChangeBtn) downloadModalChangeBtn.disabled = false;
  if (downloadModalCancel) {
    downloadModalCancel.textContent = "CANCEL";
    downloadModalCancel.disabled = false;
  }
  if (downloadModalContinue) downloadModalContinue.disabled = false;
  downloadModal.classList.remove("closing");
  downloadModal.classList.add("open");
}

function closeDownloadDialog() {
  if (!downloadModal) return;
  downloadModal.classList.remove("open");
  downloadModal.classList.add("closing");
  const group = pendingDownloadGroup;
  const button = group ? group.rowButton : null;
  setTimeout(() => {
    downloadModal.classList.remove("closing");
    if (group) group.inFlight = false;
    if (button) {
      button.disabled = false;
      button.textContent = "DOWNLOAD";
    }
  }, 200);
}

function pickDownloadFolder() {
  return open({ directory: true, multiple: false, defaultPath: modelsDirResolved || undefined }).then((picked) => {
    if (!picked) return false;
    if (currentConfig) currentConfig.models_dir = picked;
    return Promise.resolve().then(async () => { await saveConfig(); await loadModelsDir(); }).then(() => true);
  }).catch(() => false);
}

async function startPendingDownload() {
  const group = pendingDownloadGroup;
  if (!group || !downloadModal) return;
  let engineMissing = !engineStatusData
    || !(engineStatusData.stt_installed && engineStatusData.llama_installed);
  if (engineMissing) {
    if (downloadModalEngine) downloadModalEngine.textContent = "Downloading engine…";
    if (downloadModalMessage) {
      downloadModalMessage.textContent = "The engine is missing — downloading it first…";
      downloadModalMessage.className = "settings-hint";
    }
    setModelOperationRunning(true, "Downloading engine…");
    try {
      engineStatusData = await invoke("download_engines");
      renderEngineStatus();
      if (downloadModalEngine) downloadModalEngine.textContent = engineDialogText();
      setModelOperationRunning(false);
      engineMissing = false;
    } catch (err) {
      if (downloadModalMessage) {
        downloadModalMessage.textContent = `Engine download failed: ${err}`;
        downloadModalMessage.className = "settings-hint download-modal-message is-error";
      }
      setModelOperationRunning(false);
      return;
    }
  }
  if (downloadModalProgress) downloadModalProgress.hidden = false;
  if (downloadModalCancel) {
    downloadModalCancel.textContent = "CANCEL";
    downloadModalCancel.disabled = false;
  }
  if (downloadModalContinue) downloadModalContinue.disabled = true;
  if (downloadModalChangeBtn) downloadModalChangeBtn.disabled = true;
  setModelOperationRunning(true, `Downloading ${group.name}…`);
  setDownloadDialogBarPercent(0);
  try {
    modelCatalogData = await invoke("download_model", { id: group.id });
    renderModelTables();
    setDownloadDialogBarPercent(100);
    if (downloadModalMessage) {
      downloadModalMessage.textContent = `${group.name} installed`;
      downloadModalMessage.className = "settings-hint download-modal-message is-ok";
    }
    if (downloadModalCancel) {
      downloadModalCancel.textContent = "CLOSE";
      downloadModalCancel.disabled = true;
    }
    if (downloadModalContinue) downloadModalContinue.disabled = true;
    setTimeout(() => closeDownloadDialog(), 1400);
  } catch (err) {
    if (downloadModalMessage) {
      const cancelled = String(err).includes("cancelled");
      downloadModalMessage.textContent = cancelled
        ? "Download cancelled — the partial file is kept. Press DOWNLOAD to resume."
        : `Model download failed: ${err}`;
      downloadModalMessage.className = cancelled
        ? "settings-hint download-modal-message is-ok"
        : "settings-hint download-modal-message is-error";
    }
    if (downloadModalCancel) {
      downloadModalCancel.textContent = "CLOSE";
      downloadModalCancel.disabled = true;
    }
    if (downloadModalContinue) downloadModalContinue.disabled = false;
  } finally {
    setModelOperationRunning(false);
  }
}

async function cancelPendingDownload() {
  const cancelBtn = downloadModalCancel;
  if (!cancelBtn) return;
  cancelBtn.disabled = true;
  cancelBtn.textContent = "CANCELLING…";
  try {
    await invoke("cancel_downloads");
    if (downloadModalMessage) {
      downloadModalMessage.textContent = "Cancelled — the partial file is kept. Press DOWNLOAD to resume.";
      downloadModalMessage.className = "settings-hint download-modal-message is-ok";
    }
  } catch (err) {
    if (downloadModalMessage) {
      downloadModalMessage.textContent = `Cancel failed: ${err}`;
      downloadModalMessage.className = "settings-hint download-modal-message is-error";
    }
  } finally {
    if (cancelBtn) {
      cancelBtn.textContent = "CLOSE";
      cancelBtn.disabled = false;
    }
  }
}

function setupDownloadModal() {
  if (downloadModalClose) downloadModalClose.addEventListener("click", closeDownloadDialog);
  document.addEventListener("click", (e) => {
    const target = e.target && e.target.closest ? e.target.closest("#download-modal") : null;
    if (target) closeDownloadDialog();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && downloadModal && downloadModal.classList.contains("open")) closeDownloadDialog();
  });
  if (downloadModalCancel) {
    downloadModalCancel.addEventListener("click", async () => {
      if (downloadModalCancel.disabled) return;
      if (downloadModalContinue && !downloadModalContinue.disabled) return; // downloading
      await cancelPendingDownload();
    });
  }
  if (downloadModalContinue) {
    downloadModalContinue.addEventListener("click", async () => {
      if (downloadModalContinue.disabled) return;
      if (!pendingDownloadGroup) return;
      await startPendingDownload();
    });
  }
  if (downloadModalChangeBtn) {
    downloadModalChangeBtn.addEventListener("click", async () => {
      const picked = await pickDownloadFolder();
      if (picked && downloadModalDest) {
        downloadModalDest.textContent = modelsDirResolved
          ? `${modelsDirResolved}${modelsDirIsCustom ? " (custom)" : ""}`
          : "Not chosen yet — you'll pick a folder";
      }
      if (downloadModalEngine) downloadModalEngine.textContent = engineDialogText();
    });
  }
}

function openRemoveModelsModal() {
  if (!confirmModal || !confirmModalMessage) return;
  if (offlineModelStatusData) {
    confirmModalMessage.textContent = `Remove all downloaded SmoothFlow models? This frees ${formatModelSize(offlineModelStatusData.total_bytes)} from ${modelsDirResolved || "the default models folder"}. The engine files are kept.`;
  } else {
    confirmModalMessage.textContent = "Remove all downloaded SmoothFlow models?";
  }
  if (confirmModalTitle) confirmModalTitle.textContent = "Remove models";
  if (confirmModalConfirm) confirmModalConfirm.disabled = false;
  if (confirmModalCancel) confirmModalCancel.disabled = false;
  confirmModal.classList.remove("closing");
  confirmModal.classList.add("open");
}

function closeRemoveModelsModal() {
  if (!confirmModal) return;
  confirmModal.classList.remove("open");
  confirmModal.classList.add("closing");
  setTimeout(() => {
    confirmModal.classList.remove("closing");
  }, 200);
}

async function confirmRemoveModels() {
  if (confirmModalConfirm) confirmModalConfirm.disabled = true;
  setModelOperationRunning(true, "Deleting downloaded models...");
  try {
    offlineModelStatusData = await invoke("delete_models");
    if (offlineModelStatus) offlineModelStatus.textContent = "Downloaded models deleted.";
    await loadModelCatalog();
  } catch (err) {
    setModelOperationRunning(false);
    await loadOfflineModelStatus(`Model deletion failed: ${err}`);
  }
  closeRemoveModelsModal();
}

setupDownloadModal();
if (confirmModalClose) confirmModalClose.addEventListener("click", closeRemoveModelsModal);
if (confirmModalCancel) confirmModalCancel.addEventListener("click", closeRemoveModelsModal);
if (confirmModalConfirm) confirmModalConfirm.addEventListener("click", async () => {
  if (modelOperationRunning) return;
  await confirmRemoveModels();
});
document.addEventListener("click", (e) => {
  const target = e.target && e.target.closest ? e.target.closest("#confirm-modal") : null;
  if (target) closeRemoveModelsModal();
});

// Models panel: which machine runs each stage.
// The Rust side needs BOTH fields for a cleanup model to run: `cleanup_engine`
// ("off" | "local" | "cloud") and a non-empty `cleanup_model`. saveConfig()
// derives that pair from the card radios so they cannot drift, and the cloud
// model id is remembered in localStorage so a trip through Local/Off keeps it.
const CLEANUP_CLOUD_MODEL_STORAGE_KEY = "smoothflow_cleanup_cloud_model";

function cleanupCloudModel() {
  if (cleanupModelSelect && cleanupModelSelect.value) return cleanupModelSelect.value;
  // Only a *visible* custom box counts - a hidden stale value is not a choice.
  if (cleanupModelInput && cleanupModelInput.style.display !== "none") {
    return (cleanupModelInput.value || "").trim();
  }
  return "";
}

function writeCleanupCloudModel(value) {
  const next = (value || "").trim();
  if (!cleanupModelSelect) {
    if (cleanupModelInput) cleanupModelInput.value = next;
    return;
  }
  // "" is the "Custom..." option, never a model id: an empty value belongs in
  // the text box, not selected as the placeholder option.
  const hasOption = next !== ""
    && [...cleanupModelSelect.options].some((option) => option.value === next);
  if (hasOption) {
    cleanupModelSelect.value = next;
    if (cleanupModelInput) {
      cleanupModelInput.value = "";
      cleanupModelInput.style.display = "none";
    }
    return;
  }
  cleanupModelSelect.value = "";
  if (cleanupModelInput) {
    cleanupModelInput.value = next;
    cleanupModelInput.style.display = next ? "block" : "none";
  }
}

function rememberedCleanupCloudModel() {
  const fallback = defaultCleanupCloudModel(sttProviderSelect ? sttProviderSelect.value : "groq");
  try {
    return (localStorage.getItem(CLEANUP_CLOUD_MODEL_STORAGE_KEY) || "").trim() || fallback;
  } catch (err) {
    return fallback;
  }
}

function rememberCleanupCloudModel() {
  const model = cleanupCloudModel();
  if (!model) return;
  try {
    localStorage.setItem(CLEANUP_CLOUD_MODEL_STORAGE_KEY, model);
  } catch (err) { /* storage blocked: the provider default still restores */ }
}

// One-line name for the summary strip: the catalog carries the full name, but
// "Parakeet TDT 0.6B v2" is too long to sit beside another value.
function shortModelName(group) {
  if (!group) return "";
  return String(group.name || "").replace(/^Parakeet TDT 0\.6B\s+/i, "Parakeet ");
}

function catalogGroup(kind, id) {
  return (modelCatalogData || []).find((group) => group.kind === kind && group.id === id) || null;
}

// The dialog is the only consumer of the resolved folder now: the destination
// line is where the choice is confirmed, and the picker is right next to it.
function renderDownloadDialogDestination() {
  if (!downloadModalDest) return;
  downloadModalDest.textContent = modelsDirResolved
    ? `${modelsDirResolved}${modelsDirIsCustom ? " (custom)" : ""}`
    : "Not chosen yet — you'll pick a folder";
}

// The single place that paints the two cards: which panel is open, which option
// box is selected, and the two summary chips. Every listener ends up here, so a
// state restored from config and a state the user just clicked look identical.
function renderModelPanel() {
  const speech = speechSource();
  const cleanup = cleanupSource();

  if (offlineSpeechControls) offlineSpeechControls.hidden = speech !== "local";
  if (sttCloudControls) sttCloudControls.hidden = speech !== "cloud";
  if (cleanupLocalControls) cleanupLocalControls.hidden = cleanup !== "local";
  if (cleanupCloudControls) cleanupCloudControls.hidden = cleanup !== "cloud";
  // A custom endpoint is the only case where the URL box is worth showing.
  if (sttUrlRow) sttUrlRow.hidden = !sttProviderSelect || sttProviderSelect.value !== "custom";

  // The radio itself is 1px and transparent, so the box carries the state.
  for (const radio of document.querySelectorAll(".source-option input[type=radio]")) {
    radio.closest(".source-option")?.classList.toggle("is-selected", radio.checked);
  }

  if (summarySpeech) {
    // `currentConfig` first, then the DOM: the table can still be loading when
    // this runs, and reading a not-yet-rendered radio would fall back to "v2".
    const sttId = (currentConfig && currentConfig.stt_version) || getSttChoice();
    const sttName = modelSelect && modelSelect.value
      ? modelSelect.value
      : (modelInput ? modelInput.value.trim() : "");
    summarySpeech.textContent = speech === "local"
      ? `Local · ${shortModelName(catalogGroup("stt", sttId)) || "Parakeet " + sttId}`
      : `Cloud · ${sttName || "not set"}`;
  }
  if (summaryCleanup) {
    const provider = sttProviderSelect ? sttProviderSelect.value : "groq";
    const llmId = (currentConfig && currentConfig.llm_model_size) || getLlmChoice();
    summaryCleanup.textContent = cleanup === "off"
      ? "Off · rules only"
      : cleanup === "local"
        ? `Local · ${shortModelName(catalogGroup("llm", llmId)) || "Qwen " + llmId}`
        : `Cloud · ${cleanupCloudModel() || defaultCleanupCloudModel(provider)}`;
  }

  // Cloud cleanup borrows the Speech provider and key - it has nothing of its
  // own to configure, so the note names the provider to make that obvious.
  if (cleanupProviderNote) {
    const provider = sttProviderSelect ? sttProviderSelect.value : "custom";
    const providerName = provider === "groq" ? "Groq" : provider === "openai" ? "OpenAI" : "";
    cleanupProviderNote.textContent = providerName
      ? `Uses the Speech provider (${providerName}) and key.`
      : "Uses the Speech provider and key.";
  }
  if (cleanupFallbackNote) {
    const fallback = (currentConfig?.cleanup_fallback_model || "").trim();
    cleanupFallbackNote.textContent = fallback
      ? `If the first model fails, the next one is tried: ${fallback}.`
      : "If the first model fails, the rules-only result is used.";
  }
}

function afterModelChoiceChange() {
  renderModelPanel();
  saveConfig();
}

function afterCleanupCloudModelChange() {
  rememberCleanupCloudModel();
  renderModelPanel();
  saveConfig();
}

// Provider switch: the URL and both model dropdowns follow the select. Custom
// keeps whatever URL is already there - rewriting it would discard the value
// the user is about to type over.
function applyProviderChange() {
  const provider = sttProviderSelect ? sttProviderSelect.value : "custom";
  if (apiUrlInput && PROVIDER_URLS[provider]) apiUrlInput.value = PROVIDER_URLS[provider];
  updateModelDropdowns(provider);
  afterCleanupCloudModelChange();
}

if (sttProviderSelect) sttProviderSelect.addEventListener("change", applyProviderChange);

// Source radios: the only thing that decides where each stage runs.
for (const radio of document.querySelectorAll('input[name="speech-source"], input[name="cleanup-source"]')) {
  radio.addEventListener("change", () => { renderModelPanel(); saveConfig(); });
}

// Models folder: the resolved path shown in the download dialog.
let modelsDirResolved = "";
let modelsDirIsCustom = false;

async function loadModelsDir() {
  try {
    const location = await invoke("get_models_dir");
    modelsDirResolved = location?.resolved || "";
    modelsDirIsCustom = !!location?.is_custom;
  } catch (err) {
    log(`Models folder error: ${err}`);
  }
}

function applyCustomLlmPath() {
  if (!currentConfig) return;
  // BYO field removed from UI; keep any previously set value in config.
  saveConfig();
}



function offlineStatusSummary() {
  const parts = [];
  if (modelCatalogData) {
    const sttGroup = catalogGroup("stt", getSttChoice());
    const llmGroup = catalogGroup("llm", getLlmChoice());
    if (sttGroup) parts.push(`${sttGroup.name}: ${sttGroup.installed ? "installed" : "not installed"}`);
    if (llmGroup) parts.push(`${llmGroup.name}: ${llmGroup.installed ? "installed" : "not installed"}`);
  }
  if (offlineModelStatusData) parts.push(`${formatModelSize(offlineModelStatusData.total_bytes)} on disk`);
  return parts.join(" · ");
}

function setOfflineModelError(message) {
  if (!offlineModelStatus) return;
  offlineModelStatus.textContent = message;
  offlineModelStatus.style.color = "#ff6b6b";
}

function renderOfflineModelStatus() {
  if (!offlineModelStatus) return;
  offlineModelStatus.textContent = modelOperationRunning
    ? `${offlineStatusSummary()} · Model operation in progress`
    : offlineStatusSummary() || "No local model status available.";
  offlineModelStatus.style.color = modelOperationRunning ? "#f4d1ba" : "#cdafa0";
}

function setModelOperationRunning(running, message = "") {
  modelOperationRunning = running;
  if (deleteModelsBtn) deleteModelsBtn.disabled = running;
  if (refreshModelsBtn) refreshModelsBtn.disabled = running;
  // Switching the source mid-transfer would change what the download is for.
  for (const radio of document.querySelectorAll('input[name="speech-source"], input[name="cleanup-source"]')) {
    radio.disabled = running;
  }
  if (running) {
    if (offlineModelStatus) {
      offlineModelStatus.textContent = message;
      offlineModelStatus.style.color = "#f4d1ba";
    }
  } else {
    renderOfflineModelStatus();
  }
}

async function loadOfflineModelStatus(operationError = "") {
  loadEngineStatus();
  loadModelCatalog();
  if (!offlineModelStatus) return;
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

// Engine binaries (smoothflow-stt + llama-server) are required for local models
// to run at all. They are separate from the models and downloaded on demand, so
// this row reports on-disk reality only - get_engine_status never hits the wire.
function enginesReady() {
  return !!(engineStatusData && engineStatusData.stt_installed && engineStatusData.llama_installed);
}

function renderEngineStatus(status) {
  if (!engineStatusEl) return;
  if (status) engineStatusData = status;
  const sttInstalled = !!engineStatusData?.stt_installed;
  const llamaInstalled = !!engineStatusData?.llama_installed;
  if (sttInstalled && llamaInstalled) {
    const megabytes = Math.round((Number(engineStatusData.installed_bytes) || 0) / (1024 * 1024));
    engineStatusEl.textContent = `Engine ready — ${megabytes} MB on this computer`;
    engineStatusEl.style.color = "#cdafa0";
  } else if (sttInstalled || llamaInstalled) {
    engineStatusEl.textContent = sttInstalled
      ? "Spoken-word engine installed — cleanup engine missing"
      : "Cleanup engine installed — spoken-word engine missing";
    engineStatusEl.style.color = "#f4d1ba";
  } else {
    engineStatusEl.textContent = "Engine not downloaded yet — required for local models";
    engineStatusEl.style.color = "#f4d1ba";
  }
}

async function loadEngineStatus() {
  if (!engineStatusEl) return;
  try {
    engineStatusData = await invoke("get_engine_status");
    renderEngineStatus();
  } catch (err) {
    engineStatusData = null;
    engineStatusEl.textContent = `Could not check engine: ${err}`;
    engineStatusEl.style.color = "#ff6b6b";
  }
}

// â”€â”€ Activation shortcut: choose keys, don't type them â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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
      hotkeyChip.textContent = mods.length ? mods.join(" + ") + " â€¦" : "Press keysâ€¦";
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
    if (hotkeyChip) hotkeyChip.textContent = "Press keysâ€¦";
    window.addEventListener("keydown", hotkeyCaptureKeydown, true);
    document.addEventListener("click", hotkeyCancelAway, true);
  });
}

// â”€â”€ Activation style: choose-only dropdown, description follows it â”€â”€
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

// â”€â”€ Reload: put the default Ctrl+Space back â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
if (hotkeyReset) {
  hotkeyReset.addEventListener("click", () => {
    if (hotkeyCapturing) endHotkeyCapture(false);
    if (hotkeyInput) hotkeyInput.value = "Ctrl+Space";
    renderHotkeyChip("Ctrl+Space");
    saveConfig();
  });
}

// â”€â”€ "!" circle: one delegated listener drops every row's context
// open, and the same click drops it closed again. â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

// ── Model table: every row comes from get_model_catalog ─────────────────────
// The old panel hardcoded model names, sizes and a SenseVoice card that the
// backend could never download ("Unknown STT model variant 'sv'"). The table is
// data-driven now: `supported: false` renders greyed as "Not available yet"
// instead of a button that fails.
let modelCatalogData = null;

function formatCatalogBytes(bytes) {
  const n = Number(bytes) || 0;
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (n > 0) return `${Math.round(n / (1024 * 1024))} MB`;
  return "—";
}

function modelBenchmarkLine(group) {
  const bench = group.benchmark || {};
  if (bench.wer_earnings22 != null && bench.realtime_factor != null) {
    return `${(bench.wer_earnings22 * 100).toFixed(1)}% WER · ${bench.realtime_factor.toFixed(1)}× realtime (measured)`;
  }
  return "not benchmarked";
}

function modelRowLanguages(group) {
  const languages = group.languages || [];
  if (group.kind === "llm" || languages.length === 0) return "";
  if (languages.length > 4) return `${languages.length} languages`;
  return languages.map((code) => code.toUpperCase()).join(" · ");
}

function modelRowMatches(group, query) {
  const needle = (query || "").trim().toLowerCase();
  if (!needle) return true;
  const haystack = [group.name, group.description, ...(group.languages || [])]
    .join(" ")
    .toLowerCase();
  return haystack.includes(needle);
}

function createModelRow(group) {
  const row = document.createElement("label");
  const supported = !!group.supported;
  // Source the selection from `currentConfig`, not the radios: config is
  // restored at startup with `setSttChoice(...)`, which can run before the
  // table is rendered, so reading the DOM here would fall back to "v2" and
  // silently rewrite a v3 user's choice on the next unrelated change.
  const selected = group.kind === "stt"
    ? ((currentConfig && currentConfig.stt_version) || getSttChoice())
    : ((currentConfig && currentConfig.llm_model_size) || getLlmChoice());
  row.className = "model-row";
  if (group.id === selected && supported) row.classList.add("is-selected");
  if (!supported) row.classList.add("is-disabled");

  const input = document.createElement("input");
  input.type = "radio";
  input.className = "model-radio-input";
  input.name = group.kind === "stt" ? "offline-stt-choice" : "offline-llm-choice";
  input.value = group.id;
  input.disabled = !supported || !group.installed;
  input.checked = group.id === selected && supported;
  // Dynamic rows miss the startup-time listener loop, so the change handler is
  // attached here.
  input.addEventListener("change", () => updateOfflineConfig());

  const main = document.createElement("div");
  main.className = "model-row-main";
  const name = document.createElement("span");
  name.className = "model-row-name";
  name.textContent = group.name;
  const desc = document.createElement("span");
  desc.className = "model-row-desc";
  desc.textContent = group.description;
  main.appendChild(name);
  main.appendChild(desc);
  const meta = modelRowLanguages(group);
  if (meta) {
    const tags = document.createElement("span");
    tags.className = "model-row-meta";
    tags.style.fontSize = "10px";
    tags.style.color = "var(--color-text-secondary)";
    tags.textContent = meta;
    main.appendChild(tags);
  }

  const metric = document.createElement("span");
  metric.className = "model-metric";
  metric.textContent = modelBenchmarkLine(group);

  const side = document.createElement("div");
  side.className = "model-row-side";
  side.appendChild(metric);

  if (!supported) {
    const note = document.createElement("span");
    note.className = "model-metric";
    note.textContent = "Not available yet";
    side.appendChild(note);
  } else if (group.installed) {
    const state = document.createElement("span");
    state.className = "model-metric";
    state.textContent = "Installed";
    side.appendChild(state);
  } else {
    const action = document.createElement("button");
    action.type = "button";
    action.className = "reset-cancel-btn";
    action.style.cssText = "padding: 8px 12px; white-space: nowrap;";
    action.textContent = group.partial
      ? `RESUME (${group.installed_file_count}/${group.file_count})`
      : "DOWNLOAD";
    action.disabled = modelOperationRunning;
    action.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      downloadModelGroup(group, action);
    });
    side.appendChild(action);
    const size = document.createElement("span");
    if (group.size_bytes > 0) {
      size.className = "model-metric";
      size.textContent = formatCatalogBytes(group.size_bytes);
      side.appendChild(size);
    }
  }

  row.appendChild(input);
  row.appendChild(main);
  row.appendChild(side);
  return row;
}

function renderModelTables() {
  const groups = modelCatalogData || [];
  const query = modelSearchInput ? modelSearchInput.value : "";
  for (const kind of ["stt", "llm"]) {
    const container = kind === "stt" ? modelTableEl : modelTableCleanupEl;
    if (!container) continue;
    container.replaceChildren();
    const rows = groups.filter((g) => g.kind === kind && modelRowMatches(g, query));
    if (rows.length === 0) {
      const empty = document.createElement("span");
      empty.className = "settings-hint";
      empty.style.marginTop = "0";
      empty.textContent = query ? "No model matches that search." : "Loading...";
      container.appendChild(empty);
      continue;
    }
    for (const group of rows) container.appendChild(createModelRow(group));
  }
}

async function loadModelCatalog() {
  try {
    modelCatalogData = await invoke("get_model_catalog");
    renderModelTables();
    // The summary chips prefer the catalog's model names over the id fallback,
    // so they are repainted once the real names exist.
    renderModelPanel();
  } catch (err) {
    log(`Model catalog error: ${err}`);
    if (offlineModelStatus) {
      offlineModelStatus.textContent = `Could not load the model list: ${err}`;
      offlineModelStatus.style.color = "#ff6b6b";
    }
  }
}

function downloadModelGroup(group, button) {
  if (modelOperationRunning || group.inFlight) return;
  group.inFlight = true;
  group.rowButton = button;
  button.disabled = true;
  button.textContent = "WORKING...";
  openDownloadDialog(group);
}

if (modelSearchInput) {
  modelSearchInput.addEventListener("input", () => renderModelTables());
}

function updateOfflineConfig() {
  if (!currentConfig) return;
  // A model row only chooses which local model; whether local runs at all is the
  // source radio's job, so repaint first and let saveConfig derive the fields.
  currentConfig.stt_version = getSttChoice();
  currentConfig.llm_model_size = getLlmChoice();
  renderModelPanel();
  saveConfig();
}

for (const r of document.querySelectorAll('input[name="offline-stt-choice"], input[name="offline-llm-choice"]')) {
  r.addEventListener("change", updateOfflineConfig);
}
for (const btn of document.querySelectorAll(".model-card-more")) {
  btn.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    const panel = document.getElementById(btn.getAttribute("data-more"));
    if (!panel) return;
    panel.hidden = !panel.hidden;
    btn.textContent = panel.hidden ? "show more" : "show less";
  });
}
if (refreshModelsBtn) refreshModelsBtn.addEventListener("click", () => loadOfflineModelStatus());

if (deleteModelsBtn) {
  deleteModelsBtn.addEventListener("click", async () => {
    if (modelOperationRunning) return;
    openRemoveModelsModal();
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

try {
  listen("model-group-download-progress", ({ payload }) => {
    if (!payload?.id) return;
    offlineModelProgress.set(payload.id, { status: payload.status, message: payload.message });
    renderOfflineModelStatus();
    if (downloadModal && downloadModal.classList.contains("open")) {
      if (payload.status === "progress" && payload.message) {
        if (downloadModalMessage) downloadModalMessage.textContent = payload.message;
      }
      setDownloadDialogBarPercent(payload.percent);
    }
    if (payload.status === "failed") {
      if (downloadModal && downloadModal.classList.contains("open")) {
        if (downloadModalMessage) {
          downloadModalMessage.textContent = `Model download failed: ${payload.message || "Unknown error"}`;
          downloadModalMessage.className = "settings-hint download-modal-message is-error";
        }
      }
      setOfflineModelError(`Model download failed: ${payload.message || "Unknown error"}`);
    }
  }).catch((err) => setOfflineModelError(`Could not listen for model download progress: ${err}`));
} catch (err) {
  setOfflineModelError(`Could not listen for model download progress: ${err}`);
}

try {
  listen("engine-download-progress", ({ payload }) => {
    if (!payload?.id) return;
    const engineName = payload.id === "stt"
      ? "spoken-word engine"
      : payload.id === "llama" ? "cleanup engine" : payload.id;
    if (payload.status === "failed") {
      if (downloadModal && downloadModal.classList.contains("open") && pendingDownloadGroup) {
        if (downloadModalMessage) {
          downloadModalMessage.textContent = `Engine download failed: ${payload.message || "Unknown error"}`;
          downloadModalMessage.className = "settings-hint download-modal-message is-error";
        }
      }
      setOfflineModelError(`Engine download failed: ${payload.message || "Unknown error"}`);
      return;
    }
    if (downloadModal && downloadModal.classList.contains("open") && pendingDownloadGroup) {
      if (downloadModalEngine) {
        downloadModalEngine.textContent = payload.status === "complete"
          ? `${engineName} downloaded.`
          : payload.status === "progress"
            ? payload.message || `Downloading ${engineName}...`
            : `Downloading ${engineName}...`;
      }
      setDownloadDialogBarPercent(payload.percent);
    }
  }).catch((err) => setOfflineModelError(`Could not listen for engine download progress: ${err}`));
} catch (err) {
  setOfflineModelError(`Could not listen for engine download progress: ${err}`);
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

// â”€â”€ History list â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Handy-style reverse-chronological rows, paged 30 at a time out of SQLite.
// Every piece of dictation text lands via textContent â€” never innerHTML â€” so a
// transcript can never inject markup. Stats are derived from these rows on the
// Rust side, so the UI has no counters of its own.
const HISTORY_PAGE_SIZE = 30;
const historyList = document.getElementById("history-list");
const historyEmpty = document.getElementById("history-empty");
const historyLoadingEl = document.getElementById("history-loading");
const historySentinel = document.getElementById("history-sentinel");
const transcribeFileBtn = document.getElementById("transcribe-file-btn");
const transcribeFileStatus = document.getElementById("transcribe-file-status");

// Static icon markup only â€” no user data ever goes through innerHTML.
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
// enabled â€” which let a second run of the same clip start while the first was
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
// actually laid out â€” inside a hidden section clientHeight is 0 and every row
// would look truncated.
function measureHistoryRow(row) {
  const text = row.querySelector(".history-text");
  const more = row.querySelector(".history-more");
  if (!text || !more) return;
  if (text.clientHeight === 0) return;            // section hidden â€” measure later
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
  // transcript, and a failure with clips disabled writes no row at all â€” so an
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

  // Nothing to copy on a failed row â€” the clipboard would get an empty string.
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
        // Rust reports the refreshed entry â€” nothing to patch here.
        await invoke("redictate_from_clip", { id: entry.id });
      } catch (err) {
        console.error("redictate_from_clip failed:", err);
        showRowError(String(err));
      } finally {
        redictatingIds.delete(entry.id);
        // `history-updated` has usually replaced this row already, so re-enable
        // whichever button is live now â€” not just this (possibly detached) one.
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
    ? "Transcription failed â€” press â†» to retry from the saved recording"
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
  // No toggle on a failed row â€” measureHistoryRow() already bails when either
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

    // An empty page means the cursor cannot advance â€” stop paging instead of
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
// optimistically by the row itself â€” handling them here would double-apply.
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

// â”€â”€ Transcribe an existing WAV file into History â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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
    transcribeFileBtn.textContent = "Transcribingâ€¦";
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
    updateMarquee.textContent = `UPDATE v${version} AVAILABLE â€” CLICK THE BELL TO VIEW`;
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
      console.warn("Update check failed:", err); // offline / endpoint not configured â€” non-fatal
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
        log("Update installed â€” restarting");
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
