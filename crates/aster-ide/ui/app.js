// Aster IDE — Core Application Architecture
const invoke = window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke || (async (cmd, args) => {
  console.warn(`[Tauri Mock] Invoke '${cmd}'`, args);
  return null;
});

// App State
const state = {
  mode: "workspace", // 'home' | 'workspace'
  workspace: null, // { name, root }
  files: new Map(), // path -> { path, contents }
  tree: null, // FileTreeNode
  openTabs: [], // array of file paths
  active: null, // active file path
  dirty: new Set(), // paths with unsaved changes
  diagnostics: [], // array of DiagnosticDto
  classes: [], // array of ClassStructureDto
  moduleGraph: null, // { modules, edges, entry_points, cycles }
  recentProjects: [],
  activeSidebarView: "explorer", // 'explorer' | 'search' | 'git' | 'graph'
  activeDrawerTab: "problems", // 'problems' | 'suggestions' | 'output' | 'repl'
  drawerCollapsed: true,
  drawerExpanded: false,
  sidebarCollapsed: false,
  inspectorCollapsed: true,
  isRunning: false,
  stopRequested: false,
  runtimes: [],
  findMatches: [],
  findIndex: -1,
  selectedMember: null,
  expandedFolders: new Set([""]),
  contextTarget: null,
};

let explainTimer = null;
let inspectorRequestId = 0;
let pendingInspectorTarget = null;
let lastInspectorHoverKey = null;
let analyzeTimer = null;
let statusIdleTimer = null;

// DOM Elements
const appEl = document.querySelector("#app");
const editorEl = document.querySelector("#editor");
const editorView = new AsterEditor.EditorView(editorEl);
const numbersEl = document.querySelector("#line-numbers");
const markersEl = document.querySelector("#gutter-markers");
const syntaxLayerEl = document.querySelector("#syntax-layer");
const tabsContainerEl = document.querySelector("#tabs");
const fileTreeEl = document.querySelector("#file-tree");
const treeRootHeaderEl = document.querySelector("#tree-root-header");
const treeRootNameEl = document.querySelector("#tree-root-name");
const recentsListEl = document.querySelector("#recents-list");
const recentsEmptyEl = document.querySelector("#recents-empty");
const recentsFilterInput = document.querySelector("#recents-filter-input");
const homeRuntimeSelect = document.querySelector("#home-runtime-select");
const settingsRuntimeSelect = document.querySelector("#settings-runtime-select");
const statusRuntimeLabel = document.querySelector("#status-runtime-label");
const homeRuntimeNote = document.querySelector("#home-runtime-note");
const settingsRuntimeNote = document.querySelector("#settings-runtime-note");
const runButton = document.querySelector("#top-run-btn");
const toastRegion = document.querySelector("#toast-region");

// Breadcrumbs & Status
const bcRootEl = document.querySelector("#bc-root");
const bcFileEl = document.querySelector("#bc-file");
const bcSymbolEl = document.querySelector("#bc-symbol");
const bcSymbolSepEl = document.querySelector("#bc-symbol-sep");
const statusCursorEl = document.querySelector("#status-cursor");
const statusSelectionEl = document.querySelector("#status-selection-info");
const statusBranchEl = document.querySelector("#status-git-branch");
const statusSyncEl = document.querySelector("#status-analysis-sync");
const statusSyncTextEl = document.querySelector("#status-sync-text");
const statusErrorCountEl = document.querySelector("#status-error-count");
const statusWarnCountEl = document.querySelector("#status-warn-count");

// Inspector elements
const traceLineTagEl = document.querySelector("#trace-line-tag");
const classHierarchyTreeEl = document.querySelector("#class-hierarchy-tree");
const classesCountBadgeEl = document.querySelector("#classes-count-badge");
const detailNameEl = document.querySelector("#detail-name");
const detailKindEl = document.querySelector("#detail-kind");
const detailOwnerEl = document.querySelector("#detail-owner");
const detailLocationEl = document.querySelector("#detail-location");
const detailInheritanceEl = document.querySelector("#detail-inheritance");

// Drawer elements
const bottomDrawerEl = document.querySelector("#bottom-drawer");
const drawerProblemCountEl = document.querySelector("#drawer-problem-count");
const drawerSuggestionCountEl = document.querySelector("#drawer-suggestion-count");
const problemListRowsEl = document.querySelector("#problem-list-rows");
const suggestionListRowsEl = document.querySelector("#suggestion-list-rows");
const outputLogEl = document.querySelector("#output-log");
const replOutputEl = document.querySelector("#repl-output");
const replInputEl = document.querySelector("#repl-input");

// Modals
const commandModalOverlay = document.querySelector("#command-modal-overlay");
const paletteInput = document.querySelector("#palette-input");
const paletteResults = document.querySelector("#palette-results");
const promptModalOverlay = document.querySelector("#prompt-modal-overlay");
const settingsModalOverlay = document.querySelector("#settings-modal-overlay");
const promptTitleEl = document.querySelector("#prompt-title");
const promptInputEl = document.querySelector("#prompt-input");
const promptOkBtn = document.querySelector("#prompt-ok-btn");
const promptCancelBtn = document.querySelector("#prompt-cancel-btn");
const contextMenuEl = document.querySelector("#context-menu");
const editorFindEl = document.querySelector("#editor-find");
const editorFindInput = document.querySelector("#editor-find-input");
const editorFindCount = document.querySelector("#editor-find-count");
const editorReplaceRow = document.querySelector("#editor-replace-row");
const editorReplaceInput = document.querySelector("#editor-replace-input");

function showToast(message, kind = "info", timeout = 4200) {
  const toast = document.createElement("div");
  toast.className = `toast ${kind}`;
  toast.textContent = String(message);
  toastRegion.append(toast);
  window.setTimeout(() => toast.remove(), timeout);
}

// Native window controls for the frameless Tauri shell.
document.querySelector("#window-minimize").addEventListener("click", () => {
  invoke("window_minimize").catch(console.error);
});
document.querySelector("#window-maximize").addEventListener("click", () => {
  invoke("window_toggle_maximize").catch(console.error);
});
let nativeCloseArmed = false;

async function requestWindowClose() {
  try {
    await stopLuaIfRunning();
    await saveAllDirty();
    nativeCloseArmed = true;
    await invoke("window_close");
  } catch (error) {
    nativeCloseArmed = false;
    showToast(`Aster could not close safely: ${error}`, "error", 6500);
  }
}

document.querySelector("#window-close").addEventListener("click", requestWindowClose);

const nativeWindow = window.__TAURI__?.window?.getCurrentWindow?.();
if (nativeWindow?.onCloseRequested) {
  nativeWindow.onCloseRequested(async (event) => {
    if (nativeCloseArmed) return;
    event.preventDefault();
    await requestWindowClose();
  }).catch(console.error);
}

const titlebarEl = document.querySelector("#titlebar");
titlebarEl.addEventListener("mousedown", (event) => {
  if (event.button !== 0 || event.target.closest("button, input, a, textarea")) return;
  invoke("window_start_dragging").catch(console.error);
});
titlebarEl.addEventListener("dblclick", (event) => {
  if (event.target.closest("button, input, a, textarea")) return;
  invoke("window_toggle_maximize").catch(console.error);
});

// ============================================================================
// State Machine & View Switching
// ============================================================================
function setAppMode(mode) {
  state.mode = mode;
  appEl.className = mode === "home" ? "state-home" : "state-workspace";
  if (mode === "home") {
    loadRecentProjects().catch(console.error);
  }
}

// Format relative timestamps
function formatTimeAgo(timestampSec) {
  if (!timestampSec) return "Recently";
  const diffSec = Math.floor(Date.now() / 1000) - timestampSec;
  if (diffSec < 60) return "Just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} mins ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} hours ago`;
  if (diffSec < 172800) return "Yesterday";
  return `${Math.floor(diffSec / 86400)} days ago`;
}

function displayPath(path) {
  if (!path) return "";
  return path.replace(/^(?:\\\\\?\\|\/\/\?\/)/, "");
}

function fileIconMarkup(path) {
  return String(path).toLowerCase().endsWith(".lua")
    ? '<img class="lua-file-icon" src="./assets/lua-logo.svg" alt="" />'
    : '<span class="generic-file-icon" aria-hidden="true"></span>';
}

// ============================================================================
// Home Screen / Recent Projects
// ============================================================================
async function loadRecentProjects() {
  try {
    const recents = await invoke("get_recent_projects");
    state.recentProjects = recents || [];
    renderRecentProjects();
  } catch (err) {
    showToast(`Could not load recent projects: ${err}`, "error");
  }
}

function renderRecentProjects(filterText = "") {
  recentsListEl.innerHTML = "";
  recentsFilterInput.closest(".recents-filter-wrap").hidden = state.recentProjects.length < 5;
  const filter = filterText.toLowerCase().trim();
  const filtered = state.recentProjects.filter((project) => {
    return (
      project.name.toLowerCase().includes(filter) ||
      project.path.toLowerCase().includes(filter)
    );
  });

  if (!filtered.length) {
    recentsEmptyEl.hidden = false;
    return;
  }
  recentsEmptyEl.hidden = true;

  // Sort pinned to top, then recent
  filtered.sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return b.last_opened - a.last_opened;
  });

  filtered.forEach((project) => {
    const row = document.createElement("div");
    row.className = `recent-project-row ${project.exists ? "" : "missing"}`;
    row.innerHTML = `
      <div class="recent-left">
        <svg class="recent-folder-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor">
          <path d="M1.75 2.5A1.75 1.75 0 0 0 0 4.25v7.5C0 12.716.784 13.5 1.75 13.5h12.5A1.75 1.75 0 0 0 16 11.75v-6A1.75 1.75 0 0 0 14.25 4.25H7.707l-1.353-1.354A1.75 1.75 0 0 0 5.116 2.5H1.75z"/>
        </svg>
        <div class="recent-info">
          <div class="recent-name-wrap">
            <span class="recent-name">${escapeHtml(project.name)}</span>
            ${project.pinned ? '<span class="recent-pin-badge">Pinned</span>' : ""}
          </div>
          <div class="recent-path-meta">
            <span title="${escapeHtml(displayPath(project.path))}">${escapeHtml(displayPath(project.path))}</span>
            ${project.exists ? "" : '<span class="recent-branch-tag">Folder missing</span>'}
          </div>
        </div>
      </div>
      <div class="recent-right">
        <span class="recent-time">${formatTimeAgo(project.last_opened)}</span>
        <div class="recent-actions">
          <button class="recent-action-btn pin-btn ${project.pinned ? "is-pinned" : ""}" title="${project.pinned ? "Unpin" : "Pin to top"}" aria-label="${project.pinned ? "Unpin" : "Pin to top"}"><span class="css-icon pin-icon"></span></button>
          <button class="recent-action-btn remove-btn" title="Remove from Recents" aria-label="Remove from Recents"><span class="css-icon close-icon"></span></button>
        </div>
      </div>
    `;

    row.addEventListener("click", (e) => {
      if (e.target.closest(".pin-btn") || e.target.closest(".remove-btn")) return;
      if (!project.exists) {
        showToast("That folder has moved or was deleted. Remove it from Recents or open its new location.", "error");
        return;
      }
      openWorkspace(project.path).catch((error) => showToast(error, "error"));
    });

    row.querySelector(".pin-btn").addEventListener("click", async (e) => {
      e.stopPropagation();
      await invoke("pin_recent_project", { path: project.path, pinned: !project.pinned });
      await loadRecentProjects();
    });

    row.querySelector(".remove-btn").addEventListener("click", async (e) => {
      e.stopPropagation();
      await invoke("remove_recent_project", { path: project.path });
      await loadRecentProjects();
    });

    recentsListEl.append(row);
  });
}

recentsFilterInput.addEventListener("input", () => {
  renderRecentProjects(recentsFilterInput.value);
});

const runtimeLabels = {
  lua54: "Lua 5.4",
  luajit21: "LuaJIT 2.1",
  lua53: "Lua 5.3",
  lua51: "Lua 5.1",
};

const hadStoredRuntimePreference = Boolean(localStorage.getItem("aster_lua_runtime"));

async function setSelectedRuntime(runtime, { persist = true } = {}) {
  const selected = runtimeLabels[runtime] ? runtime : "lua54";
  localStorage.setItem("aster_lua_runtime", selected);
  homeRuntimeSelect.value = selected;
  settingsRuntimeSelect.value = selected;
  statusRuntimeLabel.textContent = runtimeLabels[selected];
  updateRuntimeNote();
  if (persist && state.workspace) {
    try {
      await invoke("set_project_runtime", { request: { runtime: selected } });
    } catch (error) {
      showToast(`Could not save the project runtime: ${error}`, "error");
    }
  }
}

function updateRuntimeNote() {
  const selected = homeRuntimeSelect.value;
  const runtime = state.runtimes.find((item) => item.id === selected);
  const rawVersion = runtime?.version || runtime?.executable || "";
  const parsedVersion = rawVersion.match(/LuaJIT\s+[^\s]+|Lua\s+\d+(?:\.\d+)+/)?.[0];
  const vendor = /PUC-Rio/i.test(rawVersion) ? "PUC-Rio" : /LuaJIT/i.test(rawVersion) ? "LuaJIT" : "";
  const note = !runtime
    ? "Checking installation..."
    : runtime.available
      ? `${parsedVersion || runtimeLabels[selected]}${vendor ? ` · ${vendor}` : ""}`
      : "Not installed or not available on PATH";
  homeRuntimeNote.textContent = note;
  settingsRuntimeNote.textContent = state.workspace
    ? `${note}. Used to run files in this project.`
    : `${note}. Used for new projects.`;
  const runtimeDetail = runtime?.available
    ? [runtime.version, runtime.executable].filter(Boolean).join("\n")
    : note;
  homeRuntimeNote.title = runtimeDetail;
  settingsRuntimeNote.title = runtimeDetail;
  statusRuntimeLabel.title = runtimeDetail;
}

async function refreshRuntimeAvailability() {
  try {
    state.runtimes = (await invoke("detect_lua_runtimes")) || [];
    if (!hadStoredRuntimePreference) {
      const available = state.runtimes.find((runtime) => runtime.available);
      if (available) await setSelectedRuntime(available.id, { persist: false });
    }
    updateRuntimeNote();
  } catch (error) {
    state.runtimes = [];
    homeRuntimeNote.textContent = "Runtime detection unavailable";
    settingsRuntimeNote.textContent = "Runtime detection unavailable.";
  }
}

homeRuntimeSelect.addEventListener("change", () => setSelectedRuntime(homeRuntimeSelect.value));
settingsRuntimeSelect.addEventListener("change", () => setSelectedRuntime(settingsRuntimeSelect.value));
setSelectedRuntime(localStorage.getItem("aster_lua_runtime") || "lua54", { persist: false });

// ============================================================================
// Workspace Loading & Initialization
// ============================================================================
async function openWorkspace(folderPath = null) {
  try {
    const info = await invoke("open_workspace", { folderPath });
    if (!info) return;
    setWorkspaceInfo(info);
    setAppMode("workspace");
    await loadWorkspace();
  } catch (err) {
    showToast(`Could not open workspace: ${err}`, "error", 6500);
  }
}

async function closeWorkspace() {
  await stopLuaIfRunning();
  await saveAllDirty();
  await invoke("close_workspace");
  state.workspace = null;
  state.files.clear();
  state.openTabs = [];
  state.active = null;
  setAppMode("home");
}

function setWorkspaceInfo(info) {
  state.workspace = info;
  document.querySelector("#top-workspace-name").textContent = info.name;
  treeRootNameEl.textContent = info.name;
  bcRootEl.textContent = info.name;
}

async function loadWorkspace() {
  try {
    const projectRuntime = await invoke("get_project_runtime").catch(() => null);
    if (projectRuntime) {
      await setSelectedRuntime(projectRuntime, { persist: false });
    }
    const files = await invoke("workspace_files");
    state.files.clear();
    state.dirty.clear();
    (files || []).forEach((file) => {
      state.files.set(file.path, {
        ...file,
        model: new AsterEditor.TextModel(file.contents, file.path),
      });
    });

    await refreshWorkspaceTree();
    await refreshClassStructures();
    await refreshModuleGraph();

    // Select main.lua or first lua file
    const mainFile = files.find((f) => f.path === "main.lua") || files[0];
    if (mainFile) {
      openFile(mainFile.path);
    } else {
      clearEditor();
    }

    await analyzeWorkspace();
  } catch (err) {
    showToast(`Could not load this workspace: ${err}`, "error", 6500);
  }
}

async function refreshWorkspaceTree() {
  try {
    state.tree = await invoke("workspace_tree");
    renderFileTree();
  } catch (_) {
    // Fallback: build simple tree from flat file list
    renderFlatFileTree();
  }
}

// ============================================================================
// Hierarchical Project Explorer
// ============================================================================
function renderFileTree() {
  fileTreeEl.innerHTML = "";
  if (!state.tree || !state.tree.children) {
    renderFlatFileTree();
    return;
  }
  renderTreeChildren(state.tree.children, fileTreeEl, 1);
}

function ensureParentFoldersExpanded(filePath) {
  if (!filePath) return;
  const parts = filePath.replace(/\\/g, "/").split("/");
  let current = "";
  for (let i = 0; i < parts.length - 1; i++) {
    current = current ? `${current}/${parts[i]}` : parts[i];
    state.expandedFolders.add(current);
  }
}

function renderTreeChildren(children, container, depth) {
  children.forEach((node) => {
    const isExpanded = state.expandedFolders.has(node.path);
    const row = document.createElement("div");
    row.className = `tree-node-row ${node.path === state.active ? "selected" : ""}`;
    row.style.paddingLeft = `${depth * 14}px`;
    row.dataset.path = node.path;

    const diagCount = countFileDiagnostics(node.path);

    if (node.is_dir) {
      row.innerHTML = `
        <span class="chevron ${isExpanded ? "down" : ""}">›</span>
        <span class="tree-icon folder-icon">
          <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor">
            <path d="M1.75 2.5A1.75 1.75 0 0 0 0 4.25v7.5C0 12.716.784 13.5 1.75 13.5h12.5A1.75 1.75 0 0 0 16 11.75v-6A1.75 1.75 0 0 0 14.25 4.25H7.707l-1.353-1.354A1.75 1.75 0 0 0 5.116 2.5H1.75z"/>
          </svg>
        </span>
        <span class="tree-name">${escapeHtml(node.name)}</span>
      `;
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        state.contextTarget = node;
        if (state.expandedFolders.has(node.path)) {
          state.expandedFolders.delete(node.path);
        } else {
          state.expandedFolders.add(node.path);
        }
        renderFileTree();
      });
    } else {
      row.innerHTML = `
        <span class="tree-indent-spacer"></span>
        ${fileIconMarkup(node.name)}
        <span class="tree-name">${escapeHtml(node.name)}</span>
        ${diagCount > 0 ? `<span class="tree-diag-badge" title="${diagCount} issues"></span>` : ""}
      `;
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        state.contextTarget = node;
        openFile(node.path);
      });
    }

    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      e.stopPropagation();
      state.contextTarget = node;
      showContextMenu(e.clientX, e.clientY, node);
    });

    container.append(row);

    if (node.is_dir && isExpanded && node.children && node.children.length > 0) {
      renderTreeChildren(node.children, container, depth + 1);
    }
  });
}

function renderFlatFileTree() {
  fileTreeEl.innerHTML = "";
  state.files.forEach((_, path) => {
    const row = document.createElement("div");
    row.className = `tree-node-row ${path === state.active ? "selected" : ""}`;
    row.style.paddingLeft = "14px";
    row.dataset.path = path;
    const diagCount = countFileDiagnostics(path);
    row.innerHTML = `
      ${fileIconMarkup(path)}
      <span class="tree-name">${escapeHtml(path)}</span>
      ${diagCount > 0 ? `<span class="tree-diag-badge"></span>` : ""}
    `;
    row.addEventListener("click", (e) => {
      e.stopPropagation();
      state.contextTarget = { path, is_dir: false };
      openFile(path);
    });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      e.stopPropagation();
      state.contextTarget = { path, is_dir: false };
      showContextMenu(e.clientX, e.clientY, { path, is_dir: false });
    });
    fileTreeEl.append(row);
  });
}

function countFileDiagnostics(path) {
  return state.diagnostics.filter((d) => d.file === path && !d.suggestion).length;
}

// ============================================================================
// Editor & Tabs Management
// ============================================================================
async function openFile(path) {
  if (!state.files.has(path)) {
    try {
      const contents = await invoke("read_file", { request: { path } });
      state.files.set(path, {
        path,
        contents,
        model: new AsterEditor.TextModel(contents, path),
      });
    } catch (error) {
      showToast(`Could not open ${path}: ${error}`, "error", 6500);
      return;
    }
  }
  ensureParentFoldersExpanded(path);
  if (!state.openTabs.includes(path)) {
    state.openTabs.push(path);
  }
  selectTab(path);
}

function selectTab(path) {
  if (state.active && state.files.has(state.active)) {
    state.files.get(state.active).contents = editorView.getText();
  }
  state.active = path;
  const file = state.files.get(path);
  if (file && !file.model) file.model = new AsterEditor.TextModel(file.contents, file.path);
  editorView.setModel(file?.model || null);
  bcFileEl.textContent = path;

  renderTabs();
  renderFileTree();
  updateEditorView();
  updateInspector();
  editorView.focus();
}

async function closeTab(path, event) {
  if (event) event.stopPropagation();
  const idx = state.openTabs.indexOf(path);
  if (idx === -1) return;
  if (state.dirty.has(path)) {
    const file = state.files.get(path);
    if (path === state.active) file.contents = editorView.getText();
    try {
      await invoke("save_file", { request: { path, contents: file.contents } });
      state.dirty.delete(path);
    } catch (error) {
      showToast(`Could not close ${path} because it was not saved: ${error}`, "error", 6500);
      return;
    }
  }
  state.openTabs.splice(idx, 1);

  if (state.active === path) {
    if (state.openTabs.length > 0) {
      const nextIdx = Math.max(0, idx - 1);
      selectTab(state.openTabs[nextIdx]);
    } else {
      clearEditor();
    }
  } else {
    renderTabs();
  }
}

function clearEditor() {
  state.active = null;
  editorView.setModel(null);
  bcFileEl.textContent = "No file open";
  syntaxLayerEl.innerHTML = "";
  numbersEl.textContent = "";
  markersEl.innerHTML = "";
  renderTabs();
  renderFileTree();
  clearInspector();
}

function renderTabs() {
  tabsContainerEl.innerHTML = "";
  state.openTabs.forEach((path) => {
    const tab = document.createElement("div");
    tab.className = `tab-item ${path === state.active ? "active" : ""}`;
    const isDirty = state.dirty.has(path);
    const fileName = path.split("/").pop();

    tab.innerHTML = `
      ${fileIconMarkup(path)}
      <span class="tab-title">${escapeHtml(fileName)}</span>
      ${isDirty ? '<span class="tab-dirty-dot"></span>' : ""}
      <button class="tab-close-btn" title="Close Tab" aria-label="Close Tab"><span class="css-icon close-icon"></span></button>
    `;

    tab.addEventListener("click", () => selectTab(path));
    tab.querySelector(".tab-close-btn").addEventListener("click", (e) => closeTab(path, e));
    tabsContainerEl.append(tab);
  });
}

// ============================================================================
// Lua Syntax Highlighting & Line Numbers
// ============================================================================
function updateEditorView() {
  updateLineNumbers();
  highlightLuaSyntax();
  updateCursorStatus();
}

function updateLineNumbers() {
  const lineCount = editorView.model?.getLineCount() || 1;
  numbersEl.textContent = Array.from({ length: lineCount }, (_, i) => i + 1).join("\n");
  updateGutterMarkers();
}

function updateGutterMarkers() {
  markersEl.innerHTML = "";
  if (!state.active) return;
  const activeDiags = state.diagnostics.filter((d) => d.file === state.active && d.line);
  const lineDiags = new Map();
  activeDiags.forEach((d) => {
    const current = lineDiags.get(d.line);
    if (!current || (current.suggestion && !d.suggestion)) lineDiags.set(d.line, d);
  });

  const lineCount = editorView.model?.getLineCount() || 0;
  for (let idx = 0; idx < lineCount; idx++) {
    const lineNum = idx + 1;
    if (lineDiags.has(lineNum)) {
      const diag = lineDiags.get(lineNum);
      const dot = document.createElement("div");
      dot.className = `gutter-marker ${diag.suggestion ? "suggestion" : diag.kind.includes("warning") ? "warn" : "error"}`;
      dot.title = `Line ${lineNum}: ${diag.message}`;
      markersEl.append(dot);
    } else {
      const empty = document.createElement("div");
      empty.style.height = "20px";
      markersEl.append(empty);
    }
  }
}

function updateCursorStatus() {
  const selection = editorView.getSelection();
  const position = editorView.getCursorPosition();
  statusCursorEl.textContent = `Ln ${position.lineNumber}, Col ${position.column}`;

  const selLength = Math.abs(selection.end - selection.start);
  if (selLength > 0) {
    statusSelectionEl.hidden = false;
    statusSelectionEl.textContent = `${selLength} selected`;
  } else {
    statusSelectionEl.hidden = true;
  }
}

// Fast regex-based Lua syntax highlighter
function highlightLuaSyntax() {
  const code = editorView.getText();
  if (!code) {
    syntaxLayerEl.innerHTML = "";
    return;
  }

  // Escape HTML helper
  function escape(str) {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  // Tokenize regex patterns (strictly single-line for "" and '', multiline for [[ ]])
  const tokenRegex = /(--\[\[[\s\S]*?\]\]|--[^\r\n]*)|("(?:[^"\\\r\n]|\\.)*"|'(?:[^'\\\r\n]|\\.)*'|\[\[[\s\S]*?\]\])|\b(local|function|return|if|then|else|elseif|end|for|in|while|do|repeat|until|not|and|or|nil|true|false|goto|break)\b|\b(__index|__newindex|__tostring|__call|__add|__sub|__mul|__div|__mod|__pow|__concat|__eq|__lt|__le|__mode|__gc|__metatable)\b|\b(require|setmetatable|getmetatable|type|tostring|tonumber|print|pairs|ipairs|pcall|xpcall|table|string|math|coroutine|os|io|debug|package|select|rawget|rawset|rawequal|next|collectgarbage)\b|\b(self)\b|\b(\d+\.?\d*(?:[eE][+-]?\d+)?|0x[0-9a-fA-F]+)\b|([a-zA-Z_]\w*(?=\s*\())|([a-zA-Z_]\w*)|([^\s\w]+)/g;

  let html = "";
  let lastIndex = 0;
  let match;

  while ((match = tokenRegex.exec(code)) !== null) {
    // Crucial: preserve any spaces, newlines, and indentation between tokens!
    if (match.index > lastIndex) {
      html += escape(code.slice(lastIndex, match.index));
    }

    const text = match[0];
    const [
      ,
      comment,
      string,
      keyword,
      metamethod,
      builtin,
      selfWord,
      number,
      fnCall,
      identifier,
      operator,
    ] = match;

    if (comment) {
      html += `<span class="syn-comment">${escape(text)}</span>`;
    } else if (string) {
      html += `<span class="syn-str">${escape(text)}</span>`;
    } else if (keyword) {
      html += `<span class="syn-kw">${escape(text)}</span>`;
    } else if (metamethod) {
      html += `<span class="syn-meta">${escape(text)}</span>`;
    } else if (builtin) {
      html += `<span class="syn-builtin">${escape(text)}</span>`;
    } else if (selfWord) {
      html += `<span class="syn-self">${escape(text)}</span>`;
    } else if (number) {
      html += `<span class="syn-num">${escape(text)}</span>`;
    } else if (fnCall) {
      html += `<span class="syn-fn">${escape(text)}</span>`;
    } else if (identifier) {
      html += `<span class="syn-id">${escape(text)}</span>`;
    } else if (operator) {
      html += `<span class="syn-op">${escape(text)}</span>`;
    } else {
      html += escape(text);
    }
    lastIndex = tokenRegex.lastIndex;
  }

  // Append any trailing whitespace or code
  if (lastIndex < code.length) {
    html += escape(code.slice(lastIndex));
  }

  // Trailing newline in pre block needs a phantom space so cursor on blank line lines up
  if (code.endsWith("\n")) {
    html += " ";
  }

  syntaxLayerEl.innerHTML = html;
}

// Synchronized Editor Scrolling
editorEl.addEventListener("scroll", () => {
  numbersEl.scrollTop = editorEl.scrollTop;
  markersEl.scrollTop = editorEl.scrollTop;
  syntaxLayerEl.scrollTop = editorEl.scrollTop;
  syntaxLayerEl.scrollLeft = editorEl.scrollLeft;
});

// Editor Input & Auto-Save Debounce
editorEl.addEventListener("input", () => {
  if (!state.active) return;
  editorView.syncFromDom();
  state.files.get(state.active).contents = editorView.getText();
  state.dirty.add(state.active);
  updateEditorView();
  renderTabs();

  clearTimeout(analyzeTimer);
  analyzeTimer = setTimeout(async () => {
    try {
      await saveActive();
      await analyzeWorkspace();
      if (!state.inspectorCollapsed) {
        await refreshClassStructures();
        await updateInspector();
      }
    } catch (error) {
      showToast(`Could not save ${state.active}: ${error}`, "error", 6500);
    }
  }, 700);
});

// Cursor Tracking & Inspector Trigger
editorEl.addEventListener("keyup", () => {
  updateCursorStatus();
  triggerInspectorUpdate();
});
editorEl.addEventListener("click", () => {
  updateCursorStatus();
  triggerInspectorUpdate();
});

// Smart Tab & Indentation
editorEl.addEventListener("keydown", (e) => {
  const isUndo = (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z" && !e.shiftKey;
  const isRedo =
    (e.ctrlKey || e.metaKey) &&
    (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey));
  if (isUndo || isRedo) {
    e.preventDefault();
    const selection = isUndo ? editorView.model?.undo() : editorView.model?.redo();
    if (selection) {
      editorView.setSelection(selection.start, selection.end);
      editorEl.dispatchEvent(new Event("input"));
    }
    return;
  }

  if (e.key === "Tab") {
    e.preventDefault();
    const { start, end } = editorView.getSelection();
    if (e.shiftKey) {
      // Unindent 4 spaces if possible
      const before = editorView.getText().slice(0, start);
      if (before.endsWith("    ")) {
        editorView.model.applyEdit(
          { start: start - 4, end: start, text: "" },
          {
            source: "indent",
            selectionBefore: { start, end },
            selectionAfter: { start: start - 4, end: end - 4 },
          },
        );
        editorView.setSelection(start - 4, end - 4);
      }
    } else {
      // Indent 4 spaces
      editorView.replaceSelection("    ", "indent");
    }
    editorEl.dispatchEvent(new Event("input"));
  }
});

async function saveActive() {
  if (!state.active || !state.dirty.has(state.active)) return;
  const content = editorView.getText();
  state.files.get(state.active).contents = content;
  await invoke("save_file", { request: { path: state.active, contents: content } });
  state.dirty.delete(state.active);
  renderTabs();
}

async function saveAllDirty() {
  if (state.active && state.files.has(state.active)) {
    state.files.get(state.active).contents = editorView.getText();
  }
  for (const path of [...state.dirty]) {
    const file = state.files.get(path);
    if (!file) continue;
    await invoke("save_file", { request: { path, contents: file.contents } });
    state.dirty.delete(path);
  }
  renderTabs();
}

// ============================================================================
// Signature Lua Metatable Inspector
// ============================================================================
const inspectorEmptyStateEl = document.querySelector("#inspector-empty-state");
const inspectorPopulatedEl = document.querySelector("#inspector-populated-content");
const chainFlowEl = document.querySelector("#chain-flow");
const chainStatusEl = document.querySelector("#chain-status");
const traceStepsListEl = document.querySelector("#trace-steps-list");

function triggerInspectorUpdate(target = null) {
  pendingInspectorTarget = target;
  clearTimeout(explainTimer);
  explainTimer = setTimeout(() => {
    const requestedTarget = pendingInspectorTarget;
    pendingInspectorTarget = null;
    updateInspector(requestedTarget);
  }, 80);
}

function getCursorLine() {
  return editorView.getCursorPosition().lineNumber;
}

function getCursorColumn() {
  return editorView.getCursorPosition().column;
}

function editorPositionFromPoint(event) {
  const position = editorView.positionFromPoint(event);
  return position ? { line: position.lineNumber, column: position.column } : null;
}

editorEl.addEventListener("mousemove", (event) => {
  const target = editorPositionFromPoint(event);
  const key = target ? `${target.line}:${target.column}` : "none";
  if (key === lastInspectorHoverKey) return;
  lastInspectorHoverKey = key;
  triggerInspectorUpdate(target || { line: 0, column: 0 });
});

editorEl.addEventListener("mouseleave", () => {
  lastInspectorHoverKey = null;
  triggerInspectorUpdate();
});

async function updateInspector(target = null) {
  if (!state.active || state.mode !== "workspace") return;
  const requestId = ++inspectorRequestId;
  const activePath = state.active;
  const line = target?.line ?? getCursorLine();
  const column = target?.column ?? getCursorColumn();
  traceLineTagEl.textContent = `Line ${line}`;

  // If buffer has unsaved changes, save immediately so backend analyzes fresh state
  if (state.dirty.has(state.active)) {
    await saveActive();
  }

  try {
    const explanations = await invoke("explain_at_cursor", {
      request: { path: activePath, line, column },
    });

    if (requestId !== inspectorRequestId || state.active !== activePath) return;

    if (!explanations || explanations.length === 0) {
      clearInspector();
      return;
    }

    renderPopulatedInspector(explanations[0]);
  } catch (_) {
    if (requestId !== inspectorRequestId) return;
    clearInspector();
  }
}

function clearInspector() {
  inspectorEmptyStateEl.hidden = false;
  inspectorPopulatedEl.hidden = true;
}

function renderPopulatedInspector(explanation) {
  inspectorEmptyStateEl.hidden = true;
  inspectorPopulatedEl.hidden = false;

  const isResolved = explanation.result.startsWith("Resolved");
  const isUnknown = explanation.result.startsWith("Unknown");
  chainStatusEl.textContent = isResolved ? "Resolved" : isUnknown ? "Unknown" : "Not Found";
  chainStatusEl.className = `chain-status ${isResolved ? "resolved" : "unknown"}`;

  // Build the clean horizontal resolution flow:
  // player → metatable → Player → __index → Entity → level()
  chainFlowEl.innerHTML = "";
  const steps = explanation.steps || [];

  const flowNodes = [];
  const exprParts = explanation.expression.split(".");
  const instanceName = exprParts[0] || "instance";
  const memberName = exprParts.slice(1).join(".") || "member";

  flowNodes.push({ name: instanceName, kind: "instance" });

  steps.forEach((step) => {
    if (step.includes("constructor parent:")) {
      const match = step.match(/constructor parent:\s*([^\s(]+)/);
      if (match) flowNodes.push({ name: match[1], kind: "class" });
    } else if (step.includes("metatable __index:")) {
      const match = step.match(/metatable __index:\s*([^\s(]+)/);
      if (match) flowNodes.push({ name: match[1], kind: "metatable" });
    } else if (step.includes("found direct member on")) {
      const match = step.match(/found direct member on\s*([^\s]+)/);
      if (match) flowNodes.push({ name: match[1].replace(/\.lua$/, ""), kind: "class" });
    }
  });

  flowNodes.push({ name: memberName + "()", kind: "resolved" });

  flowNodes.forEach((node, idx) => {
    const chip = document.createElement("span");
    chip.className = "chain-node";
    chip.textContent = node.name;
    chainFlowEl.append(chip);

    if (idx < flowNodes.length - 1) {
      const arrow = document.createElement("span");
      arrow.className = "chain-arrow";
      arrow.textContent = "→";
      chainFlowEl.append(arrow);
    }
  });

  // Render stepped walk
  traceStepsListEl.innerHTML = "";
  steps.forEach((step, idx) => {
    const row = document.createElement("div");
    row.className = "trace-step-row";

    let tagClass = "tag-instance";
    let tagText = "Instance";

    if (step.includes("raw instance")) {
      tagClass = "tag-instance";
      tagText = "Instance";
    } else if (step.includes("constructor parent")) {
      tagClass = "tag-metatable";
      tagText = "Parent";
    } else if (step.includes("__index")) {
      tagClass = "tag-index";
      tagText = "__index";
    } else if (step.includes("raw table lookup")) {
      tagClass = "tag-direct";
      tagText = "Table";
    } else if (step.includes("found direct member")) {
      tagClass = "tag-direct";
      tagText = "Direct";
    }

    row.innerHTML = `
      <span class="step-tag ${tagClass}">${tagText}</span>
      <span class="step-text">${escapeHtml(step)}</span>
    `;

    traceStepsListEl.append(row);
  });

  // Final result step
  const finalRow = document.createElement("div");
  finalRow.className = "trace-step-row";
  finalRow.innerHTML = `
    <span class="step-tag tag-resolved">Result</span>
    <span class="step-text">${escapeHtml(explanation.result)}</span>
  `;
  traceStepsListEl.append(finalRow);

  // Auto-populate member details from resolution result
  if (isResolved) {
    const resText = explanation.result.replace(/^Resolved:\s*/, "");
    const parts = resText.split(" in ");
    const memberName = parts[0] || explanation.expression;
    const fileLoc = parts[1] || "";
    showMemberDetails({
      name: memberName,
      kind: memberName.includes("(") ? "Method" : "Field",
      owner: fileLoc.replace(/\.lua.*$/, ""),
      file: fileLoc,
      line: 1,
      resolution: explanation.steps.join(" → "),
    });
  }
}

// Refresh project classes structure
async function refreshClassStructures() {
  try {
    const classes = await invoke("inspect_classes");
    state.classes = classes || [];
    renderClassHierarchy();
  } catch (err) {
    console.error("Error inspecting classes:", err);
  }
}

function renderClassHierarchy() {
  classHierarchyTreeEl.innerHTML = "";
  classesCountBadgeEl.textContent = `${state.classes.length} classes`;

  if (!state.classes.length) {
    classHierarchyTreeEl.innerHTML = '<div class="trace-empty">No Lua classes detected.</div>';
    return;
  }

  state.classes.forEach((cls) => {
    const node = document.createElement("div");
    node.className = "class-node";

    let membersHtml = "";

    // Metatable link
    if (cls.metatable_name) {
      membersHtml += `
        <div class="member-item-row" data-kind="Metatable" data-name="${escapeHtml(cls.metatable_name)}" data-file="${escapeHtml(cls.file)}">
          <div class="member-item-left">
            <span class="member-kind-tag meta">meta</span>
            <span class="member-item-name">__index</span>
          </div>
          <span class="member-item-loc">→ ${escapeHtml(cls.metatable_name)}</span>
        </div>
      `;
    }

    // Parent Constructor
    if (cls.parent_constructor) {
      membersHtml += `
        <div class="member-item-row" data-kind="Constructor" data-name="${escapeHtml(cls.parent_constructor)}.new" data-file="${escapeHtml(cls.file)}">
          <div class="member-item-left">
            <span class="member-kind-tag ctor">parent</span>
            <span class="member-item-name">${escapeHtml(cls.parent_constructor)}.new()</span>
          </div>
        </div>
      `;
    }

    // Instance Fields
    (cls.instance_members || []).forEach((field) => {
      membersHtml += `
        <div class="member-item-row" data-kind="Field" data-name="${escapeHtml(field)}" data-file="${escapeHtml(cls.file)}">
          <div class="member-item-left">
            <span class="member-kind-tag field">field</span>
            <span class="member-item-name">${escapeHtml(field)}</span>
          </div>
          <span class="member-item-loc">self.${escapeHtml(field)}</span>
        </div>
      `;
    });

    // Methods
    (cls.methods || []).forEach((method) => {
      const isCtor = method.is_constructor;
      membersHtml += `
        <div class="member-item-row" data-kind="${isCtor ? "Constructor" : "Method"}" data-name="${escapeHtml(method.name)}" data-file="${escapeHtml(cls.file)}" data-line="${method.line || ""}">
          <div class="member-item-left">
            <span class="member-kind-tag ${isCtor ? "ctor" : "method"}">${isCtor ? "ctor" : "method"}</span>
            <span class="member-item-name">${escapeHtml(method.name)}()</span>
          </div>
          <span class="member-item-loc">${method.line ? `line ${method.line}` : ""}</span>
        </div>
      `;
    });

    node.innerHTML = `
      <div class="class-node-hdr">
        <span class="class-node-title">
          <span class="chevron down">›</span>
          <span>${escapeHtml(cls.name)}</span>
        </span>
        <span class="class-node-file">${escapeHtml(cls.file)}</span>
      </div>
      <div class="class-node-members">${membersHtml}</div>
    `;

    // Collapsible class node toggle
    const hdr = node.querySelector(".class-node-hdr");
    const membersList = node.querySelector(".class-node-members");
    const chevron = node.querySelector(".chevron");
    hdr.addEventListener("click", () => {
      const isHidden = membersList.hidden;
      membersList.hidden = !isHidden;
      chevron.classList.toggle("down", isHidden);
    });

    // Click handler for members
    node.querySelectorAll(".member-item-row").forEach((row) => {
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        const kind = row.dataset.kind;
        const name = row.dataset.name;
        const file = row.dataset.file;
        const line = row.dataset.line ? parseInt(row.dataset.line, 10) : null;
        showMemberDetails({ name, kind, file, line, owner: cls.name });
      });
    });

    classHierarchyTreeEl.append(node);
  });
}

function showMemberDetails(member) {
  state.selectedMember = member;

  detailNameEl.textContent = member.name;
  detailKindEl.textContent = member.kind;
  detailOwnerEl.textContent = member.owner || "-";
  detailLocationEl.textContent = `${member.file}${member.line ? `:${member.line}` : ""}`;
  detailInheritanceEl.textContent = member.resolution || (member.kind === "Constructor" ? "Constructor parent lookup" : "Direct table definition");

  detailLocationEl.onclick = (e) => {
    e.preventDefault();
    if (member.file) {
      openFile(member.file);
      if (member.line) {
        jumpToLine(member.line, 1);
      }
    }
  };

  // Populate horizontal resolution chain and stepped trace on member click
  if (member.owner) {
    const rawName = (member.name || "").replace(/\(\)$/, "");
    const explanation = {
      expression: `${member.owner}.${rawName}`,
      result: `Resolved: ${rawName} in ${member.file}`,
      steps: [
        `raw table lookup: ${member.owner}.${rawName}`,
        member.kind === "Constructor"
          ? `constructor parent: ${member.name}`
          : (member.kind === "Metatable" ? `metatable __index: ${member.name}` : `found direct member on ${member.file}`),
        `resolved ${rawName} in ${member.file}`
      ]
    };
    renderPopulatedInspector(explanation);
  }
}

// ============================================================================
// Diagnostics & Bottom Drawer Management
// ============================================================================
function initBottomDrawer() {
  const savedHeight = localStorage.getItem("aster_drawer_height") || "180";
  document.documentElement.style.setProperty("--drawer-height", `${savedHeight}px`);

  // Default collapsed per user requirement
  state.drawerCollapsed = localStorage.getItem("aster_drawer_collapsed") !== "false";
  bottomDrawerEl.classList.toggle("collapsed", state.drawerCollapsed);

  // Resize drag handle for bottom drawer
  const drawerResizer = document.querySelector("#drawer-resizer");
  let isDraggingDrawer = false;
  let startY = 0;
  let startHeight = 0;

  drawerResizer.addEventListener("mousedown", (e) => {
    isDraggingDrawer = true;
    startY = e.clientY;
    startHeight = bottomDrawerEl.getBoundingClientRect().height;
    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";
  });

  document.addEventListener("mousemove", (e) => {
    if (!isDraggingDrawer) return;
    const delta = startY - e.clientY;
    const newHeight = Math.max(90, Math.min(500, startHeight + delta));
    document.documentElement.style.setProperty("--drawer-height", `${newHeight}px`);
    if (state.drawerCollapsed) {
      toggleDrawer(false);
    }
  });

  document.addEventListener("mouseup", () => {
    if (isDraggingDrawer) {
      isDraggingDrawer = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      const currentHeight = bottomDrawerEl.getBoundingClientRect().height;
      localStorage.setItem("aster_drawer_height", String(Math.round(currentHeight)));
    }
  });

  // Resize drag handle for inspector
  const inspectorResizer = document.querySelector("#inspector-resizer");
  let isDraggingInspector = false;
  let startX = 0;
  let startWidth = 0;

  const savedWidth = localStorage.getItem("aster_inspector_width") || "320";
  document.querySelector(".workbench").style.setProperty("--inspector-width", `${savedWidth}px`);

  inspectorResizer.addEventListener("mousedown", (e) => {
    isDraggingInspector = true;
    startX = e.clientX;
    startWidth = document.querySelector("#inspector-sidebar").getBoundingClientRect().width;
    document.body.style.cursor = "ew-resize";
    document.body.style.userSelect = "none";
  });

  document.addEventListener("mousemove", (e) => {
    if (!isDraggingInspector) return;
    const delta = startX - e.clientX;
    const newWidth = Math.max(240, Math.min(520, startWidth + delta));
    document.querySelector(".workbench").style.setProperty("--inspector-width", `${newWidth}px`);
  });

  document.addEventListener("mouseup", () => {
    if (isDraggingInspector) {
      isDraggingInspector = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      const currentWidth = document.querySelector("#inspector-sidebar").getBoundingClientRect().width;
      localStorage.setItem("aster_inspector_width", String(Math.round(currentWidth)));
    }
  });
}

async function analyzeWorkspace() {
  const analyzeButton = document.querySelector("#top-analyze-btn");
  const analyzeLabel = analyzeButton.querySelector(".btn-text");
  try {
    window.clearTimeout(statusIdleTimer);
    analyzeButton.disabled = true;
    analyzeLabel.textContent = "Checking...";
    statusSyncEl.hidden = false;
    statusSyncTextEl.textContent = "Analyzing...";
    const diags = await invoke("check_workspace");
    state.diagnostics = diags || [];
    renderDiagnostics();
    updateGutterMarkers();
    renderFileTree();
    statusSyncTextEl.textContent = "Analysis current";
    analyzeLabel.textContent = "Analysis current";
    statusIdleTimer = window.setTimeout(() => { statusSyncEl.hidden = true; }, 1200);
  } catch (err) {
    statusSyncEl.hidden = false;
    statusSyncTextEl.textContent = "Error";
    analyzeLabel.textContent = "Analysis failed";
    showToast(`Code check failed: ${err}`, "error", 6500);
  } finally {
    analyzeButton.disabled = false;
    window.setTimeout(() => { analyzeLabel.textContent = "Check"; }, 1400);
  }
}

function renderDiagnostics() {
  problemListRowsEl.innerHTML = "";
  suggestionListRowsEl.innerHTML = "";
  const problems = state.diagnostics.filter((diag) => !diag.suggestion);
  const suggestions = state.diagnostics.filter((diag) => diag.suggestion);
  drawerProblemCountEl.textContent = problems.length;
  drawerSuggestionCountEl.textContent = suggestions.length;

  let errors = 0;
  let warnings = 0;

  if (problems.length === 0) {
    problemListRowsEl.innerHTML = '<div class="problems-empty">No problems detected across workspace.</div>';
    drawerProblemCountEl.classList.remove("has-errors");
  } else {
    drawerProblemCountEl.classList.add("has-errors");
  }

  if (suggestions.length === 0) {
    suggestionListRowsEl.innerHTML = '<div class="problems-empty">No performance suggestions.</div>';
    drawerSuggestionCountEl.classList.remove("has-suggestions");
  } else {
    drawerSuggestionCountEl.classList.add("has-suggestions");
  }

  problems.forEach((diag) => {
    const isWarn = diag.kind.includes("warning") || diag.kind.includes("info");
    if (isWarn) warnings++;
    else errors++;
    problemListRowsEl.append(createDiagnosticRow(diag, isWarn ? "warn" : "error"));
  });

  suggestions.forEach((diag) => {
    suggestionListRowsEl.append(createDiagnosticRow(diag, "suggestion"));
  });

  statusErrorCountEl.textContent = errors;
  statusWarnCountEl.textContent = warnings;
}

function createDiagnosticRow(diag, presentation) {
  const row = document.createElement("div");
  row.className = "problem-row";
  const iconClass = presentation === "suggestion" ? "suggestion-icon" : presentation === "warn" ? "warning-icon" : "close-icon";
  row.innerHTML = `
    <span class="problem-icon ${presentation} css-icon ${iconClass}" aria-hidden="true"></span>
    <span class="problem-message">${escapeHtml(diag.message)}</span>
    <span class="problem-kind-badge">${escapeHtml(diag.kind)}</span>
    <span class="problem-location">${escapeHtml(diag.file || "workspace")}${diag.line ? `:${diag.line}:${diag.column || 1}` : ""}</span>
  `;

  row.addEventListener("click", () => {
    if (diag.file) {
      openFile(diag.file);
      if (diag.line) jumpToLine(diag.line, diag.column || 1);
    }
  });
  return row;
}

function jumpToLine(targetLine, targetCol = 1) {
  if (!editorView.model) return;
  const offset = editorView.model.getOffsetAt({ lineNumber: targetLine, column: targetCol });
  editorView.focus();
  editorView.setSelection(offset);
  updateCursorStatus();
  triggerInspectorUpdate();
}

function toggleDrawer(forceState = null) {
  if (forceState !== null) {
    state.drawerCollapsed = forceState;
  } else {
    state.drawerCollapsed = !state.drawerCollapsed;
  }
  bottomDrawerEl.classList.toggle("collapsed", state.drawerCollapsed);
  localStorage.setItem("aster_drawer_collapsed", String(state.drawerCollapsed));
}

// Drawer Tabs Switching with smart collapse
document.querySelector("#drawer-tab-problems").addEventListener("click", () => handleDrawerTabClick("problems"));
document.querySelector("#drawer-tab-suggestions").addEventListener("click", () => handleDrawerTabClick("suggestions"));
document.querySelector("#drawer-tab-output").addEventListener("click", () => handleDrawerTabClick("output"));
document.querySelector("#drawer-tab-repl").addEventListener("click", () => handleDrawerTabClick("repl"));

function handleDrawerTabClick(tab) {
  if (state.drawerCollapsed) {
    // Expand drawer and show tab
    toggleDrawer(false);
    switchDrawerTab(tab);
  } else if (state.activeDrawerTab === tab) {
    // Clicking the already active tab collapses the drawer
    toggleDrawer(true);
  } else {
    // Switching to another tab while expanded
    switchDrawerTab(tab);
  }
}

function switchDrawerTab(tab) {
  state.activeDrawerTab = tab;
  document.querySelectorAll(".drawer-tab").forEach((el) => el.classList.remove("active"));
  document.querySelectorAll(".drawer-panel").forEach((el) => (el.hidden = true));

  document.querySelector(`#drawer-tab-${tab}`).classList.add("active");
  document.querySelector(`#panel-${tab}`).hidden = false;
}

document.querySelector("#drawer-toggle-size").addEventListener("click", () => toggleDrawer());
document.querySelector("#status-problems-btn").addEventListener("click", () => toggleDrawer());

function appendOutput(message, kind = "output") {
  if (!message) return;
  const line = document.createElement("div");
  line.className = `log-line ${kind}`;
  line.textContent = String(message).replace(/\r\n/g, "\n").replace(/\n$/, "");
  outputLogEl.append(line);
  outputLogEl.scrollTop = outputLogEl.scrollHeight;
}

function setRunState(running) {
  state.isRunning = running;
  runButton.classList.toggle("running", running);
  runButton.querySelector(".btn-text").textContent = running ? "Stop" : "Run";
  runButton.title = running ? "Stop running Lua process" : "Run current Lua file (F5)";
  runButton.querySelector("svg").innerHTML = running
    ? '<rect x="3.5" y="3.5" width="9" height="9" rx="1" />'
    : '<path d="M4 2.8v10.4a.8.8 0 0 0 1.22.68l7.55-5.2a.82.82 0 0 0 0-1.36L5.22 2.12A.8.8 0 0 0 4 2.8z"/>';
}

async function stopLuaIfRunning() {
  if (!state.isRunning) return;
  state.stopRequested = true;
  await invoke("stop_lua");
}

async function runCurrentFile() {
  if (state.isRunning) {
    await stopLuaIfRunning();
    return;
  }
  if (!state.active) {
    showToast("Open a Lua file before running.", "error");
    return;
  }
  if (!state.active.toLowerCase().endsWith(".lua")) {
    showToast("Only Lua files can be run.", "error");
    return;
  }

  try {
    await saveAllDirty();
  } catch (error) {
    showToast(`Run cancelled because the file could not be saved: ${error}`, "error", 6500);
    return;
  }

  state.stopRequested = false;
  switchDrawerTab("output");
  toggleDrawer(false);
  outputLogEl.innerHTML = "";
  const runtime = homeRuntimeSelect.value;
  appendOutput(`Running ${state.active} with ${runtimeLabels[runtime]}…`, "info");
  setRunState(true);

  try {
    const result = await invoke("run_lua", {
      request: { path: state.active, runtime },
    });
    if (result.output) appendOutput(result.output, "output");
    if (state.stopRequested) {
      appendOutput("Process stopped.", "info");
    } else if (result.success) {
      appendOutput("Process finished successfully.", "success");
    } else {
      appendOutput(result.error || "Lua process failed.", "error");
    }
  } catch (error) {
    appendOutput(String(error), "error");
    showToast(String(error), "error", 7000);
  } finally {
    setRunState(false);
    state.stopRequested = false;
  }
}

// ============================================================================
// Module Require Graph View
// ============================================================================
async function refreshModuleGraph() {
  try {
    state.moduleGraph = await invoke("module_graph");
    renderModuleGraph();
  } catch (err) {
    console.error("Error loading module graph:", err);
  }
}

function renderModuleGraph() {
  const summaryEl = document.querySelector("#graph-summary");
  const listEl = document.querySelector("#graph-nodes-list");
  listEl.innerHTML = "";

  if (!state.moduleGraph) {
    summaryEl.textContent = "No module graph available.";
    return;
  }

  const { modules, edges, entry_points, cycles } = state.moduleGraph;
  summaryEl.innerHTML = `<strong>${modules.length} Modules</strong> · ${edges.length} require links`;

  modules.forEach((mod) => {
    const isEntry = entry_points.includes(mod);
    const modDeps = edges.filter((e) => e.from === mod).map((e) => e.to);

    const card = document.createElement("div");
    card.className = "class-card";
    card.style.marginBottom = "6px";
    card.innerHTML = `
      <div class="class-card-header">
        <span class="class-card-name">${escapeHtml(mod)}</span>
        <span class="block-badge">${isEntry ? "entry" : "module"}</span>
      </div>
      <div class="class-card-body">
        ${
          modDeps.length > 0
            ? modDeps.map((d) => `<div class="class-item-row"><span class="item-name">require("${escapeHtml(d)}")</span></div>`).join("")
            : '<span class="trace-empty">No dependencies</span>'
        }
      </div>
    `;
    card.addEventListener("click", () => openFile(mod));
    listEl.append(card);
  });
}

// ============================================================================
// Lua syntax validation
// ============================================================================
document.querySelector("#repl-run-btn").addEventListener("click", runReplCode);
replInputEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.shiftKey) {
    e.preventDefault();
    runReplCode();
  }
});

async function runReplCode() {
  const code = replInputEl.value.trim();
  if (!code) return;

  const promptLine = document.createElement("div");
  promptLine.className = "repl-line prompt";
  promptLine.textContent = `> ${code}`;
  replOutputEl.append(promptLine);

  try {
    const res = await invoke("validate_lua_snippet", { code });
    const resultLine = document.createElement("div");
    if (res.success) {
      resultLine.className = "repl-line res-ok";
      resultLine.textContent = res.output;
    } else {
      resultLine.className = "repl-line res-err";
      resultLine.textContent = res.error;
    }
    replOutputEl.append(resultLine);
    replOutputEl.scrollTop = replOutputEl.scrollHeight;
    replInputEl.value = "";
  } catch (err) {
    const errLine = document.createElement("div");
    errLine.className = "repl-line res-err";
    errLine.textContent = String(err);
    replOutputEl.append(errLine);
  }
}

// ============================================================================
// Search in Workspace Files
// ============================================================================
const globalSearchInput = document.querySelector("#global-search-input");
const searchMatchCase = document.querySelector("#search-match-case");
const searchWholeWord = document.querySelector("#search-whole-word");
const searchResultsList = document.querySelector("#search-results-list");

function performWorkspaceSearch() {
  const query = globalSearchInput.value;
  searchResultsList.innerHTML = "";
  if (!query) return;

  const matchCase = searchMatchCase.checked;
  const wholeWord = searchWholeWord.checked;
  let totalMatches = 0;

  state.files.forEach((file, path) => {
    const lines = file.contents.split("\n");
    const matchingLines = [];

    lines.forEach((lineText, idx) => {
      let isMatch = false;
      if (wholeWord) {
        const escapedQuery = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const regex = new RegExp(`\\b${escapedQuery}\\b`, matchCase ? "g" : "gi");
        isMatch = regex.test(lineText);
      } else {
        isMatch = matchCase
          ? lineText.includes(query)
          : lineText.toLowerCase().includes(query.toLowerCase());
      }

      if (isMatch) {
        matchingLines.push({ lineNum: idx + 1, text: lineText.trim() });
      }
    });

    if (matchingLines.length > 0) {
      totalMatches += matchingLines.length;
      const fileCard = document.createElement("div");
      fileCard.className = "class-card";
      fileCard.style.marginBottom = "4px";
      fileCard.innerHTML = `
        <div class="class-card-header">
          <span class="class-card-name">${escapeHtml(path)}</span>
          <span class="block-badge">${matchingLines.length}</span>
        </div>
        <div class="class-card-body">
          ${matchingLines
            .map(
              (m) => `
            <div class="class-item-row" data-file="${escapeHtml(path)}" data-line="${m.lineNum}">
              <span class="item-name mono"><span class="item-badge field">${m.lineNum}</span> ${escapeHtml(m.text)}</span>
            </div>
          `
            )
            .join("")}
        </div>
      `;

      fileCard.querySelectorAll(".class-item-row").forEach((row) => {
        row.addEventListener("click", () => {
          openFile(row.dataset.file);
          jumpToLine(parseInt(row.dataset.line, 10));
        });
      });

      searchResultsList.append(fileCard);
    }
  });

  if (totalMatches === 0) {
    searchResultsList.innerHTML = '<div class="trace-empty">No matching lines found.</div>';
  }
}

globalSearchInput.addEventListener("input", performWorkspaceSearch);
searchMatchCase.addEventListener("change", performWorkspaceSearch);
searchWholeWord.addEventListener("change", performWorkspaceSearch);

// ============================================================================
// Source Control / Git View
// ============================================================================
function updateGitView() {
  const gitFilesList = document.querySelector("#git-files-list");
  const branchBadge = document.querySelector("#git-branch-badge");
  gitFilesList.innerHTML = "";

  const branch = state.recentProjects.find((p) => p.path === state.workspace?.root)?.git_branch || "main";
  branchBadge.textContent = `branch: ${branch}`;

  if (state.dirty.size === 0) {
    gitFilesList.innerHTML = '<div class="trace-empty">Working tree clean. No uncommitted changes.</div>';
    return;
  }

  state.dirty.forEach((path) => {
    const row = document.createElement("div");
    row.className = "tree-node-row";
    row.innerHTML = `
      ${fileIconMarkup(path)}
      <span class="tree-name">${escapeHtml(path)}</span>
      <span class="item-badge meta">modified</span>
    `;
    row.addEventListener("click", () => openFile(path));
    gitFilesList.append(row);
  });
}

// ============================================================================
// Activity Rail & Sidebar Views
// ============================================================================
const railButtons = {
  explorer: document.querySelector("#rail-explorer"),
  search: document.querySelector("#rail-search"),
  git: document.querySelector("#rail-git"),
  graph: document.querySelector("#rail-graph"),
};

function switchSidebarView(view) {
  state.activeSidebarView = view;
  Object.keys(railButtons).forEach((k) => {
    railButtons[k]?.classList.toggle("active", k === view);
    const viewEl = document.querySelector(`#view-${k}`);
    if (viewEl) viewEl.hidden = k !== view;
  });
  if (view === "git") updateGitView();
  if (view === "graph") refreshModuleGraph();
}

Object.keys(railButtons).forEach((k) => {
  railButtons[k]?.addEventListener("click", () => switchSidebarView(k));
});

function showSettings() {
  settingsModalOverlay.hidden = false;
  settingsRuntimeSelect.focus();
}

function hideSettings() {
  settingsModalOverlay.hidden = true;
}

document.querySelector("#rail-settings").addEventListener("click", showSettings);
document.querySelector("#settings-close-btn").addEventListener("click", hideSettings);
settingsModalOverlay.addEventListener("click", (event) => {
  if (event.target === settingsModalOverlay) hideSettings();
});
document.querySelector("#titlebar-home-btn").addEventListener("click", () => {
  if (state.mode === "workspace") closeWorkspace().catch((error) => showToast(error, "error"));
});
document.querySelector("#home-open-folder-btn").addEventListener("click", () => openWorkspace());
document.querySelector("#home-new-workspace-btn").addEventListener("click", createLuaProject);
document.querySelector("#start-create-project").addEventListener("click", createLuaProject);
document.querySelector("#start-configure-runtime").addEventListener("click", () => {
  showSettings();
});
document.querySelector("#start-open-docs").addEventListener("click", () => {
  window.open("https://www.lua.org/manual/5.4/", "_blank", "noopener");
});
document.querySelector("#start-command-palette").addEventListener("click", showCommandPalette);
document.querySelector("#top-analyze-btn").addEventListener("click", analyzeWorkspace);
runButton.addEventListener("click", runCurrentFile);
document.querySelector("#top-inspector-toggle").addEventListener("click", () => toggleInspector());
document.querySelector("#inspector-close-btn").addEventListener("click", () => toggleInspector(true));

function createLuaProject() {
  showPrompt("New Lua Project", "my-lua-project", async (name) => {
    if (!name) return;
    try {
      const info = await invoke("create_lua_project", {
        request: { name, runtime: homeRuntimeSelect.value },
      });
      if (!info) return;
      setWorkspaceInfo(info);
      setAppMode("workspace");
      await loadWorkspace();
      showToast(`Created ${info.name}.`, "success");
    } catch (error) {
      showToast(`Could not create project: ${error}`, "error", 6500);
    }
  });
}

function toggleInspector(forceState = null) {
  if (forceState !== null) {
    state.inspectorCollapsed = forceState;
  } else {
    state.inspectorCollapsed = !state.inspectorCollapsed;
  }
  document.querySelector(".workbench").classList.toggle("inspector-collapsed", state.inspectorCollapsed);
  document.querySelector("#top-inspector-toggle").classList.toggle("active", !state.inspectorCollapsed);
  localStorage.setItem("aster_inspector_collapsed", String(state.inspectorCollapsed));
  if (!state.inspectorCollapsed) {
    refreshClassStructures();
    updateInspector();
  }
}

// ============================================================================
// Quick Open / Command Palette
// ============================================================================
function showCommandPalette() {
  commandModalOverlay.hidden = false;
  paletteInput.value = "";
  renderPaletteResults("");
  paletteInput.focus();
}

function hideCommandPalette() {
  commandModalOverlay.hidden = true;
}

paletteInput.addEventListener("input", () => {
  renderPaletteResults(paletteInput.value);
});

paletteInput.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    hideCommandPalette();
  } else if (e.key === "Enter") {
    const first = paletteResults.querySelector(".palette-item");
    if (first) first.click();
  }
});

commandModalOverlay.addEventListener("click", (e) => {
  if (e.target === commandModalOverlay) hideCommandPalette();
});

document.querySelector("#command-center-btn").addEventListener("click", showCommandPalette);

function renderPaletteResults(query) {
  paletteResults.innerHTML = "";
  const q = query.toLowerCase().trim();

  // Commands
  const commands = [
    { title: "Aster: Run Current File", icon: "run-icon", action: runCurrentFile },
    { title: "Aster: Check Code", icon: "refresh-icon", action: analyzeWorkspace },
    { title: "Aster: New Lua Project...", icon: "add-icon", action: createLuaProject },
    { title: "Aster: Open Folder...", icon: "folder-css-icon", action: () => openWorkspace() },
    { title: "Aster: Close Workspace (Return to Home)", icon: "close-icon", action: closeWorkspace },
    { title: "Aster: Toggle Member Lookup", icon: "settings-icon", action: toggleInspector },
    { title: "Aster: Toggle Problems Drawer", icon: "warning-icon", action: () => toggleDrawer() },
    { title: "Aster: Open Settings", icon: "settings-icon", action: showSettings },
    { title: "Aster: New Lua File...", icon: "add-icon", action: promptNewFile },
  ];

  const matchedCommands = commands.filter((c) => c.title.toLowerCase().includes(q));
  matchedCommands.forEach((cmd) => {
    const item = document.createElement("div");
    item.className = "palette-item";
    item.innerHTML = `
      <div class="palette-item-left">
        <span class="palette-command-icon css-icon ${cmd.icon}" aria-hidden="true"></span>
        <span>${escapeHtml(cmd.title)}</span>
      </div>
      <kbd>Action</kbd>
    `;
    item.addEventListener("click", () => {
      hideCommandPalette();
      cmd.action();
    });
    paletteResults.append(item);
  });

  // Files
  state.files.forEach((_, path) => {
    if (!q || path.toLowerCase().includes(q)) {
      const item = document.createElement("div");
      item.className = "palette-item";
      item.innerHTML = `
        <div class="palette-item-left">
          ${fileIconMarkup(path)}
          <span>${escapeHtml(path)}</span>
        </div>
        <kbd>File</kbd>
      `;
      item.addEventListener("click", () => {
        hideCommandPalette();
        openFile(path);
      });
      paletteResults.append(item);
    }
  });
}

// ============================================================================
// Context Menus & File Creation
// ============================================================================
function showContextMenu(x, y, target) {
  state.contextTarget = target;
  contextMenuEl.hidden = false;
  contextMenuEl.style.left = `${Math.min(x, window.innerWidth - 180)}px`;
  contextMenuEl.style.top = `${Math.min(y, window.innerHeight - 160)}px`;
}

document.addEventListener("click", (e) => {
  if (!contextMenuEl.contains(e.target)) {
    contextMenuEl.hidden = true;
  }
});

document.querySelector("#ctx-new-file").addEventListener("click", () => {
  contextMenuEl.hidden = true;
  const target = state.contextTarget;
  const baseDir = target ? (target.is_dir ? target.path : target.path.split("/").slice(0, -1).join("/")) : "";
  promptNewFile(baseDir);
});

document.querySelector("#ctx-new-folder").addEventListener("click", () => {
  contextMenuEl.hidden = true;
  const target = state.contextTarget;
  const baseDir = target ? (target.is_dir ? target.path : target.path.split("/").slice(0, -1).join("/")) : "";
  promptNewFolder(baseDir);
});

document.querySelector("#ctx-rename").addEventListener("click", () => {
  contextMenuEl.hidden = true;
  if (state.contextTarget) {
    promptRename(state.contextTarget);
  }
});

document.querySelector("#ctx-delete").addEventListener("click", async () => {
  contextMenuEl.hidden = true;
  if (!state.contextTarget?.path) return;
  const targetPath = state.contextTarget.path;
  const isDir = state.contextTarget.is_dir;

  if (confirm(`Delete ${isDir ? "folder" : "file"} '${targetPath}'?`)) {
    try {
      await invoke("delete_entry", { request: { path: targetPath } });
      if (isDir) {
        state.openTabs = state.openTabs.filter((t) => !t.startsWith(targetPath + "/"));
        if (state.active && state.active.startsWith(targetPath + "/")) {
          state.active = state.openTabs[0] || null;
        }
        state.expandedFolders.delete(targetPath);
      } else {
        if (state.openTabs.includes(targetPath)) {
          await closeTab(targetPath);
        }
      }
      await loadWorkspace();
    } catch (err) {
      showToast(`Delete failed: ${err}`, "error", 6500);
    }
  }
});

document.querySelector("#ctx-reveal").addEventListener("click", () => {
  contextMenuEl.hidden = true;
  invoke("reveal_in_explorer", { path: state.contextTarget?.path }).catch(console.error);
});

document.querySelector("#ctx-copy-path").addEventListener("click", () => {
  contextMenuEl.hidden = true;
  if (state.contextTarget?.path) {
    navigator.clipboard.writeText(state.contextTarget.path);
  }
});

document.querySelector("#explorer-new-file").addEventListener("click", () => promptNewFile(""));
document.querySelector("#explorer-new-folder").addEventListener("click", () => promptNewFolder(""));
document.querySelector("#explorer-refresh").addEventListener("click", loadWorkspace);
document.querySelector("#explorer-collapse-all").addEventListener("click", () => {
  state.expandedFolders.clear();
  renderFileTree();
});

function promptNewFile(baseDir = "") {
  showPrompt("New Lua File", "filename.lua", async (name) => {
    if (!name) return;
    if (!name.endsWith(".lua")) name += ".lua";
    const fullPath = baseDir ? `${baseDir}/${name}` : name;
    try {
      await invoke("create_file", { request: { path: fullPath } });
      if (baseDir) state.expandedFolders.add(baseDir);
      await loadWorkspace();
      openFile(fullPath);
    } catch (err) {
      showToast(`Could not create file: ${err}`, "error", 6500);
    }
  });
}

function promptNewFolder(baseDir = "") {
  showPrompt("New Folder", "folder_name", async (name) => {
    if (!name) return;
    const fullPath = baseDir ? `${baseDir}/${name}` : name;
    try {
      await invoke("create_directory", { request: { path: fullPath } });
      if (baseDir) state.expandedFolders.add(baseDir);
      await loadWorkspace();
    } catch (err) {
      showToast(`Could not create folder: ${err}`, "error", 6500);
    }
  });
}

function promptRename(target) {
  if (!target?.path) return;
  const oldPath = target.path;
  const parts = oldPath.replace(/\\/g, "/").split("/");
  const oldName = parts.pop();
  const parentDir = parts.join("/");

  showPrompt("Rename", oldName, async (newName) => {
    if (!newName || newName === oldName) return;
    const newPath = parentDir ? `${parentDir}/${newName}` : newName;

    try {
      await invoke("rename_entry", {
        request: { old_path: oldPath, new_path: newPath },
      });

      const tabIdx = state.openTabs.indexOf(oldPath);
      if (tabIdx !== -1) {
        state.openTabs[tabIdx] = newPath;
      }
      if (state.active === oldPath) {
        state.active = newPath;
      }
      if (state.dirty.has(oldPath)) {
        state.dirty.delete(oldPath);
        state.dirty.add(newPath);
      }
      if (state.expandedFolders.has(oldPath)) {
        state.expandedFolders.delete(oldPath);
        state.expandedFolders.add(newPath);
      }

      await loadWorkspace();
      if (state.active === newPath) {
        openFile(newPath);
      }
    } catch (err) {
      showToast(`Rename failed: ${err}`, "error", 6500);
    }
  });
}

function showPrompt(title, defaultValue, onConfirm) {
  promptTitleEl.textContent = title;
  promptInputEl.value = defaultValue;
  promptModalOverlay.hidden = false;
  promptInputEl.focus();
  promptInputEl.select();

  const handleOk = () => {
    promptModalOverlay.hidden = true;
    onConfirm(promptInputEl.value.trim());
    cleanup();
  };

  const handleCancel = () => {
    promptModalOverlay.hidden = true;
    cleanup();
  };

  const handleKey = (e) => {
    if (e.key === "Enter") handleOk();
    if (e.key === "Escape") handleCancel();
  };

  function cleanup() {
    promptOkBtn.removeEventListener("click", handleOk);
    promptCancelBtn.removeEventListener("click", handleCancel);
    promptInputEl.removeEventListener("keydown", handleKey);
  }

  promptOkBtn.addEventListener("click", handleOk);
  promptCancelBtn.addEventListener("click", handleCancel);
  promptInputEl.addEventListener("keydown", handleKey);
}

// ============================================================================
// Find / Replace in the Active File
// ============================================================================
function showEditorFind(showReplace = false) {
  if (!state.active) return;
  editorFindEl.hidden = false;
  editorReplaceRow.hidden = !showReplace;
  const selection = editorView.getSelection();
  const selectedText = editorView.getText().slice(selection.start, selection.end);
  if (selectedText && !selectedText.includes("\n")) editorFindInput.value = selectedText;
  updateEditorFindMatches();
  editorFindInput.focus();
  editorFindInput.select();
}

function hideEditorFind() {
  editorFindEl.hidden = true;
  editorView.focus();
}

function updateEditorFindMatches() {
  const query = editorFindInput.value;
  const text = editorView.getText();
  state.findMatches = [];
  state.findIndex = -1;
  if (!query) {
    editorFindCount.textContent = "0 results";
    return;
  }
  let offset = 0;
  while (offset <= text.length - query.length) {
    const match = text.indexOf(query, offset);
    if (match < 0) break;
    state.findMatches.push(match);
    offset = match + Math.max(query.length, 1);
  }
  editorFindCount.textContent = `${state.findMatches.length} ${state.findMatches.length === 1 ? "result" : "results"}`;
}

function moveToFindMatch(direction = 1) {
  updateEditorFindMatches();
  if (!state.findMatches.length) return;
  const selection = editorView.getSelection();
  if (direction > 0) {
    state.findIndex = state.findMatches.findIndex((offset) => offset >= selection.end);
    if (state.findIndex < 0) state.findIndex = 0;
  } else {
    state.findIndex = state.findMatches.findLastIndex((offset) => offset < selection.start);
    if (state.findIndex < 0) state.findIndex = state.findMatches.length - 1;
  }
  const start = state.findMatches[state.findIndex];
  editorView.focus();
  editorView.setSelection(start, start + editorFindInput.value.length);
  editorFindCount.textContent = `${state.findIndex + 1} of ${state.findMatches.length}`;
}

function replaceCurrentMatch() {
  const query = editorFindInput.value;
  if (!query) return;
  const selection = editorView.getSelection();
  if (editorView.getText().slice(selection.start, selection.end) !== query) {
    moveToFindMatch(1);
    return;
  }
  editorView.replaceSelection(editorReplaceInput.value, "replace");
  editorEl.dispatchEvent(new Event("input"));
  moveToFindMatch(1);
}

function replaceAllMatches() {
  const query = editorFindInput.value;
  if (!query) return;
  const text = editorView.getText();
  const count = text.split(query).length - 1;
  if (!count) return;
  const replaced = text.split(query).join(editorReplaceInput.value);
  editorView.setSelection(0, text.length);
  editorView.replaceSelection(replaced, "replace-all");
  editorEl.dispatchEvent(new Event("input"));
  updateEditorFindMatches();
  showToast(`Replaced ${count} ${count === 1 ? "match" : "matches"}.`, "success");
}

editorFindInput.addEventListener("input", updateEditorFindMatches);
editorFindInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    moveToFindMatch(event.shiftKey ? -1 : 1);
  } else if (event.key === "Escape") {
    event.preventDefault();
    hideEditorFind();
  }
});
document.querySelector("#editor-find-prev").addEventListener("click", () => moveToFindMatch(-1));
document.querySelector("#editor-find-next").addEventListener("click", () => moveToFindMatch(1));
document.querySelector("#editor-find-close").addEventListener("click", hideEditorFind);
document.querySelector("#editor-find-toggle-replace").addEventListener("click", () => {
  editorReplaceRow.hidden = !editorReplaceRow.hidden;
  if (!editorReplaceRow.hidden) editorReplaceInput.focus();
});
document.querySelector("#editor-replace-one").addEventListener("click", replaceCurrentMatch);
document.querySelector("#editor-replace-all").addEventListener("click", replaceAllMatches);

// ============================================================================
// Global Keyboard Shortcuts
// ============================================================================
document.addEventListener("keydown", (e) => {
  const isCmd = e.ctrlKey || e.metaKey;

  if (isCmd && e.key === ",") {
    e.preventDefault();
    showSettings();
  } else if (e.key === "Escape" && !editorFindEl.hidden) {
    e.preventDefault();
    hideEditorFind();
  } else if (e.key === "Escape" && !settingsModalOverlay.hidden) {
    e.preventDefault();
    hideSettings();
  } else if (e.key === "F2") {
    e.preventDefault();
    if (state.contextTarget) {
      promptRename(state.contextTarget);
    } else if (state.active) {
      promptRename({ path: state.active, is_dir: false });
    }
  } else if (isCmd && e.key.toLowerCase() === "s") {
    e.preventDefault();
    saveActive().catch((error) => showToast(`Save failed: ${error}`, "error", 6500));
  } else if (isCmd && e.key.toLowerCase() === "f") {
    e.preventDefault();
    showEditorFind(false);
  } else if (isCmd && e.key.toLowerCase() === "h") {
    e.preventDefault();
    showEditorFind(true);
  } else if (isCmd && e.key.toLowerCase() === "p") {
    e.preventDefault();
    showCommandPalette();
  } else if (isCmd && e.key.toLowerCase() === "m") {
    e.preventDefault();
    toggleInspector();
  } else if (isCmd && e.key.toLowerCase() === "j") {
    e.preventDefault();
    toggleDrawer();
  } else if (isCmd && e.key.toLowerCase() === "w") {
    e.preventDefault();
    if (state.active) closeTab(state.active);
  } else if (e.key === "F5") {
    e.preventDefault();
    runCurrentFile();
  } else if (isCmd && e.key.toLowerCase() === "r") {
    e.preventDefault();
    analyzeWorkspace();
  }
});

function escapeHtml(str) {
  if (!str) return "";
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

// ============================================================================
// App Launch
// ============================================================================
async function init() {
  initBottomDrawer();
  toggleInspector(localStorage.getItem("aster_inspector_collapsed") !== "false");
  await refreshRuntimeAvailability();
  try {
    const current = await invoke("current_workspace");
    if (current) {
      setWorkspaceInfo(current);
      setAppMode("workspace");
      await loadWorkspace();
    } else {
      setAppMode("home");
    }
  } catch (err) {
    console.error("Init error:", err);
    setAppMode("home");
  }
}

init();
