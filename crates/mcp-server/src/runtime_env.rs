use rmcp::schemars;
use serde::Serialize;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const ORT_PROVIDER_SO: &str = "libonnxruntime_providers_cuda.so";

const CUBLAS_LT_CANDIDATES: &[&str] = &["libcublasLt.so.12", "libcublasLt.so.13"];
const CUBLAS_CANDIDATES: &[&str] = &["libcublas.so.12", "libcublas.so.13"];
const NVRTC_CANDIDATES: &[&str] = &["libnvrtc.so.12"];

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GpuEnvReport {
    pub ort_lib_location: Option<String>,
    pub ld_library_path: Option<String>,
    pub provider_present: bool,
    pub cublas_present: bool,
    pub nvrtc_present: bool,
    pub provider_dir: Option<String>,
    pub cublas_dir: Option<String>,
    pub nvrtc_dir: Option<String>,
    pub searched_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BootstrapReport {
    pub repo_root: Option<String>,
    pub model_dir: Option<String>,
    pub applied_env: Vec<String>,
    pub warnings: Vec<String>,
    pub gpu: GpuEnvReport,
}

pub fn bootstrap_best_effort() -> BootstrapReport {
    let exe = env::current_exe().ok();
    let repo_root = exe.as_deref().and_then(infer_repo_root_from_exe);
    bootstrap_from_repo_root(repo_root.as_deref())
}

fn bootstrap_from_repo_root(repo_root: Option<&Path>) -> BootstrapReport {
    let mut applied_env = Vec::new();
    let mut warnings = Vec::new();

    let repo_root = repo_root.map(|p| p.to_path_buf());

    // Model dir: optional, but helps avoid surprises when the MCP server is launched
    // from an arbitrary working directory.
    let model_dir = if env::var_os("CONTEXT_FINDER_MODEL_DIR").is_none() {
        repo_root.as_deref().and_then(|root| {
            let candidate = root.join("models");
            if candidate.join("manifest.json").exists() {
                env::set_var("CONTEXT_FINDER_MODEL_DIR", &candidate);
                applied_env.push("CONTEXT_FINDER_MODEL_DIR".to_string());
                Some(candidate)
            } else {
                None
            }
        })
    } else {
        env::var("CONTEXT_FINDER_MODEL_DIR").ok().map(PathBuf::from)
    };

    // GPU env: do best-effort bootstrap, but never fail server startup.
    if !is_cuda_disabled() {
        let before = diagnose_gpu_env();
        if !(before.provider_present && before.cublas_present) {
            if let Some(root) = repo_root.as_deref() {
                if let Err(err) = try_bootstrap_gpu_env_from_repo(root, &mut applied_env) {
                    warnings.push(err);
                }
            } else if let Err(err) = try_bootstrap_gpu_env_from_global_cache(&mut applied_env) {
                warnings.push(err);
            }
        }
    }

    let gpu = diagnose_gpu_env();
    if !is_cuda_disabled() && (!gpu.provider_present || !gpu.cublas_present) {
        warnings.push("CUDA libraries are not fully configured (provider/cublas missing). Run `bash scripts/setup_cuda_deps.sh` in the Context Finder repo or set ORT_LIB_LOCATION/LD_LIBRARY_PATH. If you want CPU fallback, set CONTEXT_FINDER_ALLOW_CPU=1.".to_string());
    }

    BootstrapReport {
        repo_root: repo_root.as_ref().map(|p| display_path(p)),
        model_dir: model_dir.as_ref().map(|p| display_path(p)),
        applied_env,
        warnings,
        gpu,
    }
}

pub fn is_cuda_disabled() -> bool {
    env::var("ORT_DISABLE_CUDA")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || env::var("ORT_USE_CUDA")
            .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
            .unwrap_or(false)
}

fn infer_repo_root_from_exe(exe_path: &Path) -> Option<PathBuf> {
    let exe = exe_path
        .canonicalize()
        .unwrap_or_else(|_| exe_path.to_path_buf());
    let release_or_debug = exe.parent()?;
    let name = release_or_debug.file_name()?.to_string_lossy();
    if name != "release" && name != "debug" {
        return None;
    }
    let target = release_or_debug.parent()?;
    if target.file_name() != Some(OsStr::new("target")) {
        return None;
    }
    Some(target.parent()?.to_path_buf())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn collect_env_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(path) = env::var("ORT_LIB_LOCATION") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(ld) = env::var("LD_LIBRARY_PATH") {
        paths.extend(
            ld.split(':')
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
        );
    }
    paths
}

fn find_first_with_file(paths: &[PathBuf], name: &str) -> Option<PathBuf> {
    paths.iter().find(|dir| dir.join(name).exists()).cloned()
}

fn find_first_with_any(paths: &[PathBuf], candidates: &[&str]) -> Option<PathBuf> {
    for name in candidates {
        if let Some(dir) = find_first_with_file(paths, name) {
            return Some(dir);
        }
    }
    None
}

pub fn diagnose_gpu_env() -> GpuEnvReport {
    let ort_lib_location = env::var("ORT_LIB_LOCATION").ok();
    let ld_library_path = env::var("LD_LIBRARY_PATH").ok();

    let paths = collect_env_paths();
    let provider_dir = find_first_with_file(&paths, ORT_PROVIDER_SO);
    let cublas_dir = find_first_with_any(&paths, CUBLAS_LT_CANDIDATES)
        .or_else(|| find_first_with_any(&paths, CUBLAS_CANDIDATES));
    let nvrtc_dir = find_first_with_any(&paths, NVRTC_CANDIDATES);

    GpuEnvReport {
        ort_lib_location,
        ld_library_path,
        provider_present: provider_dir.is_some(),
        cublas_present: cublas_dir.is_some(),
        nvrtc_present: nvrtc_dir.is_some(),
        provider_dir: provider_dir.as_ref().map(|p| display_path(p)),
        cublas_dir: cublas_dir.as_ref().map(|p| display_path(p)),
        nvrtc_dir: nvrtc_dir.as_ref().map(|p| display_path(p)),
        searched_paths: paths.iter().map(|p| display_path(p)).collect(),
    }
}

fn has_cuda_provider(dir: &Path) -> bool {
    dir.join(ORT_PROVIDER_SO).exists()
}

fn try_bootstrap_gpu_env_from_repo(
    root: &Path,
    applied_env: &mut Vec<String>,
) -> Result<(), String> {
    let deps = root.join(".deps").join("ort_cuda");
    let provider_dir = if has_cuda_provider(&deps) {
        Some(deps.clone())
    } else {
        find_best_official_ort_dir(root)
    };

    apply_gpu_env(provider_dir.as_deref(), Some(&deps), applied_env)
}

fn try_bootstrap_gpu_env_from_global_cache(applied_env: &mut Vec<String>) -> Result<(), String> {
    let dir = find_global_ort_cache_dir();
    apply_gpu_env(dir.as_deref(), None, applied_env)
}

fn find_global_ort_cache_dir() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let root = Path::new(&home)
        .join(".cache")
        .join("ort.pyke.io")
        .join("dfbin")
        .join("x86_64-unknown-linux-gnu");

    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("onnxruntime").join("lib");
        if has_cuda_provider(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn find_best_official_ort_dir(root: &Path) -> Option<PathBuf> {
    let base = root.join(".deps").join("ort_cuda_official");
    let entries = std::fs::read_dir(&base).ok()?;

    let mut best: Option<(VersionTriple, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let version = match parse_version_triple(&name) {
            Some(v) => v,
            None => continue,
        };
        let lib_dir = path.join("lib");
        if !has_cuda_provider(&lib_dir) {
            continue;
        }
        match &best {
            None => best = Some((version, lib_dir)),
            Some((best_v, _)) if &version > best_v => best = Some((version, lib_dir)),
            _ => {}
        }
    }

    best.map(|(_, dir)| dir)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VersionTriple(u32, u32, u32);

fn parse_version_triple(name: &str) -> Option<VersionTriple> {
    // e.g. "onnxruntime-linux-x64-gpu-1.22.0" -> "1.22.0"
    let ver = name.rsplit('-').next()?;
    let mut parts = ver.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some(VersionTriple(major, minor, patch))
}

fn apply_gpu_env(
    provider_dir: Option<&Path>,
    cuda_deps_dir: Option<&Path>,
    applied_env: &mut Vec<String>,
) -> Result<(), String> {
    let mut paths_to_prepend: Vec<PathBuf> = Vec::new();

    if let Some(dir) = cuda_deps_dir {
        if dir.exists() {
            paths_to_prepend.push(dir.to_path_buf());
        }
    }
    if let Some(dir) = provider_dir {
        if dir.exists() {
            // ORT provider dir can be different from CUDA deps dir.
            paths_to_prepend.push(dir.to_path_buf());
        }
    }

    if let Some(dir) = provider_dir {
        if !env_var_has_provider("ORT_LIB_LOCATION") {
            env::set_var("ORT_LIB_LOCATION", dir);
            applied_env.push("ORT_LIB_LOCATION".to_string());
        }
        if env::var_os("ORT_DYLIB_PATH").is_none() {
            env::set_var("ORT_DYLIB_PATH", dir);
            applied_env.push("ORT_DYLIB_PATH".to_string());
        }
    }

    if !paths_to_prepend.is_empty() {
        prepend_ld_library_path(&paths_to_prepend);
        applied_env.push("LD_LIBRARY_PATH".to_string());
    }

    if env::var_os("ORT_DISABLE_TENSORRT").is_none() {
        env::set_var("ORT_DISABLE_TENSORRT", "1");
        applied_env.push("ORT_DISABLE_TENSORRT".to_string());
    }
    if env::var_os("ORT_STRATEGY").is_none() {
        env::set_var("ORT_STRATEGY", "system");
        applied_env.push("ORT_STRATEGY".to_string());
    }
    if env::var_os("ORT_USE_CUDA").is_none() && env::var_os("ORT_DISABLE_CUDA").is_none() {
        env::set_var("ORT_USE_CUDA", "1");
        applied_env.push("ORT_USE_CUDA".to_string());
    }

    Ok(())
}

fn env_var_has_provider(key: &str) -> bool {
    match env::var_os(key) {
        Some(val) => PathBuf::from(val).join(ORT_PROVIDER_SO).exists(),
        None => false,
    }
}

fn prepend_ld_library_path(paths: &[PathBuf]) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ordered: Vec<String> = Vec::new();

    for path in paths {
        if !path.exists() {
            continue;
        }
        let value = path.to_string_lossy().into_owned();
        if seen.insert(value.clone()) {
            ordered.push(value);
        }
    }

    if let Ok(existing) = env::var("LD_LIBRARY_PATH") {
        for part in existing.split(':').filter(|p| !p.is_empty()) {
            if seen.insert(part.to_string()) {
                ordered.push(part.to_string());
            }
        }
    }

    if !ordered.is_empty() {
        env::set_var("LD_LIBRARY_PATH", ordered.join(":"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&str]) -> Self {
            let mut saved = Vec::new();
            for key in keys {
                saved.push((key.to_string(), env::var_os(key)));
                env::remove_var(key);
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(v) => env::set_var(&key, v),
                    None => env::remove_var(&key),
                }
            }
        }
    }

    fn bootstrap_for_test_repo(root: &Path) -> BootstrapReport {
        bootstrap_from_repo_root(Some(root))
    }

    #[test]
    fn version_parse() {
        assert_eq!(
            parse_version_triple("onnxruntime-linux-x64-gpu-1.22.0"),
            Some(VersionTriple(1, 22, 0))
        );
        assert_eq!(parse_version_triple("nope"), None);
    }

    #[test]
    fn bootstrap_sets_model_and_gpu_env_when_repo_layout_present() {
        let _guard = EnvGuard::new(&[
            "CONTEXT_FINDER_MODEL_DIR",
            "ORT_LIB_LOCATION",
            "ORT_DYLIB_PATH",
            "LD_LIBRARY_PATH",
            "ORT_DISABLE_TENSORRT",
            "ORT_STRATEGY",
            "ORT_USE_CUDA",
        ]);

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // models/manifest.json
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("models").join("manifest.json"), "{}").unwrap();

        // .deps/ort_cuda with expected filenames (empty files are enough for presence checks)
        let deps = root.join(".deps").join("ort_cuda");
        std::fs::create_dir_all(&deps).unwrap();
        std::fs::write(deps.join(ORT_PROVIDER_SO), "").unwrap();
        std::fs::write(deps.join("libcublasLt.so.12"), "").unwrap();

        let report = bootstrap_for_test_repo(root);
        assert!(report.model_dir.as_deref().unwrap().ends_with("/models"));
        assert_eq!(
            env::var("CONTEXT_FINDER_MODEL_DIR").unwrap(),
            root.join("models").to_string_lossy()
        );

        assert!(report.gpu.provider_present);
        assert!(report.gpu.cublas_present);
        assert_eq!(
            env::var("ORT_LIB_LOCATION").unwrap(),
            deps.to_string_lossy()
        );
        assert!(env::var("LD_LIBRARY_PATH")
            .unwrap()
            .contains(deps.to_string_lossy().as_ref()));
    }
}
