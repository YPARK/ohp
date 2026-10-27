//! The models ohp suggests to sum up with, and the weights of the one
//! given or picked: a GGUF file here, or one of a repo on Hugging Face's
//! Hub.

use std::path::PathBuf;

/// A model ohp suggests: its name, its GGUF file on Hugging Face's Hub, how
/// large it is, and what it is like.
#[derive(Debug, PartialEq)]
pub struct Suggested {
    pub name: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub gb: f32,
    pub about: &'static str,
}

pub const SUGGESTED: [Suggested; 5] = [
    Suggested {
        name: "qwen2.5-3b",
        repo: "Qwen/Qwen2.5-3B-Instruct-GGUF",
        file: "qwen2.5-3b-instruct-q4_k_m.gguf",
        gb: 2.1,
        about: "apt and quick on a GPU",
    },
    Suggested {
        name: "qwen2.5-1.5b",
        repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        gb: 1.1,
        about: "twice as quick, its picks less apt",
    },
    Suggested {
        name: "qwen3-4b",
        repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        gb: 2.5,
        about: "newer, and a little slower",
    },
    Suggested {
        name: "llama3.2-3b",
        repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        file: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        gb: 2.0,
        about: "Meta's",
    },
    Suggested {
        name: "qwen2.5-7b",
        repo: "bartowski/Qwen2.5-7B-Instruct-GGUF",
        file: "Qwen2.5-7B-Instruct-Q4_K_M.gguf",
        gb: 4.7,
        about: "the best of these, and the slowest",
    },
];

/// The model pages are summed up with unless one is given: the larger
/// where a GPU runs it, the quicker on a CPU.
fn default_model() -> &'static Suggested {
    if cfg!(target_os = "macos") {
        &SUGGESTED[0]
    } else {
        &SUGGESTED[1]
    }
}

/// The weights of the model to sum up with.
#[derive(Clone, Debug, PartialEq)]
pub enum Weights {
    /// A GGUF file of a repo on Hugging Face's Hub, or else the one `gguf`
    /// picks of its files.
    Hub { repo: String, file: Option<String> },
    /// A GGUF file here.
    File(PathBuf),
}

impl Default for Weights {
    fn default() -> Self {
        Weights::from(default_model())
    }
}

impl From<&Suggested> for Weights {
    fn from(model: &Suggested) -> Self {
        Weights::Hub {
            repo: model.repo.into(),
            file: Some(model.file.into()),
        }
    }
}

impl Weights {
    /// What the model is called: its name if ohp suggests it, else its file,
    /// or its repo.
    pub fn name(&self) -> String {
        if let Some(model) = SUGGESTED.iter().find(|m| Weights::from(*m) == *self) {
            return model.name.into();
        }
        match self {
            Weights::Hub {
                file: Some(file), ..
            } => file.trim_end_matches(".gguf").into(),
            Weights::Hub { repo, file: None } => repo.clone(),
            Weights::File(path) => crate::render::name(path),
        }
    }
}

/// The weights `s` names: a model ohp suggests, by name, a GGUF file here,
/// or a repo on Hugging Face's Hub, as `hub_named` reads it.
pub fn weights(s: &str) -> Result<Weights, String> {
    if let Some(model) = SUGGESTED.iter().find(|m| m.name.eq_ignore_ascii_case(s)) {
        return Ok(Weights::from(model));
    }
    let path = PathBuf::from(s);
    if path.is_file() {
        return Ok(Weights::File(path));
    }
    hub_named(s).ok_or_else(|| {
        let names: Vec<_> = SUGGESTED.iter().map(|m| m.name).collect();
        format!(
            "`{s}` is neither a GGUF file, a repo on Hugging Face as `owner/repo`, nor one of {}",
            names.join(", ")
        )
    })
}

/// The repo on Hugging Face's Hub, and its GGUF file, `s` names: as
/// `owner/repo` or `owner/repo/file.gguf`, or as the Hub's page for either.
pub fn hub_named(s: &str) -> Option<Weights> {
    let s = s.trim();
    let s = ["https://", "http://"]
        .iter()
        .fold(s, |s, scheme| s.strip_prefix(scheme).unwrap_or(s));
    let s = ["huggingface.co/", "hf.co/"]
        .iter()
        .fold(s, |s, host| s.strip_prefix(host).unwrap_or(s));
    let named = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && (part.chars()).all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    let mut parts = s.splitn(3, '/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if !named(owner) || !named(name) {
        return None;
    }
    let file = match parts.next() {
        None => None,
        Some(rest) => {
            let rest = ["resolve/main/", "blob/main/", "tree/main/"]
                .iter()
                .fold(rest, |rest, at| rest.strip_prefix(at).unwrap_or(rest));
            if !is_gguf(rest) {
                return None;
            }
            Some(rest.to_string())
        }
    };
    Some(Weights::Hub {
        repo: format!("{owner}/{name}"),
        file,
    })
}

/// Whether `file` is a GGUF file, by its name, in any case.
pub fn is_gguf(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".gguf")
}

/// Quantisations to take, most liked first: 4 bits a weight is apt and
/// quick enough.
const QUANTS: [&str; 6] = ["q4_k_m", "q4_k_s", "q5_k_m", "q4_0", "q6_k", "q8_0"];

/// The GGUF file of a repo's `files` to sum up with: of one piece, and
/// quantised as most liked.
pub fn gguf<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let files: Vec<&str> = (files.into_iter())
        .filter(|f| {
            let f = f.to_ascii_lowercase();
            f.ends_with(".gguf") && !f.contains("-of-0") && !f.contains("mmproj")
        })
        .collect();
    let quantised = |q: &str| (files.iter()).find(|f| f.to_ascii_lowercase().contains(q));
    QUANTS
        .iter()
        .find_map(|q| quantised(q))
        .or(files.first())
        .copied()
}
