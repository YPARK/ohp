use super::*;
use crate::latex::{Formula, Rendered};
use crate::markdown::{Setting, convert};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

static NONE: LazyLock<HashSet<usize>> = LazyLock::new(HashSet::new);
static LATEX: LazyLock<HashMap<Formula, Rendered>> = LazyLock::new(HashMap::new);

fn setting() -> Setting<'static> {
    Setting {
        base: Path::new("."),
        citations: false,
        paper: None,
        raw_math: &NONE,
        source_only: &NONE,
        latex: &LATEX,
        broken: &NONE,
    }
}

fn md(html: &str) -> String {
    read(html).markdown
}

fn typst_of(html: &str) -> String {
    convert(&md(html), &setting()).source
}

#[test]
fn a_page_keeps_its_text_and_leaves_out_the_chrome() {
    let html = r##"
        <html><head><title>Notes</title><style>p { color: red }</style></head>
        <body>
          <nav><a href="/">Home</a></nav>
          <article>
            <h1>Notes</h1>
            <p>Hello <strong>there</strong> &amp; <em>welcome</em>.</p>
            <p>See <a href="https://example.com">the note</a>.</p>
          </article>
          <footer>copyright</footer>
          <div id="TOC"><a href="#notes">Notes</a></div>
        </body></html>"##;
    let out = md(html);
    assert!(out.contains("Hello"), "{out}");
    assert!(out.contains("there"), "{out}");
    assert!(out.contains("&"), "{out}");
    assert!(out.contains("https://example.com"), "{out}");
    assert!(!out.contains("Home"), "{out}");
    assert!(!out.contains("copyright"), "{out}");
    assert!(!out.contains("color: red"), "{out}");
    // The title and the h1 are the same, so it is a title, not a heading too.
    assert!(out.contains("title: \"Notes\""), "{out}");
    assert!(!out.contains("# Notes"), "{out}");
    let set = typst_of(html);
    assert!(set.contains("#strong["), "{set}");
    assert!(set.contains("#emph["), "{set}");
    assert!(set.contains("#link(\"https://example.com\")"), "{set}");
}

#[test]
fn lists_tables_code_and_a_rule_are_set() {
    let html = r#"
        <ul><li>one</li><li>two<ul><li>nested</li></ul></li></ul>
        <ol><li>first</li><li>second</li></ol>
        <ul><li><input type="checkbox" checked> done</li></ul>
        <table>
          <thead><tr><th>A</th><th align="right">B</th></tr></thead>
          <tbody><tr><td>1</td><td>2</td></tr></tbody>
        </table>
        <pre><code class="language-python">print(1)</code></pre>
        <p>before</p><hr><p>after</p>
        <p>price is $5 today</p>
        <p>x<sup>2</sup></p>"#;
    let out = md(html);
    assert!(out.contains("- one"), "{out}");
    assert!(
        out.contains("- nested") || out.contains("  - nested"),
        "{out}"
    );
    assert!(out.contains("1. first"), "{out}");
    assert!(out.contains("- [x] done"), "{out}");
    assert!(out.contains("| A |"), "{out}");
    assert!(out.contains("---:"), "{out}");
    assert!(out.contains("```python"), "{out}");
    assert!(out.contains("print(1)"), "{out}");
    assert!(out.contains("<hr>"), "{out}");
    assert!(out.contains("\\$5"), "{out}");
    assert!(out.contains("$^{2}$"), "{out}");
    let set = typst_of(html);
    assert!(set.matches("#list").count() >= 2, "{set}");
    assert!(set.contains("#enum"), "{set}");
    assert!(set.contains('☒'), "{set}");
    assert!(set.contains("#table"), "{set}");
    assert!(set.contains("right"), "{set}");
    assert!(set.contains("lang: \"python\""), "{set}");
    assert!(set.contains("#line(length: 100%"), "{set}");
}

#[test]
fn a_formula_is_taken_from_its_tex() {
    let html = r#"
        <p>Energy <span class="math inline">\(E = mc^2\)</span>.</p>
        <p><span class="math display">\[a^2\]</span></p>
        <p>See \(x\) too.</p>
        <math display="block"><semantics>
          <annotation encoding="application/x-tex">\frac{1}{2}</annotation>
        </semantics></math>"#;
    let out = md(html);
    assert!(out.contains("$E = mc^2$"), "{out}");
    assert!(out.contains("$$"), "{out}");
    assert!(out.contains("a^2"), "{out}");
    assert!(out.contains("$x$"), "{out}");
    assert!(out.contains("\\frac{1}{2}"), "{out}");
    assert!(!out.contains("semantics"), "{out}");
}

#[test]
fn a_script_is_not_shown_even_when_it_looks_like_markup() {
    let html = r#"<script>if (a < b) { document.write("<p>secret</p>"); }</script><p>Shown</p>"#;
    let out = md(html);
    assert!(out.contains("Shown"), "{out}");
    assert!(!out.contains("secret"), "{out}");
}

#[test]
fn a_reveal_deck_is_slides_and_remark_keeps_its_source() {
    let reveal = r#"<div class="reveal"><div class="slides">
        <section><h2>One</h2><p>a</p></section>
        <section><h2>Two</h2><p>b</p></section>
        <section class="notes"><p>speaker</p></section>
      </div></div>"#;
    let out = md(reveal);
    assert!(out.contains("output: revealjs"), "{out}");
    let one = out.find("One").expect(&out);
    let two = out.find("Two").expect(&out);
    assert!(out[one..two].contains("---"), "{out}");
    assert!(!out.contains("speaker"), "{out}");
    let vertical = r#"<div class="slides"><section>
        <section><h2>A</h2><p>a</p></section>
        <section><h2>B</h2><p>b</p></section>
      </section></div>"#;
    let out = md(vertical);
    let a = out.find("## A").expect(&out);
    let b = out.find("## B").expect(&out);
    assert!(out[a..b].contains("---"), "{out}");
    let set = convert(&out, &setting()).source;
    assert!(set.contains("presentation-16-9"), "{set}");

    let remark =
        "<textarea id=\"source\">\n---\ntitle: Talk\noutput: xaringan\n---\n\n# Hello\n</textarea>";
    let out = md(remark);
    assert!(out.contains("title: Talk"), "{out}");
    assert!(out.contains("xaringan"), "{out}");
    assert!(out.contains("# Hello"), "{out}");
    assert!(!out.contains("<textarea"), "{out}");

    let bare = "<textarea id=\"source\"># Hello\n\n---\n\n## Next\n</textarea>";
    let out = md(bare);
    assert!(out.contains("output: revealjs"), "{out}");
    assert!(out.contains("## Next"), "{out}");

    let crlf =
        "<textarea id=\"source\">+++\r\ntitle = \"Talk\"\r\n+++\r\n\r\n# Hello\r\n</textarea>";
    let out = md(crlf);
    assert!(out.starts_with("+++"), "{out}");
    assert!(!out.contains("revealjs"), "{out}");
}

#[test]
fn katex_and_mathjax_keep_the_tex_not_the_drawing() {
    let katex = r#"
        <p>Energy <span class="katex">
          <span class="katex-mathml"><math><semantics>
            <annotation encoding="application/x-tex">E = mc^2</annotation>
          </semantics></math></span>
          <span class="katex-html" aria-hidden="true">RENDERED</span>
        </span>.</p>
        <span class="katex-display"><span class="katex">
          <span class="katex-mathml"><math><semantics>
            <annotation encoding="application/x-tex">a^2</annotation>
          </semantics></math></span>
          <span class="katex-html" aria-hidden="true">RENDERED</span>
        </span></span>"#;
    let out = md(katex);
    assert!(out.contains("$E = mc^2$"), "{out}");
    assert!(out.contains("a^2"), "{out}");
    assert!(out.contains("$$"), "{out}");
    assert!(!out.contains("RENDERED"), "{out}");

    let mathjax = r#"
        <mjx-container class="MathJax" display="true">
          <mjx-assistive-mml><math><semantics>
            <annotation encoding="application/x-tex">c^2</annotation>
          </semantics></math></mjx-assistive-mml>
          <span>RENDERED</span>
        </mjx-container>
        <span class="MathJax">
          <span class="MathJax_Preview">PREVIEW</span>
          <script type="math/tex">b^2</script>
        </span>"#;
    let out = md(mathjax);
    assert!(out.contains("$b^2$"), "{out}");
    assert!(out.contains("c^2"), "{out}");
    assert!(!out.contains("RENDERED"), "{out}");
    assert!(!out.contains("PREVIEW"), "{out}");

    // A slide that is only the rendered formula is still that slide.
    let only = r#"<div class="slides"><section>
        <span class="katex"><span class="katex-mathml"><math><semantics>
          <annotation encoding="application/x-tex">x</annotation>
        </semantics></math></span></span>
      </section></div>"#;
    let out = md(only);
    assert!(out.contains("output: revealjs"), "{out}");
    assert!(out.contains("$x$"), "{out}");
}

#[test]
fn a_drawing_in_the_page_is_shown() {
    let html = r#"
        <svg aria-label="box"><rect width="10" height="10"/></svg>
        <div class="mermaid"><svg><rect width="4" height="4"/></svg></div>
        <div class="mermaid">graph LR
  A --> B</div>
        <pre class="mermaid">sequenceDiagram
Alice->>Bob: hi</pre>
        <img alt="lazy" data-src="plots/dot.png">
        <object type="image/png" data="plots/dot.png"></object>"#;
    let read = read(html);
    let out = &read.markdown;
    assert!(out.contains("sequenceDiagram"), "{out}");
    assert!(!out.contains("graph LR"), "{out}");
    assert!(!out.contains("<rect"), "{out}");
    let paths = image_paths(out);
    assert_eq!(paths.len(), 5, "{out}");
    let svg = std::fs::read_to_string(paths[0]).unwrap();
    assert!(svg.contains("<rect"), "{svg}");
    assert!(
        svg.contains("xmlns=\"http://www.w3.org/2000/svg\""),
        "{svg}"
    );
    assert!(out.contains("plots/dot.png"), "{out}");
    let drawn = std::fs::read_to_string(paths[2]).unwrap();
    assert!(drawn.contains(">A<") && drawn.contains(">B<"), "{drawn}");
    let set = convert(out, &setting()).source;
    assert!(set.matches("image(").count() >= 2, "{set}");
}

#[test]
fn a_mermaid_flowchart_is_drawn() {
    let html = r#"<article><pre class="mermaid">
graph LR
  G[Genotype] -->|eQTL effect| E[Gene expression]
  G --> C[Cell type / expression PCs]
  E --> C
</pre></article>"#;
    let read = read(html);
    assert!(!read.markdown.contains("graph LR"), "{}", read.markdown);
    let paths = image_paths(&read.markdown);
    assert_eq!(paths.len(), 1, "{}", read.markdown);
    let svg = std::fs::read_to_string(paths[0]).unwrap();
    assert!(svg.contains("Genotype"), "{svg}");
    assert!(svg.contains("eQTL effect"), "{svg}");
    assert!(svg.contains("Cell type / expression PCs"), "{svg}");
}

#[test]
fn an_embedded_image_is_a_file_the_markdown_names() {
    // Padding is omitted, as exported slides often do.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
        .trim_end_matches('=');
    let html = format!(
        r#"<img alt="dot" src="data:image/png;base64,{png}">
           <img alt="box" src="data:image/svg+xml,%3Csvg%3E%3C/svg%3E">"#
    );
    let read = read(&html);
    let paths = image_paths(&read.markdown);
    assert_eq!(paths.len(), 2, "{}", read.markdown);
    let png_bytes = std::fs::read(paths[0]).unwrap();
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(std::fs::read_to_string(paths[1]).unwrap(), "<svg></svg>");
    assert!(read.files().is_some());
}

fn image_paths(markdown: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("](<") {
        rest = &rest[at + 3..];
        let end = rest.find(">)").expect(markdown);
        paths.push(&rest[..end]);
        rest = &rest[end + 2..];
    }
    paths
}
