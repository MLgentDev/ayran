//! Fetch only during explicit install; temporary checkouts live in ayran's cache.
use ayran_core::{config::GitSkillBinding, diagnostic::Diagnostic};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

pub(crate) struct Fetched {
    pub _directory: tempfile::TempDir,
    pub path: PathBuf,
    pub commit: String,
}
pub(crate) fn fetch(binding: &GitSkillBinding, prompting: bool) -> Result<Fetched, Diagnostic> {
    let failure = |e: String| {
        Diagnostic::error(
            "skill-fetch-failed",
            e,
            Some(
                "check the git source/ref and credentials (HTTPS credential helper or SSH agent), then retry ayran install --skill",
            ),
        )
    };
    let root = crate::generated_cache::root()
        .ok_or_else(|| failure(crate::generated_cache::missing_root_message("ayran")))?
        .join("git-skills");
    fs::create_dir_all(&root).map_err(|e| failure(e.to_string()))?;
    let directory = tempfile::tempdir_in(root).map_err(|e| failure(e.to_string()))?;
    let run = |args: &[&str]| -> Result<String, Diagnostic> {
        let mut command = Command::new("git");
        command
            .current_dir(directory.path())
            .args(args)
            .env("GIT_LFS_SKIP_SMUDGE", "1");
        if prompting {
            command.stdin(Stdio::inherit());
        } else {
            command
                .stdin(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
                .env("GIT_SSH_VARIANT", "ssh")
                .env("SSH_ASKPASS", "")
                .env("GIT_ASKPASS", "");
        }
        let output = command.output().map_err(|e| failure(e.to_string()))?;
        if !output.status.success() {
            return Err(failure(format!(
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    run(&["init", "--quiet"])?;
    let remote = binding
        .source
        .strip_prefix("github:")
        .map(|r| format!("https://github.com/{r}.git"))
        .unwrap_or_else(|| binding.source.clone());
    run(&["remote", "add", "origin", &remote])?;
    run(&[
        "fetch",
        "--depth=1",
        "--no-tags",
        "origin",
        binding.r#ref.as_deref().unwrap_or("HEAD"),
    ])?;
    let commit = run(&["rev-parse", "FETCH_HEAD"])?;
    // Validate gitlinks before checkout, including a subdir that itself is a submodule.
    let tree = run(&["ls-tree", "-r", "-z", "FETCH_HEAD"])?;
    let subdir = binding
        .subdir
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/");
    let subdir = subdir.as_str();
    let selected = |path: &str| {
        subdir == "."
            || subdir.is_empty()
            || path == subdir
            || path
                .strip_prefix(subdir)
                .is_some_and(|p| p.starts_with('/'))
    };
    for line in tree.split('\0') {
        if let Some((entry, path)) = line.split_once('\t')
            && (selected(path)
                || subdir
                    .strip_prefix(path)
                    .is_some_and(|p| p.starts_with('/')))
            && (entry.starts_with("160000") || entry.starts_with("120000"))
        {
            return Err(crate::skill_snapshot::error(
                "skill-install-invalid",
                "Skill tree contains a submodule or symlink",
            ));
        }
    }
    if subdir != "." && !subdir.is_empty() {
        run(&["sparse-checkout", "set", "--cone", "--", subdir])?;
    }
    run(&[
        "-c",
        "filter.lfs.required=false",
        "-c",
        "filter.lfs.smudge=",
        "-c",
        "filter.lfs.process=",
        "checkout",
        "--detach",
        "FETCH_HEAD",
    ])?;
    // Git metadata is not part of a root Skill snapshot.
    fs::remove_dir_all(directory.path().join(".git")).map_err(|e| failure(e.to_string()))?;
    let path = directory.path().join(&binding.subdir);
    reject_lfs(&path)?;
    Ok(Fetched {
        path,
        _directory: directory,
        commit,
    })
}
fn reject_lfs(path: &std::path::Path) -> Result<(), Diagnostic> {
    let invalid = |e: String| crate::skill_snapshot::error("skill-install-invalid", e);
    for entry in fs::read_dir(path).map_err(|e| invalid(e.to_string()))? {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| invalid(e.to_string()))?;
        if metadata.is_dir() {
            reject_lfs(&entry.path())?;
        } else if metadata.is_file() {
            use std::io::Read;
            let mut prefix = [0; 43];
            let n = fs::File::open(entry.path())
                .and_then(|mut f| f.read(&mut prefix))
                .map_err(|e| invalid(e.to_string()))?;
            if prefix[..n].starts_with(b"version https://git-lfs.github.com/spec/v1") {
                return Err(invalid("Skill tree contains a Git LFS pointer".into()));
            }
        }
    }
    Ok(())
}
