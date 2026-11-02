//! ohp's library: what presents beamer PDF slides, markdown, R Markdown
//! and HTML in the terminal, for the `ohp` program and for others, as
//! letterbox shows a message's pages with [`page::pages`].
//!
//! Slides are rasterised in pure Rust by hayro and shown through
//! ratatui-image, over the kitty graphics protocol where the terminal has it.
//! Markdown, R Markdown and HTML are typeset into a PDF first, by typst.

#[cfg(feature = "markdown")]
mod album;
pub mod app;
pub mod browse;
#[cfg(feature = "markdown")]
mod cite;
#[cfg(feature = "markdown")]
mod csl;
#[cfg(feature = "summary")]
mod embed;
#[cfg(any(feature = "speech", feature = "summary"))]
mod fetch;
#[cfg(feature = "markdown")]
mod html;
#[cfg(feature = "summary")]
mod hub;
#[cfg(feature = "markdown")]
mod knit;
#[cfg(feature = "markdown")]
mod latex;
pub mod link;
#[cfg(feature = "summary")]
mod llm;
#[cfg(feature = "markdown")]
mod markdown;
#[cfg(feature = "markdown")]
mod math;
mod menu;
#[cfg(feature = "markdown")]
mod mermaid;
#[cfg(feature = "summary")]
mod models;
mod notes;
pub mod print;
#[cfg(any(feature = "markdown", feature = "speech", feature = "summary"))]
pub mod process;
pub mod remote;
pub mod render;
mod salient;
#[cfg(feature = "sign")]
pub mod sign;
pub mod speak;
#[cfg(feature = "sign")]
mod split;
pub mod summary;
mod text;
#[cfg(feature = "markdown")]
pub mod typeset;
#[cfg(feature = "speech")]
pub mod voices;

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod fixture;

/// Where ohp keeps what it is given and what is picked, to sign and print
/// with: `$XDG_CONFIG_HOME/ohp`, else `~/.config/ohp`.
pub fn config() -> Option<std::path::PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
    };
    let dir = var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))?;
    Some(dir.join("ohp"))
}

/// Write `bytes` to `path` whole, its folder made if it is not there: a
/// reader never finds it half written.
pub fn write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(dir.unwrap_or(std::path::Path::new(".")))?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub mod page;

/// The status a shell gives a process a signal ended.
pub fn shell_status(signal: usize) -> u8 {
    u8::try_from(128 + signal).unwrap_or(u8::MAX)
}
