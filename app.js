import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const debug = document.getElementById("debug-bar");
function log(msg) { if (debug) debug.textContent = msg; }
log("App loaded");

// DOM Elements
const recordBtn = document.getElementById("record-btn");
const btnLabel = document.getElementById("btn-label");
const statusText = document.getElementById("status-text");
const transcriptEl = document.getElementById("transcript");
const rawTranscriptEl = document.getElementById("raw-transcript");

const settingsToggle = document.getElementById("settings-toggle");
const settingsClose = document.getElementById("settings-close");
const settingsModal = document.getElementById("settings-modal");

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
const overlayTopToggle = document.getElementById("overlay-top-toggle");
const settingsStatus = document.getElementById("settings-status");

const dictionaryTags = document.getElementById("dictionary-tags");
const dictionaryInput = document.getElementById("dictionary-input");
const dictionaryAddBtn = document.getElementById("dictionary-add-btn");

let isRecording = false;
let currentConfig = null;

function updateRecordingUI(recording) {
  isRecording = recording;
  if (recording) {
    recordBtn.classList.add("recording");
    btnLabel.textContent = "Stop";
    statusText.textContent = "Recording...";
    log("Recording started");
  } else {
    recordBtn.classList.remove("recording");
    btnLabel.textContent = "Record";
    statusText.textContent = "Idle";
    log("Recording stopped");
  }
}

// Events from Rust
listen("recording-state", (event) => {
  updateRecordingUI(event.payload);
}).catch(e => log(`listen error recording-state: ${e}`));

listen("transcript-result", (event) => {
  const text = event.payload;
  transcriptEl.textContent = text || "(silence)";
  log(`Transcript: ${text}`);
}).catch(e => log(`listen error transcript: ${e}`));

listen("raw-transcript", (event) => {
  rawTranscriptEl.textContent = event.payload || "(silence)";
  log(`Raw: ${event.payload}`);
}).catch(e => log(`listen error raw: ${e}`));

listen("transcription-error", (event) => {
  transcriptEl.textContent = `Error: ${event.payload}`;
  statusText.textContent = "Error";
  recordBtn.classList.remove("recording");
  btnLabel.textContent = "Record";
  isRecording = false;
  log(`ERROR: ${event.payload}`);
}).catch(e => log(`listen error: ${e}`));

listen("recording-error", (event) => {
  transcriptEl.textContent = `Recording error: ${event.payload}`;
  statusText.textContent = "Error";
  recordBtn.classList.remove("recording");
  btnLabel.textContent = "Record";
  isRecording = false;
  log(`REC ERROR: ${event.payload}`);
}).catch(e => log(`listen error: ${e}`));

// Hold-to-record: Space (record button is status indicator only)
document.addEventListener("keydown", (e) => {
  if (document.activeElement.tagName === "INPUT" || document.activeElement.tagName === "SELECT") return;
  if (settingsModal.classList.contains("open")) return;
  if (e.code === "Space" && !e.repeat && !isRecording) {
    e.preventDefault();
    invoke("start_recording").catch(err => log(`start error: ${err}`));
  }
});

document.addEventListener("keyup", (e) => {
  if (e.code === "Space" && isRecording) {
    e.preventDefault();
    invoke("stop_recording").catch(err => log(`stop error: ${err}`));
  }
});

// Settings modal
let isModalClosing = false;

function openSettings() {
  if (!settingsModal) return;
  settingsModal.classList.remove("closing");
  settingsModal.classList.add("open");
  log("Settings opened");
}

function closeSettings() {
  if (!settingsModal || isModalClosing) return;

  isModalClosing = true;
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
    if (overlayTopToggle) overlayTopToggle.checked = config.overlay_position === "top";
    log("Config loaded");
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
    auto_paste:     autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
    dictionary: currentConfig.dictionary || [],
    hotkey: hotkeyInput ? hotkeyInput.value : "Ctrl+Space",
    overlay_position: overlayTopToggle && overlayTopToggle.checked ? "top" : "bottom",
  };
  try {
    await invoke("update_config", { newConfig: updatedConfig });
    currentConfig = updatedConfig;
    settingsStatus.textContent = "Settings saved";
    setTimeout(() => { settingsStatus.textContent = "Settings saved automatically"; }, 2000);
    log("Config saved");
  } catch (err) {
    log(`Config save error: ${err}`);
    settingsStatus.textContent = "Save failed";
  }
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
  if (overlayTopToggle) overlayTopToggle.addEventListener("change", saveConfig);
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
    tag.innerHTML = `${word} <span class="dictionary-tag-remove" data-word="${word}">&times;</span>`;
    tag.querySelector(".dictionary-tag-remove").addEventListener("click", () => removeDictionaryWord(word));
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

loadConfig();
