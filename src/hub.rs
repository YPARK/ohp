//! Files of Hugging Face's Hub, as models are, fetched with curl into
//! Hugging Face's own cache and laid out as its libraries lay it out: a
//! file is kept once, as a blob named by its hash, and each revision's
//! snapshot links to the blobs of its files, `refs/main` naming the
//! revision last fetched. A model one of Hugging Face's libraries
//! downloaded is found here, and one downloaded here is found by them.
//!
//! A repo's revision and files are listed by the Hub's API, and files are
//! downloaded from where the Hub resolves them at that revision. A download
//! cut short is taken up again where it stopped. For a gated repo, the
//! token `HF_TOKEN` gives, or `hf auth login` keeps, is sent.

use crate::fetch::{self, Fetch};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const HUB: &str = "https://huggingface.co";

/// Hugging Face's cache of the Hub's files: `HF_HUB_CACHE`, or else `hub`
/// in Hugging Face's home.
pub fn cache() -> Option<PathBuf> {
    var("HF_HUB_CACHE").or_else(|| home().map(|home| home.join("hub")))
}

/// Hugging Face's home: `HF_HOME`, or else `huggingface` in the user's
/// cache directory.
fn home() -> Option<PathBuf> {
    var("HF_HOME")
        .or_else(|| var("XDG_CACHE_HOME").map(|cache| cache.join("huggingface")))
        .or_else(|| var("HOME").map(|home| home.join(".cache/huggingface")))
}

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The token to send the Hub, if one is given or kept.
fn token() -> Option<String> {
    let given = std::env::var("HF_TOKEN").ok();
    let kept = || std::fs::read_to_string(home()?.join("token")).ok();
    given
        .or_else(kept)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Where `repo`'s files are kept in `cache`.
pub fn dir(cache: &Path, repo: &str) -> PathBuf {
    cache.join(format!("models--{}", repo.replace('/', "--")))
}

/// A file of a repo, as the Hub lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    /// Where it is in the repo.
    pub path: String,
    pub size: u64,
    /// What its blob is named: its LFS hash, or else its git hash.
    pub blob: String,
}

/// A repo's files at a revision.
#[derive(Debug, PartialEq)]
pub struct Listing {
    pub commit: String,
    pub files: Vec<Remote>,
}

/// `repo`'s files at its main revision, as the Hub's API lists them.
pub fn list(repo: &str, stop: &AtomicBool) -> Result<Listing, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let read = |url: &str, name: &str| {
        let to = dir.path().join(name);
        get(repo, url, &to, false, stop)?;
        std::fs::read_to_string(&to).map_err(|e| e.to_string())
    };
    let revision = read(
        &format!("{HUB}/api/models/{repo}/revision/main"),
        "revision.json",
    )?;
    let commit = commit(&revision)?;
    let tree = read(
        &format!("{HUB}/api/models/{repo}/tree/{commit}?recursive=true"),
        "tree.json",
    )?;
    Ok(Listing {
        files: files(&tree)?,
        commit,
    })
}

/// Fetch `url`, of `repo`, to `to`, sending the token if there is one, and
/// taking up where it stopped a fetch cut short if `resume`.
fn get(repo: &str, url: &str, to: &Path, resume: bool, stop: &AtomicBool) -> Result<(), String> {
    let header = token().map(|t| format!("Authorization: Bearer {t}"));
    let how = Fetch {
        header: header.as_deref(),
        resume,
    };
    fetch::fetch_with(url, to, &how, stop).map_err(|e| trouble(repo, &e))
}

/// Why `repo` cannot be had from the Hub, as curl said `why`.
fn trouble(repo: &str, why: &str) -> String {
    if why.contains("401") || why.contains("403") {
        format!(
            "{repo} is gated or private on Hugging Face: accept its terms there, and set HF_TOKEN"
        )
    } else if why.contains("404") {
        format!("no {repo} on Hugging Face")
    } else {
        format!("cannot reach {repo} on Hugging Face: {why}")
    }
}

/// The revision the Hub's API describes in `json`.
fn commit(json: &str) -> Result<String, String> {
    let revision: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let sha = revision["sha"].as_str();
    sha.map(String::from)
        .ok_or_else(|| format!("the Hub gave no revision: {json}"))
}

/// The files the Hub's API lists in `json`, a repo's tree.
fn files(json: &str) -> Result<Vec<Remote>, String> {
    let tree: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let entries = tree
        .as_array()
        .ok_or_else(|| format!("the Hub gave no files: {json}"))?;
    Ok(entries
        .iter()
        .filter(|entry| entry["type"] == "file")
        .filter_map(|entry| {
            let lfs = &entry["lfs"];
            Some(Remote {
                path: entry["path"].as_str()?.into(),
                size: lfs["size"].as_u64().or_else(|| entry["size"].as_u64())?,
                blob: lfs["oid"]
                    .as_str()
                    .or_else(|| entry["oid"].as_str())?
                    .into(),
            })
        })
        .collect())
}

/// Where `file` of `repo` is kept in `cache`, if it is: in the snapshot
/// `refs/main` names, or else in any.
pub fn cached(cache: &Path, repo: &str, file: &str) -> Option<PathBuf> {
    let dir = dir(cache, repo);
    let main = std::fs::read_to_string(dir.join("refs/main")).ok();
    let snapshots = dir.join("snapshots");
    let at_main = main.map(|commit| snapshots.join(commit.trim()).join(file));
    at_main.filter(|path| path.is_file()).or_else(|| {
        let snapshots = std::fs::read_dir(&snapshots).ok()?.flatten();
        snapshots
            .map(|snapshot| snapshot.path().join(file))
            .find(|path| path.is_file())
    })
}

/// The files kept in `cache`, each its repo and its path there, by repo.
pub fn kept(cache: &Path) -> Vec<(String, String)> {
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(cache).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((owner, repo)) = name
            .strip_prefix("models--")
            .and_then(|n| n.split_once("--"))
        else {
            continue;
        };
        let repo = format!("{owner}/{repo}");
        let snapshots = std::fs::read_dir(entry.path().join("snapshots"));
        for snapshot in snapshots.into_iter().flatten().flatten() {
            for file in files_in(&snapshot.path()) {
                if !kept.contains(&(repo.clone(), file.clone())) {
                    kept.push((repo.clone(), file));
                }
            }
        }
    }
    kept.sort();
    kept
}

/// The files under `dir`, whole ones only, each as its path there.
fn files_in(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(at) = dirs.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.is_file()
                && let Ok(file) = path.strip_prefix(dir)
            {
                files.push(file.to_string_lossy().into_owned());
            }
        }
    }
    files
}

/// Where the blob of `remote`, a file of `repo`, is kept in `cache`.
pub fn blob(cache: &Path, repo: &str, remote: &Remote) -> PathBuf {
    dir(cache, repo).join("blobs").join(&remote.blob)
}

/// The blob at `blob` as it is while being downloaded.
pub fn incomplete(blob: &Path) -> PathBuf {
    fetch::suffixed(blob, ".incomplete")
}

/// Download `remote`, a file of `repo` at `commit`, into `cache`: its blob,
/// unless it is kept, then its snapshot's link to it, and `refs/main`.
/// Where it is, once it is whole.
pub fn download(
    cache: &Path,
    repo: &str,
    commit: &str,
    remote: &Remote,
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    let dir = dir(cache, repo);
    let blob = blob(cache, repo, remote);
    let whole = |path: &Path| std::fs::metadata(path).is_ok_and(|m| m.len() == remote.size);
    if !whole(&blob) {
        let part = incomplete(&blob);
        if std::fs::metadata(&part).is_ok_and(|m| m.len() > remote.size) {
            let _ = std::fs::remove_file(&part);
        }
        let parent = blob.parent().unwrap_or(&dir);
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        let url = format!("{HUB}/{repo}/resolve/{commit}/{}", remote.path);
        get(repo, &url, &part, true, stop)?;
        if !whole(&part) {
            let len = std::fs::metadata(&part).map_or(0, |m| m.len());
            return Err(format!(
                "{} came short: {len} of {} bytes",
                remote.path, remote.size
            ));
        }
        std::fs::rename(&part, &blob).map_err(|e| e.to_string())?;
    }
    let link = dir.join("snapshots").join(commit).join(&remote.path);
    place(&link, &blob)?;
    let refs = dir.join("refs");
    std::fs::create_dir_all(&refs).map_err(|e| e.to_string())?;
    std::fs::write(refs.join("main"), commit).map_err(|e| e.to_string())?;
    Ok(link)
}

/// Make `link` a snapshot's file for `blob`: a link to it, relative, as
/// Hugging Face's libraries make it.
#[cfg(unix)]
fn place(link: &Path, blob: &Path) -> Result<(), String> {
    let parent = link.parent().ok_or("no snapshot")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // From the snapshot's directory, or one in it, up to the repo's.
    let snapshots = (link.ancestors())
        .position(|a| a.file_name().is_some_and(|n| n == "snapshots"))
        .unwrap_or(2);
    let mut to = PathBuf::new();
    for _ in 0..snapshots {
        to.push("..");
    }
    to.push("blobs");
    to.push(blob.file_name().ok_or("no blob")?);
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(&to, link).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn place(link: &Path, blob: &Path) -> Result<(), String> {
    let parent = link.parent().ok_or("no snapshot")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    std::fs::copy(blob, link)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "tests/hub.rs"]
mod tests;
