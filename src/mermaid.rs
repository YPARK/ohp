//! A mermaid flowchart drawn as SVG.
//!
//! The page loads mermaid in the browser and leaves `graph LR` as text.
//! That script does not run here, so a flowchart is drawn from the source.
//! Anything else mermaid knows, a sequence or a pie, is left as code.

/// `source` drawn, when it is a `graph` or `flowchart`.
pub fn svg(source: &str) -> Option<String> {
    let graph = parse(source)?;
    if graph.nodes.is_empty() {
        return None;
    }
    Some(draw(&graph))
}

#[derive(Clone, Copy)]
enum Dir {
    LR,
    RL,
    TD,
    BT,
}

struct Node {
    id: String,
    label: String,
}

struct Edge {
    from: usize,
    to: usize,
    label: String,
}

struct Graph {
    dir: Dir,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

fn parse(source: &str) -> Option<Graph> {
    let mut graph = Graph {
        dir: Dir::TD,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut headed = false;
    for line in source.lines() {
        let line = strip_comment(line);
        for piece in split_statements(line) {
            let piece = piece.trim();
            if piece.is_empty() || directive(piece) {
                continue;
            }
            if let Some(dir) = direction(piece) {
                graph.dir = dir;
                headed = true;
                continue;
            }
            let Some((nodes, edges)) = statement(piece) else {
                continue;
            };
            let mut ids = Vec::new();
            for node in nodes {
                ids.push(touch(&mut graph, node.id, node.label));
            }
            for edge in edges {
                graph.edges.push(Edge {
                    from: ids[edge.from],
                    to: ids[edge.to],
                    label: edge.label,
                });
            }
        }
    }
    headed.then_some(graph)
}

fn strip_comment(line: &str) -> &str {
    line.find("%%").map(|i| &line[..i]).unwrap_or(line)
}

fn split_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth: i32 = 0;
    for (i, c) in line.char_indices() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                out.push(&line[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

fn directive(line: &str) -> bool {
    let word = line.split_whitespace().next().unwrap_or("");
    matches!(
        word.to_ascii_lowercase().as_str(),
        "style" | "classdef" | "class" | "click" | "linkstyle" | "subgraph" | "end" | "direction"
    )
}

fn direction(line: &str) -> Option<Dir> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    if !kind.eq_ignore_ascii_case("graph") && !kind.eq_ignore_ascii_case("flowchart") {
        return None;
    }
    let dir = match parts.next() {
        None => Dir::TD,
        Some("LR" | "lr") => Dir::LR,
        Some("RL" | "rl") => Dir::RL,
        Some("BT" | "bt") => Dir::BT,
        Some("TD" | "td" | "TB" | "tb") => Dir::TD,
        Some(_) => return None,
    };
    parts.next().is_none().then_some(dir)
}

struct Mention {
    id: String,
    label: String,
}

struct Link {
    from: usize,
    to: usize,
    label: String,
}

/// Nodes in order, and edges as indexes into that list.
fn statement(line: &str) -> Option<(Vec<Mention>, Vec<Link>)> {
    let mut rest = line.trim();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let (id, label, n) = node_at(rest)?;
    rest = rest[n..].trim_start();
    nodes.push(Mention { id, label });
    while !rest.is_empty() {
        let (label, n) = edge_at(rest)?;
        rest = rest[n..].trim_start();
        let (id, text, n) = node_at(rest)?;
        rest = rest[n..].trim_start();
        let from = nodes.len() - 1;
        nodes.push(Mention { id, label: text });
        edges.push(Link {
            from,
            to: nodes.len() - 1,
            label,
        });
    }
    Some((nodes, edges))
}

fn touch(graph: &mut Graph, id: String, label: String) -> usize {
    if let Some(i) = graph.nodes.iter().position(|n| n.id == id) {
        if graph.nodes[i].label == id && label != id {
            graph.nodes[i].label = label;
        }
        return i;
    }
    graph.nodes.push(Node { id, label });
    graph.nodes.len() - 1
}

fn node_at(s: &str) -> Option<(String, String, usize)> {
    let ws = s.len() - s.trim_start().len();
    let t = s.trim_start();
    let (id, n) = ident(t)?;
    let (label, sn) = shape(&t[n..])?;
    let label = if label.is_empty() { id.clone() } else { label };
    Some((id, label, ws + n + sn))
}

fn ident(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.first() == Some(&b'"') {
        let end = s[1..].find('"')? + 1;
        return Some((s[1..end].to_string(), end + 1));
    }
    let n = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .count();
    if n == 0 || s.as_bytes()[..n].iter().all(|c| *c == b'-') {
        return None;
    }
    Some((s[..n].to_string(), n))
}

fn shape(s: &str) -> Option<(String, usize)> {
    let Some(open) = s.chars().next() else {
        return Some((String::new(), 0));
    };
    let close = match open {
        '[' => ']',
        '(' => ')',
        '{' => '}',
        '>' => ']',
        _ => return Some((String::new(), 0)),
    };
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                let inner = s[open.len_utf8()..i].trim();
                let inner = inner.trim_matches(|c: char| {
                    matches!(c, '[' | ']' | '(' | ')' | '{' | '}' | '/' | '\\')
                });
                return Some((inner.trim().to_string(), i + c.len_utf8()));
            }
        }
    }
    None
}

fn edge_at(s: &str) -> Option<(String, usize)> {
    let ws = s.len() - s.trim_start().len();
    let (label, n) = edge_body(s.trim_start())?;
    Some((label, ws + n))
}

fn edge_body(t: &str) -> Option<(String, usize)> {
    for prefix in ["-.->", "==>", "-->", "---"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(label_after(rest, prefix.len()));
        }
    }
    let rest = t.strip_prefix("--")?;
    let end = rest.find("-->")?;
    let label = rest[..end].trim().trim_matches('|').trim().to_string();
    Some((label, 2 + end + 3))
}

fn label_after(rest: &str, prefix: usize) -> (String, usize) {
    let ws = rest.len() - rest.trim_start().len();
    let t = rest.trim_start();
    if let Some(inner) = t.strip_prefix('|')
        && let Some(end) = inner.find('|')
    {
        return (inner[..end].trim().to_string(), prefix + ws + 1 + end + 1);
    }
    (String::new(), prefix)
}

struct Box {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: String,
}

fn draw(graph: &Graph) -> String {
    let boxes = place(graph);
    let horizontal = matches!(graph.dir, Dir::LR | Dir::RL);
    let mut lane_y = boxes.iter().map(|b| b.y + b.h).fold(0.0, f32::max);
    let mut lane_x = boxes.iter().map(|b| b.x + b.w).fold(0.0, f32::max);
    let mut routes = Vec::new();
    for edge in &graph.edges {
        let from = &boxes[edge.from];
        let to = &boxes[edge.to];
        // An edge that skips a column would run through the node between.
        let points = if rank_span(graph, edge.from, edge.to) > 1 {
            if horizontal {
                lane_y += 22.0;
                around(from, to, true, lane_y)
            } else {
                lane_x += 22.0;
                around(from, to, false, lane_x)
            }
        } else {
            across(from, to, horizontal)
        };
        routes.push((points, edge.label.clone()));
    }
    let width = boxes
        .iter()
        .map(|b| b.x + b.w)
        .chain(routes.iter().flat_map(|(p, _)| p.iter().map(|q| q.0)))
        .fold(0.0, f32::max)
        + 16.0;
    let height = boxes
        .iter()
        .map(|b| b.y + b.h)
        .chain(routes.iter().flat_map(|(p, _)| p.iter().map(|q| q.1)))
        .fold(0.0, f32::max)
        + 16.0;
    let mut out = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.1} {height:.1}">"##
    );
    for (points, label) in &routes {
        out.push_str(&path(points));
        if !label.is_empty() {
            out.push_str(&edge_label(points, label));
        }
    }
    for b in &boxes {
        out.push_str(&format!(
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="4" fill="#fff" stroke="#222"/>"##,
            b.x, b.y, b.w, b.h
        ));
        let baseline = b.y + b.h / 2.0 + 5.0;
        out.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="14" fill="#111">{}</text>"##,
            b.x + b.w / 2.0,
            baseline,
            xml(&b.label)
        ));
    }
    out.push_str("</svg>");
    out
}

fn place(graph: &Graph) -> Vec<Box> {
    let rank = ranks(graph);
    let max_rank = rank.iter().copied().max().unwrap_or(0);
    let along = |r: usize| match graph.dir {
        Dir::LR | Dir::TD => r,
        Dir::RL | Dir::BT => max_rank - r,
    };
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (i, &r) in rank.iter().enumerate() {
        columns[along(r)].push(i);
    }
    let size: Vec<(f32, f32)> = graph
        .nodes
        .iter()
        .map(|n| {
            let w = n.label.chars().count() as f32 * 7.4 + 28.0;
            (w.max(48.0), 36.0)
        })
        .collect();
    let gap = graph
        .edges
        .iter()
        .map(|e| e.label.chars().count() as f32 * 7.2)
        .fold(40.0, f32::max)
        + 28.0;
    let horizontal = matches!(graph.dir, Dir::LR | Dir::RL);
    let col_span: Vec<f32> = columns
        .iter()
        .map(|col| {
            col.iter()
                .map(|&i| if horizontal { size[i].0 } else { size[i].1 })
                .fold(0.0, f32::max)
        })
        .collect();
    let mut origin = vec![16.0];
    for span in &col_span {
        let last = *origin.last().unwrap();
        origin.push(last + span + gap);
    }
    let stacks: Vec<f32> = columns
        .iter()
        .map(|col| {
            let n = col.len() as f32;
            let body: f32 = col
                .iter()
                .map(|&i| if horizontal { size[i].1 } else { size[i].0 })
                .sum();
            body + (n - 1.0).max(0.0) * 18.0
        })
        .collect();
    let tallest = stacks.iter().copied().fold(0.0, f32::max);
    let mut boxes: Vec<Box> = (0..graph.nodes.len())
        .map(|_| Box {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            label: String::new(),
        })
        .collect();
    for (c, col) in columns.iter().enumerate() {
        let mut at = 16.0 + (tallest - stacks[c]) / 2.0;
        for &i in col {
            let (w, h) = size[i];
            let (x, y) = if horizontal {
                let x = origin[c] + (col_span[c] - w) / 2.0;
                (x, at)
            } else {
                let x = at;
                let y = origin[c] + (col_span[c] - h) / 2.0;
                (x, y)
            };
            if horizontal {
                at += h + 18.0;
            } else {
                at += w + 18.0;
            }
            boxes[i] = Box {
                x,
                y,
                w,
                h,
                label: graph.nodes[i].label.clone(),
            };
        }
    }
    boxes
}

fn ranks(graph: &Graph) -> Vec<usize> {
    let mut rank = vec![None; graph.nodes.len()];
    let mut visiting = vec![false; graph.nodes.len()];
    fn depth(i: usize, graph: &Graph, rank: &mut [Option<usize>], visiting: &mut [bool]) -> usize {
        if let Some(r) = rank[i] {
            return r;
        }
        if visiting[i] {
            return 0;
        }
        visiting[i] = true;
        let mut r = 0;
        for e in &graph.edges {
            if e.to == i {
                r = r.max(depth(e.from, graph, rank, visiting) + 1);
            }
        }
        visiting[i] = false;
        rank[i] = Some(r);
        r
    }
    for i in 0..graph.nodes.len() {
        depth(i, graph, &mut rank, &mut visiting);
    }
    rank.into_iter().map(|r| r.unwrap_or(0)).collect()
}

fn rank_span(graph: &Graph, from: usize, to: usize) -> usize {
    let rank = ranks(graph);
    rank[from].abs_diff(rank[to])
}

fn across(from: &Box, to: &Box, horizontal: bool) -> Vec<(f32, f32)> {
    let (a, b) = ends(from, to, horizontal);
    if horizontal && (a.1 - b.1).abs() > 1.0 || !horizontal && (a.0 - b.0).abs() > 1.0 {
        let mid = if horizontal {
            ((a.0 + b.0) / 2.0, a.1)
        } else {
            (a.0, (a.1 + b.1) / 2.0)
        };
        let turn = if horizontal {
            (mid.0, b.1)
        } else {
            (b.0, mid.1)
        };
        vec![a, mid, turn, b]
    } else {
        vec![a, b]
    }
}

fn around(from: &Box, to: &Box, horizontal: bool, lane: f32) -> Vec<(f32, f32)> {
    if horizontal {
        let a = (from.x + from.w / 2.0, from.y + from.h);
        let b = (to.x + to.w / 2.0, to.y + to.h);
        vec![a, (a.0, lane), (b.0, lane), b]
    } else {
        let a = (from.x + from.w, from.y + from.h / 2.0);
        let b = (to.x + to.w, to.y + to.h / 2.0);
        vec![a, (lane, a.1), (lane, b.1), b]
    }
}

fn ends(from: &Box, to: &Box, horizontal: bool) -> ((f32, f32), (f32, f32)) {
    if horizontal {
        if from.x <= to.x {
            (
                (from.x + from.w, from.y + from.h / 2.0),
                (to.x, to.y + to.h / 2.0),
            )
        } else {
            (
                (from.x, from.y + from.h / 2.0),
                (to.x + to.w, to.y + to.h / 2.0),
            )
        }
    } else if from.y <= to.y {
        (
            (from.x + from.w / 2.0, from.y + from.h),
            (to.x + to.w / 2.0, to.y),
        )
    } else {
        (
            (from.x + from.w / 2.0, from.y),
            (to.x + to.w / 2.0, to.y + to.h),
        )
    }
}

fn path(points: &[(f32, f32)]) -> String {
    let mut d = format!(r#"<path d="M{:.1},{:.1}"#, points[0].0, points[0].1);
    for p in &points[1..] {
        d.push_str(&format!(" L{:.1},{:.1}", p.0, p.1));
    }
    let (x1, y1) = points[points.len() - 2];
    let (x2, y2) = points[points.len() - 1];
    d.push_str(r##"" fill="none" stroke="#222" stroke-width="1.4"/>"##);
    d.push_str(&head(x1, y1, x2, y2));
    d
}

fn head(x1: f32, y1: f32, x2: f32, y2: f32) -> String {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt().max(0.01);
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (-uy, ux);
    let back = 9.0;
    let wing = 4.0;
    let (bx, by) = (x2 - ux * back, y2 - uy * back);
    format!(
        r##"<polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="#222"/>"##,
        x2,
        y2,
        bx + px * wing,
        by + py * wing,
        bx - px * wing,
        by - py * wing
    )
}

fn edge_label(points: &[(f32, f32)], label: &str) -> String {
    let (a, b) = points
        .windows(2)
        .max_by(|p, q| {
            let len = |s: &[(f32, f32)]| {
                let (dx, dy) = (s[1].0 - s[0].0, s[1].1 - s[0].1);
                dx * dx + dy * dy
            };
            len(p).total_cmp(&len(q))
        })
        .map(|w| (w[0], w[1]))
        .unwrap_or((points[0], *points.last().unwrap()));
    // On the longest run, above the stroke.
    let (x, y) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0 - 6.0);
    let w = label.chars().count() as f32 * 7.2 + 8.0;
    format!(
        r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="16" fill="#fff"/><text x="{:.1}" y="{:.1}" text-anchor="middle" font-family="sans-serif" font-size="12" fill="#333">{}</text>"##,
        x - w / 2.0,
        y - 12.0,
        w,
        x,
        y,
        xml(label)
    )
}

fn xml(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collider_is_drawn_left_to_right() {
        let src = "\
graph LR
  G[Genotype] -->|eQTL effect| E[Gene expression]
  G --> C[Cell type / expression PCs]
  E --> C";
        let svg = svg(src).unwrap();
        assert!(svg.contains("Genotype"), "{svg}");
        assert!(svg.contains("Gene expression"), "{svg}");
        assert!(svg.contains("Cell type / expression PCs"), "{svg}");
        assert!(svg.contains("eQTL effect"), "{svg}");
        let g = text_x(&svg, "Genotype");
        let e = text_x(&svg, "Gene expression");
        let c = text_x(&svg, "Cell type / expression PCs");
        assert!(g < e && e < c, "{g} {e} {c}");
    }

    #[test]
    fn a_sequence_is_not_a_flowchart() {
        assert!(svg("sequenceDiagram\nAlice->>Bob: hi").is_none());
    }

    fn text_x(svg: &str, label: &str) -> f32 {
        let at = svg.find(&format!(">{label}<")).expect(svg);
        let x = svg[..at].rfind("x=\"").unwrap();
        svg[x + 3..at].split('"').next().unwrap().parse().unwrap()
    }
}
