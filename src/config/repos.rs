use crate::runtime::command_inventory::CommandInventory;
use crate::runtime::python_host::PythonCompatibilityHost;
use crate::runtime::talon_ast::TalonFile;
use crate::runtime::talon_parser::parse_talon_file;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityReposConfig {
    pub community: PathBuf,
    pub cursorless_talon: PathBuf,
    pub rango_talon: PathBuf,
}

#[derive(Debug)]
pub enum LoadError {
    MissingRepoPath(String),
    Io { path: PathBuf, source: std::io::Error },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::MissingRepoPath(name) => {
                write!(f, "missing compatibility repository path for `{name}`")
            }
            LoadError::Io { path, source } => {
                write!(f, "{}: io error: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug)]
pub struct LoadedCompatibility {
    repo_names: Vec<String>,
    pub inventory: CommandInventory,
    pub python_host: PythonCompatibilityHost,
    pub diagnostics: Vec<String>,
}

impl LoadedCompatibility {
    pub fn repo_names(&self) -> &[String] {
        &self.repo_names
    }
}

/// Loads the three supported Talon compatibility repositories (`community`,
/// `cursorless-talon`, and `rango-talon`).
pub fn load_supported_repositories(
    config: &CompatibilityReposConfig,
) -> Result<LoadedCompatibility, LoadError> {
    let entries = [
        ("community", &config.community),
        ("cursorless-talon", &config.cursorless_talon),
        ("rango-talon", &config.rango_talon),
    ];

    let mut repo_names = Vec::with_capacity(entries.len());
    let mut files = Vec::new();
    let mut python_host = PythonCompatibilityHost::default();
    let mut diagnostics = Vec::new();

    for (name, path) in entries {
        if path.as_os_str().is_empty() {
            return Err(LoadError::MissingRepoPath(name.to_string()));
        }
        repo_names.push(name.to_string());
        if path.exists() {
            load_repository_dir(path, &mut files, &mut python_host, &mut diagnostics)?;
        } else {
            diagnostics.push(format!(
                "repository `{name}` path `{}` does not exist on disk",
                path.display()
            ));
        }
    }

    Ok(LoadedCompatibility {
        repo_names,
        inventory: CommandInventory::from_files(files),
        python_host,
        diagnostics,
    })
}

/// Recursively loads `.py` and `.talon` files from `dir` into `python_host` and `files`.
///
/// Files that rely on out-of-scope constructs record structured diagnostics instead of
/// aborting the rest of the repository load.
pub fn load_repository_dir(
    dir: &Path,
    files: &mut Vec<TalonFile>,
    python_host: &mut PythonCompatibilityHost,
    diagnostics: &mut Vec<String>,
) -> Result<(), LoadError> {
    if !dir.exists() {
        return Err(LoadError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "path does not exist"),
        });
    }

    let mut py_paths = Vec::new();
    let mut talon_paths = Vec::new();
    collect_files(dir, &mut py_paths, &mut talon_paths)?;
    py_paths.sort();
    talon_paths.sort();

    for py_path in py_paths {
        if let Err(err) = python_host.load_python_file(&py_path) {
            diagnostics.push(format!("{}: {err}", py_path.display()));
        }
    }

    for talon_path in talon_paths {
        let src = std::fs::read_to_string(&talon_path).map_err(|e| LoadError::Io {
            path: talon_path.clone(),
            source: e,
        })?;
        match parse_talon_file(&src) {
            Ok(mut file) => {
                for diag in &file.diagnostics {
                    diagnostics.push(format!("{}: {diag}", talon_path.display()));
                }
                for cmd in &mut file.commands {
                    cmd.source_path = Some(talon_path.clone());
                }
                files.push(file);
            }
            Err(err) => {
                diagnostics.push(format!("{}: {err}", talon_path.display()));
            }
        }
    }

    Ok(())
}

fn collect_files(
    dir: &Path,
    py_paths: &mut Vec<PathBuf>,
    talon_paths: &mut Vec<PathBuf>,
) -> Result<(), LoadError> {
    if dir.is_file() {
        match dir.extension().and_then(|s| s.to_str()) {
            Some("py") => py_paths.push(dir.to_path_buf()),
            Some("talon") => talon_paths.push(dir.to_path_buf()),
            _ => {}
        }
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|e| LoadError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| LoadError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if matches!(
                name,
                ".git" | "node_modules" | "__pycache__" | ".venv" | "target"
            ) {
                continue;
            }
            collect_files(&path, py_paths, talon_paths)?;
        } else {
            match path.extension().and_then(|s| s.to_str()) {
                Some("py") => py_paths.push(path),
                Some("talon") => talon_paths.push(path),
                _ => {}
            }
        }
    }

    Ok(())
}
