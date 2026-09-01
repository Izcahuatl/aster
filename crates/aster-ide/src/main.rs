use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, SystemTime},
};

use full_moon::node::Node;

use aster_analysis::{
    AnalysisOptions, Diagnostic, LookupExplanation, LookupResult, analyze, check, explain,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use walkdir::WalkDir;

struct AppState(Mutex<Option<PathBuf>>);
struct RunState(Mutex<Option<Child>>);

#[derive(Serialize, Deserialize, Clone)]
struct WorkspaceInfo {
    name: String,
    root: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct WorkspaceFile {
    path: String,
    contents: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct FileTreeNode {
    name: String,
    path: String,
    is_dir: bool,
    children: Vec<FileTreeNode>,
    size: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct RecentProject {
    name: String,
    path: String,
    last_opened: u64,
    pinned: bool,
    git_branch: Option<String>,
    exists: bool,
}

#[derive(Deserialize)]
struct SaveRequest {
    path: String,
    contents: String,
}

#[derive(Deserialize)]
struct PathRequest {
    path: String,
}

#[derive(Deserialize)]
struct RenameRequest {
    old_path: String,
    new_path: String,
}

#[derive(Deserialize)]
struct ExplainRequest {
    path: String,
    line: usize,
    column: usize,
}

#[derive(Serialize)]
struct DiagnosticDto {
    kind: String,
    suggestion: bool,
    file: String,
    line: Option<usize>,
    column: Option<usize>,
    message: String,
}

#[derive(Serialize)]
struct ExplanationDto {
    expression: String,
    line: usize,
    column: usize,
    steps: Vec<String>,
    result: String,
}

#[derive(Serialize)]
struct ClassStructureDto {
    name: String,
    file: String,
    metatable_name: Option<String>,
    parent_constructor: Option<String>,
    index_self: bool,
    index_other: Option<String>,
    instance_members: Vec<String>,
    methods: Vec<MethodDto>,
    direct_fields: Vec<String>,
}

#[derive(Serialize)]
struct MethodDto {
    name: String,
    is_constructor: bool,
    arity: usize,
    line: Option<usize>,
}

#[derive(Serialize)]
struct ModuleGraphDto {
    modules: Vec<String>,
    edges: Vec<ModuleEdgeDto>,
    entry_points: Vec<String>,
    cycles: Vec<Vec<String>>,
}

#[derive(Serialize)]
struct ModuleEdgeDto {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct LuaRunResultDto {
    success: bool,
    output: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct RuntimeInfoDto {
    id: String,
    label: String,
    available: bool,
    executable: Option<String>,
    version: Option<String>,
}

#[derive(Deserialize)]
struct RunRequest {
    path: String,
    runtime: String,
}

#[derive(Deserialize)]
struct ProjectRuntimeRequest {
    runtime: String,
}

#[derive(Serialize, Deserialize)]
struct ProjectConfig {
    runtime: String,
    entry: String,
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    name: String,
    runtime: String,
}

fn active_root(state: &State<AppState>) -> Result<PathBuf, String> {
    state
        .0
        .lock()
        .map_err(|_| "workspace state is unavailable".to_string())?
        .clone()
        .ok_or_else(|| "no workspace is currently open".to_string())
}

fn workspace_info(root: &Path) -> WorkspaceInfo {
    WorkspaceInfo {
        name: root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_string(),
        root: root.display().to_string().replace('\\', "/"),
    }
}

fn sanitize_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(path);
    if requested.as_os_str().is_empty()
        || requested
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("invalid relative path components".to_string());
    }

    let root_canon = root
        .canonicalize()
        .map_err(|error| format!("could not open workspace root: {error}"))?;
    let target = root_canon.join(requested);
    let mut existing_ancestor = target.as_path();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| "path has no existing ancestor".to_string())?;
    }
    let canonical_ancestor = existing_ancestor
        .canonicalize()
        .map_err(|error| format!("could not resolve path ancestor: {error}"))?;
    if !canonical_ancestor.starts_with(&root_canon) {
        return Err("path is outside the workspace".to_string());
    }
    Ok(target)
}

fn workspace_file(root: &Path, path: &str) -> Result<PathBuf, String> {
    let target = sanitize_path(root, path)?;
    if !target.exists() {
        return Err(format!("file does not exist: {path}"));
    }
    Ok(target)
}

fn runtime_candidates(runtime: &str) -> (&'static str, &'static [&'static str]) {
    match runtime {
        "luajit21" => ("LuaJIT 2.1", &["luajit"]),
        "lua53" => ("Lua 5.3", &["lua5.3", "lua53", "lua"]),
        "lua51" => ("Lua 5.1", &["lua5.1", "lua51", "lua"]),
        _ => ("Lua 5.4", &["lua5.4", "lua54", "lua"]),
    }
}

fn hide_child_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

fn find_runtime(runtime: &str) -> Option<(String, String)> {
    let (_, candidates) = runtime_candidates(runtime);
    let expected = match runtime {
        "luajit21" => "LuaJIT 2.1",
        "lua53" => "Lua 5.3",
        "lua51" => "Lua 5.1",
        _ => "Lua 5.4",
    };
    candidates.iter().find_map(|candidate| {
        let mut command = Command::new(candidate);
        command.arg("-v");
        hide_child_console(&mut command);
        let result = command.output().ok()?;
        if !result.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let version = if stdout.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        version
            .contains(expected)
            .then(|| ((*candidate).to_string(), version.to_string()))
    })
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "file has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create parent directory: {error}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create temporary save file: {error}"))?;
    temp.write_all(contents)
        .and_then(|_| temp.flush())
        .map_err(|error| format!("could not write temporary save file: {error}"))?;
    temp.persist(path)
        .map_err(|error| format!("could not replace saved file: {}", error.error))?;
    Ok(())
}

fn valid_project_name(name: &str) -> bool {
    let name = name.trim();
    let mut components = Path::new(name).components();
    !name.is_empty()
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

fn diagnostic_dto(diagnostic: Diagnostic) -> DiagnosticDto {
    let suggestion = matches!(
        diagnostic.kind,
        aster_analysis::DiagnosticKind::GlobalInLoop
            | aster_analysis::DiagnosticKind::StringConcatInLoop
            | aster_analysis::DiagnosticKind::TableAllocationInLoop
    );
    DiagnosticDto {
        kind: format!("{:?}", diagnostic.kind).to_lowercase(),
        suggestion,
        file: diagnostic
            .file
            .map(|path| path.display().to_string().replace('\\', "/"))
            .unwrap_or_default(),
        line: diagnostic.line,
        column: diagnostic.column,
        message: diagnostic.message,
    }
}

fn explanation_dto(explanation: LookupExplanation) -> ExplanationDto {
    let result = match explanation.result {
        LookupResult::Found(member) => format!("Resolved: {member}"),
        LookupResult::NotFound => "Not found".to_string(),
        LookupResult::Unknown(reason) => format!("Unknown: {reason}"),
    };
    ExplanationDto {
        expression: explanation.expression,
        line: explanation.line,
        column: explanation.column,
        steps: explanation.steps,
        result,
    }
}

fn recents_file_path() -> Option<PathBuf> {
    let home = dirs_fallback();
    Some(home.join(".aster").join("recent_projects.json"))
}

fn dirs_fallback() -> PathBuf {
    if let Ok(path) = std::env::var("USERPROFILE") {
        PathBuf::from(path)
    } else if let Ok(path) = std::env::var("HOME") {
        PathBuf::from(path)
    } else {
        PathBuf::from(".")
    }
}

fn load_recent_projects_internal() -> Vec<RecentProject> {
    let Some(path) = recents_file_path() else {
        return Vec::new();
    };
    if let Ok(contents) = fs::read_to_string(path)
        && let Ok(mut recents) = serde_json::from_str::<Vec<RecentProject>>(&contents)
    {
        for item in &mut recents {
            item.exists = Path::new(&item.path).exists();
        }
        return recents;
    }
    Vec::new()
}

fn save_recent_projects_internal(recents: &[RecentProject]) {
    if let Some(path) = recents_file_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(recents) {
            let _ = fs::write(path, json);
        }
    }
}

fn record_recent_project(root: &Path) {
    let path_str = root.display().to_string().replace('\\', "/");
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut recents = load_recent_projects_internal();
    let mut pinned = false;
    if let Some(pos) = recents.iter().position(|r| r.path == path_str) {
        pinned = recents[pos].pinned;
        recents.remove(pos);
    }

    // Attempt to read git branch if .git/HEAD exists
    let git_head = root.join(".git").join("HEAD");
    let git_branch = if let Ok(head_str) = fs::read_to_string(git_head) {
        if head_str.starts_with("ref: refs/heads/") {
            Some(
                head_str
                    .trim_start_matches("ref: refs/heads/")
                    .trim()
                    .to_string(),
            )
        } else {
            None
        }
    } else {
        None
    };

    recents.insert(
        0,
        RecentProject {
            name,
            path: path_str,
            last_opened: now,
            pinned,
            git_branch,
            exists: true,
        },
    );

    save_recent_projects_internal(&recents);
}

#[tauri::command]
fn open_workspace(
    state: State<AppState>,
    folder_path: Option<String>,
) -> Result<Option<WorkspaceInfo>, String> {
    let selected = if let Some(path) = folder_path {
        PathBuf::from(path)
    } else {
        let current = state
            .0
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let Some(picked) = rfd::FileDialog::new().set_directory(current).pick_folder() else {
            return Ok(None);
        };
        picked
    };

    let selected = selected
        .canonicalize()
        .map_err(|error| format!("could not open selected folder: {error}"))?;

    record_recent_project(&selected);

    *state
        .0
        .lock()
        .map_err(|_| "workspace state is unavailable".to_string())? = Some(selected.clone());

    Ok(Some(workspace_info(&selected)))
}

#[tauri::command]
fn close_workspace(state: State<AppState>) -> Result<(), String> {
    *state
        .0
        .lock()
        .map_err(|_| "workspace state is unavailable".to_string())? = None;
    Ok(())
}

#[tauri::command]
fn current_workspace(state: State<AppState>) -> Result<Option<WorkspaceInfo>, String> {
    state
        .0
        .lock()
        .map_err(|_| "workspace state is unavailable".to_string())
        .map(|guard| guard.as_ref().map(|root| workspace_info(root)))
}

#[tauri::command]
fn workspace_files(state: State<AppState>) -> Result<Vec<WorkspaceFile>, String> {
    let root = active_root(&state)?;
    let mut files = WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("lua"))
        .collect::<Vec<_>>();
    files.sort();

    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| "workspace file is outside the workspace".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let contents = fs::read_to_string(workspace_file(&root, &relative)?)
                .map_err(|error| format!("could not read {relative}: {error}"))?;
            Ok(WorkspaceFile {
                path: relative,
                contents,
            })
        })
        .collect()
}

fn build_tree_node(dir_path: &Path, root: &Path) -> Result<FileTreeNode, String> {
    let name = dir_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let relative = dir_path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "".to_string());

    let mut children = Vec::new();
    if let Ok(entries) = fs::read_dir(dir_path) {
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (!is_dir, e.file_name())
        });

        for entry in entries {
            let child_path = entry.path();
            let child_name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden git directories and target caches
            if child_name.starts_with(".git")
                || child_name == "target"
                || child_name == "node_modules"
            {
                continue;
            }

            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if let Ok(node) = build_tree_node(&child_path, root) {
                    children.push(node);
                }
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let rel_child = child_path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or(child_name.clone());

                children.push(FileTreeNode {
                    name: child_name,
                    path: rel_child,
                    is_dir: false,
                    children: Vec::new(),
                    size,
                });
            }
        }
    }

    Ok(FileTreeNode {
        name,
        path: relative,
        is_dir: true,
        children,
        size: 0,
    })
}

#[tauri::command]
fn workspace_tree(state: State<AppState>) -> Result<FileTreeNode, String> {
    let root = active_root(&state)?;
    let canon_root = root
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize root: {e}"))?;
    build_tree_node(&canon_root, &canon_root)
}

#[tauri::command]
fn read_file(state: State<AppState>, request: PathRequest) -> Result<String, String> {
    let root = active_root(&state)?;
    let path = workspace_file(&root, &request.path)?;
    let size = path
        .metadata()
        .map_err(|error| format!("could not inspect file: {error}"))?
        .len();
    if size > 2 * 1024 * 1024 {
        return Err("file is larger than Aster's 2 MB editor limit".to_string());
    }
    fs::read_to_string(path).map_err(|error| format!("could not read file: {error}"))
}

#[tauri::command]
fn save_file(state: State<AppState>, request: SaveRequest) -> Result<(), String> {
    let root = active_root(&state)?;
    let path = sanitize_path(&root, &request.path)?;
    write_atomic(&path, request.contents.as_bytes())
}

#[tauri::command]
fn create_lua_project(
    state: State<AppState>,
    request: CreateProjectRequest,
) -> Result<Option<WorkspaceInfo>, String> {
    let name = request.name.trim();
    if !valid_project_name(name) {
        return Err("project name must be a single folder name".to_string());
    }

    let Some(parent) = rfd::FileDialog::new().pick_folder() else {
        return Ok(None);
    };
    let project_root = parent.join(name);
    if project_root.exists() {
        return Err("a file or folder with that project name already exists".to_string());
    }

    fs::create_dir_all(&project_root)
        .map_err(|error| format!("could not create project folder: {error}"))?;
    let main_source = "print(\"Hello from Aster!\")\n";
    write_atomic(&project_root.join("main.lua"), main_source.as_bytes())?;
    let config = ProjectConfig {
        runtime: request.runtime,
        entry: "main.lua".to_string(),
    };
    let config_json = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("could not create project settings: {error}"))?;
    write_atomic(&project_root.join(".aster.json"), &config_json)?;

    let canonical = project_root
        .canonicalize()
        .map_err(|error| format!("could not open new project: {error}"))?;
    record_recent_project(&canonical);
    *state
        .0
        .lock()
        .map_err(|_| "workspace state is unavailable".to_string())? = Some(canonical.clone());
    Ok(Some(workspace_info(&canonical)))
}

#[tauri::command]
fn get_project_runtime(state: State<AppState>) -> Result<Option<String>, String> {
    let root = active_root(&state)?;
    let path = root.join(".aster.json");
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read project settings: {error}"))?;
    let config: ProjectConfig =
        serde_json::from_str(&contents).map_err(|error| format!("invalid .aster.json: {error}"))?;
    Ok(Some(config.runtime))
}

#[tauri::command]
fn set_project_runtime(
    state: State<AppState>,
    request: ProjectRuntimeRequest,
) -> Result<(), String> {
    let root = active_root(&state)?;
    let path = root.join(".aster.json");
    let entry = fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str::<ProjectConfig>(&contents).ok())
        .map(|config| config.entry)
        .unwrap_or_else(|| "main.lua".to_string());
    let config = ProjectConfig {
        runtime: request.runtime,
        entry,
    };
    let contents = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("could not encode project settings: {error}"))?;
    write_atomic(&path, &contents)
}

#[tauri::command]
fn detect_lua_runtimes() -> Vec<RuntimeInfoDto> {
    ["lua54", "luajit21", "lua53", "lua51"]
        .into_iter()
        .map(|id| {
            let (label, _) = runtime_candidates(id);
            let found = find_runtime(id);
            RuntimeInfoDto {
                id: id.to_string(),
                label: label.to_string(),
                available: found.is_some(),
                executable: found.as_ref().map(|value| value.0.clone()),
                version: found.map(|value| value.1),
            }
        })
        .collect()
}

#[tauri::command]
fn run_lua(
    state: State<AppState>,
    run_state: State<RunState>,
    request: RunRequest,
) -> Result<LuaRunResultDto, String> {
    let root = active_root(&state)?;
    let script = workspace_file(&root, &request.path)?;
    if script.extension().and_then(|value| value.to_str()) != Some("lua") {
        return Err("only .lua files can be run".to_string());
    }
    run_lua_process(&root, &script, &request.runtime, &run_state)
}

fn run_lua_process(
    root: &Path,
    script: &Path,
    runtime: &str,
    run_state: &RunState,
) -> Result<LuaRunResultDto, String> {
    let (executable, version) = find_runtime(runtime).ok_or_else(|| {
        let (label, candidates) = runtime_candidates(runtime);
        format!(
            "{label} was not found. Install it and make one of these commands available: {}",
            candidates.join(", ")
        )
    })?;

    let mut running = run_state
        .0
        .lock()
        .map_err(|_| "run state is unavailable".to_string())?;
    if running.is_some() {
        return Err("a Lua process is already running".to_string());
    }

    let lua_path = format!(
        "{}{}?.lua;{}{}?{}init.lua;;",
        root.display(),
        std::path::MAIN_SEPARATOR,
        root.display(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    let mut command = Command::new(&executable);
    command
        .arg(script)
        .current_dir(root)
        .env("LUA_PATH", lua_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_child_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start {version}: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not capture Lua output".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not capture Lua errors".to_string())?;
    *running = Some(child);
    drop(running);

    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let status = loop {
        let status = {
            let mut guard = run_state
                .0
                .lock()
                .map_err(|_| "run state is unavailable".to_string())?;
            let child = guard
                .as_mut()
                .ok_or_else(|| "Lua process state was lost".to_string())?;
            child
                .try_wait()
                .map_err(|error| format!("could not wait for Lua process: {error}"))?
        };
        if let Some(status) = status {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    *run_state
        .0
        .lock()
        .map_err(|_| "run state is unavailable".to_string())? = None;

    let output = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).to_string();
    let error_output =
        String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).to_string();
    Ok(LuaRunResultDto {
        success: status.success(),
        output,
        error: if error_output.trim().is_empty() {
            (!status.success()).then(|| format!("Lua exited with {status}"))
        } else {
            Some(error_output)
        },
    })
}

#[tauri::command]
fn stop_lua(run_state: State<RunState>) -> Result<bool, String> {
    let mut running = run_state
        .0
        .lock()
        .map_err(|_| "run state is unavailable".to_string())?;
    if let Some(child) = running.as_mut() {
        child
            .kill()
            .map_err(|error| format!("could not stop Lua process: {error}"))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn create_file(state: State<AppState>, request: PathRequest) -> Result<(), String> {
    let root = active_root(&state)?;
    let path = sanitize_path(&root, &request.path)?;
    if path.exists() {
        return Err("file already exists".to_string());
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, "").map_err(|error| format!("could not create file: {error}"))
}

#[tauri::command]
fn create_directory(state: State<AppState>, request: PathRequest) -> Result<(), String> {
    let root = active_root(&state)?;
    let path = sanitize_path(&root, &request.path)?;
    fs::create_dir_all(path).map_err(|error| format!("could not create directory: {error}"))
}

#[tauri::command]
fn delete_entry(state: State<AppState>, request: PathRequest) -> Result<(), String> {
    let root = active_root(&state)?;
    let path = workspace_file(&root, &request.path)?;
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("could not delete directory: {error}"))
    } else {
        fs::remove_file(path).map_err(|error| format!("could not delete file: {error}"))
    }
}

#[tauri::command]
fn rename_entry(state: State<AppState>, request: RenameRequest) -> Result<(), String> {
    let root = active_root(&state)?;
    let old_path = workspace_file(&root, &request.old_path)?;
    let new_path = sanitize_path(&root, &request.new_path)?;
    fs::rename(old_path, new_path).map_err(|error| format!("could not rename: {error}"))
}

#[tauri::command]
fn reveal_in_explorer(state: State<AppState>, path: Option<String>) -> Result<(), String> {
    let root = active_root(&state)?;
    let target = if let Some(p) = path {
        sanitize_path(&root, &p)?
    } else {
        root
    };

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(if target.is_dir() {
                target.display().to_string()
            } else {
                format!("/select,{}", target.display())
            })
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(target)
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }

    Ok(())
}

#[tauri::command]
fn get_recent_projects() -> Result<Vec<RecentProject>, String> {
    Ok(load_recent_projects_internal())
}

#[tauri::command]
fn pin_recent_project(path: String, pinned: bool) -> Result<(), String> {
    let mut recents = load_recent_projects_internal();
    if let Some(item) = recents.iter_mut().find(|r| r.path == path) {
        item.pinned = pinned;
        save_recent_projects_internal(&recents);
    }
    Ok(())
}

#[tauri::command]
fn remove_recent_project(path: String) -> Result<(), String> {
    let mut recents = load_recent_projects_internal();
    recents.retain(|r| r.path != path);
    save_recent_projects_internal(&recents);
    Ok(())
}

#[tauri::command]
fn check_workspace(state: State<AppState>) -> Result<Vec<DiagnosticDto>, String> {
    let root = active_root(&state)?;
    Ok(check(&AnalysisOptions::new(root))
        .into_iter()
        .map(diagnostic_dto)
        .collect())
}

#[tauri::command]
fn explain_at_cursor(
    state: State<AppState>,
    request: ExplainRequest,
) -> Result<Vec<ExplanationDto>, String> {
    let root = active_root(&state)?;
    Ok(
        explain(&AnalysisOptions::new(root), &request.path, request.line)
            .into_iter()
            .filter(|explanation| {
                request.column >= explanation.start_column
                    && request.column <= explanation.end_column.saturating_add(1)
            })
            .map(explanation_dto)
            .collect(),
    )
}

#[tauri::command]
fn window_minimize(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|error| error.to_string())
}

#[tauri::command]
fn window_toggle_maximize(window: tauri::Window) -> Result<(), String> {
    let is_maximized = window.is_maximized().map_err(|error| error.to_string())?;
    let result = if is_maximized {
        window.unmaximize()
    } else {
        window.maximize()
    };
    result.map_err(|error| error.to_string())
}

#[tauri::command]
fn window_start_dragging(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn window_close(window: tauri::Window) -> Result<(), String> {
    window.destroy().map_err(|error| error.to_string())
}

#[tauri::command]
fn module_graph(state: State<AppState>) -> Result<ModuleGraphDto, String> {
    let root = active_root(&state)?;
    let analysis = analyze(&AnalysisOptions::new(root));
    let modules: Vec<String> = analysis
        .graph
        .modules()
        .into_iter()
        .map(|p| p.display().to_string().replace('\\', "/"))
        .collect();

    let mut edges = Vec::new();
    for module in analysis.graph.modules() {
        let from_str = module.display().to_string().replace('\\', "/");
        for dep in analysis.graph.dependencies(module) {
            edges.push(ModuleEdgeDto {
                from: from_str.clone(),
                to: dep.display().to_string().replace('\\', "/"),
            });
        }
    }

    let entry_points = analysis
        .graph
        .entry_points()
        .into_iter()
        .map(|p| p.display().to_string().replace('\\', "/"))
        .collect();

    let cycles = analysis
        .diagnostics
        .into_iter()
        .filter_map(|d| {
            if matches!(d.kind, aster_analysis::DiagnosticKind::CircularDependency) {
                Some(vec![d.message])
            } else {
                None
            }
        })
        .collect();

    Ok(ModuleGraphDto {
        modules,
        edges,
        entry_points,
        cycles,
    })
}

#[tauri::command]
fn inspect_classes(state: State<AppState>) -> Result<Vec<ClassStructureDto>, String> {
    let root = active_root(&state)?;
    let files = WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("lua"))
        .collect::<Vec<_>>();

    let mut class_structures = Vec::new();

    for file_path in files {
        let relative = file_path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        let Ok(source) = fs::read_to_string(&file_path) else {
            continue;
        };

        let Ok(ast) = full_moon::parse(&source) else {
            continue;
        };

        // Inspect AST for class patterns, functions, metatables
        let mut class_name = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Class")
            .to_string();

        // Capitalize first letter for class display if simple name
        if let Some(first) = class_name.chars().next()
            && first.is_lowercase()
        {
            class_name = format!("{}{}", first.to_uppercase(), &class_name[1..]);
        }

        let mut index_self = false;
        let mut index_other = None;
        let mut metatable_name = None;
        let mut parent_constructor = None;
        let mut instance_members = Vec::new();
        let mut methods = Vec::new();
        let mut direct_fields = Vec::new();

        // Scan AST statements
        for stmt in ast.nodes().stmts() {
            match stmt {
                full_moon::ast::Stmt::FunctionDeclaration(decl) => {
                    let mut is_constructor = false;
                    let mut name = String::new();

                    if let Some(method) = decl.name().method_name() {
                        name = method.to_string().trim().to_string();
                    } else {
                        let names: Vec<String> = decl
                            .name()
                            .names()
                            .iter()
                            .map(|t| t.to_string().trim().to_string())
                            .collect();
                        if names.len() >= 2 {
                            name = names.last().cloned().unwrap_or_default();
                            if name == "new" {
                                is_constructor = true;
                            }
                        }
                    }

                    if !name.is_empty() {
                        let line = decl
                            .name()
                            .names()
                            .first()
                            .and_then(|t| t.start_position())
                            .map(|p| p.line());
                        methods.push(MethodDto {
                            name,
                            is_constructor,
                            arity: decl.body().parameters().len(),
                            line,
                        });
                    }
                }
                full_moon::ast::Stmt::Assignment(assign) => {
                    for (index, var) in assign.variables().iter().enumerate() {
                        let var_str = var.to_string();
                        let clean = var_str.trim();
                        if clean.ends_with(".__index") {
                            let owner = clean.trim_end_matches(".__index").trim();
                            let value = assign
                                .expressions()
                                .iter()
                                .nth(index)
                                .map(ToString::to_string)
                                .unwrap_or_default();
                            let target = value.trim();
                            if target == owner {
                                index_self = true;
                            } else if !target.is_empty() {
                                index_other = Some(target.to_string());
                            }
                        } else if let Some(field) = clean.split('.').nth(1)
                            && field != "__index"
                            && !direct_fields.contains(&field.to_string())
                        {
                            direct_fields.push(field.to_string());
                        }
                    }
                }
                full_moon::ast::Stmt::FunctionCall(call) => {
                    let call_str = call.to_string();
                    if call_str.contains("setmetatable") && call_str.contains("__index") {
                        // Extract parent class name if in setmetatable(Player, { __index = Entity })
                        if let Some(pos) = call_str.find("__index") {
                            let after = &call_str[pos + 7..];
                            if let Some(eq_pos) = after.find('=') {
                                let target = after[eq_pos + 1..]
                                    .replace(['}', ')', ';', ' '], "")
                                    .trim()
                                    .to_string();
                                if !target.is_empty() {
                                    metatable_name = Some(target);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Search for self.field and Parent.new() in constructor body
        for method in &methods {
            if method.is_constructor {
                // Heuristic inspection of constructor fields
                for line in source.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("self.")
                        && let Some(eq_pos) = trimmed.find('=')
                    {
                        let field_name = trimmed[5..eq_pos].trim().to_string();
                        if !instance_members.contains(&field_name) && !field_name.is_empty() {
                            instance_members.push(field_name);
                        }
                    }
                    if trimmed.contains(".new(")
                        && !trimmed.contains(&format!("{class_name}.new("))
                        && let Some(dot_pos) = trimmed.find(".new(")
                    {
                        let before = &trimmed[..dot_pos];
                        let parent = before
                            .split([' ', '=', '('])
                            .next_back()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !parent.is_empty() && parent != "self" {
                            parent_constructor = Some(parent);
                        }
                    }
                }
            }
        }

        if !methods.is_empty()
            || !instance_members.is_empty()
            || metatable_name.is_some()
            || index_self
        {
            class_structures.push(ClassStructureDto {
                name: class_name,
                file: relative,
                metatable_name,
                parent_constructor,
                index_self,
                index_other,
                instance_members,
                methods,
                direct_fields,
            });
        }
    }

    Ok(class_structures)
}

#[tauri::command]
fn validate_lua_snippet(code: String) -> Result<LuaRunResultDto, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Ok(LuaRunResultDto {
            success: true,
            output: "Nothing to validate.".to_string(),
            error: None,
        });
    }

    match full_moon::parse(trimmed) {
        Ok(_) => Ok(LuaRunResultDto {
            success: true,
            output: format!(
                "Syntax is valid.\n[Parsed {} source lines; code was not executed]",
                trimmed.lines().count()
            ),
            error: None,
        }),
        Err(errors) => {
            let error_msg = errors
                .into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(LuaRunResultDto {
                success: false,
                output: String::new(),
                error: Some(format!("Syntax error:\n{error_msg}")),
            })
        }
    }
}

fn main() {
    tauri::Builder::default()
        .manage(AppState(Mutex::new(None)))
        .manage(RunState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            create_lua_project,
            close_workspace,
            current_workspace,
            workspace_files,
            workspace_tree,
            read_file,
            save_file,
            get_project_runtime,
            set_project_runtime,
            detect_lua_runtimes,
            run_lua,
            stop_lua,
            create_file,
            create_directory,
            delete_entry,
            rename_entry,
            reveal_in_explorer,
            get_recent_projects,
            pin_recent_project,
            remove_recent_project,
            check_workspace,
            explain_at_cursor,
            module_graph,
            inspect_classes,
            validate_lua_snippet,
            window_minimize,
            window_toggle_maximize,
            window_start_dragging,
            window_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aster");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_cannot_escape_the_selected_parent() {
        assert!(valid_project_name("hello-lua"));
        assert!(!valid_project_name(""));
        assert!(!valid_project_name("../outside"));
        assert!(!valid_project_name("nested/project"));
        assert!(!valid_project_name("C:\\absolute"));
    }

    #[test]
    fn atomic_save_replaces_existing_contents() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("main.lua");
        fs::write(&path, "old").expect("initial file");
        write_atomic(&path, b"print('new')\n").expect("atomic save");
        assert_eq!(
            fs::read_to_string(path).expect("saved file"),
            "print('new')\n"
        );
    }

    #[test]
    fn runtime_candidates_include_generic_commands() {
        assert!(runtime_candidates("lua54").1.contains(&"lua"));
        assert!(runtime_candidates("lua51").1.contains(&"lua"));
        assert_eq!(runtime_candidates("luajit21").0, "LuaJIT 2.1");
    }

    #[test]
    fn installed_lua_runtime_executes_and_captures_output() {
        if find_runtime("lua51").is_none() {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary project");
        let script = directory.path().join("main.lua");
        fs::write(&script, "print('aster-run-ok')\n").expect("Lua fixture");
        let run_state = RunState(Mutex::new(None));
        let result = run_lua_process(directory.path(), &script, "lua51", &run_state)
            .expect("run installed Lua");
        assert!(result.success);
        assert!(result.output.contains("aster-run-ok"));
        assert!(result.error.is_none());
    }
}
