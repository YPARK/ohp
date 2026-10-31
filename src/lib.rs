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
#[cfg(any(feature = "markdown", feature = "speech", feature = "summary"))]
pub mod process;
pub mod remote;
pub mod render;
mod salient;
#[cfg(feature = "sign")]
pub mod sign;
pub mod speak;
pub mod summary;
mod text;
#[cfg(feature = "markdown")]
pub mod typeset;
#[cfg(feature = "speech")]
pub mod voices;

#[cfg(test)]
#[path = "tests/fixture.rs"]
mod fixture;

pub mod page;

/// The status a shell gives a process a signal ended.
pub fn shell_status(signal: usize) -> u8 {
    u8::try_from(128 + signal).unwrap_or(u8::MAX)
}
