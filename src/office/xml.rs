//! Elements and attributes by the names Office writes them with, as `w:p`,
//! whichever prefix a file binds, in transitional or strict Office Open XML.

use roxmltree::{Document, Node};

/// Elements nested deeper than this are not parsed: roxmltree parses
/// nested elements by recursing, so a file could otherwise run it out of
/// stack. Office nests a few dozen deep; past a hundred or so, unoptimized roxmltree runs out of a thread's 2 MiB.
const MOST_NESTED: usize = 100;

pub(super) fn parse(text: &str) -> Result<Document<'_>, roxmltree::Error> {
    if nested(text) > MOST_NESTED {
        return Err(roxmltree::Error::NodesLimitReached);
    }
    Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 20_000_000,
            ..roxmltree::ParsingOptions::default()
        },
    )
}

/// How deep `text`'s elements nest, as its tags say, `<` in comments and
/// character data counted as tags too, which only makes it deeper.
fn nested(text: &str) -> usize {
    let bytes = text.as_bytes();
    let (mut depth, mut most) = (0usize, 0usize);
    let mut i = 0;
    while let Some(at) = memchr(b'<', &bytes[i..]) {
        let start = i + at;
        let end = memchr(b'>', &bytes[start..]).map_or(bytes.len(), |e| start + e);
        match bytes.get(start + 1) {
            Some(b'/') => depth = depth.saturating_sub(1),
            Some(b'?' | b'!') => {}
            _ if bytes[..end].ends_with(b"/") => {}
            _ => {
                depth += 1;
                most = most.max(depth);
            }
        }
        i = end.min(bytes.len().saturating_sub(1)) + 1;
        if i >= bytes.len() {
            break;
        }
    }
    most
}

fn memchr(c: u8, bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&b| b == c)
}

/// The prefix Office writes for `uri`, or `""`.
fn prefix(uri: &str) -> &'static str {
    match uri {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
        | "http://purl.oclc.org/ooxml/wordprocessingml/main" => "w",
        "http://schemas.openxmlformats.org/officeDocument/2006/math"
        | "http://purl.oclc.org/ooxml/officeDocument/math" => "m",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        | "http://purl.oclc.org/ooxml/officeDocument/relationships" => "r",
        "http://schemas.openxmlformats.org/drawingml/2006/main"
        | "http://purl.oclc.org/ooxml/drawingml/main" => "a",
        "http://schemas.openxmlformats.org/presentationml/2006/main"
        | "http://purl.oclc.org/ooxml/presentationml/main" => "p",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
        | "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" => "wp",
        "http://schemas.openxmlformats.org/markup-compatibility/2006" => "mc",
        "urn:schemas-microsoft-com:vml" => "v",
        _ => "",
    }
}

/// Whether `n` is the element `name`, as `w:p`.
pub(super) fn is(n: Node, name: &str) -> bool {
    if !n.is_element() {
        return false;
    }
    let (want, local) = name.split_once(':').unwrap_or(("", name));
    n.tag_name().name() == local && prefix(n.tag_name().namespace().unwrap_or("")) == want
}

/// The attribute `name` of `n`, as `w:val`, or bare, as `val`.
pub(super) fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    let (want, local) = name.split_once(':').unwrap_or(("", name));
    n.attributes()
        .find(|a| a.name() == local && prefix(a.namespace().unwrap_or("")) == want)
        .map(|a| a.value())
}

/// The first child element of `n` named `name`.
pub(super) fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(*c, name))
}

/// The child elements of `n` named `name`.
pub(super) fn children<'a, 'i: 'a>(
    n: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| is(*c, name))
}

/// The first element under `n`, not `n` itself, named `name`.
pub(super) fn find<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.descendants().skip(1).find(|c| is(*c, name))
}

/// The `w:val` of `n`'s child `name`, as `w:pStyle`'s.
pub(super) fn val<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    let c = child(n, name)?;
    attr(c, "w:val").or_else(|| attr(c, "m:val"))
}

/// Whether an on-off property, as `w:b`, is on: there, and not `0`, `false`
/// or `off`.
pub(super) fn on(n: Option<Node>) -> bool {
    n.is_some_and(|n| {
        !matches!(
            attr(n, "w:val").or_else(|| attr(n, "m:val")),
            Some("0" | "false" | "off")
        )
    })
}

/// Of markup compatibility's alternatives, the one read: the first choice,
/// else the fallback.
pub(super) fn chosen<'a, 'i>(n: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    child(n, "mc:Choice").or_else(|| child(n, "mc:Fallback"))
}
