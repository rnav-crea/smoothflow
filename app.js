import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const debug = document.getElementById("debug-bar");
function log(msg) { if (debug) debug.textContent = `▶ ${msg}`; }
log("JS loaded");

// DOM Elements
const recordBtn = document.getElementById("record-btn");
if (!recordBtn) { log("ERROR: #record-btn not found"); throw new Error("record-btn missing"); }
const glowRing = document.getElementById("glow-ring");
const visualizerContainer = recordBtn.parentElement;
const statusText = document.getElementById("status-text");
const transcriptEl = document.getElementById("transcript");
const copyBtn = document.getElementById("copy-btn");

const settingsToggle = document.getElementById("settings-toggle");
const settingsClose = document.getElementById("settings-close");
const settingsDrawer = document.getElementById("settings-drawer");
const drawerBackdrop = document.getElementById("drawer-backdrop");

const apiUrlInput = document.getElementById("api-url-input");
const modelInput = document.getElementById("model-input");
const cleanupModelInput = document.getElementById("cleanup-model-input");
const apiKeyInput = document.getElementById("api-key-input");
const toggleKeyVisibility = document.getElementById("toggle-key-visibility");
const punctuationToggle = document.getElementById("punctuation-toggle");
const fillersToggle = document.getElementById("fillers-toggle");
const autopasteToggle = document.getElementById("autopaste-toggle");
const startupToggle = document.getElementById("startup-toggle");
const settingsStatus = document.getElementById("settings-status");

if (!settingsToggle) log("WARNING: settings-toggle not found");

let isRecording = false;
let currentConfig = null;

// Helper to update recording state UI
function updateRecordingUI(recording) {
  isRecording = recording;
  if (isRecording) {
    visualizerContainer.classList.add("recording");
    statusText.textContent = "Recording...";
    log("Recording started");
  } else {
    visualizerContainer.classList.remove("recording");
    statusText.textContent = "Idle";
    log("Recording stopped");
  }
}

// Event Listeners from Rust Backend (e.g. triggered via Global Shortcut)
listen("recording-state", (event) => {
  log(`Event recording-state: ${event.payload}`);
  updateRecordingUI(event.payload);
}).catch(e => log(`listen error recording-state: ${e}`));

listen("transcript-result", (event) => {
  log(`Transcript: "${event.payload}"`);
  const text = event.payload;
  if (text) {
    transcriptEl.textContent = text;
    copyBtn.style.display = "block";
  } else {
    transcriptEl.textContent = "(silence)";
    copyBtn.style.display = "none";
  }
}).catch(e => log(`listen error transcript: ${e}`));

listen("transcription-error", (event) => {
  log(`ERROR: ${event.payload}`);
  transcriptEl.textContent = `Error: ${event.payload}`;
  statusText.textContent = "Error";
  visualizerContainer.classList.remove("recording");
  isRecording = false;
}).catch(e => log(`listen error: ${e}`));

listen("recording-error", (event) => {
  log(`REC ERROR: ${event.payload}`);
  transcriptEl.textContent = `Recording error: ${event.payload}`;
  statusText.textContent = "Error";
  visualizerContainer.classList.remove("recording");
  isRecording = false;
}).catch(e => log(`listen error: ${e}`));

// Toggle Recording manually
async function toggleRecording() {
  try {
    if (isRecording) {
      log("Calling stop_recording...");
      await invoke("stop_recording");
    } else {
      log("Calling start_recording...");
      await invoke("start_recording");
    }
  } catch (err) {
    log(`Toggle error: ${err}`);
    transcriptEl.textContent = `Error: ${err}`;
  }
}

recordBtn.addEventListener("click", toggleRecording);

// Hold-to-record: Space keydown = start, Space keyup = stop
document.addEventListener("keydown", (e) => {
  if (document.activeElement.tagName === "INPUT" || document.activeElement.tagName === "SELECT") return;
  if (e.code === "Space" && !e.repeat && !isRecording) {
    e.preventDefault();
    log("Hold: Space down → start");
    invoke("start_recording").catch(err => log(`start error: ${err}`));
  }
});

document.addEventListener("keyup", (e) => {
  if (e.code === "Space" && isRecording) {
    e.preventDefault();
    log("Hold: Space up → stop");
    invoke("stop_recording").catch(err => log(`stop error: ${err}`));
  }
});

// Copy Transcript to Clipboard
copyBtn.addEventListener("click", () => {
  const text = transcriptEl.textContent;
  if (text && text !== "Your dictated text will appear here. Press start to record." && text !== "(silence)") {
    navigator.clipboard.writeText(text);
    copyBtn.textContent = "Copied!";
    setTimeout(() => {
      copyBtn.textContent = "Copy";
    }, 1500);
  }
});

// Drawer toggle open/close
function openDrawer() {
  log("Settings drawer opened");
  settingsDrawer.classList.add("open");
}

function closeDrawer() {
  log("Settings drawer closed");
  settingsDrawer.classList.remove("open");
}

if (settingsToggle) settingsToggle.addEventListener("click", openDrawer);
if (settingsClose) settingsClose.addEventListener("click", closeDrawer);
if (drawerBackdrop) drawerBackdrop.addEventListener("click", closeDrawer);

// Show/Hide API Key password field
toggleKeyVisibility.addEventListener("click", () => {
  const type = apiKeyInput.getAttribute("type") === "password" ? "text" : "password";
  apiKeyInput.setAttribute("type", type);
});

// Configuration management
async function loadConfig() {
  try {
    log("Loading config...");
    const config = await invoke("get_config");
    currentConfig = config;
    log("Config loaded: " + config.api_base_url);
    
    apiUrlInput.value = config.api_base_url;
    modelInput.value = config.model;
    cleanupModelInput.value = config.cleanup_model;
    apiKeyInput.value = config.api_key;
    punctuationToggle.checked = config.auto_punctuation;
    fillersToggle.checked = config.remove_fillers;
    autopasteToggle.checked = config.auto_paste;
    startupToggle.checked = config.launch_on_startup;
  } catch (err) {
    log(`Config load error: ${err}`);
  }
}

async function saveConfig() {
  if (!currentConfig) { log("saveConfig skipped: no currentConfig"); return; }
  
  const updatedConfig = {
    api_base_url: apiUrlInput.value,
    model: modelInput.value,
    cleanup_model: cleanupModelInput.value,
    api_key: apiKeyInput.value,
    auto_punctuation: punctuationToggle.checked,
    remove_fillers: fillersToggle.checked,
    auto_paste: autopasteToggle.checked,
    launch_on_startup: startupToggle.checked,
  };
  
  try {
    log("Saving config...");
    await invoke("update_config", { newConfig: updatedConfig });
    currentConfig = updatedConfig;
    log("Config saved");
    showSaveStatus("Settings saved");
  } catch (err) {
    log(`Config save error: ${err}`);
    showSaveStatus("Save failed");
  }
}

function showSaveStatus(text) {
  settingsStatus.textContent = text;
  settingsStatus.style.opacity = 1;
  setTimeout(() => {
    settingsStatus.textContent = "Settings saved automatically";
  }, 2000);
}

// Bind config events
apiUrlInput.addEventListener("change", saveConfig);
modelInput.addEventListener("change", saveConfig);
cleanupModelInput.addEventListener("change", saveConfig);
apiKeyInput.addEventListener("input", saveConfig);
punctuationToggle.addEventListener("change", saveConfig);
fillersToggle.addEventListener("change", saveConfig);
autopasteToggle.addEventListener("change", saveConfig);
startupToggle.addEventListener("change", saveConfig);

// Initialize
loadConfig();
