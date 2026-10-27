//! Summaries kept from one run of ohp to the next, in the user's cache
//! directory, by the model and the sentences summed up.

#[cfg(feature = "summary")]
use super::Key;
use super::{Summary, Weights};
use std::path::{Path, PathBuf};

/// Counts the ways summaries have been asked for and read: those kept from
/// another are not used.
const KEPT: u32 = 1;

/// Where summaries are kept from one run of ohp to the next: `ohp/summaries`
/// in the user's cache directory.
pub(super) fn kept_dir() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("ohp/summaries"))
}

/// The name the summary of `pages`, each its sentences, `what` it is of, by
/// the model in `weights`, is kept by: a hash of them all, FNV-1a's, the
/// same from one build of ohp to the next.
pub(super) fn kept_name<'a>(
    weights: &Weights,
    what: &str,
    pages: impl IntoIterator<Item = &'a [String]>,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(format!("{KEPT}\0{weights:?}\0{what}\0").as_bytes());
    for page in pages {
        for sentence in page {
            eat(sentence.as_bytes());
            eat(b"\0");
        }
        eat(b"\x01");
    }
    format!("{hash:016x}.json")
}

/// The summary kept as `name` in `dir`, if there is one.
#[cfg(feature = "summary")]
pub(super) fn recall(dir: &Path, name: &str) -> Option<Summary> {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(name)).ok()?).ok()?;
    let index = |v: &serde_json::Value| usize::try_from(v.as_u64()?).ok();
    let key = (json["key"].as_array()?.iter())
        .map(|k| {
            Some(Key {
                page: index(&k[0])?,
                sentence: index(&k[1])?,
                text: k[2].as_str()?.into(),
            })
        })
        .collect::<Option<_>>()?;
    Some(Summary {
        text: json["text"].as_str()?.into(),
        key,
    })
}

/// Keep `summary` as `name` in `dir`, whole or not at all.
#[cfg(feature = "summary")]
pub(super) fn keep(dir: &Path, name: &str, summary: &Summary) {
    let key: Vec<_> = (summary.key.iter())
        .map(|k| serde_json::json!([k.page, k.sentence, k.text]))
        .collect();
    let json = serde_json::json!({ "text": summary.text, "key": key });
    let part = dir.join(format!("{name}.part"));
    let kept = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&part, json.to_string()))
        .and_then(|()| std::fs::rename(&part, dir.join(name)));
    if kept.is_err() {
        let _ = std::fs::remove_file(&part);
    }
}

#[cfg(not(feature = "summary"))]
pub(super) fn recall(_: &Path, _: &str) -> Option<Summary> {
    None
}

#[cfg(not(feature = "summary"))]
pub(super) fn keep(_: &Path, _: &str, _: &Summary) {}
