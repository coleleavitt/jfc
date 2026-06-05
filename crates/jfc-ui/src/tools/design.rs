use std::path::{Path, PathBuf};

use super::ExecutionResult;
use jfc_design::project::ProjectStore;

fn store(cwd: &Path) -> Result<ProjectStore, String> {
    ProjectStore::default_in(cwd).map_err(|e| e.to_string())
}

fn pretty<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

fn output_path_for(input: &str, output: Option<&str>) -> PathBuf {
    output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(input).with_extension("standalone.html"))
}

pub(super) fn execute_design_project_create(cwd: &Path, title: &str) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let project = store.create(title).map_err(|e| e.to_string())?;
        pretty(&serde_json::json!({
            "id": project.meta().id,
            "title": project.meta().title,
            "root": project.root(),
            "assets": project.meta().assets,
        }))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_project_list(cwd: &Path) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let projects = store.list().map_err(|e| e.to_string())?;
        pretty(&serde_json::json!({
            "base": store.base(),
            "projects": projects,
        }))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_project_set_meta(
    cwd: &Path,
    project_id: &str,
    title: Option<&str>,
    is_design_system: Option<bool>,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let mut project = store.open(project_id).map_err(|e| e.to_string())?;
        if let Some(title) = title {
            project.set_title(title).map_err(|e| e.to_string())?;
        }
        if let Some(is_design_system) = is_design_system {
            project
                .set_is_design_system(is_design_system)
                .map_err(|e| e.to_string())?;
        }
        pretty(project.meta())
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_list_files(cwd: &Path, project_id: &str) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let project = store.open(project_id).map_err(|e| e.to_string())?;
        pretty(&serde_json::json!({
            "project_id": project.meta().id,
            "root": project.root(),
            "files": project.list_files(),
            "assets": project.meta().assets,
        }))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_read_file(
    cwd: &Path,
    project_id: &str,
    path: &str,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let project = store.open(project_id).map_err(|e| e.to_string())?;
        let bytes = project.read_file(path).map_err(|e| e.to_string())?;
        match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(e) => Ok(format!(
                "binary file: {} bytes at {project_id}/{path}",
                e.into_bytes().len()
            )),
        }
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_write_file(
    cwd: &Path,
    project_id: &str,
    path: &str,
    content: &str,
    asset_name: Option<&str>,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let mut project = store.open(project_id).map_err(|e| e.to_string())?;
        project
            .write_file(path, content.as_bytes(), asset_name)
            .map_err(|e| e.to_string())?;
        pretty(&serde_json::json!({
            "project_id": project.meta().id,
            "path": path,
            "bytes": content.len(),
            "assets": project.meta().assets,
        }))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_delete_file(
    cwd: &Path,
    project_id: &str,
    path: &str,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let mut project = store.open(project_id).map_err(|e| e.to_string())?;
        project.delete_path(path).map_err(|e| e.to_string())?;
        Ok(format!("Deleted design file {project_id}/{path}"))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_copy_file(
    cwd: &Path,
    project_id: &str,
    from_path: &str,
    to_path: &str,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let project = store.open(project_id).map_err(|e| e.to_string())?;
        project
            .copy_file(from_path, to_path)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "Copied design file {project_id}/{from_path} -> {to_path}"
        ))
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_register_asset(
    cwd: &Path,
    project_id: &str,
    name: &str,
    path: &str,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let mut project = store.open(project_id).map_err(|e| e.to_string())?;
        project
            .register_asset(name, path)
            .map_err(|e| e.to_string())?;
        pretty(project.meta())
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_unregister_asset(
    cwd: &Path,
    project_id: &str,
    path: &str,
) -> ExecutionResult {
    let result: std::result::Result<String, String> = (|| {
        let store = store(cwd)?;
        let mut project = store.open(project_id).map_err(|e| e.to_string())?;
        project.unregister_asset(path).map_err(|e| e.to_string())?;
        pretty(project.meta())
    })();
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_bundle_html(
    input: &str,
    output: Option<&str>,
    require_thumbnail: Option<bool>,
) -> ExecutionResult {
    let output = output_path_for(input, output);
    match jfc_design::inline::bundle(input, &output, require_thumbnail.unwrap_or(true)) {
        Ok(report) => ExecutionResult::success(report.summary()),
        Err(e) => ExecutionResult::failure(e.to_string()),
    }
}

pub(super) fn execute_design_handoff(
    project_dir: &str,
    feature: &str,
    files: &[String],
) -> ExecutionResult {
    match jfc_design::handoff::scaffold(project_dir, feature, files) {
        Ok(pkg) => {
            let copied = if pkg.copied.is_empty() {
                "(none)".to_owned()
            } else {
                pkg.copied.join(", ")
            };
            ExecutionResult::success(format!(
                "Created {}\nREADME: {}\nCopied: {copied}",
                pkg.dir.display(),
                pkg.readme.display()
            ))
        }
        Err(e) => ExecutionResult::failure(e.to_string()),
    }
}

pub(super) fn execute_design_check_system(project_dir: &str) -> ExecutionResult {
    match jfc_design::design_system::index_and_write(project_dir) {
        Ok(manifest) => ExecutionResult::success(format!(
            "{}\n\nWrote {}",
            manifest.report(),
            Path::new(project_dir).join("_ds_manifest.json").display()
        )),
        Err(e) => ExecutionResult::failure(e.to_string()),
    }
}

pub(super) fn execute_design_capabilities(format: Option<&str>) -> ExecutionResult {
    let result: std::result::Result<String, String> = match format.unwrap_or("text") {
        "json" => pretty(&jfc_design::capabilities::matrix()),
        "markdown" | "md" => Ok(jfc_design::capabilities::markdown_table()),
        _ => Ok(jfc_design::capabilities::matrix()
            .into_iter()
            .map(|c| format!("{:<14} {:<55} {}", c.status.glyph(), c.feature, c.surface))
            .collect::<Vec<_>>()
            .join("\n")),
    };
    result.map_or_else(ExecutionResult::failure, ExecutionResult::success)
}

pub(super) fn execute_design_serve(
    project_dir: &str,
    port: Option<u32>,
    file: Option<&str>,
) -> ExecutionResult {
    let port = port.unwrap_or(0);
    let addr = format!("127.0.0.1:{port}");
    match jfc_design::server::spawn(project_dir, &addr) {
        Ok(server) => {
            let file = file.unwrap_or("").trim_start_matches('/');
            let url = if file.is_empty() {
                format!("http://{}/", server.local_addr)
            } else {
                format!("http://{}/{}", server.local_addr, file)
            };
            ExecutionResult::success(format!("Serving {} at {url}", server.root.display()))
        }
        Err(e) => ExecutionResult::failure(e.to_string()),
    }
}
