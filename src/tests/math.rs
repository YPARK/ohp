use super::*;

fn ok(latex: &str) -> String {
    convert(latex).unwrap_or_else(|| panic!("{latex:?} not converted"))
}

#[test]
fn symbols_become_the_characters_they_stand_for() {
    assert_eq!(ok(r"\alpha + \beta \le \infty"), "α + β ≤ ∞");
    assert_eq!(ok(r"\epsilon \varepsilon \phi \varphi"), "ϵ ε ϕ φ");
}

#[test]
fn letters_are_set_apart_so_typst_does_not_read_a_name() {
    assert_eq!(ok("xy + 12.5z"), "x y + 12.5 z");
}

#[test]
fn attachments_take_one_argument_or_a_group() {
    assert_eq!(ok("x_{ij}^2"), "x_(i j)^(2)");
    assert_eq!(ok(r"\sum_{i=1}^n x_i"), "∑_(i = 1)^(n) x_(i)");
}

#[test]
fn an_empty_group_is_a_base_to_attach_to() {
    assert_eq!(ok("a {}^{14}C"), "a \"\"^(14) C");
    assert_eq!(ok("{}_{n}C_{k}"), "\"\"_(n) C_(k)");
    assert_eq!(ok("a{}b"), "a b");
}

#[test]
fn what_is_attached_to_a_group_is_attached_to_all_of_it() {
    assert_eq!(ok("{a+b}^2"), "scripts(a + b)^(2)");
    assert_eq!(ok("{x_1}^2"), "scripts(x_(1))^(2)");
    assert_eq!(ok("{\\sum}_i"), "scripts(∑)_(i)");
    assert_eq!(ok("{a}b"), "a b");
}

#[test]
fn commands_with_arguments_become_calls() {
    assert_eq!(ok(r"\frac{a}{b}"), "frac(a, b)");
    assert_eq!(ok(r"\sqrt[3]{x}"), "root(3, x)");
    assert_eq!(ok(r"\mathbf{x}^\top"), "bold(upright(x))^(⊤)");
    assert_eq!(ok(r"\hat\theta"), "hat(θ)");
    assert_eq!(ok(r"\max_\theta"), "op(\"max\", limits: #true)_(θ)");
}

#[test]
fn separators_inside_a_call_belong_to_the_formula() {
    assert_eq!(ok(r"\frac{a,b}{2}"), "frac(a \",\" b, 2)");
    assert_eq!(ok(r"\frac{\text{a, b}}{2}"), "frac(\"a, b\", 2)");
}

#[test]
fn text_is_kept_as_written() {
    assert_eq!(ok(r"x \text{if } x > 0"), "x \"if \" x > 0");
}

#[test]
fn characters_typst_reads_as_syntax_are_escaped() {
    assert_eq!(ok("a/b"), "a \\/ b");
    assert_eq!(ok(r"\{x\}"), "\\{ x \\}");
}

#[test]
fn environments_become_matrices_cases_and_aligned_rows() {
    assert_eq!(
        ok(r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}"),
        "mat(delim: \"(\", 1, 2; 3, 4)"
    );
    assert_eq!(
        ok(r"\begin{cases} 1 & x > 0 \\ 0 & \text{else} \end{cases}"),
        "cases(1 & x > 0, 0 & \"else\")"
    );
    assert_eq!(
        ok(r"\begin{aligned} a &= b \\ c &= d \\ \end{aligned}"),
        "a & = b \\ c & = d"
    );
}

#[test]
fn delimiter_sizes_are_left_to_typst() {
    assert_eq!(ok(r"\left( x \right."), "( x");
}

#[test]
fn what_is_not_known_or_not_whole_is_not_converted() {
    for latex in [
        r"\weird{x}",
        "x^",
        r"\frac{1}",
        "a}",
        r"\begin{tabular}x\end{tabular}",
    ] {
        assert_eq!(convert(latex), None, "{latex}");
    }
}
