import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
const debug = document.getElementById("debug-bar");
function log(msg) { if (debug) debug.textContent = msg; }
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
const modelInput = document.getElementById("model-input");
const cleanupModelInput = document.getElementById("cleanup-model-input");
const apiKeyInput = document.getElementById("api-key-input");
const toggleKeyVisibility = document.getElementById("toggle-key-visibility");
const punctuationToggle = document.getElementById("punctuation-toggle");
const fillersToggle = document.getElementById("fillers-toggle");
const autopasteToggle = document.getElementById("autopaste-toggle");
const startupToggle = document.getElementById("startup-toggle");
const hotkeyInput = document.getElementById("hotkey-input");
const settingsStatus = document.getElementById("settings-status");

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

// Toggle API key visibility
if (!toggleKeyVisibility) console.warn("toggle-key-visibility not found");
else toggleKeyVisibility.addEventListener("click", () => {
  const type = apiKeyInput.getAttribute("type") === "password" ? "text" : "password";
  apiKeyInput.setAttribute("type", type);
});

// Auto-validate API connection when key or URL is changed/pasted
const testApiStatus = document.getElementById("test-api-status");
let apiValidationTimer = null;

async function validateApiKeyAuto() {
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
    testApiStatus.textContent = ("Invalid API Key");
    testApiStatus.className = "invalid";
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
    modelInput.value = config.model;
    cleanupModelInput.value = config.cleanup_model;
    apiKeyInput.value = config.api_key;
    renderDictionary(config.dictionary);
    punctuationToggle.checked = config.auto_punctuation;
    fillersToggle.checked = config.remove_fillers;
    autopasteToggle.checked = config.auto_paste;
    startupToggle.checked = config.launch_on_startup;
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
    model: modelInput.value,
    cleanup_model: cleanupModelInput.value,
    api_key: apiKeyInput.value,
    auto_punctuation: punctuationToggle.checked,
    remove_fillers: fillersToggle.checked,
    auto_paste: autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
    dictionary: currentConfig.dictionary || [],
    hotkey: hotkeyInput ? hotkeyInput.value : "Ctrl+Space",
    overlay_position: currentConfig.overlay_position || "bottom",
  };
  try {
    await invoke("update_config", { newConfig: updatedConfig });
    currentConfig = updatedConfig;
    if (settingsStatus) {
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
    if (settingsStatus) settingsStatus.textContent = "Save failed: " + err;
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
  modelInput.addEventListener("change", saveConfig);
  cleanupModelInput.addEventListener("change", saveConfig);
  apiKeyInput.addEventListener("change", saveConfig);
  punctuationToggle.addEventListener("change", saveConfig);
  fillersToggle.addEventListener("change", saveConfig);
  autopasteToggle.addEventListener("change", saveConfig);
  startupToggle.addEventListener("change", saveConfig);
  if (hotkeyInput) hotkeyInput.addEventListener("change", saveConfig);
} catch (e) { console.warn("Config bind error:", e); }

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

loadConfig();

// Auto-updater
async function checkForUpdates() {
  const banner = document.getElementById("update-banner");
  const info = document.getElementById("update-info");
  const updateBtn = document.getElementById("update-btn");
  const laterBtn = document.getElementById("update-later-btn");
  if (!banner || !info) return; // banner DOM absent — keep the app working

  try {
    const update = await check();
    if (!update) return; // no update available

    info.textContent = update.body
      ? `SmoothFlow v${update.version} is available — ${update.body}`
      : `SmoothFlow v${update.version} is available`;
    banner.style.display = "block";

    if (updateBtn) {
      updateBtn.addEventListener("click", async () => {
        updateBtn.disabled = true;
        updateBtn.textContent = "UPDATING...";
        try {
          await update.downloadAndInstall();
          log("Update installed — restarting");
          await invoke("restart_app");
        } catch (err) {
          log(`Update failed: ${err}`);
          updateBtn.disabled = false;
          updateBtn.textContent = "UPDATE";
        }
      });
    }
    if (laterBtn) {
      laterBtn.addEventListener("click", () => { banner.style.display = "none"; });
    }
  } catch (err) {
    console.warn("Update check failed:", err); // offline / endpoint not configured — non-fatal
  }
}

checkForUpdates();
