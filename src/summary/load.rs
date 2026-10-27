//! The models loaded, downloaded into Hugging Face's cache first where
//! they are not kept there, each step shown as it goes.

#[cfg(feature = "summary")]
use super::lock;
#[cfg(not(feature = "summary"))]
use super::{Embedder, Llm, NO_SUMMARY};
#[cfg(feature = "summary")]
use super::{Fetching, Weighed, Weights, gguf};
use super::{Loader, Shared};
#[cfg(feature = "summary")]
use crate::embed::{self, Embedder};
#[cfg(feature = "summary")]
use crate::hub;
#[cfg(feature = "summary")]
use crate::llm::Llm;
#[cfg(feature = "summary")]
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
#[cfg(feature = "summary")]
use std::sync::atomic::Ordering;

#[cfg(feature = "summary")]
impl Loader {
    /// The language model wanted now, loaded, and downloaded first if it is
    /// not yet kept; another loaded before is let go first. One given up,
    /// as `stop` says, is not taken for one that cannot be had.
    pub(super) fn llm(&mut self, shared: &Shared, stop: &AtomicBool) -> Result<&mut Llm, String> {
        let weights = {
            let state = lock(shared);
            if let Weighed::Broken(why) = &state.model {
                return Err(why.clone());
            }
            state.weights.clone()
        };
        if self.llm.as_ref().is_none_or(|(w, _)| *w != weights) {
            self.llm = None;
            let loaded = locate(shared, &weights, stop).and_then(|path| {
                set(shared, Weighed::Loading(weights.name()));
                Llm::load(&path)
            });
            match loaded {
                Ok(llm) => {
                    set(shared, Weighed::Loaded);
                    self.llm = Some((weights, llm));
                }
                Err(e) if stop.load(Ordering::Relaxed) => {
                    set(shared, Weighed::Unloaded);
                    return Err(e);
                }
                Err(e) => {
                    set(shared, Weighed::Broken(e.clone()));
                    return Err(e);
                }
            }
        }
        (self.llm.as_mut().map(|(_, llm)| llm)).ok_or_else(|| "no model".into())
    }

    /// The embedding model, loaded, and downloaded first if it is not yet
    /// kept; none if it cannot be had, as offline, or is given up, as
    /// `stop` says, and so tried again next time.
    pub(super) fn embedder(&mut self, shared: &Shared, stop: &AtomicBool) -> Option<&Embedder> {
        if self.embedder.is_none() {
            let loaded = fetch_all(shared, embed::REPO, &embed::FILES, stop)
                .and_then(|dir| Embedder::load(&dir));
            if loaded.is_ok() || !stop.load(Ordering::Relaxed) {
                self.embedder = Some(loaded);
            }
        }
        self.embedder.as_ref()?.as_ref().ok()
    }
}

#[cfg(feature = "summary")]
fn set(shared: &Shared, model: Weighed) {
    let mut state = lock(shared);
    state.model = model;
    state.changed();
}

/// Where the model in `weights` is, downloading it into Hugging Face's
/// cache first if it is not kept there.
#[cfg(feature = "summary")]
fn locate(shared: &Shared, weights: &Weights, stop: &AtomicBool) -> Result<PathBuf, String> {
    let (repo, file) = match weights {
        Weights::File(path) => return Ok(path.clone()),
        Weights::Hub { repo, file } => (repo, file),
    };
    let cache = cache()?;
    let kept = (hub::kept(&cache).into_iter())
        .filter(|(r, _)| r == repo)
        .map(|(_, f)| f)
        .collect::<Vec<_>>();
    let wanted = match file {
        Some(file) => Some(file.as_str()),
        None => gguf(kept.iter().map(String::as_str)),
    };
    if let Some(path) = wanted.and_then(|file| hub::cached(&cache, repo, file)) {
        return Ok(path);
    }
    set(shared, Weighed::Asking(repo.clone()));
    let listing = hub::list(repo, stop)?;
    let file = match file {
        Some(file) => file.as_str(),
        None => gguf(listing.files.iter().map(|f| f.path.as_str()))
            .ok_or_else(|| format!("{repo} has no GGUF file"))?,
    };
    let remote = remote(&listing, repo, file)?;
    fetch(
        shared,
        &cache,
        repo,
        &listing.commit,
        remote,
        &weights.name(),
        stop,
    )
}

/// Hugging Face's cache, where models are kept.
#[cfg(feature = "summary")]
fn cache() -> Result<PathBuf, String> {
    hub::cache().ok_or_else(|| "no home directory for Hugging Face's cache".into())
}

/// `file` of `repo`, as `listing` has it.
#[cfg(feature = "summary")]
fn remote<'a>(
    listing: &'a hub::Listing,
    repo: &str,
    file: &str,
) -> Result<&'a hub::Remote, String> {
    (listing.files.iter())
        .find(|f| f.path == file)
        .ok_or_else(|| format!("{repo} has no {file}"))
}

/// The directory of a snapshot of `repo` with all of `files`, downloading
/// them into Hugging Face's cache first if they are not kept there.
#[cfg(feature = "summary")]
fn fetch_all(
    shared: &Shared,
    repo: &str,
    files: &[&str],
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    let cache = cache()?;
    let dir = (hub::cached(&cache, repo, files[0]))
        .and_then(|path| Some(path.parent()?.to_path_buf()))
        .filter(|dir| files.iter().all(|f| dir.join(f).is_file()));
    if let Some(dir) = dir {
        return Ok(dir);
    }
    let listing = hub::list(repo, stop)?;
    let name = repo.rsplit('/').next().unwrap_or(repo);
    let mut dir = None;
    for file in files {
        let remote = remote(&listing, repo, file)?;
        let path = fetch(shared, &cache, repo, &listing.commit, remote, name, stop)?;
        dir = path.parent().map(Path::to_path_buf);
    }
    dir.ok_or_else(|| format!("nothing of {repo} to fetch"))
}

/// Download `remote` of `repo` at `commit` into `cache`, shown as `what`.
#[cfg(feature = "summary")]
fn fetch(
    shared: &Shared,
    cache: &Path,
    repo: &str,
    commit: &str,
    remote: &hub::Remote,
    what: &str,
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    {
        let mut state = lock(shared);
        state.fetching = Some(Fetching {
            what: what.into(),
            part: hub::incomplete(&hub::blob(cache, repo, remote)),
            size: remote.size,
        });
        state.changed();
    }
    let fetched = hub::download(cache, repo, commit, remote, stop);
    let mut state = lock(shared);
    state.fetching = None;
    state.changed();
    fetched
}

#[cfg(not(feature = "summary"))]
impl Loader {
    pub(super) fn llm(&mut self, _: &Shared, _: &AtomicBool) -> Result<&mut Llm, String> {
        Err(NO_SUMMARY.into())
    }

    pub(super) fn embedder(&mut self, _: &Shared, _: &AtomicBool) -> Option<&Embedder> {
        None
    }
}
