use super::*;

const NS: &str = r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

fn tex(inner: &str) -> String {
    let xml = format!("<m:oMath {NS}>{inner}</m:oMath>");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    latex(doc.root_element())
}

fn r(t: &str) -> String {
    format!("<m:r><m:t>{t}</m:t></m:r>")
}

fn e(name: &str, inner: &str) -> String {
    format!("<m:{name}>{inner}</m:{name}>")
}

#[test]
fn fractions_scripts_and_roots() {
    assert_eq!(
        tex(&e("f", &(e("num", &r("1")) + &e("den", &r("x+1"))))),
        "\\frac{1}{x+1}"
    );
    assert_eq!(
        tex(&e(
            "sSubSup",
            &(e("e", &r("x")) + &e("sub", &r("i")) + &e("sup", &r("2")))
        )),
        "x_{i}^{2}"
    );
    assert_eq!(
        tex(&e("sSup", &(e("e", &r("ab")) + &e("sup", &r("n"))))),
        "{ab}^{n}"
    );
    assert_eq!(
        tex(&e("rad", &(e("deg", "") + &e("e", &r("2"))))),
        "\\sqrt{2}"
    );
    assert_eq!(
        tex(&e("rad", &(e("deg", &r("3")) + &e("e", &r("x"))))),
        "\\sqrt[3]{x}"
    );
}

#[test]
fn sums_integrals_and_delimiters() {
    let sum = "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary>";
    assert_eq!(tex(sum), "\\sum_{i=1}^{n} i");
    let int = format!(
        "<m:nary>{}{}{}</m:nary>",
        e("sub", &r("0")),
        e("sup", &r("1")),
        e("e", &r("x dx"))
    );
    assert_eq!(tex(&int), "\\int_{0}^{1} x dx");
    let bracket = format!(
        "<m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"\"/></m:dPr>{}{}</m:d>",
        e("e", &r("a")),
        e("e", &r("b"))
    );
    assert_eq!(tex(&bracket), "\\left[ a | b \\right.");
}

#[test]
fn functions_limits_accents_and_text() {
    let sin = format!(
        "<m:func>{}{}</m:func>",
        e("fName", &r("sin")),
        e("e", &r("x"))
    );
    assert_eq!(tex(&sin), "\\sin x");
    let lim = format!(
        "<m:func><m:fName><m:limLow>{}{}</m:limLow></m:fName>{}</m:func>",
        e("e", &r("lim")),
        e("lim", &r("n→∞")),
        e("e", &r("a")),
    );
    assert_eq!(tex(&lim), "\\lim_{n→∞} a");
    let hat = format!("<m:acc>{}</m:acc>", e("e", &r("θ")));
    assert_eq!(tex(&hat), "\\hat{θ}");
    let words = "<m:r><m:rPr><m:nor/></m:rPr><m:t>if x</m:t></m:r>";
    assert_eq!(tex(words), "\\text{if x}");
    assert_eq!(tex(&r("50% of {a}")), "50\\% of \\{a\\}");
    assert_eq!(tex(&r("L\u{200B}")), "L");
}

#[test]
fn matrices_and_arrays() {
    let row = |a: &str, b: &str| format!("<m:mr>{}{}</m:mr>", e("e", &r(a)), e("e", &r(b)));
    let m = format!("<m:m>{}{}</m:m>", row("1", "0"), row("0", "1"));
    assert_eq!(tex(&m), "\\begin{matrix}1 & 0 \\\\ 0 & 1\\end{matrix}");
    let arr = format!(
        "<m:eqArr>{}{}</m:eqArr>",
        e("e", &r("x&amp;=1")),
        e("e", &r("y&amp;=2"))
    );
    assert_eq!(tex(&arr), "\\begin{aligned}x&=1 \\\\ y&=2\\end{aligned}");
}

#[test]
fn deep_nesting_is_refused_or_read_for_its_text() {
    let mut inner = r("z");
    for _ in 0..200 {
        inner = e("sSup", &(e("e", &inner) + &e("sup", &r("2"))));
    }
    let xml = format!("<m:oMath {NS}>{inner}</m:oMath>");
    assert!(crate::office::xml::parse(&xml).is_err());
    // As deep as it may be parsed, it is read for its text past its depth.
    let mut inner = r("z");
    for _ in 0..30 {
        inner = e("sSup", &(e("e", &inner) + &e("sup", &r("2"))));
    }
    assert!(tex(&inner).contains('z'));
}

#[test]
fn text_in_an_equation_keeps_dollars_and_percents_in_the_formula() {
    let run = "<m:r><m:rPr><m:nor/></m:rPr><m:t>costs $5, 50% off</m:t></m:r>";
    assert_eq!(tex(run), "\\text{costs \\$5, 50\\% off}");
}
