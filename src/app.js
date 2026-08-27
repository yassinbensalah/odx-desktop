// ======================================================
// Tauri API
// ======================================================

const { invoke } = window.__TAURI__.core;


// ======================================================
// Tabs
// ======================================================

const railBtns = document.querySelectorAll(".rail-btn");
const tabs = document.querySelectorAll(".tab");

function showTab(name) {
  railBtns.forEach((button) => {
    button.classList.toggle(
      "active",
      button.dataset.tab === name
    );
  });

  tabs.forEach((tab) => {
    tab.classList.toggle(
      "active",
      tab.id === `tab-${name}`
    );
  });
}

railBtns.forEach((button) => {
  button.addEventListener("click", () => {
    showTab(button.dataset.tab);
  });
});

// ======================================================
// Unified Diagnostics tool switcher
// ======================================================

const diagnosticToolBtns = document.querySelectorAll(".diagnostic-tool-btn");
const diagnosticTools = document.querySelectorAll(".diagnostic-tool");

function showDiagnosticTool(name) {
  diagnosticToolBtns.forEach((button) => {
    button.classList.toggle("active", button.dataset.diagnosticTool === name);
  });
  diagnosticTools.forEach((panel) => {
    panel.classList.toggle("active", panel.id === `diagnostic-tool-${name}`);
  });
}

diagnosticToolBtns.forEach((button) => {
  button.addEventListener("click", () => showDiagnosticTool(button.dataset.diagnosticTool));
});


// ======================================================
// Bundled tools
// ======================================================

// The converter and MDD parser are compiled into the Rust backend.
// The PDX validator is bundled as an application resource.

// ======================================================
// Convert PDX tab
// ======================================================

const convertPickBtn =
  document.getElementById("convert-pick-btn");

const convertFilePath =
  document.getElementById("convert-file-path");

const convertRunBtn =
  document.getElementById("convert-run-btn");

const convertResult =
  document.getElementById("convert-result");

const convertStatus =
  document.getElementById("convert-status");

const convertLog =
  document.getElementById("convert-log");

const convertActions =
  document.getElementById("convert-actions");

const convertSendToParse =
  document.getElementById("convert-send-to-parse");


// ======================================================
// PDX validation elements
// ======================================================

const validatePdxBtn =
  document.getElementById("validate-pdx-btn");

const validateResult =
  document.getElementById("validate-result");

const validateStatus =
  document.getElementById("validate-status");

const validateLog =
  document.getElementById("validate-log");

const projectOutputPath = document.getElementById("project-output-path");
const projectOutputPickBtn = document.getElementById("project-output-pick-btn");
const projectOutputDefaultBtn = document.getElementById("project-output-default-btn");

let selectedPdx = null;
let workspaceConversionSourceIds = [];
let lastMddOutput = null;

// ======================================================
// Persistent Diagnostic Workspace
// ======================================================

const workspaceImportBtn = document.getElementById("workspace-import-btn");
const workspaceConvertBtn = document.getElementById("workspace-convert-btn");
const workspaceRefreshBtn = document.getElementById("workspace-refresh-btn");
const workspaceClearSelectionBtn = document.getElementById("workspace-clear-selection-btn");
const workspaceActiveSource = document.getElementById("workspace-active-source");
const workspaceActiveExplanation = document.getElementById("workspace-active-explanation");
const workspaceActiveEcu = document.getElementById("workspace-active-ecu");
const workspaceStatus = document.getElementById("workspace-status");
const workspaceSourceList = document.getElementById("workspace-source-list");
const udsSavedSourceSelect = document.getElementById("uds-saved-source-select");
const udsUseSavedSourceBtn = document.getElementById("uds-use-saved-source-btn");
const udsBuilderEcuSelect = document.getElementById("uds-builder-ecu-select");
const udsEcuConnectionStatus = document.getElementById("uds-ecu-connection-status");
const udsDoipHost = document.getElementById("uds-doip-host");
const udsDoipPort = document.getElementById("uds-doip-port");
const udsDoipSourceAddress = document.getElementById("uds-doip-source-address");
const udsDoipTargetAddress = document.getElementById("uds-doip-target-address");

let diagnosticSources = [];
let activeBuilderSourceId = localStorage.getItem("odx-active-builder-source") || "";
let loadedMddEcuNames = [];

function activeBuilderSource() {
  return diagnosticSources.find((item) => item.id === activeBuilderSourceId) || null;
}

function builderEcuStorageKey(sourceId = activeBuilderSourceId) {
  return sourceId ? `odx-builder-ecu:${sourceId}` : "";
}

function selectedBuilderEcu() {
  return udsBuilderEcuSelect?.value || "";
}

function ecuDoipProfileKey(sourceId = activeBuilderSourceId, ecu = selectedBuilderEcu()) {
  return sourceId && ecu ? `odx-doip-profile:${sourceId}:${ecu}` : "";
}

function readEcuDoipProfile() {
  const key = ecuDoipProfileKey();
  if (!key) return null;
  try {
    const value = JSON.parse(localStorage.getItem(key) || "null");
    return value && typeof value === "object" ? value : null;
  } catch (_) {
    return null;
  }
}

function saveCurrentEcuDoipProfile() {
  const key = ecuDoipProfileKey();
  if (!key) return;
  const profile = {
    host: udsDoipHost?.value.trim() || "127.0.0.1",
    port: udsDoipPort?.value.trim() || "13400",
    sourceAddress: udsDoipSourceAddress?.value.trim() || "0x0E80",
    targetAddress: udsDoipTargetAddress?.value.trim() || ""
  };
  localStorage.setItem(key, JSON.stringify(profile));
  if (udsEcuConnectionStatus) {
    udsEcuConnectionStatus.textContent = `Connection saved for ${selectedBuilderEcu()}.`;
    udsEcuConnectionStatus.className = "result-status ok";
  }
}

function loadCurrentEcuDoipProfile() {
  const ecu = selectedBuilderEcu();
  const profile = readEcuDoipProfile();
  if (udsDoipHost) udsDoipHost.value = profile?.host || "127.0.0.1";
  if (udsDoipPort) udsDoipPort.value = profile?.port || "13400";
  if (udsDoipSourceAddress) udsDoipSourceAddress.value = profile?.sourceAddress || "0x0E80";
  if (udsDoipTargetAddress) udsDoipTargetAddress.value = profile?.targetAddress || "";
  if (udsEcuConnectionStatus) {
    udsEcuConnectionStatus.textContent = ecu
      ? (profile ? `Loaded saved DoIP connection for ${ecu}.` : `Enter the DoIP logical address for ${ecu}. The values will be remembered automatically.`)
      : "Choose an ECU before sending.";
    udsEcuConnectionStatus.className = "result-status";
  }
}

function renderBuilderEcuOptions() {
  if (!udsBuilderEcuSelect) return;
  const source = activeBuilderSource();
  const sourceNames = Array.isArray(source?.ecu_names) ? source.ecu_names.filter(Boolean) : [];
  const names = loadedMddEcuNames.length ? loadedMddEcuNames : sourceNames;
  const remembered = source ? localStorage.getItem(builderEcuStorageKey(source.id)) || "" : "";
  udsBuilderEcuSelect.innerHTML = "";
  if (!udsMddPath && !source) {
    udsBuilderEcuSelect.innerHTML = '<option value="">Choose a diagnostic source first</option>';
    udsBuilderEcuSelect.disabled = true;
  } else if (!names.length) {
    udsBuilderEcuSelect.innerHTML = '<option value="">No ECU declared in this MDD</option>';
    udsBuilderEcuSelect.disabled = true;
  } else {
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Choose target ECU…";
    udsBuilderEcuSelect.appendChild(placeholder);
    names.forEach((name) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      udsBuilderEcuSelect.appendChild(option);
    });
    udsBuilderEcuSelect.disabled = false;
    if (names.includes(remembered)) udsBuilderEcuSelect.value = remembered;
    else if (names.length === 1) udsBuilderEcuSelect.value = names[0];
  }
  loadCurrentEcuDoipProfile();
  updateWorkspaceContextCard();
}

function updateWorkspaceContextCard() {
  const source = activeBuilderSource();
  if (workspaceActiveSource) {
    workspaceActiveSource.textContent = source ? source.name : "No diagnostic source selected";
  }
  if (workspaceActiveExplanation) {
    workspaceActiveExplanation.textContent = source
      ? `Builder uses this ${source.kind.toUpperCase()} source and its declared ECU list.`
      : "Choose a saved MDD from the Diagnostics source selector.";
  }
  if (workspaceActiveEcu) {
    const remembered = source ? localStorage.getItem(builderEcuStorageKey(source.id)) || "" : "";
    workspaceActiveEcu.textContent = remembered || "No ECU selected";
  }
}

function workspaceEscape(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function savedMddPath(source) {
  if (!source) return null;
  if (source.kind === "mdd") return source.stored_path;
  return source.generated_mdd_path || null;
}

function sourceEcuText(source) {
  const names = Array.isArray(source.ecu_names) ? source.ecu_names : [];
  return names.length ? names.join(", ") : "No ECU name detected";
}

function updateWorkspaceSelectionActions() {
  const selectedCount = document.querySelectorAll(".workspace-source-checkbox:checked").length;
  const hasSelection = selectedCount > 0;
  if (workspaceConvertBtn) workspaceConvertBtn.disabled = !hasSelection;
  if (workspaceClearSelectionBtn) workspaceClearSelectionBtn.disabled = !hasSelection;
}

function renderSavedMddSelector() {
  if (!udsSavedSourceSelect) return;
  udsSavedSourceSelect.innerHTML = '<option value="">Saved MDD / converted source…</option>';

  // Prefer real MDD entries over the PDX/ODX source that points to the same
  // generated MDD. Otherwise a converted test.pdx was displayed as
  // "test.pdx (PDX)" even though Diagnostics actually loads the .mdd file.
  const seenMddPaths = new Set();
  const ordered = [
    ...diagnosticSources.filter((source) => source.kind === "mdd"),
    ...diagnosticSources.filter((source) => source.kind !== "mdd")
  ];

  ordered.forEach((source) => {
    const mdd = savedMddPath(source);
    if (!mdd || seenMddPaths.has(mdd)) return;
    seenMddPaths.add(mdd);
    const option = document.createElement("option");
    option.value = source.id;
    option.textContent = source.kind === "mdd"
      ? `${source.name} (MDD)`
      : `${source.name} (converted MDD)`;
    udsSavedSourceSelect.appendChild(option);
  });
}

function renderDiagnosticSources() {
  renderSavedMddSelector();
  renderBuilderEcuOptions();
  updateWorkspaceContextCard();
  if (!workspaceSourceList) return;
  if (!diagnosticSources.length) {
    workspaceSourceList.innerHTML = '<p class="hint">No saved files yet. Import PDX, ODX or MDD files once and they will remain available after restarting the application.</p>';
    updateWorkspaceSelectionActions();
    return;
  }

  workspaceSourceList.innerHTML = diagnosticSources.map((source) => {
    const mdd = savedMddPath(source);
    const convertible = source.kind === "pdx" || source.kind === "odx";
    return `
      <div class="workspace-source-row" data-source-id="${workspaceEscape(source.id)}">
        <label class="workspace-source-check" title="Select PDX/ODX source(s) for Validator & Converter">
          ${convertible ? `<input type="checkbox" class="workspace-source-checkbox" value="${workspaceEscape(source.id)}"> Select` : `<span class="workspace-source-spacer">—</span>`}
        </label>
        <div class="workspace-source-main">
          <div class="workspace-source-title"><span class="workspace-kind">${workspaceEscape(source.kind.toUpperCase())}</span> ${workspaceEscape(source.name)}</div>
          <div class="workspace-source-meta">ECUs: ${workspaceEscape(sourceEcuText(source))}</div>
          ${mdd ? `<div class="workspace-source-meta">MDD ready: ${workspaceEscape(mdd)}</div>` : ""}
        </div>
        <div class="workspace-source-actions">
          <button class="btn small workspace-remove-source" type="button">Remove</button>
        </div>
      </div>`;
  }).join("");
  updateWorkspaceSelectionActions();
}

async function refreshDiagnosticSources() {
  try {
    const sources = await invoke("list_diagnostic_sources");
    diagnosticSources = Array.isArray(sources) ? sources : [];
    renderDiagnosticSources();
  } catch (error) {
    if (workspaceStatus) {
      workspaceStatus.textContent = `Could not load saved diagnostic files: ${String(error)}`;
      workspaceStatus.className = "result-status fail";
    }
  }
}

workspaceImportBtn?.addEventListener("click", async () => {
  workspaceImportBtn.disabled = true;
  if (workspaceStatus) {
    workspaceStatus.textContent = "Importing and scanning diagnostic files…";
    workspaceStatus.className = "result-status";
  }
  try {
    const sources = await invoke("import_diagnostic_sources");
    diagnosticSources = Array.isArray(sources) ? sources : [];
    renderDiagnosticSources();
    if (workspaceStatus) {
      workspaceStatus.textContent = `${diagnosticSources.length} diagnostic file(s) saved in the workspace.`;
      workspaceStatus.className = "result-status ok";
    }
  } catch (error) {
    if (workspaceStatus) {
      workspaceStatus.textContent = `Import failed: ${String(error)}`;
      workspaceStatus.className = "result-status fail";
    }
  } finally {
    workspaceImportBtn.disabled = false;
  }
});

workspaceRefreshBtn?.addEventListener("click", refreshDiagnosticSources);

workspaceClearSelectionBtn?.addEventListener("click", () => {
  document.querySelectorAll(".workspace-source-checkbox").forEach((input) => { input.checked = false; });
  updateWorkspaceSelectionActions();
  if (workspaceStatus) {
    workspaceStatus.textContent = "Conversion checks cleared. Your saved files were not removed.";
    workspaceStatus.className = "result-status";
  }
});

workspaceConvertBtn?.addEventListener("click", () => {
  const ids = [...document.querySelectorAll(".workspace-source-checkbox:checked")].map((input) => input.value);
  const selected = ids.map((id) => diagnosticSources.find((item) => item.id === id)).filter(Boolean);
  if (!selected.length) {
    workspaceStatus.textContent = "Nothing selected. Select one PDX, or several related ODX files, then open Validator & Converter.";
    workspaceStatus.className = "result-status fail";
    return;
  }

  const kinds = new Set(selected.map((source) => source.kind));
  const validPdx = kinds.size === 1 && kinds.has("pdx") && selected.length === 1;
  const validOdx = kinds.size === 1 && kinds.has("odx");
  if (!validPdx && !validOdx) {
    workspaceStatus.textContent = "Validator & Converter accepts exactly one PDX or a group containing only ODX files.";
    workspaceStatus.className = "result-status fail";
    return;
  }

  workspaceConversionSourceIds = ids;
  lastMddOutput = null;
  convertResult.hidden = true;
  convertActions.hidden = true;
  validateResult.hidden = true;

  if (validPdx) {
    selectedPdx = selected[0].stored_path;
    convertFilePath.textContent = selectedPdx;
    convertFilePath.classList.add("has-file");
    validatePdxBtn.disabled = false;
  } else {
    selectedPdx = null;
    convertFilePath.textContent = `${selected.length} saved ODX source(s): ${selected.map((source) => source.name).join(", ")}`;
    convertFilePath.classList.add("has-file");
    validatePdxBtn.disabled = true;
  }
  convertRunBtn.disabled = false;
  workspaceStatus.textContent = "Selection loaded in Validator & Converter.";
  workspaceStatus.className = "result-status ok";
  showTab("convert");
});

workspaceSourceList?.addEventListener("change", (event) => {
  if (event.target.matches(".workspace-source-checkbox")) {
    updateWorkspaceSelectionActions();
  }
});

workspaceSourceList?.addEventListener("click", async (event) => {
  const row = event.target.closest(".workspace-source-row");
  if (!row) return;
  const source = diagnosticSources.find((item) => item.id === row.dataset.sourceId);
  if (!source) return;

  if (event.target.closest(".workspace-remove-source")) {
    try {
      const sources = await invoke("remove_diagnostic_source", { sourceId: source.id });
      diagnosticSources = Array.isArray(sources) ? sources : [];
      renderDiagnosticSources();
      workspaceStatus.textContent = `Removed ${source.name} from the workspace.`;
      workspaceStatus.className = "result-status";
    } catch (error) {
      workspaceStatus.textContent = `Could not remove source: ${String(error)}`;
      workspaceStatus.className = "result-status fail";
    }
  }
});

udsUseSavedSourceBtn?.addEventListener("click", async () => {
  const source = diagnosticSources.find((item) => item.id === udsSavedSourceSelect.value);
  const path = savedMddPath(source);
  if (!path) {
    udsSourceStatus.textContent = "Choose a saved source that has an MDD.";
    udsSourceStatus.className = "result-status fail";
    return;
  }
  activeBuilderSourceId = source.id;
  localStorage.setItem("odx-active-builder-source", activeBuilderSourceId);
  loadedMddEcuNames = [];
  renderDiagnosticSources();
  renderBuilderEcuOptions();
  await loadMddForUds(path);
  await loadServicesForSelectedEcu();
});


// ======================================================
// Project output folder
// ======================================================
async function refreshProjectOutputFolder() {
  if (!projectOutputPath) return;
  try {
    const info = await invoke("get_project_output_folder");
    projectOutputPath.textContent = info.path;
    projectOutputPath.classList.add("has-file");
    projectOutputPath.title = info.is_default
      ? "Workspace default output folder"
      : "Custom project output folder";
  } catch (error) {
    projectOutputPath.textContent = `Could not resolve output folder: ${String(error)}`;
    projectOutputPath.classList.remove("has-file");
  }
}

projectOutputPickBtn?.addEventListener("click", async () => {
  projectOutputPickBtn.disabled = true;
  try {
    const path = await invoke("pick_project_output_folder");
    if (path) await refreshProjectOutputFolder();
  } catch (error) {
    projectOutputPath.textContent = `Could not set output folder: ${String(error)}`;
  } finally {
    projectOutputPickBtn.disabled = false;
  }
});

projectOutputDefaultBtn?.addEventListener("click", async () => {
  projectOutputDefaultBtn.disabled = true;
  try {
    await invoke("reset_project_output_folder");
    await refreshProjectOutputFolder();
  } catch (error) {
    projectOutputPath.textContent = `Could not restore default output folder: ${String(error)}`;
  } finally {
    projectOutputDefaultBtn.disabled = false;
  }
});

// ======================================================
// Choose PDX file
// ======================================================

convertPickBtn.addEventListener("click", async () => {
  try {
    const path = await invoke("pick_pdx_file");

    if (!path) {
      return;
    }

    selectedPdx = path;
    workspaceConversionSourceIds = [];

    convertFilePath.textContent = path;
    convertFilePath.classList.add("has-file");

    // Enable validation
    validatePdxBtn.disabled = false;

    // Enable conversion
    convertRunBtn.disabled = false;

    // Reset previous validation result
    validateResult.hidden = true;
    validateStatus.textContent = "";
    validateLog.textContent = "";

  } catch (error) {
    console.error("Failed to select PDX:", error);
  }
});


// ======================================================
// Validate PDX
// ======================================================

validatePdxBtn.addEventListener("click", async () => {
  if (!selectedPdx) {
    validateResult.hidden = false;
    validateStatus.textContent = "No PDX selected";
    validateStatus.className = "result-status fail";
    validateLog.textContent =
      "Please select a PDX file first.";

    return;
  }

  validatePdxBtn.disabled = true;
  validatePdxBtn.textContent = "Validating…";

  validateResult.hidden = false;

  validateStatus.textContent = "Validating...";
  validateStatus.className = "result-status";

  validateLog.textContent = "";

  try {

    const result = await invoke(
      "validate_pdx_file",
      {
        pdxFilePath: selectedPdx
      }
    );

    validateStatus.textContent =
      "PDX validation succeeded";

    validateStatus.className =
      "result-status ok";

    validateLog.textContent =
      result || "PDX file is valid.";

  } catch (error) {

    validateStatus.textContent =
      "PDX validation failed";

    validateStatus.className =
      "result-status fail";

    validateLog.textContent =
      String(error);

  } finally {

    validatePdxBtn.disabled = false;

    validatePdxBtn.textContent =
      "Validate PDX";
  }
});


// ======================================================
// Run PDX -> MDD conversion
// ======================================================

convertRunBtn.addEventListener("click", async () => {
  if (!selectedPdx && !workspaceConversionSourceIds.length) {
    return;
  }

  convertRunBtn.disabled = true;
  convertRunBtn.textContent = "Running…";

  convertResult.hidden = false;
  convertActions.hidden = true;
  convertStatus.textContent = "";
  convertLog.textContent = "";

  try {
    let result;
    if (workspaceConversionSourceIds.length) {
      const savedResult = await invoke("convert_saved_sources", { sourceIds: workspaceConversionSourceIds });
      result = {
        success: true,
        stdout: `Conversion succeeded.\nOutput: ${savedResult.stored_path}`,
        stderr: "",
        output_path: savedResult.stored_path
      };
    } else {
      result = await invoke("convert_pdx", { inputPath: selectedPdx });
    }

    convertStatus.textContent = result.success ? "Conversion succeeded" : "Conversion failed";
    convertStatus.className = "result-status " + (result.success ? "ok" : "fail");
    convertLog.textContent = [result.stdout, result.stderr].filter(Boolean).join("\n") || "(no output)";

    if (result.output_path) {
      lastMddOutput = result.output_path;
      convertActions.hidden = false;
      await refreshDiagnosticSources();
    }
  } catch (error) {
    convertStatus.textContent = "Error";
    convertStatus.className = "result-status fail";
    convertLog.textContent = String(error);
  } finally {
    convertRunBtn.disabled = false;
    convertRunBtn.textContent = "Convert & save MDD";
  }
});


// ======================================================
// Send generated MDD to Parse tab
// ======================================================

convertSendToParse.addEventListener("click", () => {
  if (!lastMddOutput) {
    return;
  }

  selectedMdd = lastMddOutput;

  parseFilePath.textContent =
    selectedMdd;

  parseFilePath.classList.add(
    "has-file"
  );

  parseRunBtn.disabled = false;

  showTab("parse");
});


// ======================================================
// Parse MDD tab
// ======================================================

const parsePickBtn =
  document.getElementById("parse-pick-btn");

const parseFilePath =
  document.getElementById("parse-file-path");

const parseRunBtn =
  document.getElementById("parse-run-btn");

const parseResult =
  document.getElementById("parse-result");

const parseStatus =
  document.getElementById("parse-status");

const parseLog =
  document.getElementById("parse-log");

const jsonPanel =
  document.getElementById("json-panel");

const jsonView =
  document.getElementById("json-view");

const jsonSourceLabel =
  document.getElementById("json-source-label");

const saveJsonBtn =
  document.getElementById("save-json-btn");


let selectedMdd = null;
let lastJson = null;


// ======================================================
// Choose MDD
// ======================================================

parsePickBtn.addEventListener("click", async () => {
  try {

    const path =
      await invoke("pick_mdd_file");

    if (!path) {
      return;
    }

    selectedMdd = path;

    parseFilePath.textContent =
      path;

    parseFilePath.classList.add(
      "has-file"
    );

    parseRunBtn.disabled = false;

  } catch (error) {
    console.error(
      "Failed to select MDD:",
      error
    );
  }
});


// ======================================================
// Parse MDD
// ======================================================

parseRunBtn.addEventListener("click", async () => {
  if (!selectedMdd) {
    return;
  }

  parseRunBtn.disabled = true;
  parseRunBtn.textContent = "Running…";

  parseResult.hidden = false;
  jsonPanel.hidden = true;

  parseStatus.textContent = "";
  parseLog.textContent = "";

  try {

    const result =
      await invoke(
        "parse_mdd",
        {
          inputPath:
            selectedMdd
        }
      );

    const ok =
      result.success &&
      result.json != null;

    if (ok) {

      parseStatus.textContent =
        "Parsed successfully";

    } else if (result.success) {

      parseStatus.textContent =
        "Command ran, but no JSON was found in stdout or a sibling .json file";

    } else {

      parseStatus.textContent =
        "Parser failed";
    }

    parseStatus.className =
      "result-status " +
      (ok ? "ok" : "fail");

    parseLog.textContent =
      [result.stdout, result.stderr]
        .filter(Boolean)
        .join("\n") ||
      "(no output)";

    if (result.json != null) {

      lastJson =
        result.json;

      jsonView.innerHTML =
        renderJson(result.json);

      jsonPanel.hidden =
        false;

      if (
        result.json_source === "stdout"
      ) {

        jsonSourceLabel.textContent =
          "Parsed output (from stdout)";

      } else if (result.json_source) {

        jsonSourceLabel.textContent =
          `Parsed output (from ${result.json_source})`;

      } else {

        jsonSourceLabel.textContent =
          "Parsed output";
      }
    }

  } catch (error) {

    parseStatus.textContent =
      "Error";

    parseStatus.className =
      "result-status fail";

    parseLog.textContent =
      String(error);

  } finally {

    parseRunBtn.disabled = false;

    parseRunBtn.textContent =
      "Run parser";
  }
});


// ======================================================
// Save JSON
// ======================================================

saveJsonBtn.addEventListener("click", async () => {
  if (lastJson == null) {
    return;
  }

  const contents =
    JSON.stringify(
      lastJson,
      null,
      2
    );

  try {

    await invoke(
      "save_json_as",
      {
        contents
      }
    );

  } catch (error) {

    console.error(
      "Failed to save JSON:",
      error
    );
  }
});


// ======================================================
// JSON pretty renderer
// ======================================================

function renderJson(value) {
  const json =
    JSON.stringify(
      value,
      null,
      2
    );

  return json
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(
      /("(\\u[a-zA-Z0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g,
      (match) => {

        let cls =
          "json-number";

        if (/^"/.test(match)) {

          cls =
            /:$/.test(match)
              ? "json-key"
              : "json-string";

        } else if (
          /true|false/.test(match)
        ) {

          cls =
            "json-bool";

        } else if (
          /null/.test(match)
        ) {

          cls =
            "json-null";
        }

        return `<span class="${cls}">${match}</span>`;
      }
    );
}


// ======================================================
// Init
// ======================================================


// ======================================================
// UDS Request Builder (guided direct MDD reader)
// ======================================================

const udsPickJsonBtn = document.getElementById("uds-pick-json-btn");
const udsJsonPath = document.getElementById("uds-json-path");
const udsSourceStatus = document.getElementById("uds-source-status");
const udsServiceSelect = document.getElementById("uds-service-select");
const udsOperationBlock = document.getElementById("uds-operation-block");
const udsOperationSelect = document.getElementById("uds-operation-select");
const udsOperationHint = document.getElementById("uds-operation-hint");
const udsServiceSid = document.getElementById("uds-service-sid");
const udsPositiveSid = document.getElementById("uds-positive-sid");
const udsServiceDescription = document.getElementById("uds-service-description");
const udsParameters = document.getElementById("uds-parameters");
const udsRequestBytes = document.getElementById("uds-request-bytes");
const udsByteBreakdown = document.getElementById("uds-byte-breakdown");
const udsResetBtn = document.getElementById("uds-reset-btn");
const udsWriteBinBtn = document.getElementById("uds-send-request-btn");
const udsWriteStatus = document.getElementById("uds-write-status");
const udsAddSequenceBtn = document.getElementById("uds-add-sequence-btn");
const udsSelectedJson = document.getElementById("uds-selected-json");

let udsMddPath = null;
// Only the currently selected SID family is kept in the WebView.
// The complete MDD service tree stays cached in Rust to avoid a huge IPC/JSON transfer.
let udsMddServices = [];
let udsMddFamilies = [];
let udsMddServiceCount = 0;
let currentGeneratedRequest = "";
let currentGeneratedComplete = false;

const UDS_SERVICE_NAMES = {
  "10": "DiagnosticSessionControl",
  "11": "ECUReset",
  "14": "ClearDiagnosticInformation",
  "19": "ReadDTCInformation",
  "22": "ReadDataByIdentifier",
  "23": "ReadMemoryByAddress",
  "27": "SecurityAccess",
  "28": "CommunicationControl",
  "2E": "WriteDataByIdentifier",
  "2F": "InputOutputControlByIdentifier",
  "31": "RoutineControl",
  "34": "RequestDownload",
  "35": "RequestUpload",
  "36": "TransferData",
  "37": "RequestTransferExit",
  "3D": "WriteMemoryByAddress",
  "3E": "TesterPresent",
  "85": "ControlDTCSetting"
};

function normalizeHex(value) {
  return String(value ?? "")
    .replace(/0x/gi, "")
    .replace(/[^0-9a-f]/gi, "")
    .toUpperCase();
}

function hexToBytes(value) {
  const clean = normalizeHex(value);
  if (!clean) return [];
  const even = clean.length % 2 === 0 ? clean : `0${clean}`;
  return even.match(/.{1,2}/g) || [];
}

function udsFamilyName(sid) {
  const clean = normalizeHex(sid).padStart(2, "0").slice(-2);
  return UDS_SERVICE_NAMES[clean] || `UDS service 0x${clean}`;
}

function servicesForCurrentFamily() {
  return udsMddServices;
}

function currentMddService() {
  return udsMddServices.find((service) => service.id === udsOperationSelect.value) || null;
}

function fixedNonSidParams(service) {
  return (service.parameters || []).filter((parameter) =>
    parameter.fixed &&
    parameter.value_hex &&
    parameter.name.toUpperCase() !== "SID"
  );
}

function shortNameSuffix(service) {
  const name = String(service.name || "");
  const sid = normalizeHex(service.sid_hex).padStart(2, "0").slice(-2);
  // Common MDD names such as FV22_F180 / FS2F_0CC3_00.
  const pattern = new RegExp(`^[A-Z]+${sid}_`, "i");
  if (pattern.test(name)) {
    return name.replace(pattern, "").replaceAll("_", " / ");
  }
  return "";
}

function operationLabel(service) {
  const fixed = fixedNonSidParams(service);
  if (fixed.length) {
    const values = fixed
      .map((parameter) => `${parameter.name} 0x${normalizeHex(parameter.value_hex)}`)
      .join(" • ");
    const readable = service.long_name && service.long_name !== service.name
      ? ` — ${service.long_name}`
      : "";
    return `${values}${readable}`;
  }

  const suffix = shortNameSuffix(service);
  if (suffix) {
    const readable = service.long_name && service.long_name !== service.name
      ? ` — ${service.long_name}`
      : "";
    return `${suffix}${readable}`;
  }

  return service.long_name || service.name;
}

function renderUdsFamilyList() {
  udsServiceSelect.innerHTML = "";

  if (!udsMddFamilies.length) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "No UDS service with a detected SID";
    udsServiceSelect.appendChild(option);
    udsServiceSelect.disabled = true;
    udsOperationBlock.hidden = true;
    return;
  }

  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "Choose a UDS service…";
  udsServiceSelect.appendChild(placeholder);

  for (const family of udsMddFamilies) {
    const option = document.createElement("option");
    option.value = family.sid_hex;
    option.textContent = `0x${family.sid_hex} — ${family.name || udsFamilyName(family.sid_hex)} (${family.option_count})`;
    udsServiceSelect.appendChild(option);
  }

  udsServiceSelect.disabled = false;
  udsServiceSelect.value = "";
  udsOperationBlock.hidden = true;
}

async function loadCurrentFamilyOptions() {
  const sid = udsServiceSelect.value;
  udsMddServices = [];
  if (!sid || !udsMddPath) {
    renderOperationList();
    return;
  }

  udsOperationHint.textContent = `Loading ${udsFamilyName(sid)} options…`;
  udsOperationBlock.hidden = false;
  udsOperationSelect.disabled = true;

  try {
    const services = await invoke("get_uds_builder_family_options", {
      path: udsMddPath,
      sidHex: sid,
      ecuName: selectedBuilderEcu()
    });
    udsMddServices = Array.isArray(services) ? services : [];
    renderOperationList();
  } catch (error) {
    udsMddServices = [];
    udsOperationSelect.innerHTML = "";
    udsOperationSelect.disabled = true;
    udsOperationHint.textContent = `Failed to load service options: ${String(error)}`;
    clearCurrentService();
  }
}

function renderOperationList() {
  let services = servicesForCurrentFamily();
  udsOperationSelect.innerHTML = "";

  // Many MDDs contain both a generic service template (for example
  // "DiagnosticSessionControl") and concrete operations
  // ("StartDiagnosticSession.Default", "Reprog", ...).
  // The guided builder should present the concrete operations to the user,
  // not the low-level generic template. Keep the template only when it is
  // the only definition available for this SID.
  if (services.length > 1) {
    const familyName = udsFamilyName(udsServiceSelect.value).toLowerCase();
    const concrete = services.filter((service) => {
      const shortName = (service.name || "").trim().toLowerCase();
      const longName = (service.long_name || "").trim().toLowerCase();
      return shortName !== familyName && longName !== familyName;
    });
    if (concrete.length) services = concrete;
  }

  if (!udsServiceSelect.value || !services.length) {
    udsOperationBlock.hidden = true;
    udsOperationSelect.disabled = true;
    clearCurrentService();
    return;
  }

  udsOperationBlock.hidden = false;
  udsOperationSelect.disabled = false;

  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "Choose an available operation / identifier…";
  udsOperationSelect.appendChild(placeholder);

  const sorted = [...services].sort((a, b) => operationLabel(a).localeCompare(operationLabel(b)));
  for (const service of sorted) {
    const option = document.createElement("option");
    option.value = service.id;
    option.textContent = operationLabel(service);
    option.title = service.name;
    udsOperationSelect.appendChild(option);
  }

  udsOperationHint.textContent = `${services.length} definition(s) are available for ${udsFamilyName(udsServiceSelect.value)} in this MDD.`;
  clearCurrentService();
}

function clearCurrentService() {
  const sid = udsServiceSelect.value;
  udsServiceSid.textContent = sid ? `0x${sid}` : "--";
  udsPositiveSid.textContent = sid
    ? `0x${((parseInt(sid, 16) + 0x40) & 0xFF).toString(16).padStart(2, "0").toUpperCase()}`
    : "--";
  udsServiceDescription.textContent = sid
    ? `Choose one available ${udsFamilyName(sid)} operation.`
    : "Choose a UDS service first.";
  udsParameters.innerHTML = '<p class="hint">Choose an operation / identifier first.</p>';
  udsSelectedJson.textContent = "No operation selected.";
  udsRequestBytes.textContent = "--";
  udsByteBreakdown.innerHTML = '<p class="hint">No operation selected.</p>';
  currentGeneratedRequest = "";
  currentGeneratedComplete = false;
  if (udsWriteBinBtn) udsWriteBinBtn.disabled = !currentGeneratedComplete;
  if (udsAddSequenceBtn) udsAddSequenceBtn.disabled = true;
}

function parameterPathId(path) {
  return path.join("-");
}

const MAX_BUILDER_FIELD_BYTES = 1024 * 1024;

function expectedByteLength(parameter) {
  let value = null;
  if (parameter.byte_length != null) value = Number(parameter.byte_length);
  else if (parameter.bit_length != null) value = Math.ceil(Number(parameter.bit_length) / 8);
  if (value == null || !Number.isFinite(value) || value < 0) return null;
  if (value > MAX_BUILDER_FIELD_BYTES) return null;
  return value;
}

function parameterMetaText(parameter) {
  const parts = [parameter.param_type];
  if (parameter.byte_position != null) parts.push(`byte position: ${parameter.byte_position}`);
  if (parameter.bit_position != null && Number(parameter.bit_position) !== 0) parts.push(`bit position: ${parameter.bit_position}`);
  if (parameter.bit_length != null) {
    const bits = Number(parameter.bit_length);
    const bytes = Math.ceil(bits / 8);
    parts.push(bits % 8 === 0 ? `${bytes} byte${bytes === 1 ? "" : "s"} (${bits} bits)` : `${bits} bits`);
  } else if (parameter.byte_length != null) {
    const bytes = Number(parameter.byte_length);
    parts.push(`${bytes} byte${bytes === 1 ? "" : "s"}`);
  }
  if (parameter.data_type) parts.push(parameter.data_type);
  if (parameter.min_value != null || parameter.max_value != null) {
    const min = parameter.min_value ?? "−∞";
    const max = parameter.max_value ?? "+∞";
    parts.push(`range ${min}..${max}`);
  }
  if (parameter.unit) parts.push(`unit: ${parameter.unit}`);
  return parts.join(" • ");
}

function runtimePlaceholder(parameter) {
  const bytes = expectedByteLength(parameter);
  if (bytes === 1) return "Hex value, e.g. 20";
  if (bytes != null) return `${bytes} bytes in hex, e.g. ${Array.from({ length: Math.min(bytes, 4) }, () => "00").join(" ")}`;
  return "Hex value required at runtime";
}

function friendlyParameterName(parameter, service, depth = 0) {
  const raw = String(parameter.name || "Parameter");
  const sid = normalizeHex(service?.sid_hex || "").padStart(2, "0").slice(-2);

  if (/^Value_\d+$/i.test(raw) && parameter.fixed) {
    const names = {
      "10": "Diagnostic session type",
      "11": "Reset type",
      "27": "SecurityAccess sub-function",
      "28": "Communication control type",
      "31": "Routine control type",
      "3E": "TesterPresent sub-function",
      "85": "DTC setting type"
    };
    return names[sid] || "Fixed request value";
  }

  if (/^PA_ST_/i.test(raw)) {
    if (sid === "27") return "SecurityAccess request data";
    if (sid === "31") return "Routine option data";
    if (sid === "2E") return "Value to write";
    if (sid === "2F") return "I/O control data";
    return "Additional request data";
  }

  if (raw === "DataRecord") return "Request data";
  return raw;
}

function formatHexBytes(value) {
  const bytes = hexToBytes(value);
  return bytes.join(" ");
}

function validationMessage(parameter, rawValue) {
  const expected = expectedByteLength(parameter);
  const actual = hexToBytes(rawValue).length;
  if (expected == null) return actual ? `${actual} byte${actual === 1 ? "" : "s"} entered` : "Enter a hexadecimal value";
  if (actual === expected) return `✓ ${actual}/${expected} bytes — valid length`;
  return `${actual}/${expected} bytes entered — ${expected - actual > 0 ? `${expected - actual} more byte${expected - actual === 1 ? "" : "s"} required` : `${actual - expected} byte${actual - expected === 1 ? "" : "s"} too many`}`;
}

function renderParameterNode(parameter, path, depth = 0, service = null) {
  const wrapper = document.createElement("div");
  wrapper.className = depth > 0 ? "uds-param-row uds-param-child" : "uds-param-row";

  const label = document.createElement("label");
  label.className = "uds-label";
  label.textContent = friendlyParameterName(parameter, service, depth);
  wrapper.appendChild(label);

  if (parameter.fixed && parameter.value_hex) {
    const fixed = document.createElement("div");
    fixed.className = "uds-fixed-inline";
    fixed.innerHTML = `<span>Automatically from MDD</span><strong>0x${normalizeHex(parameter.value_hex)}</strong>`;
    wrapper.appendChild(fixed);
  } else if ((parameter.children || []).length) {
    const structureHint = document.createElement("div");
    structureHint.className = "uds-guidance";
    structureHint.textContent = parameter.input_hint || "Complete the variable fields below. Fixed bytes are inserted automatically.";
    wrapper.appendChild(structureHint);

    const children = document.createElement("div");
    children.className = "uds-structure-children";
    parameter.children.forEach((child, index) => {
      children.appendChild(renderParameterNode(child, [...path, index], depth + 1, service));
    });
    wrapper.appendChild(children);
  } else if ((parameter.choices || []).length) {
    const select = document.createElement("select");
    select.className = "uds-select uds-runtime-select";
    select.id = `uds-mdd-param-${parameterPathId(path)}`;
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Choose a value defined by the MDD…";
    select.appendChild(placeholder);
    parameter.choices.forEach((choice) => {
      const option = document.createElement("option");
      option.value = normalizeHex(choice.value_hex);
      option.textContent = `${choice.label} — 0x${normalizeHex(choice.value_hex)}`;
      select.appendChild(option);
    });
    select.addEventListener("change", updateMddRequest);
    wrapper.appendChild(select);
  } else {
    const control = document.createElement("input");
    control.type = "text";
    control.className = "uds-input";
    control.id = `uds-mdd-param-${parameterPathId(path)}`;
    control.value = parameter.value_hex || parameter.default_value_hex || "";
    control.placeholder = runtimePlaceholder(parameter);
    const validation = document.createElement("div");
    validation.className = "uds-input-validation";
    const refreshValidation = () => {
      validation.textContent = validationMessage(parameter, control.value);
      validation.classList.toggle("valid", expectedByteLength(parameter) == null || hexToBytes(control.value).length === expectedByteLength(parameter));
    };
    control.addEventListener("input", () => { refreshValidation(); updateMddRequest(); });
    control.addEventListener("blur", () => {
      if (control.value.trim()) control.value = formatHexBytes(control.value);
      refreshValidation();
      updateMddRequest();
    });
    wrapper.appendChild(control);
    refreshValidation();
    wrapper.appendChild(validation);
  }

  const meta = document.createElement("div");
  meta.className = "uds-param-meta";
  meta.textContent = parameterMetaText(parameter);
  wrapper.appendChild(meta);

  if (!parameter.fixed && parameter.input_hint) {
    const guide = document.createElement("div");
    guide.className = "uds-guidance";
    guide.textContent = parameter.input_hint;
    wrapper.appendChild(guide);
  }

  return wrapper;
}

function renderMddParameters(service) {
  udsParameters.innerHTML = "";

  const parameters = service.parameters || [];
  const nonSid = parameters.filter((parameter) => parameter.name.toUpperCase() !== "SID");
  const automatic = nonSid.filter((parameter) => parameter.fixed && parameter.value_hex);
  const remaining = nonSid.filter((parameter) => !parameter.fixed);

  if (automatic.length) {
    const summary = document.createElement("div");
    summary.className = "uds-auto-summary";
    summary.innerHTML = `<div class="uds-auto-title">Selected automatically from MDD</div>${automatic
      .map((parameter) => `<div class="uds-auto-value"><span>${friendlyParameterName(parameter, service)}</span><strong>0x${normalizeHex(parameter.value_hex)}</strong></div>`)
      .join("")}`;
    udsParameters.appendChild(summary);
  }

  if (!remaining.length) {
    const hint = document.createElement("p");
    hint.className = "hint";
    hint.textContent = "Nothing to type. Your selections completely define this UDS request.";
    udsParameters.appendChild(hint);
    return;
  }

  const intro = document.createElement("div");
  intro.className = "uds-runtime-intro";
  intro.innerHTML = `<strong>Runtime data still required</strong><span>The MDD cannot choose these values for you. The builder shows the expected size, type, range and allowed choices whenever that information exists.</span>`;
  udsParameters.appendChild(intro);

  remaining.forEach((parameter) => {
    const originalIndex = nonSid.indexOf(parameter);
    udsParameters.appendChild(renderParameterNode(parameter, [originalIndex], 0, service));
  });
}

function readControlHex(path) {
  const control = document.getElementById(`uds-mdd-param-${parameterPathId(path)}`);
  return control ? control.value : "";
}

function parameterCompletion(parameter, path) {
  if (parameter.fixed && parameter.value_hex) return { complete: true, bytes: hexToBytes(parameter.value_hex), reason: "" };

  if ((parameter.children || []).length) {
    const size = expectedByteLength(parameter) || (() => {
      let inferred = 0;
      parameter.children.forEach((child) => {
        const start = Number(child.byte_position || 0);
        const len = expectedByteLength(child) || 1;
        inferred = Math.max(inferred, start + len);
      });
      return inferred;
    })();
    if (!size) return { complete: false, bytes: [], reason: "Unknown or unsupported DataRecord length" };
    if (!Number.isFinite(size) || size < 0 || size > MAX_BUILDER_FIELD_BYTES) {
      return { complete: false, bytes: [], reason: `DataRecord length ${size} is outside the safe Builder limit` };
    }

    const buffer = Array(size).fill(0);
    for (let i = 0; i < parameter.children.length; i += 1) {
      const child = parameter.children[i];
      const result = parameterCompletion(child, [...path, i]);
      if (!result.complete) return result;
      const childBytes = result.bytes;
      const start = Number(child.byte_position || 0);
      const bitPos = Number(child.bit_position || 0);
      const bitLength = child.bit_length != null ? Number(child.bit_length) : childBytes.length * 8;

      if (bitPos !== 0 || bitLength < 8) {
        if (childBytes.length !== 1 || start >= buffer.length || bitLength > 8) {
          return { complete: false, bytes: [], reason: `${friendlyParameterName(child, currentMddService())}: unsupported bit layout` };
        }
        const mask = ((1 << bitLength) - 1) << bitPos;
        buffer[start] = (buffer[start] & (~mask & 0xFF)) | ((parseInt(childBytes[0], 16) << bitPos) & mask);
      } else {
        if (start + childBytes.length > buffer.length) {
          return { complete: false, bytes: [], reason: `${friendlyParameterName(child, currentMddService())}: value exceeds DataRecord size` };
        }
        childBytes.forEach((value, offset) => { buffer[start + offset] = parseInt(value, 16); });
      }
    }
    return { complete: true, bytes: buffer.map((value) => value.toString(16).padStart(2, "0").toUpperCase()), reason: "" };
  }

  const rawValue = readControlHex(path);
  const bytes = hexToBytes(rawValue);
  const expected = expectedByteLength(parameter);
  if (!bytes.length) {
    return { complete: false, bytes: [], reason: `${friendlyParameterName(parameter, currentMddService())}: value required` };
  }
  if (expected != null && bytes.length !== expected) {
    return { complete: false, bytes: [], reason: `${friendlyParameterName(parameter, currentMddService())}: expected ${expected} bytes, got ${bytes.length}` };
  }
  return { complete: true, bytes, reason: "" };
}

function parameterValueBytes(parameter, path) {
  const result = parameterCompletion(parameter, path);
  return result.complete ? result.bytes : [];
}

function updateMddRequest() {
  const service = currentMddService();
  if (!service) {
    currentGeneratedRequest = "";
    currentGeneratedComplete = false;
    if (udsWriteBinBtn) udsWriteBinBtn.disabled = !currentGeneratedComplete;
    if (udsAddSequenceBtn) udsAddSequenceBtn.disabled = true;
    udsRequestBytes.textContent = "--";
    udsByteBreakdown.innerHTML = '<p class="hint">No operation selected.</p>';
    return;
  }

  const parts = [];
  const missing = [];
  if (service.sid_hex) {
    parts.push({ order: 0, bytes: hexToBytes(service.sid_hex), label: `${udsFamilyName(service.sid_hex)} SID` });
  }

  const nonSid = (service.parameters || []).filter((parameter) => parameter.name.toUpperCase() !== "SID");
  nonSid.forEach((parameter, index) => {
    const result = parameterCompletion(parameter, [index]);
    if (!result.complete) {
      missing.push(result.reason);
      return;
    }
    parts.push({
      order: parameter.byte_position ?? 1000 + index,
      bytes: result.bytes,
      label: parameter.fixed ? `${friendlyParameterName(parameter, service)} (MDD)` : friendlyParameterName(parameter, service)
    });
  });

  parts.sort((a, b) => a.order - b.order);
  const allBytes = parts.flatMap((part) => part.bytes);

  currentGeneratedRequest = allBytes.join(" ");
  currentGeneratedComplete = missing.length === 0 && allBytes.length > 0;
  if (udsWriteBinBtn) udsWriteBinBtn.disabled = !currentGeneratedComplete;
  if (udsAddSequenceBtn) udsAddSequenceBtn.disabled = !currentGeneratedComplete;

  if (missing.length) {
    udsRequestBytes.innerHTML = `${allBytes.join(" ")} <span class="uds-request-incomplete">— incomplete</span>`;
  } else {
    udsRequestBytes.textContent = allBytes.length ? allBytes.join(" ") : "--";
  }

  udsByteBreakdown.innerHTML = "";
  parts.forEach((part) => {
    const row = document.createElement("div");
    row.className = "uds-byte-row";
    const value = document.createElement("div");
    value.className = "uds-byte-value";
    value.textContent = part.bytes.join(" ");
    const meaning = document.createElement("div");
    meaning.className = "uds-byte-meaning";
    meaning.textContent = part.label;
    row.appendChild(value);
    row.appendChild(meaning);
    udsByteBreakdown.appendChild(row);
  });

  if (missing.length) {
    const warn = document.createElement("div");
    warn.className = "uds-request-warning";
    warn.innerHTML = `<strong>Request is not complete yet.</strong><span>${missing.join(" • ")}</span>`;
    udsByteBreakdown.appendChild(warn);
  }
}

function renderCurrentMddService() {
  const service = currentMddService();

  if (!service) {
    clearCurrentService();
    return;
  }

  udsServiceSid.textContent = service.sid_hex ? `0x${service.sid_hex}` : "Not detected";
  udsPositiveSid.textContent = service.positive_sid_hex
    ? `0x${service.positive_sid_hex}`
    : "--";

  const displayName = service.long_name || service.name;
  udsServiceDescription.textContent = `${displayName} • MDD layer: ${service.source_layer}`;
  const serviceJson = JSON.stringify(service, null, 2);
  const maxJsonPreview = 100000;
  udsSelectedJson.textContent = serviceJson.length > maxJsonPreview
    ? `${serviceJson.slice(0, maxJsonPreview)}\n... preview truncated ...`
    : serviceJson;

  renderMddParameters(service);
  updateMddRequest();
}

async function syncDiagnosticToolsForSharedMdd(path, ecuName = "") {
  if (!path) return;
  try {
    // Validator and response decoder share exactly the same MDD and, when an
    // ECU is selected, the same ECU-scoped service definitions as the Builder.
    const services = ecuName
      ? await invoke("get_uds_services_for_ecu", { path, ecuName })
      : await invoke("get_uds_services_from_mdd", { path });
    await loadValidatorMdd(path, services);
    await loadResponseMdd(path, services);
  } catch (error) {
    if (validatorSourceStatus) {
      validatorSourceStatus.textContent = `Shared MDD could not be prepared for validation: ${String(error)}`;
      validatorSourceStatus.className = "result-status fail";
    }
    if (responseSourceStatus) {
      responseSourceStatus.textContent = `Shared MDD could not be prepared for response decoding: ${String(error)}`;
      responseSourceStatus.className = "result-status fail";
    }
  }

  try {
    if (dtcLoadedMddPath !== path) {
      await loadDtcDefinitions(path);
    }
  } catch (error) {
    if (dtcSourceStatus) {
      dtcSourceStatus.textContent = `Shared MDD could not be prepared for DTC decoding: ${String(error)}`;
      dtcSourceStatus.className = "result-status fail";
    }
  }
}

async function loadServicesForSelectedEcu() {
  const ecu = selectedBuilderEcu();

  udsMddServices = [];
  udsMddFamilies = [];
  renderUdsFamilyList();
  clearCurrentService();

  if (!udsMddPath) {
    udsSourceStatus.textContent = "Choose a diagnostic source first.";
    udsSourceStatus.className = "result-status";
    return;
  }

  if (!ecu) {
    udsSourceStatus.textContent = `${udsMddServiceCount} diagnostic definition(s) loaded. Choose an ECU to see only its supported UDS services.`;
    udsSourceStatus.className = "result-status";
    return;
  }

  udsSourceStatus.textContent = `Loading UDS services for ${ecu}…`;
  udsSourceStatus.className = "result-status";

  try {
    const summary = await invoke("get_uds_builder_ecu_summary", {
      path: udsMddPath,
      ecuName: ecu
    });
    udsMddFamilies = Array.isArray(summary.families) ? summary.families : [];
    renderUdsFamilyList();
    udsSourceStatus.textContent = `${summary.service_count || 0} diagnostic definition(s) available for ${ecu}, grouped into ${summary.family_count || udsMddFamilies.length} UDS service(s).`;
    udsSourceStatus.className = "result-status ok";
    await syncDiagnosticToolsForSharedMdd(udsMddPath, ecu);
  } catch (error) {
    udsMddFamilies = [];
    renderUdsFamilyList();
    udsSourceStatus.textContent = `Could not load services for ${ecu}: ${String(error)}`;
    udsSourceStatus.className = "result-status fail";
  }
}

async function loadMddForUds(path) {
  udsSourceStatus.textContent = "Reading MDD…";
  udsSourceStatus.className = "result-status";

  // IMPORTANT: do not transfer every service and every nested parameter to
  // JavaScript here. Rust caches the full MDD and returns only a tiny summary.
  const summary = await invoke("load_uds_builder_mdd", { path });

  udsMddPath = path;
  udsMddServices = [];
  // The initial MDD summary is global and is used only for the total count.
  // Service families shown to the user are loaded only after an ECU is selected.
  udsMddFamilies = [];
  udsMddServiceCount = Number(summary.service_count || 0);
  loadedMddEcuNames = Array.isArray(summary.ecu_names) ? summary.ecu_names.filter(Boolean) : [];

  // Keep the active workspace entry synchronized with the ECU names discovered
  // directly from the MDD. This fixes converted/imported MDDs whose workspace
  // metadata was empty even though the MDD contains diagnostic variants.
  const source = activeBuilderSource();
  if (source && loadedMddEcuNames.length) {
    source.ecu_names = [...loadedMddEcuNames];
  }

  udsJsonPath.textContent = path;
  udsJsonPath.classList.add("has-file");

  renderBuilderEcuOptions();
  renderUdsFamilyList();
  clearCurrentService();

  udsSourceStatus.textContent = `${udsMddServiceCount} diagnostic definition(s) loaded. Choose an ECU to filter the service list.`;
  udsSourceStatus.className = "result-status ok";

  // The MDD is selected once for all diagnostic tools. The actual service
  // lists are synchronized as soon as the user chooses an ECU.
  if (validatorMddPath) { validatorMddPath.textContent = path; validatorMddPath.classList.add("has-file"); }
  if (responseMddPath) { responseMddPath.textContent = path; responseMddPath.classList.add("has-file"); }
  if (dtcMddPath) { dtcMddPath.textContent = path; dtcMddPath.classList.add("has-file"); }
  if (validatorSourceStatus) validatorSourceStatus.textContent = "Shared MDD loaded. Choose an ECU in Request Builder.";
  if (responseSourceStatus) responseSourceStatus.textContent = "Shared MDD loaded. Choose an ECU in Request Builder.";
  if (dtcSourceStatus) dtcSourceStatus.textContent = "Shared MDD loaded. DTC definitions will be prepared automatically.";

  // Prepare all companion tools immediately. When an ECU is selected later,
  // validator/response definitions are narrowed to that ECU automatically.
  await syncDiagnosticToolsForSharedMdd(path);
}


udsPickJsonBtn.addEventListener("click", async () => {
  udsPickJsonBtn.disabled = true;
  udsPickJsonBtn.textContent = "Reading…";

  try {
    const path = await invoke("pick_mdd_file");
    if (!path) return;
    activeBuilderSourceId = "";
    loadedMddEcuNames = [];
    localStorage.removeItem("odx-active-builder-source");
    renderBuilderEcuOptions();
    await loadMddForUds(path);
    await loadServicesForSelectedEcu();
  } catch (error) {
    udsSourceStatus.textContent = `Failed to read MDD: ${String(error)}`;
    udsSourceStatus.className = "result-status fail";
  } finally {
    udsPickJsonBtn.disabled = false;
    udsPickJsonBtn.textContent = "Open external .mdd";
  }
});

udsServiceSelect.addEventListener("change", () => {
  loadCurrentFamilyOptions();
});
udsOperationSelect.addEventListener("change", renderCurrentMddService);
udsResetBtn.addEventListener("click", renderCurrentMddService);


// ======================================================
// Manual UDS frame validator and sequencer
// ======================================================

const validatorMddPath = document.getElementById("validator-mdd-path");
const validatorUseCurrentBtn = document.getElementById("validator-use-current-btn");
const validatorPickMddBtn = document.getElementById("validator-pick-mdd-btn");
const validatorSourceStatus = document.getElementById("validator-source-status");
const validatorFrameInput = document.getElementById("validator-frame-input");
const validatorRunBtn = document.getElementById("validator-run-btn");
const validatorAddSequenceBtn = document.getElementById("validator-add-sequence-btn");
const validatorResultCard = document.getElementById("validator-result-card");
const validatorStatus = document.getElementById("validator-status");
const validatorAnalysis = document.getElementById("validator-analysis");

const sequenceList = document.getElementById("sequence-list");
const sequenceSendAllBtn = document.getElementById("sequence-send-all-btn");
const sequenceWriteBtn = document.getElementById("sequence-write-btn");
const sequenceClearBtn = document.getElementById("sequence-clear-btn");
const sequenceStatus = document.getElementById("sequence-status");
const sequenceNameInput = document.getElementById("sequence-name-input");
const sequenceCreateBtn = document.getElementById("sequence-create-btn");
const sequenceSelector = document.getElementById("sequence-selector");
const sequenceDeleteBtn = document.getElementById("sequence-delete-btn");
const sequenceManagerStatus = document.getElementById("sequence-manager-status");
const sequenceCurrentTitle = document.getElementById("sequence-current-title");
const sequenceTargetDialog = document.getElementById("sequence-target-dialog");
const sequenceTargetSelect = document.getElementById("sequence-target-select");
const sequenceTargetConfirm = document.getElementById("sequence-target-confirm");

let validatorMddServices = [];
let validatorMddLoadedPath = null;
let lastValidatorResult = null;
let udsSequences = [];
let activeSequenceId = null;
let pendingSequenceFrame = null;
let sequenceSaveTimer = null;
let sequenceStateLoaded = false;
let sequenceSaveQueue = Promise.resolve();

function sequenceStatePayload() {
  return {
    format_version: 1,
    active_sequence_id: activeSequenceId,
    sequences: udsSequences.map((sequence) => ({
      id: sequence.id,
      name: sequence.name,
      frames: sequence.frames.map((entry) => ({
        id: entry.id,
        frame: entry.frame,
        label: entry.label,
        source: entry.source,
        timeout_ms: Number(entry.timeout_ms || 2000),
        delay_ms: Number(entry.delay_ms || 0),
        runtime_ms: Number(entry.runtime_ms || 0),
        stop_on_nrc: Boolean(entry.stop_on_nrc),
        continue_on_failure: Boolean(entry.continue_on_failure),
        expected_positive_sid: entry.expected_positive_sid || null,
        condition_type: entry.condition_type || "always",
        condition_value: entry.condition_value || null
      }))
    }))
  };
}

function saveSequenceStateNow() {
  if (!sequenceStateLoaded) return sequenceSaveQueue;
  const state = sequenceStatePayload();
  sequenceSaveQueue = sequenceSaveQueue
    .catch(() => {})
    .then(() => invoke("save_uds_sequence_state", { state }))
    .catch((error) => console.error("Could not save sequencer state:", error));
  return sequenceSaveQueue;
}

function scheduleSequenceSave() {
  if (!sequenceStateLoaded) return;
  clearTimeout(sequenceSaveTimer);
  sequenceSaveTimer = setTimeout(saveSequenceStateNow, 100);
}

function normalizeLoadedSequenceState(state) {
  if (!state || !Array.isArray(state.sequences)) return;
  udsSequences = state.sequences.map((sequence) => ({
    id: sequence.id || `${Date.now()}-${Math.random().toString(16).slice(2)}`,
    name: String(sequence.name || "Sequence"),
    frames: Array.isArray(sequence.frames) ? sequence.frames.map((entry, index) => ({
      id: entry.id || `${Date.now()}-${Math.random().toString(16).slice(2)}`,
      frame: entry.frame || "",
      label: entry.label || `UDS frame ${index + 1}`,
      source: entry.source || "Manual",
      timeout_ms: Number(entry.timeout_ms || 2000),
      delay_ms: Number(entry.delay_ms || 0),
      runtime_ms: Number(entry.runtime_ms || 0),
      stop_on_nrc: entry.stop_on_nrc !== false,
      continue_on_failure: Boolean(entry.continue_on_failure),
      expected_positive_sid: entry.expected_positive_sid || "",
      condition_type: entry.condition_type || "always",
      condition_value: entry.condition_value || ""
    })) : []
  }));
  activeSequenceId = state.active_sequence_id || udsSequences[0]?.id || null;
  if (!udsSequences.some((sequence) => sequence.id === activeSequenceId)) {
    activeSequenceId = udsSequences[0]?.id || null;
  }
}

async function initializeSequencer() {
  try {
    const state = await invoke("load_uds_sequence_state");
    normalizeLoadedSequenceState(state);
  } catch (error) {
    console.error("Could not load saved sequences:", error);
    sequenceManagerStatus.textContent = `Could not load saved sequences: ${String(error)}`;
    sequenceManagerStatus.className = "result-status fail";
  } finally {
    sequenceStateLoaded = true;
    renderSequenceManager();
    renderSequence();
  }
}

function strictHexBytes(value) {
  const raw = String(value ?? "").trim();
  if (!raw) return { ok: false, bytes: [], error: "The frame is empty." };
  const withoutPrefixes = raw.replace(/0x/gi, "");
  if (/[^0-9a-fA-F\s,;:_-]/.test(withoutPrefixes)) {
    return { ok: false, bytes: [], error: "The frame contains a non-hexadecimal character." };
  }
  const clean = withoutPrefixes.replace(/[^0-9a-fA-F]/g, "");
  if (!clean) return { ok: false, bytes: [], error: "The frame is empty." };
  if (clean.length % 2 !== 0) {
    return { ok: false, bytes: [], error: "A hexadecimal byte needs exactly two digits. The frame has an odd number of hex digits." };
  }
  return { ok: true, bytes: (clean.match(/.{2}/g) || []).map((b) => b.toUpperCase()), error: "" };
}

function mergeFixedByte(layout, index, value, mask = 0xFF, label = "Fixed MDD value") {
  if (index < 0) return;
  layout.length = Math.max(layout.length, index + 1);
  while (layout.expected.length <= index) {
    layout.expected.push(0);
    layout.mask.push(0);
    layout.labels.push("");
  }
  layout.expected[index] = (layout.expected[index] & (~mask & 0xFF)) | (value & mask);
  layout.mask[index] = layout.mask[index] | mask;
  if (label) layout.labels[index] = label;
}

function occupyParameter(layout, parameter, baseOffset, service, path = []) {
  const localPos = parameter.byte_position != null ? Number(parameter.byte_position) : 0;
  const start = baseOffset + localPos;
  const bytesLen = expectedByteLength(parameter);
  if (bytesLen != null) layout.length = Math.max(layout.length, start + bytesLen);

  if (parameter.fixed && parameter.value_hex) {
    const fixedBytes = hexToBytes(parameter.value_hex);
    const bitPos = Number(parameter.bit_position || 0);
    const bitLength = parameter.bit_length != null ? Number(parameter.bit_length) : fixedBytes.length * 8;
    if ((bitPos !== 0 || bitLength < 8) && fixedBytes.length === 1 && bitLength <= 8) {
      const mask = ((1 << bitLength) - 1) << bitPos;
      mergeFixedByte(layout, start, parseInt(fixedBytes[0], 16) << bitPos, mask, friendlyParameterName(parameter, service));
    } else {
      fixedBytes.forEach((byte, offset) => {
        mergeFixedByte(layout, start + offset, parseInt(byte, 16), 0xFF, friendlyParameterName(parameter, service));
      });
    }
  }

  if ((parameter.children || []).length) {
    parameter.children.forEach((child, index) => {
      occupyParameter(layout, child, start, service, [...path, index]);
    });
  }
}

function serviceValidationLayout(service) {
  const layout = { length: 1, expected: [parseInt(service.sid_hex || "00", 16)], mask: [0xFF], labels: [`${udsFamilyName(service.sid_hex)} SID`] };
  const params = service.parameters || [];
  params.forEach((parameter) => {
    if (String(parameter.name || "").toUpperCase() === "SID") return;
    occupyParameter(layout, parameter, 0, service);
  });
  return layout;
}

function compareFrameToService(frameBytes, service) {
  const layout = serviceValidationLayout(service);
  const issues = [];
  let fixedMatches = 0;
  let fixedCompared = 0;

  const compareCount = Math.min(frameBytes.length, layout.mask.length);
  for (let i = 0; i < compareCount; i += 1) {
    const mask = layout.mask[i] || 0;
    if (!mask) continue;
    fixedCompared += 1;
    const actual = parseInt(frameBytes[i], 16);
    const expected = layout.expected[i] || 0;
    if ((actual & mask) === (expected & mask)) {
      fixedMatches += 1;
    } else {
      const expectedHex = expected.toString(16).padStart(2, "0").toUpperCase();
      issues.push(`Byte ${i}: expected 0x${expectedHex}${layout.labels[i] ? ` (${layout.labels[i]})` : ""}, received 0x${frameBytes[i]}.`);
    }
  }

  if (frameBytes.length < layout.length) {
    issues.push(`Frame is too short: expected ${layout.length} byte${layout.length === 1 ? "" : "s"}, received ${frameBytes.length}. Missing ${layout.length - frameBytes.length} byte${layout.length - frameBytes.length === 1 ? "" : "s"}.`);
  } else if (frameBytes.length > layout.length) {
    issues.push(`Frame is too long for this MDD definition: expected ${layout.length} bytes, received ${frameBytes.length}.`);
  }

  const score = fixedMatches * 10 - issues.length * 2 - Math.abs(frameBytes.length - layout.length);
  return { service, layout, issues, valid: issues.length === 0, score, fixedMatches, fixedCompared };
}

function validateManualFrame(frameText) {
  const parsed = strictHexBytes(frameText);
  if (!parsed.ok) return { valid: false, parseError: parsed.error, bytes: [], candidate: null };
  if (!validatorMddServices.length) return { valid: false, parseError: "Load an MDD file before validating the frame.", bytes: parsed.bytes, candidate: null };

  const sid = parsed.bytes[0];
  const candidates = validatorMddServices.filter((service) => normalizeHex(service.sid_hex).padStart(2, "0").slice(-2) === sid);
  if (!candidates.length) {
    return { valid: false, parseError: `SID 0x${sid} is not defined as a request service in the loaded MDD.`, bytes: parsed.bytes, candidate: null };
  }

  const comparisons = candidates.map((service) => compareFrameToService(parsed.bytes, service));
  const exact = comparisons.find((result) => result.valid);
  const best = exact || comparisons.sort((a, b) => b.score - a.score)[0];
  return { valid: Boolean(exact), parseError: "", bytes: parsed.bytes, candidate: best, candidateCount: candidates.length };
}

function renderValidatorResult(result) {
  validatorResultCard.hidden = false;
  lastValidatorResult = result;
  validatorAddSequenceBtn.disabled = !result.valid;

  if (result.parseError) {
    validatorStatus.className = "validator-status fail";
    validatorStatus.textContent = `✕ ${result.parseError}`;
    validatorAnalysis.innerHTML = "";
    return;
  }

  const candidate = result.candidate;
  const service = candidate.service;
  const frame = result.bytes.join(" ");
  validatorStatus.className = result.valid ? "validator-status ok" : "validator-status fail";
  validatorStatus.textContent = result.valid ? "✓ Valid UDS request according to this MDD definition" : "✕ Frame does not match the closest MDD definition";

  const issueHtml = candidate.issues.length
    ? `<div class="validator-issues"><strong>Problems found</strong><ul>${candidate.issues.map((issue) => `<li>${issue}</li>`).join("")}</ul></div>`
    : '<div class="validator-ok-box">All fixed bytes and the expected request length match the MDD.</div>';

  const fixedRows = [];
  candidate.layout.mask.forEach((mask, index) => {
    if (!mask || index >= result.bytes.length) return;
    fixedRows.push(`<div class="uds-byte-row"><div class="uds-byte-value">${result.bytes[index]}</div><div class="uds-byte-meaning">Byte ${index} • ${candidate.layout.labels[index] || "MDD fixed field"}</div></div>`);
  });

  validatorAnalysis.innerHTML = `
    <div class="validator-summary-grid">
      <div><span class="uds-kicker">Frame</span><code>${frame}</code></div>
      <div><span class="uds-kicker">Detected service</span><strong>0x${service.sid_hex} — ${udsFamilyName(service.sid_hex)}</strong></div>
      <div><span class="uds-kicker">Closest operation</span><strong>${service.long_name || service.name}</strong></div>
      <div><span class="uds-kicker">Expected length</span><strong>${candidate.layout.length} byte${candidate.layout.length === 1 ? "" : "s"}</strong></div>
    </div>
    ${issueHtml}
    <div class="uds-breakdown"><div class="uds-panel-title">Recognized fixed fields</div>${fixedRows.join("") || '<p class="hint">No additional fixed field available for this generic definition.</p>'}</div>
  `;
}

async function loadValidatorMdd(path, services = null) {
  validatorSourceStatus.textContent = "Reading MDD…";
  validatorSourceStatus.className = "result-status";
  validatorMddServices = services || await invoke("get_uds_services_from_mdd", { path });
  validatorMddLoadedPath = path;
  validatorMddPath.textContent = path;
  validatorMddPath.classList.add("has-file");
  validatorSourceStatus.textContent = `${validatorMddServices.length} diagnostic definitions ready for frame validation.`;
  validatorSourceStatus.className = "result-status ok";
}

validatorUseCurrentBtn?.addEventListener("click", async () => {
  if (!udsMddPath) {
    validatorSourceStatus.textContent = "No shared MDD is currently loaded in Diagnostics.";
    validatorSourceStatus.className = "result-status fail";
    return;
  }
  await loadValidatorMdd(udsMddPath);
});

validatorPickMddBtn?.addEventListener("click", async () => {
  try {
    const path = await invoke("pick_mdd_file");
    if (!path) return;
    await loadValidatorMdd(path);
  } catch (error) {
    validatorSourceStatus.textContent = `Failed to read MDD: ${String(error)}`;
    validatorSourceStatus.className = "result-status fail";
  }
});

validatorRunBtn.addEventListener("click", () => {
  const result = validateManualFrame(validatorFrameInput.value);
  renderValidatorResult(result);
});

validatorFrameInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") validatorRunBtn.click();
});

function defaultPositiveSidForFrame(frame) {
  const parsed = strictHexBytes(frame);
  if (!parsed.ok || !parsed.bytes.length) return "";
  return ((parseInt(parsed.bytes[0], 16) + 0x40) & 0xFF).toString(16).padStart(2, "0").toUpperCase();
}

function createSequence(name) {
  const cleanName = String(name || "").trim();
  if (!cleanName) return null;
  const sequence = {
    id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
    name: cleanName,
    frames: []
  };
  udsSequences.push(sequence);
  activeSequenceId = sequence.id;
  renderSequenceManager();
  renderSequence();
  scheduleSequenceSave();
  return sequence;
}

function activeSequence() {
  return udsSequences.find((sequence) => sequence.id === activeSequenceId) || null;
}

function ensureSequenceExists() {
  if (udsSequences.length) return;
  createSequence("Sequence 1");
}

function frameEntry(frame, label, source, index) {
  const parsed = strictHexBytes(frame);
  if (!parsed.ok) return null;
  return {
    id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
    frame: parsed.bytes.join(" "),
    label: label || `UDS frame ${index + 1}`,
    source: source || "Manual",
    timeout_ms: 2000,
    delay_ms: 0,
    runtime_ms: 0,
    stop_on_nrc: true,
    continue_on_failure: false,
    expected_positive_sid: defaultPositiveSidForFrame(frame),
    condition_type: "always",
    condition_value: ""
  };
}

function addFrameToSequence(sequenceId, frame, label, source) {
  const sequence = udsSequences.find((item) => item.id === sequenceId);
  if (!sequence) return false;
  const entry = frameEntry(frame, label, source, sequence.frames.length);
  if (!entry) return false;
  sequence.frames.push(entry);
  activeSequenceId = sequence.id;
  renderSequenceManager();
  renderSequence();
  scheduleSequenceSave();
  return true;
}

function requestSequenceTarget(frame, label, source) {
  ensureSequenceExists();
  pendingSequenceFrame = { frame, label, source };
  sequenceTargetSelect.innerHTML = udsSequences
    .map((sequence) => `<option value="${sequence.id}" ${sequence.id === activeSequenceId ? "selected" : ""}>${sequence.name}</option>`)
    .join("");
  sequenceTargetDialog.showModal();
}

function conditionNeedsValue(type) {
  return type === "previous_sid" || type === "previous_nrc";
}

function sequenceConditionLabel(entry) {
  const value = entry.condition_value ? ` 0x${normalizeHex(entry.condition_value).padStart(2, "0")}` : "";
  const labels = {
    always: "Always run",
    previous_positive: "Only if previous response is positive",
    previous_negative: "Only if previous response is negative",
    previous_sid: `Only if previous response SID is${value}`,
    previous_nrc: `Only if previous NRC is${value}`
  };
  return labels[entry.condition_type] || "Always run";
}


function renderSequenceManager() {
  sequenceSelector.innerHTML = "";
  if (!udsSequences.length) {
    const option = document.createElement("option");
    option.textContent = "No sequence created";
    option.value = "";
    sequenceSelector.appendChild(option);
    sequenceSelector.disabled = true;
    sequenceDeleteBtn.disabled = true;
    return;
  }

  if (!udsSequences.some((sequence) => sequence.id === activeSequenceId)) {
    activeSequenceId = udsSequences[0].id;
  }

  sequenceSelector.disabled = false;
  sequenceDeleteBtn.disabled = false;
  udsSequences.forEach((sequence) => {
    const option = document.createElement("option");
    option.value = sequence.id;
    option.textContent = `${sequence.name} (${sequence.frames.length} frame${sequence.frames.length === 1 ? "" : "s"})`;
    option.selected = sequence.id === activeSequenceId;
    sequenceSelector.appendChild(option);
  });
}

function renderSequence() {
  const sequence = activeSequence();
  sequenceList.innerHTML = "";
  const hasFrames = Boolean(sequence?.frames.length);
  const hasSequences = udsSequences.length > 0;
  sequenceSendAllBtn.disabled = !hasFrames;
  sequenceWriteBtn.disabled = !hasFrames;
  sequenceClearBtn.disabled = !hasFrames;
  sequenceCurrentTitle.textContent = sequence ? `Frames — ${sequence.name}` : "Frames";

  if (!sequence) {
    sequenceList.innerHTML = '<p class="hint">Create a sequence, then add requests from UDS Builder or Frame Validator.</p>';
    return;
  }
  if (!sequence.frames.length) {
    sequenceList.innerHTML = '<p class="hint">This sequence is empty. Add a generated or validated frame.</p>';
    return;
  }

  sequence.frames.forEach((entry, index) => {
    const row = document.createElement("div");
    row.className = "sequence-row sequence-row-config";
    row.innerHTML = `
      <div class="sequence-index">${index + 1}</div>
      <div class="sequence-content sequence-content-config">
        <strong>${entry.label}</strong>
        <code>${entry.frame}</code>
        <span>${entry.source}</span>
        <div class="sequence-config-grid">
          <label>Timeout (ms)<input class="uds-input seq-timeout" type="number" min="1" value="${entry.timeout_ms}"></label>
          <label>Delay after step (ms)<input class="uds-input seq-delay" type="number" min="0" value="${entry.delay_ms}"></label>
          <label>Last runtime (ms)<input class="uds-input" type="number" value="${entry.runtime_ms || 0}" readonly></label>
          <label>Expected positive SID<input class="uds-input seq-positive-sid" type="text" maxlength="4" value="${entry.expected_positive_sid || ""}" placeholder="e.g. 71"></label>
          <label>Run condition
            <select class="uds-input seq-condition">
              <option value="always" ${entry.condition_type === "always" ? "selected" : ""}>Always</option>
              <option value="previous_positive" ${entry.condition_type === "previous_positive" ? "selected" : ""}>Previous response positive</option>
              <option value="previous_negative" ${entry.condition_type === "previous_negative" ? "selected" : ""}>Previous response negative</option>
              <option value="previous_sid" ${entry.condition_type === "previous_sid" ? "selected" : ""}>Previous response SID equals</option>
              <option value="previous_nrc" ${entry.condition_type === "previous_nrc" ? "selected" : ""}>Previous NRC equals</option>
            </select>
          </label>
          <label class="seq-condition-value-wrap" ${conditionNeedsValue(entry.condition_type) ? "" : "hidden"}>Condition hex value<input class="uds-input seq-condition-value" type="text" maxlength="4" value="${entry.condition_value || ""}" placeholder="e.g. 67 or 22"></label>
        </div>
        <div class="sequence-checks">
          <label><input class="seq-stop-nrc" type="checkbox" ${entry.stop_on_nrc ? "checked" : ""}> Stop sequence on NRC</label>
          <label><input class="seq-continue-failure" type="checkbox" ${entry.continue_on_failure ? "checked" : ""}> Continue on timeout/unexpected response</label>
        </div>
        <div class="sequence-condition-summary">${sequenceConditionLabel(entry)}</div>
      </div>
      <div class="sequence-actions">
        <button class="btn small sequence-up" type="button" ${index === 0 ? "disabled" : ""}>↑</button>
        <button class="btn small sequence-down" type="button" ${index === sequence.frames.length - 1 ? "disabled" : ""}>↓</button>
        <button class="btn small sequence-remove" type="button">Remove</button>
      </div>`;

    const syncEntry = () => {
      entry.timeout_ms = Math.max(1, Number(row.querySelector(".seq-timeout").value || 2000));
      entry.delay_ms = Math.max(0, Number(row.querySelector(".seq-delay").value || 0));
      entry.expected_positive_sid = normalizeHex(row.querySelector(".seq-positive-sid").value).slice(-2);
      entry.condition_type = row.querySelector(".seq-condition").value;
      entry.condition_value = normalizeHex(row.querySelector(".seq-condition-value").value).slice(-2);
      entry.stop_on_nrc = row.querySelector(".seq-stop-nrc").checked;
      entry.continue_on_failure = row.querySelector(".seq-continue-failure").checked;
      row.querySelector(".seq-condition-value-wrap").hidden = !conditionNeedsValue(entry.condition_type);
      row.querySelector(".sequence-condition-summary").textContent = sequenceConditionLabel(entry);
      scheduleSequenceSave();
    };
    row.querySelectorAll("input, select").forEach((control) => control.addEventListener("change", syncEntry));
    row.querySelectorAll("input[type=text]").forEach((control) => control.addEventListener("input", syncEntry));
    row.querySelector(".sequence-up").addEventListener("click", () => {
      [sequence.frames[index - 1], sequence.frames[index]] = [sequence.frames[index], sequence.frames[index - 1]];
      renderSequence();
      scheduleSequenceSave();
    });
    row.querySelector(".sequence-down").addEventListener("click", () => {
      [sequence.frames[index + 1], sequence.frames[index]] = [sequence.frames[index], sequence.frames[index + 1]];
      renderSequence();
      scheduleSequenceSave();
    });
    row.querySelector(".sequence-remove").addEventListener("click", () => {
      sequence.frames.splice(index, 1);
      renderSequenceManager();
      renderSequence();
      scheduleSequenceSave();
    });
    sequenceList.appendChild(row);
  });
}

sequenceCreateBtn.addEventListener("click", () => {
  const name = sequenceNameInput.value.trim();
  if (!name) {
    sequenceManagerStatus.textContent = "Enter a sequence name.";
    sequenceManagerStatus.className = "result-status fail";
    return;
  }
  const sequence = createSequence(name);
  sequenceNameInput.value = "";
  sequenceManagerStatus.textContent = `Sequence created: ${sequence.name}`;
  sequenceManagerStatus.className = "result-status ok";
});

sequenceNameInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") sequenceCreateBtn.click();
});

sequenceSelector.addEventListener("change", () => {
  activeSequenceId = sequenceSelector.value || null;
  renderSequence();
  scheduleSequenceSave();
});

sequenceDeleteBtn.addEventListener("click", () => {
  const sequence = activeSequence();
  if (!sequence) return;
  udsSequences = udsSequences.filter((item) => item.id !== sequence.id);
  activeSequenceId = udsSequences[0]?.id || null;
  sequenceManagerStatus.textContent = `Sequence deleted: ${sequence.name}`;
  sequenceManagerStatus.className = "result-status";
  renderSequenceManager();
  renderSequence();
  scheduleSequenceSave();
});

sequenceTargetConfirm.addEventListener("click", () => {
  if (!pendingSequenceFrame) return;
  const targetId = sequenceTargetSelect.value;
  if (addFrameToSequence(targetId, pendingSequenceFrame.frame, pendingSequenceFrame.label, pendingSequenceFrame.source)) {
    const sequence = udsSequences.find((item) => item.id === targetId);
    sequenceStatus.textContent = `Frame added to ${sequence?.name || "sequence"}.`;
    sequenceStatus.className = "result-status ok";
  }
  pendingSequenceFrame = null;
  sequenceTargetDialog.close();
});

sequenceTargetDialog.addEventListener("close", () => {
  if (sequenceTargetDialog.returnValue === "cancel") pendingSequenceFrame = null;
});

if (udsBuilderEcuSelect) {
  udsBuilderEcuSelect.addEventListener("change", async () => {
    const source = activeBuilderSource();
    if (source && udsBuilderEcuSelect.value) {
      localStorage.setItem(builderEcuStorageKey(source.id), udsBuilderEcuSelect.value);
    }
    loadCurrentEcuDoipProfile();
    updateWorkspaceContextCard();
    await loadServicesForSelectedEcu();
  });
}

[udsDoipHost, udsDoipPort, udsDoipSourceAddress, udsDoipTargetAddress].forEach((input) => {
  input?.addEventListener("change", saveCurrentEcuDoipProfile);
});

if (udsWriteBinBtn) {
  udsWriteBinBtn.addEventListener("click", async () => {
    const frameToSend = currentGeneratedComplete ? currentGeneratedRequest : "";
    const targetEcu = selectedBuilderEcu();
    const source = activeBuilderSource();

    if (!source) {
      udsWriteStatus.textContent = "Choose a saved diagnostic source in Diagnostics first.";
      udsWriteStatus.className = "result-status fail";
      return;
    }
    if (!targetEcu) {
      udsWriteStatus.textContent = "Choose the target ECU in the Builder before sending.";
      udsWriteStatus.className = "result-status fail";
      return;
    }
    if (!frameToSend) {
      udsWriteStatus.textContent = "Complete the UDS service request before sending.";
      udsWriteStatus.className = "result-status fail";
      return;
    }

    const host = udsDoipHost?.value.trim() || "";
    const port = Number(udsDoipPort?.value || 0);
    const sourceAddress = udsDoipSourceAddress?.value.trim() || "";
    const targetAddress = udsDoipTargetAddress?.value.trim() || "";
    if (!host || !Number.isInteger(port) || port < 1 || port > 65535 || !sourceAddress || !targetAddress) {
      udsWriteStatus.textContent = `Complete the DoIP connection for ${targetEcu}: server, port, tester logical address and ECU logical address.`;
      udsWriteStatus.className = "result-status fail";
      return;
    }

    saveCurrentEcuDoipProfile();
    udsWriteBinBtn.disabled = true;
    udsWriteStatus.textContent = `Sending ${frameToSend} → ${targetEcu} (${host}:${port}, ECU ${targetAddress})…`;
    udsWriteStatus.className = "result-status";

    try {
      const result = await invoke("send_uds_request_doip", {
        frame: frameToSend,
        host,
        port,
        sourceAddress,
        targetAddress,
        timeoutMs: 3000,
        logContext: `Builder send → ${targetEcu}`
      });
      udsWriteStatus.textContent = `Response from ${targetEcu}: ${result.response || "(empty)"} · ${result.transport || "DoIP"} · ${result.acknowledgement || "ACK"}`;
      udsWriteStatus.className = "result-status ok";
    } catch (error) {
      udsWriteStatus.textContent = `Send to ${targetEcu} failed: ${String(error)}`;
      udsWriteStatus.className = "result-status fail";
    } finally {
      udsWriteBinBtn.disabled = false;
    }
  });
}

if (udsAddSequenceBtn) {
  udsAddSequenceBtn.addEventListener("click", () => {
    if (!currentGeneratedComplete || !currentGeneratedRequest) return;
    const service = currentMddService();
    requestSequenceTarget(currentGeneratedRequest, service?.long_name || service?.name || "Generated UDS request", "UDS Builder");
  });
}

validatorAddSequenceBtn.addEventListener("click", () => {
  if (!lastValidatorResult?.valid) return;
  const service = lastValidatorResult.candidate.service;
  requestSequenceTarget(lastValidatorResult.bytes.join(" "), service.long_name || service.name || "Validated UDS request", "Frame Validator");
});


function parseSequenceResponseMeta(responseText) {
  const bytes = normalizeHex(responseText || "").match(/.{1,2}/g) || [];
  const first = bytes[0] || "";
  const negative = first === "7F";
  return {
    bytes,
    positive: Boolean(first) && !negative,
    negative,
    sid: first,
    nrc: negative ? (bytes[2] || "") : ""
  };
}

function sequenceConditionAllows(entry, previousMeta) {
  switch (entry.condition_type) {
    case "previous_positive":
      return Boolean(previousMeta?.positive);
    case "previous_negative":
      return Boolean(previousMeta?.negative);
    case "previous_sid":
      return Boolean(previousMeta) &&
        previousMeta.sid === normalizeHex(entry.condition_value || "").slice(-2);
    case "previous_nrc":
      return Boolean(previousMeta) &&
        previousMeta.nrc === normalizeHex(entry.condition_value || "").slice(-2);
    default:
      return true;
  }
}

async function appendSendAllLog(message) {
  try {
    return await invoke("write_uds_transport_log", { message });
  } catch (error) {
    console.error("Could not append UDS transport log:", error);
    return "";
  }
}

sequenceSendAllBtn?.addEventListener("click", async () => {
  const sequence = activeSequence();
  const targetEcu = selectedBuilderEcu();
  const source = activeBuilderSource();

  if (!sequence || !sequence.frames.length) return;
  if (!source) {
    sequenceStatus.textContent = "Load a diagnostic MDD in Diagnostics before using Send all.";
    sequenceStatus.className = "result-status fail";
    return;
  }
  if (!targetEcu) {
    sequenceStatus.textContent = "Choose the target ECU in Diagnostics before using Send all.";
    sequenceStatus.className = "result-status fail";
    return;
  }

  const host = udsDoipHost?.value.trim() || "";
  const port = Number(udsDoipPort?.value || 0);
  const sourceAddress = udsDoipSourceAddress?.value.trim() || "";
  const targetAddress = udsDoipTargetAddress?.value.trim() || "";
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535 || !sourceAddress || !targetAddress) {
    sequenceStatus.textContent = `Complete the DoIP connection for ${targetEcu} before using Send all.`;
    sequenceStatus.className = "result-status fail";
    return;
  }

  saveCurrentEcuDoipProfile();
  sequenceSendAllBtn.disabled = true;
  sequenceWriteBtn.disabled = true;
  sequenceClearBtn.disabled = true;

  const runId = `${sequence.name} @ ${new Date().toISOString()}`;
  let logPath = await appendSendAllLog(`========== SEND ALL START | ${runId} | ECU=${targetEcu} | ${host}:${port} ==========`);

  let previousMeta = null;
  let completed = 0;
  let stopped = false;

  try {
    for (let index = 0; index < sequence.frames.length; index += 1) {
      const entry = sequence.frames[index];
      const stepNo = index + 1;
      const context = `Send all "${sequence.name}" step ${stepNo}/${sequence.frames.length} - ${entry.label}`;

      if (!sequenceConditionAllows(entry, previousMeta)) {
        await appendSendAllLog(`SKIP | ${context} | condition=${sequenceConditionLabel(entry)}`);
        continue;
      }

      sequenceStatus.textContent = `Sending ${stepNo}/${sequence.frames.length}: ${entry.label}…`;
      sequenceStatus.className = "result-status";

      const started = performance.now();
      try {
        const result = await invoke("send_uds_request_doip", {
          frame: entry.frame,
          host,
          port,
          sourceAddress,
          targetAddress,
          timeoutMs: Math.max(100, Number(entry.timeout_ms || 3000)),
          logContext: context
        });

        entry.runtime_ms = Math.round(performance.now() - started);
        previousMeta = parseSequenceResponseMeta(result.response || "");
        completed += 1;

        const expectedSid = normalizeHex(entry.expected_positive_sid || "").slice(-2);
        const unexpectedPositive = expectedSid &&
          previousMeta.positive &&
          previousMeta.sid !== expectedSid;

        if (unexpectedPositive) {
          await appendSendAllLog(
            `CHECK FAIL | ${context} | expected positive SID=${expectedSid} | received=${previousMeta.sid}`
          );
          if (!entry.continue_on_failure) {
            stopped = true;
            break;
          }
        }

        if (previousMeta.negative && entry.stop_on_nrc) {
          await appendSendAllLog(`STOP | ${context} | NRC=${previousMeta.nrc || "unknown"}`);
          stopped = true;
          break;
        }
      } catch (error) {
        entry.runtime_ms = Math.round(performance.now() - started);
        previousMeta = null;
        await appendSendAllLog(`STEP ERROR | ${context} | ${String(error)}`);
        if (!entry.continue_on_failure) {
          stopped = true;
          break;
        }
      }

      renderSequence();
      scheduleSequenceSave();

      const delayMs = Math.max(0, Number(entry.delay_ms || 0));
      if (delayMs > 0 && index < sequence.frames.length - 1) {
        await new Promise((resolve) => setTimeout(resolve, delayMs));
      }
    }
  } finally {
    logPath = await appendSendAllLog(
      `========== SEND ALL END | ${runId} | completed=${completed}/${sequence.frames.length} | stopped=${stopped} ==========`
    ) || logPath;

    renderSequence();
    scheduleSequenceSave();
    sequenceSendAllBtn.disabled = !activeSequence()?.frames.length;
    sequenceWriteBtn.disabled = !activeSequence()?.frames.length;
    sequenceClearBtn.disabled = !activeSequence()?.frames.length;
  }

  sequenceStatus.textContent = stopped
    ? `Send all stopped after ${completed} completed step(s). Log: ${logPath || "uds_transport.log"}`
    : `Send all completed ${completed} step(s). Log: ${logPath || "uds_transport.log"}`;
  sequenceStatus.className = stopped ? "result-status fail" : "result-status ok";
});

sequenceClearBtn.addEventListener("click", () => {
  const sequence = activeSequence();
  if (!sequence) return;
  sequence.frames = [];
  sequenceStatus.textContent = `Cleared ${sequence.name}.`;
  sequenceStatus.className = "result-status";
  renderSequenceManager();
  renderSequence();
  scheduleSequenceSave();
});

function selectedSequenceExportPayload() {
  const sequence = activeSequence();
  if (!sequence) return null;
  return {
    name: sequence.name,
    frames: sequence.frames.map((entry) => ({
      frame: entry.frame
    }))
  };
}

sequenceWriteBtn.addEventListener("click", async () => {
  const sequence = activeSequence();
  const payload = selectedSequenceExportPayload();
  if (!sequence || !payload || !sequence.frames.length) return;

  sequenceWriteBtn.disabled = true;
  sequenceStatus.textContent = `Exporting ${sequence.name}…`;
  sequenceStatus.className = "result-status";
  try {
    const path = await invoke("export_uds_sequence", { sequence: payload });
    if (path) {
      sequenceStatus.textContent = `Sequence exported successfully: ${path}`;
      sequenceStatus.className = "result-status ok";
    } else {
      sequenceStatus.textContent = "Export cancelled.";
      sequenceStatus.className = "result-status";
    }
  } catch (error) {
    sequenceStatus.textContent = `Failed to export sequence: ${String(error)}`;
    sequenceStatus.className = "result-status fail";
  } finally {
    sequenceWriteBtn.disabled = !activeSequence()?.frames.length;
  }
});


initializeSequencer();
refreshDiagnosticSources();
refreshProjectOutputFolder();

// ======================================================
// UDS response decoder
// ======================================================
const responseMddPath = document.getElementById("response-mdd-path");
const responseUseCurrentBtn = document.getElementById("response-use-current-btn");
const responsePickMddBtn = document.getElementById("response-pick-mdd-btn");
const responseSourceStatus = document.getElementById("response-source-status");
const responseFrameInput = document.getElementById("response-frame-input");
const responseDecodeBtn = document.getElementById("response-decode-btn");
const responseResultCard = document.getElementById("response-result-card");
const responseStatus = document.getElementById("response-status");
const responseAnalysis = document.getElementById("response-analysis");
let responseMddServices = [];

const NRC_NAMES = {
  "10": "GeneralReject", "11": "ServiceNotSupported", "12": "SubFunctionNotSupported",
  "13": "IncorrectMessageLengthOrInvalidFormat", "14": "ResponseTooLong", "21": "BusyRepeatRequest",
  "22": "ConditionsNotCorrect", "24": "RequestSequenceError", "25": "NoResponseFromSubnetComponent",
  "26": "FailurePreventsExecutionOfRequestedAction", "31": "RequestOutOfRange", "33": "SecurityAccessDenied",
  "35": "InvalidKey", "36": "ExceededNumberOfAttempts", "37": "RequiredTimeDelayNotExpired",
  "70": "UploadDownloadNotAccepted", "71": "TransferDataSuspended", "72": "GeneralProgrammingFailure",
  "73": "WrongBlockSequenceCounter", "78": "RequestCorrectlyReceivedResponsePending",
  "7E": "SubFunctionNotSupportedInActiveSession", "7F": "ServiceNotSupportedInActiveSession",
  "81": "RpmTooHigh", "82": "RpmTooLow", "83": "EngineIsRunning", "84": "EngineIsNotRunning",
  "85": "EngineRunTimeTooLow", "86": "TemperatureTooHigh", "87": "TemperatureTooLow",
  "88": "VehicleSpeedTooHigh", "89": "VehicleSpeedTooLow", "8A": "ThrottlePedalTooHigh",
  "8B": "ThrottlePedalTooLow", "8C": "TransmissionRangeNotInNeutral", "8D": "TransmissionRangeNotInGear",
  "8F": "BrakeSwitchNotClosed", "90": "ShifterLeverNotInPark", "91": "TorqueConverterClutchLocked",
  "92": "VoltageTooHigh", "93": "VoltageTooLow"
};

async function loadResponseMdd(path, services = null) {
  responseSourceStatus.textContent = "Reading MDD…";
  responseSourceStatus.className = "result-status";
  responseMddServices = services || await invoke("get_uds_services_from_mdd", { path });
  responseMddPath.textContent = path;
  responseMddPath.classList.add("has-file");
  responseSourceStatus.textContent = `${responseMddServices.length} diagnostic definitions ready for response decoding.`;
  responseSourceStatus.className = "result-status ok";
}

responseUseCurrentBtn?.addEventListener("click", async () => {
  if (!udsMddPath) {
    responseSourceStatus.textContent = "Load the shared MDD in Diagnostics first.";
    responseSourceStatus.className = "result-status fail";
    return;
  }
  await loadResponseMdd(udsMddPath);
});

responsePickMddBtn?.addEventListener("click", async () => {
  try {
    const path = await invoke("pick_mdd_file");
    if (path) await loadResponseMdd(path);
  } catch (error) {
    responseSourceStatus.textContent = `Failed to read MDD: ${String(error)}`;
    responseSourceStatus.className = "result-status fail";
  }
});

function responseParameterRows(parameter, bytes, baseOffset = 0, service = null, depth = 0) {
  const localPos = parameter.byte_position != null ? Number(parameter.byte_position) : 0;
  const start = baseOffset + localPos;
  const rows = [];
  if ((parameter.children || []).length) {
    parameter.children.forEach((child) => rows.push(...responseParameterRows(child, bytes, start, service, depth + 1)));
    return rows;
  }
  const length = expectedByteLength(parameter);
  const actual = length != null ? bytes.slice(start, start + length) : [];
  const fixed = parameter.fixed && parameter.value_hex ? normalizeHex(parameter.value_hex) : null;
  rows.push({
    name: friendlyParameterName(parameter, service),
    rawName: parameter.name,
    start,
    length,
    actual: actual.join(" "),
    fixed,
    type: parameter.data_type || parameter.param_type,
    depth
  });
  return rows;
}

function decodeResponseFrame(frameText) {
  const parsed = strictHexBytes(frameText);
  if (!parsed.ok) return { ok: false, error: parsed.error };
  if (!responseMddServices.length) return { ok: false, error: "Load an MDD before decoding the response." };
  const bytes = parsed.bytes;
  if (bytes[0] === "7F") {
    if (bytes.length < 3) return { ok: false, error: "A negative response needs at least 3 bytes: 7F + requested SID + NRC." };
    const requestedSid = bytes[1];
    const nrc = bytes[2];
    const services = responseMddServices.filter((service) => normalizeHex(service.sid_hex).padStart(2, "0").slice(-2) === requestedSid);
    return { ok: true, negative: true, bytes, requestedSid, nrc, nrcName: NRC_NAMES[nrc] || "Unknown / manufacturer-specific NRC", services };
  }
  const positiveSid = bytes[0];
  const candidates = responseMddServices.filter((service) => normalizeHex(service.positive_sid_hex).padStart(2, "0").slice(-2) === positiveSid);
  if (!candidates.length) return { ok: false, error: `Positive response SID 0x${positiveSid} is not associated with a service in the loaded MDD.` };
  let best = candidates[0];
  let bestScore = -Infinity;
  candidates.forEach((service) => {
    const params = service.positive_parameters || [];
    let score = 0;
    params.forEach((parameter) => {
      if (!parameter.fixed || !parameter.value_hex) return;
      const start = parameter.byte_position != null ? Number(parameter.byte_position) : 0;
      const fixed = hexToBytes(parameter.value_hex);
      fixed.forEach((value, offset) => { if (bytes[start + offset] === value) score += 5; else score -= 3; });
    });
    if (score > bestScore) { bestScore = score; best = service; }
  });
  const rows = (best.positive_parameters || []).flatMap((parameter) => responseParameterRows(parameter, bytes, 0, best));
  return { ok: true, negative: false, bytes, positiveSid, service: best, candidateCount: candidates.length, rows };
}

function renderResponseDecode(result) {
  responseResultCard.hidden = false;
  if (!result.ok) {
    responseStatus.className = "validator-status fail";
    responseStatus.textContent = `✕ ${result.error}`;
    responseAnalysis.innerHTML = "";
    return;
  }
  if (result.negative) {
    responseStatus.className = "validator-status fail";
    responseStatus.textContent = `Negative response — NRC 0x${result.nrc} ${result.nrcName}`;
    const serviceNames = result.services.length ? result.services.slice(0, 5).map((s) => s.long_name || s.name).join("<br>") : "No matching service definition";
    responseAnalysis.innerHTML = `
      <div class="validator-summary-grid">
        <div><span class="uds-kicker">Response</span><code>${result.bytes.join(" ")}</code></div>
        <div><span class="uds-kicker">0x7F</span><strong>NegativeResponse</strong></div>
        <div><span class="uds-kicker">Requested SID</span><strong>0x${result.requestedSid} — ${udsFamilyName(result.requestedSid)}</strong></div>
        <div><span class="uds-kicker">NRC</span><strong>0x${result.nrc} — ${result.nrcName}</strong></div>
      </div>
      <div class="uds-note"><strong>MDD service candidates</strong><span>${serviceNames}</span></div>`;
    return;
  }
  responseStatus.className = "validator-status ok";
  responseStatus.textContent = "✓ Positive UDS response decoded";
  const service = result.service;
  const rows = result.rows.filter((row) => row.actual || row.fixed).map((row) => `
    <div class="uds-byte-row">
      <div class="uds-byte-value">${row.actual || "—"}</div>
      <div class="uds-byte-meaning"><strong>${row.name}</strong> • byte ${row.start}${row.length ? ` • ${row.length} byte${row.length === 1 ? "" : "s"}` : ""} • ${row.type || "parameter"}${row.fixed ? ` • MDD fixed 0x${row.fixed}` : ""}</div>
    </div>`).join("");
  responseAnalysis.innerHTML = `
    <div class="validator-summary-grid">
      <div><span class="uds-kicker">Response</span><code>${result.bytes.join(" ")}</code></div>
      <div><span class="uds-kicker">Positive SID</span><strong>0x${result.positiveSid}</strong></div>
      <div><span class="uds-kicker">Request service</span><strong>0x${service.sid_hex} — ${udsFamilyName(service.sid_hex)}</strong></div>
      <div><span class="uds-kicker">Operation</span><strong>${service.long_name || service.name}</strong></div>
    </div>
    <div class="uds-breakdown"><div class="uds-panel-title">Response fields from MDD</div>${rows || '<p class="hint">The response SID is recognized, but this MDD response does not expose additional elementary fields that this lightweight reader can display.</p>'}</div>`;
}

responseDecodeBtn?.addEventListener("click", () => renderResponseDecode(decodeResponseFrame(responseFrameInput.value)));
responseFrameInput?.addEventListener("keydown", (event) => { if (event.key === "Enter") responseDecodeBtn.click(); });

// ======================================================
// DTC & snapshot decoder (UDS ReadDTCInformation 0x19)
// ======================================================

const dtcMddPath = document.getElementById("dtc-mdd-path");
const dtcUseCurrentBtn = document.getElementById("dtc-use-current-btn");
const dtcPickMddBtn = document.getElementById("dtc-pick-mdd-btn");
const dtcSourceStatus = document.getElementById("dtc-source-status");
const dtcFrameInput = document.getElementById("dtc-frame-input");
const dtcDecodeBtn = document.getElementById("dtc-decode-btn");
const dtcResultCard = document.getElementById("dtc-result-card");
const dtcStatus = document.getElementById("dtc-status");
const dtcAnalysis = document.getElementById("dtc-analysis");

let dtcDefinitions = [];
let dtcLoadedMddPath = null;

const DTC_SUBFUNCTION_NAMES = {
  "01": "reportNumberOfDTCByStatusMask",
  "02": "reportDTCByStatusMask",
  "03": "reportDTCSnapshotIdentification",
  "04": "reportDTCSnapshotRecordByDTCNumber",
  "05": "reportDTCStoredDataByRecordNumber",
  "06": "reportDTCExtDataRecordByDTCNumber",
  "07": "reportNumberOfDTCBySeverityMaskRecord",
  "08": "reportDTCBySeverityMaskRecord",
  "09": "reportSeverityInformationOfDTC",
  "0A": "reportSupportedDTC",
  "0B": "reportFirstTestFailedDTC",
  "0C": "reportFirstConfirmedDTC",
  "0D": "reportMostRecentTestFailedDTC",
  "0E": "reportMostRecentConfirmedDTC",
  "0F": "reportMirrorMemoryDTCByStatusMask",
  "10": "reportMirrorMemoryDTCExtDataRecordByDTCNumber",
  "11": "reportNumberOfMirrorMemoryDTCByStatusMask",
  "12": "reportNumberOfEmissionsOBDDTCByStatusMask",
  "13": "reportEmissionsOBDDTCByStatusMask",
  "14": "reportDTCFaultDetectionCounter",
  "15": "reportDTCWithPermanentStatus",
  "16": "reportDTCExtDataRecordByRecordNumber",
  "17": "reportUserDefMemoryDTCByStatusMask",
  "18": "reportUserDefMemoryDTCSnapshotRecordByDTCNumber",
  "19": "reportUserDefMemoryDTCExtDataRecordByDTCNumber",
  "1A": "reportSupportedDTCExtDataRecord"
};

const DTC_STATUS_BITS = [
  [0x01, "testFailed", "The diagnostic test currently reports the fault."],
  [0x02, "testFailedThisOperationCycle", "The test failed during the current operation cycle."],
  [0x04, "pendingDTC", "The fault is pending according to ECU diagnostic logic."],
  [0x08, "confirmedDTC", "The DTC has reached the ECU's confirmed state."],
  [0x10, "testNotCompletedSinceLastClear", "The diagnostic test has not completed since DTC information was cleared."],
  [0x20, "testFailedSinceLastClear", "The test has failed at least once since DTC information was cleared."],
  [0x40, "testNotCompletedThisOperationCycle", "The test has not completed during this operation cycle."],
  [0x80, "warningIndicatorRequested", "The ECU requests a warning indicator for this fault."],
];

function dtcEscape(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function dtcByte(value) {
  return Number(value ?? 0) & 0xFF;
}

function dtcHexByte(value) {
  return dtcByte(value).toString(16).toUpperCase().padStart(2, "0");
}

function dtcCodeHex(bytes, offset) {
  if (offset + 2 >= bytes.length) return null;
  return `${bytes[offset]}${bytes[offset + 1]}${bytes[offset + 2]}`.toUpperCase();
}

function findDtcDefinition(codeHex) {
  const key = normalizeHex(codeHex).padStart(6, "0").slice(-6);
  return dtcDefinitions.find((item) => normalizeHex(item.code_hex).padStart(6, "0").slice(-6) === key) || null;
}

async function loadDtcDefinitions(path) {
  if (!path) throw new Error("No MDD file was selected.");
  dtcSourceStatus.textContent = "Reading DTC definitions from MDD…";
  dtcSourceStatus.className = "result-status";
  const definitions = await invoke("get_dtc_definitions_from_mdd", { path });
  dtcDefinitions = definitions || [];
  dtcLoadedMddPath = path;
  dtcMddPath.textContent = path;
  dtcMddPath.classList.add("has-file");
  dtcSourceStatus.textContent = `${dtcDefinitions.length} DTC definition(s) found in the MDD.`;
  dtcSourceStatus.className = "result-status ok";
}

dtcUseCurrentBtn?.addEventListener("click", async () => {
  if (!udsMddPath) {
    dtcSourceStatus.textContent = "Load the shared MDD in Diagnostics first.";
    dtcSourceStatus.className = "result-status fail";
    return;
  }
  try {
    await loadDtcDefinitions(udsMddPath);
  } catch (error) {
    dtcSourceStatus.textContent = `Failed to read DTC definitions: ${String(error)}`;
    dtcSourceStatus.className = "result-status fail";
  }
});

dtcPickMddBtn?.addEventListener("click", async () => {
  try {
    const path = await invoke("pick_mdd_file");
    if (!path) return;
    await loadDtcDefinitions(path);
  } catch (error) {
    dtcSourceStatus.textContent = `Failed to read DTC definitions: ${String(error)}`;
    dtcSourceStatus.className = "result-status fail";
  }
});

function decodeDtcStatusByte(status) {
  return DTC_STATUS_BITS.map(([mask, name, description]) => ({
    mask,
    name,
    description,
    active: (status & mask) !== 0,
  }));
}

function dtcDefinitionHtml(codeHex) {
  const definition = findDtcDefinition(codeHex);
  if (!definition) {
    return `<div class="dtc-definition"><strong>DTC 0x${dtcEscape(codeHex)}</strong><p class="hint">No matching DTC definition was found in the loaded MDD.</p></div>`;
  }
  const display = definition.display_trouble_code || `0x${codeHex}`;
  const name = definition.short_name || "Unnamed DTC";
  const description = definition.description || "No textual problem description is present in the MDD definition.";
  return `
    <div class="dtc-definition">
      <div class="uds-panel-title">DTC problem</div>
      <div class="dtc-kv"><span>Code</span><strong>${dtcEscape(display)} <small>(0x${dtcEscape(codeHex)})</small></strong></div>
      <div class="dtc-kv"><span>Name</span><strong>${dtcEscape(name)}</strong></div>
      <div class="dtc-description">${dtcEscape(description)}</div>
    </div>`;
}

function dtcStatusHtml(status) {
  const flags = decodeDtcStatusByte(status);
  return `
    <div class="uds-breakdown">
      <div class="uds-panel-title">DTC status — 0x${dtcHexByte(status)}</div>
      <div class="dtc-status-grid">
        ${flags.map((flag) => `
          <div class="dtc-flag ${flag.active ? "active" : "inactive"}">
            <strong>${flag.active ? "✓" : "○"} ${dtcEscape(flag.name)}</strong>
            <span>${dtcEscape(flag.description)}</span>
          </div>`).join("")}
      </div>
    </div>`;
}

function dtcRecordHtml(codeHex, status, title = "DTC record") {
  return `
    <div class="dtc-record">
      <div class="uds-panel-title">${dtcEscape(title)}</div>
      ${dtcDefinitionHtml(codeHex)}
      ${dtcStatusHtml(status)}
    </div>`;
}

function decodeDtcFrame(raw) {
  const parsed = strictHexBytes(raw);
  if (!parsed.ok) return { ok: false, error: parsed.error };
  const bytesHex = parsed.bytes;
  const bytes = bytesHex.map((value) => parseInt(value, 16));
  if (bytes[0] !== 0x59) {
    if (bytes[0] === 0x7F && bytes[1] === 0x19) {
      return { ok: false, error: `Negative ReadDTCInformation response: NRC 0x${dtcHexByte(bytes[2] || 0)}.` };
    }
    return { ok: false, error: `DTC Decoder expects a positive ReadDTCInformation response beginning with 0x59. Received 0x${dtcHexByte(bytes[0])}.` };
  }
  if (bytes.length < 2) return { ok: false, error: "The 0x59 response is missing its sub-function byte." };

  const sub = bytes[1];
  const subHex = dtcHexByte(sub);
  const name = DTC_SUBFUNCTION_NAMES[subHex] || `ReadDTCInformation sub-function 0x${subHex}`;
  const result = { ok: true, sub, subHex, name, bytes, kind: "generic" };

  if (sub === 0x01 && bytes.length >= 6) {
    result.kind = "count";
    result.statusAvailabilityMask = bytes[2];
    result.dtcFormatIdentifier = bytes[3];
    result.dtcCount = (bytes[4] << 8) | bytes[5];
    return result;
  }

  if ([0x02, 0x0A, 0x0F, 0x13, 0x15].includes(sub)) {
    result.kind = "dtc-list";
    result.statusAvailabilityMask = bytes[2] ?? null;
    result.records = [];
    for (let i = 3; i + 3 < bytes.length; i += 4) {
      const codeHex = dtcCodeHex(bytesHex, i);
      if (!codeHex) break;
      result.records.push({ codeHex, status: bytes[i + 3] });
    }
    result.trailing = bytesHex.slice(3 + result.records.length * 4);
    return result;
  }

  if (sub === 0x03) {
    result.kind = "snapshot-identification";
    result.records = [];
    for (let i = 2; i + 3 < bytes.length; i += 4) {
      const codeHex = dtcCodeHex(bytesHex, i);
      if (!codeHex) break;
      result.records.push({ codeHex, recordNumber: bytes[i + 3] });
    }
    result.trailing = bytesHex.slice(2 + result.records.length * 4);
    return result;
  }

  if (sub === 0x04 && bytes.length >= 8) {
    result.kind = "snapshot";
    result.codeHex = dtcCodeHex(bytesHex, 2);
    result.status = bytes[5];
    result.recordNumber = bytes[6];
    result.numberOfIdentifiers = bytes[7];
    result.payload = bytesHex.slice(8);
    return result;
  }

  if (sub === 0x06 && bytes.length >= 7) {
    result.kind = "extended-data";
    result.codeHex = dtcCodeHex(bytesHex, 2);
    result.status = bytes[5];
    result.recordNumber = bytes[6];
    result.payload = bytesHex.slice(7);
    return result;
  }

  if ([0x09, 0x0B, 0x0C, 0x0D, 0x0E].includes(sub) && bytes.length >= 6) {
    result.kind = "single-dtc";
    result.codeHex = dtcCodeHex(bytesHex, 2);
    result.status = bytes[5];
    result.payload = bytesHex.slice(6);
    return result;
  }

  result.payload = bytesHex.slice(2);
  return result;
}

function renderDtcDecode(result) {
  dtcResultCard.hidden = false;
  if (!result.ok) {
    dtcStatus.className = "validator-status fail";
    dtcStatus.textContent = `✕ ${result.error}`;
    dtcAnalysis.innerHTML = "";
    return;
  }

  dtcStatus.className = "validator-status ok";
  dtcStatus.textContent = `✓ 0x59 ${result.name} decoded`;

  let body = `
    <div class="uds-breakdown">
      <div class="uds-panel-title">ReadDTCInformation response</div>
      <div class="dtc-kv"><span>Positive SID</span><strong>0x59</strong></div>
      <div class="dtc-kv"><span>Sub-function</span><strong>0x${result.subHex} — ${dtcEscape(result.name)}</strong></div>
      <div class="dtc-kv"><span>Raw frame</span><strong>${result.bytes.map(dtcHexByte).join(" ")}</strong></div>
    </div>`;

  if (result.kind === "count") {
    body += `
      <div class="uds-breakdown">
        <div class="uds-panel-title">DTC count</div>
        <div class="dtc-kv"><span>Status availability mask</span><strong>0x${dtcHexByte(result.statusAvailabilityMask)}</strong></div>
        <div class="dtc-kv"><span>DTC format identifier</span><strong>0x${dtcHexByte(result.dtcFormatIdentifier)}</strong></div>
        <div class="dtc-kv"><span>Number of DTCs</span><strong>${result.dtcCount}</strong></div>
      </div>`;
  } else if (result.kind === "dtc-list") {
    body += `<div class="uds-breakdown"><div class="uds-panel-title">DTC records (${result.records.length})</div>`;
    if (result.statusAvailabilityMask != null) {
      body += `<div class="dtc-kv"><span>Status availability mask</span><strong>0x${dtcHexByte(result.statusAvailabilityMask)}</strong></div>`;
    }
    body += result.records.length
      ? result.records.map((record, i) => dtcRecordHtml(record.codeHex, record.status, `DTC #${i + 1}`)).join("")
      : `<p class="hint">No complete 4-byte DTC record was found after the response header.</p>`;
    body += `</div>`;
  } else if (result.kind === "snapshot-identification") {
    body += `<div class="uds-breakdown"><div class="uds-panel-title">Available snapshot records</div>`;
    body += result.records.length ? result.records.map((record) => `
      <div class="dtc-record">
        ${dtcDefinitionHtml(record.codeHex)}
        <div class="dtc-kv"><span>Snapshot record number</span><strong>0x${dtcHexByte(record.recordNumber)}</strong></div>
      </div>`).join("") : `<p class="hint">No complete DTC/snapshot identification record found.</p>`;
    body += `</div>`;
  } else if (result.kind === "snapshot") {
    body += dtcRecordHtml(result.codeHex, result.status, "DTC snapshot");
    body += `
      <div class="uds-breakdown">
        <div class="uds-panel-title">Snapshot record</div>
        <div class="dtc-kv"><span>Record number</span><strong>0x${dtcHexByte(result.recordNumber)}</strong></div>
        <div class="dtc-kv"><span>Number of identifiers</span><strong>${result.numberOfIdentifiers}</strong></div>
        <div class="dtc-kv"><span>Raw snapshot payload</span><strong>${result.payload.join(" ") || "—"}</strong></div>
        <p class="hint">The record number is shown exactly as returned by the ECU. The decoder does not hard-code record 01/02/03 as first occurrence, latest occurrence or recovery because that mapping is ECU/project specific. If the MDD exposes that mapping, it should be used instead.</p>
      </div>`;
  } else if (result.kind === "extended-data") {
    body += dtcRecordHtml(result.codeHex, result.status, "DTC extended data");
    body += `
      <div class="uds-breakdown">
        <div class="uds-panel-title">Extended data record</div>
        <div class="dtc-kv"><span>Record number</span><strong>0x${dtcHexByte(result.recordNumber)}</strong></div>
        <div class="dtc-kv"><span>Raw extended-data payload</span><strong>${result.payload.join(" ") || "—"}</strong></div>
      </div>`;
  } else if (result.kind === "single-dtc") {
    body += dtcRecordHtml(result.codeHex, result.status, "DTC");
    if (result.payload?.length) {
      body += `<div class="dtc-kv"><span>Remaining payload</span><strong>${result.payload.join(" ")}</strong></div>`;
    }
  } else {
    body += `
      <div class="uds-breakdown">
        <div class="uds-panel-title">Raw sub-function payload</div>
        <p class="hint">This ReadDTCInformation sub-function is recognized at service level, but no specialized layout decoder is implemented yet.</p>
        <div class="dtc-kv"><span>Payload</span><strong>${(result.payload || []).join(" ") || "—"}</strong></div>
      </div>`;
  }

  if (!dtcLoadedMddPath) {
    body += `<p class="hint">Load an MDD to resolve DTC codes into their diagnostic names and problem descriptions.</p>`;
  }
  dtcAnalysis.innerHTML = body;
}

dtcDecodeBtn?.addEventListener("click", () => {
  renderDtcDecode(decodeDtcFrame(dtcFrameInput.value));
});

dtcFrameInput?.addEventListener("keydown", (event) => {
  if (event.key === "Enter") dtcDecodeBtn?.click();
});
