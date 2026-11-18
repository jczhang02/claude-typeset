//! Mermaid mindmaps, and flowcharts shaped as trees, drawn as lines of text
//! by the mermaid-text crate: the root in a rounded box, every other node on
//! a line of its own under its parent (├── and └──), so a child's parent is
//! never in doubt and a wide tree grows down, not sideways.
//!
//! The hooks' own renderer (beautiful-mermaid) stacks the nodes of one depth
//! in one column and merges edges from different parents into one trunk, so
//! a tree drawn there loses who is whose child; it stays the renderer for
//! every other diagram. A flowchart reaches the tree drawing only when it is
//! a tree: one root, at most one parent per node, no edge labels, no
//! subgraphs, no cycles, and some node with two children or more (a plain
//! chain of steps reads better as boxes and arrows than as a staircase).
//! Anything else is answered "not a tree", and the hooks draw it themselves.
//!
//! A tree whose root starts a chain written in one statement ("A --> B -->
//! C") is a sequence of steps with details hanging off them, not a
//! hierarchy seven levels deep: its steps are drawn one under another on a
//! line of their own ("● "), each with its other children as an outline.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::LazyLock;

use mermaid_text::types::{EdgeEndpoint, Graph};
use mermaid_text::{Mindmap, MindmapNode};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

pub const MAX_ROWS: usize = 80;
pub const MAX_SOURCE: usize = 8000; // characters, as the hooks take for a diagram
/// UTF-16 units of one line the hooks take (four per column of the widest Image).
const MAX_LINE_UNITS: usize = 4 * crate::protocol::MAX_BOX as usize;
const MAX_LINES: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    pub lines: Vec<String>,
    pub columns: usize,
}

/// The diagram drawn in at most `max_columns` columns and MAX_ROWS rows, or
/// why not. "not a tree" starts the answer for a flowchart that the hooks
/// must draw themselves.
pub fn draw(source: &str, max_columns: usize) -> Result<Drawing, String> {
    if source.chars().count() > MAX_SOURCE {
        return Err(format!("diagram source is over {MAX_SOURCE} characters"));
    }
    let source = join_strings(&clean(source))?;
    let lines: Vec<&str> = source.lines().filter(|line| !skipped(line)).collect();
    if lines.len() > MAX_LINES {
        return Err(format!("diagram source is over {MAX_LINES} lines"));
    }
    let Some(first) = lines.first() else {
        return Err("diagram is empty".into());
    };
    let word = first.trim().split([' ', '\t', ';']).next().unwrap_or("");
    let is_mindmap = word.eq_ignore_ascii_case("mindmap");
    let is_flowchart = word.eq_ignore_ascii_case("flowchart") || word.eq_ignore_ascii_case("graph");
    if !is_mindmap && !is_flowchart {
        let word: String = word.chars().take(40).collect();
        return Err(format!("not a tree: {word} diagrams are not drawn as trees"));
    }
    let body = lines.join("\n");
    // The crate is young: a panic in it fails this diagram alone.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if is_mindmap {
            let map = mindmap(&body)?;
            return fit(
                |width| mermaid_text::render::mindmap::render(&map, width),
                max_columns,
                "mindmap",
            );
        }
        match tree(&body)? {
            Tree::Outline(map) => fit(
                |width| mermaid_text::render::mindmap::render(&map, width),
                max_columns,
                "tree",
            ),
            Tree::Steps(steps) => fit(|width| draw_steps(&steps, width), max_columns, "tree"),
        }
    }));
    outcome.unwrap_or_else(|_| Err("diagram drawing failed on this diagram".into()))
}

/// Line ends made '\n', control characters (but tabs) dropped, front matter cut.
fn clean(source: &str) -> String {
    let text: String = source
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    let mut lines = text.lines().skip_while(|line| line.trim().is_empty()).peekable();
    if lines.peek().is_some_and(|line| line.trim() == "---") {
        let rest: Vec<&str> = lines.clone().skip(1).collect();
        if let Some(close) = rest.iter().position(|line| line.trim() == "---") {
            return rest[close + 1..].join("\n");
        }
    }
    text
}

/// A markdown string that runs over lines ("`one\ntwo`", as mermaid
/// allows) put on one line, which both parsers need: else each line of it
/// would be read as a node of its own. One that never closes is refused,
/// as mermaid refuses it.
fn join_strings(source: &str) -> Result<String, String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        let open = line
            .rfind("\"`")
            .is_some_and(|start| !line[start + 2..].contains("`\""));
        if !open {
            out.push(line.to_string());
            at += 1;
            continue;
        }
        let Some(close) = (at + 1..lines.len()).find(|&next| lines[next].contains("`\"")) else {
            return Err("a markdown string (\"`...`\") never closes".into());
        };
        // The first line's indentation stays: in a mindmap it is the depth.
        let indent = &line[..line.len() - line.trim_start().len()];
        let parts: Vec<&str> = lines[at..=close].iter().map(|part| part.trim()).collect();
        out.push(format!("{indent}{}", parts.join(" ")));
        at = close + 1;
    }
    Ok(out.join("\n"))
}

/// Blank lines and %% comments (and %%{init}%% directives), which neither parser draws.
fn skipped(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.starts_with("%%")
}

/// A node's text as the reader should see it: one line, its quotes and
/// markdown-string backticks gone, white space collapsed.
fn tidy(text: &str) -> String {
    let mut text = text.trim();
    if let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        text = inner;
    }
    if let Some(inner) = text.strip_prefix('`').and_then(|t| t.strip_suffix('`')) {
        text = inner;
    }
    let mut out = text.to_string();
    for tag in ["<br/>", "<br />", "<br>", "<BR/>", "<BR />", "<BR>"] {
        out = out.replace(tag, " ");
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tidy_all(node: &mut MindmapNode) {
    node.text = tidy(&node.text);
    for child in &mut node.children {
        tidy_all(child);
    }
}

/// A `mindmap` source as a tree. Lines of mindmap classes (`:::name`) are
/// not nodes; a second root, which the crate would drop with its subtree,
/// is refused as mermaid refuses it.
fn mindmap(body: &str) -> Result<Mindmap, String> {
    let body: Vec<&str> = body.lines().filter(|line| !line.trim().starts_with(":::")).collect();
    let nodes = body
        .iter()
        .skip(1)
        .filter(|line| {
            let line = line.trim();
            !line.starts_with("::icon(") && !line.starts_with("accTitle") && !line.starts_with("accDescr")
        })
        .count();
    let mut map = mermaid_text::parser::mindmap::parse(&body.join("\n"))
        .map_err(|error| format!("mindmap could not be parsed: {}", short(&error.to_string())))?;
    if map.node_count() < nodes {
        return Err("mindmap has more than one root".into());
    }
    tidy_all(&mut map.root);
    Ok(map)
}

/// A flowchart that is a tree: an outline, or the steps of the chain its
/// root starts, each step with its other children.
enum Tree {
    Outline(Mindmap),
    Steps(Vec<MindmapNode>),
}

/// A flowchart as a mindmap, when it is a tree: the root first, each node's
/// children in the order their edges were written, labels as written,
/// shapes dropped. When the root starts a chain, its steps instead.
fn tree(body: &str) -> Result<Tree, String> {
    let body: Vec<String> = body.lines().map(canonical_arrows).collect();
    let graph = mermaid_text::parser::flowchart::parse(&body.join("\n")).map_err(|error| {
        format!(
            "not a tree: the flowchart could not be parsed ({})",
            short(&error.to_string())
        )
    })?;
    let root = tree_root(&graph)?;
    let index = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        if let (Some(from), Some(to)) = (index(&edge.from), index(&edge.to)) {
            children[from].push(to);
        }
    }
    fn build(at: usize, graph: &Graph, children: &[Vec<usize>]) -> MindmapNode {
        MindmapNode {
            text: tidy(&graph.nodes[at].label),
            children: children[at]
                .iter()
                .map(|&child| build(child, graph, children))
                .collect(),
        }
    }
    if let Some(chain) = chain_from_root(&body, &graph, root) {
        let mut steps = Vec::with_capacity(chain.len());
        for (i, &at) in chain.iter().enumerate() {
            let next = chain.get(i + 1);
            steps.push(MindmapNode {
                text: tidy(&graph.nodes[at].label),
                children: children[at]
                    .iter()
                    .filter(|&child| Some(child) != next)
                    .map(|&child| build(child, &graph, &children))
                    .collect(),
            });
        }
        return Ok(Tree::Steps(steps));
    }
    Ok(Tree::Outline(Mindmap {
        root: build(root, &graph, &children),
    }))
}

/// Nodes a chain of steps has at least: a shorter path from the root
/// ("Root --> Group --> Item") is as often a hierarchy written in one line.
const MIN_STEPS: usize = 4;

/// The nodes of the chain the root starts, in order, when the flowchart has
/// one: the only statement that is a chain of two arrows or more, MIN_STEPS
/// nodes or more from the root ("R --> A --> B --> C"). Edges written one
/// per statement, or fanned out with '&', say nothing about order; several
/// paths written as chains are a hierarchy.
fn chain_from_root(body: &[String], graph: &Graph, root: usize) -> Option<Vec<usize>> {
    let index = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let mut found = None;
    for statement in statements(body) {
        let Ok(one) = mermaid_text::parser::flowchart::parse(&format!("graph TD\n{statement}")) else {
            continue;
        };
        let edges = &one.edges;
        // A fan-out ("A --> B & C") is no chain, and no order.
        if edges.len() < 2 || !edges.windows(2).all(|pair| pair[0].to == pair[1].from) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        if index(&edges[0].from)? != root {
            return None;
        }
        let chain: Vec<usize> = std::iter::once(&edges[0].from)
            .chain(edges.iter().map(|edge| &edge.to))
            .map(|id| index(id))
            .collect::<Option<_>>()?;
        found = Some(chain);
    }
    found.filter(|chain| chain.len() >= MIN_STEPS)
}

/// The statements of a flowchart, its header gone: each line split at the
/// ';' outside quotes, shapes and |labels|.
fn statements(body: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (at, line) in body.iter().enumerate() {
        let mut pieces = Vec::new();
        let (mut quote, mut pipe, mut depth) = (false, false, 0usize);
        let mut current = String::new();
        for c in line.chars() {
            match c {
                '"' => quote = !quote,
                '(' | '[' | '{' if !quote && !pipe => depth += 1,
                ')' | ']' | '}' if !quote && !pipe => depth = depth.saturating_sub(1),
                '|' if !quote && depth == 0 => pipe = !pipe,
                ';' if !quote && !pipe && depth == 0 => {
                    pieces.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        pieces.push(current);
        // The header ("graph TD") is the first piece of the first line.
        let skip = usize::from(at == 0);
        out.extend(pieces.into_iter().skip(skip).filter(|piece| !piece.trim().is_empty()));
    }
    out
}

/// Steps drawn one under another, joined by the line on the left, each
/// with its other children as an outline under its text. With a width,
/// every label is cut with '…' to fit it.
fn draw_steps(steps: &[MindmapNode], width: Option<usize>) -> String {
    fn cut(text: &str, room: Option<usize>) -> String {
        let Some(room) = room else {
            return text.to_string();
        };
        if UnicodeWidthStr::width(text) <= room {
            return text.to_string();
        }
        let mut out = String::new();
        let mut used = 0;
        for c in text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + w + 1 > room {
                break;
            }
            out.push(c);
            used += w;
        }
        let mut out = out.trim_end().to_string();
        out.push('…');
        out
    }
    fn branch(out: &mut Vec<String>, node: &MindmapNode, prefix: &str, last: bool, width: Option<usize>) {
        let lead = format!("{prefix}{}", if last { "└── " } else { "├── " });
        let room = width.map(|w| w.saturating_sub(UnicodeWidthStr::width(lead.as_str())));
        out.push(format!("{lead}{}", cut(&node.text, room)));
        let below = format!("{prefix}{}", if last { "    " } else { "│   " });
        for (i, child) in node.children.iter().enumerate() {
            branch(out, child, &below, i + 1 == node.children.len(), width);
        }
    }
    let mut out = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        out.push(format!("● {}", cut(&step.text, width.map(|w| w.saturating_sub(2)))));
        let spine = if last { "  " } else { "│ " };
        for (j, child) in step.children.iter().enumerate() {
            branch(&mut out, child, spine, j + 1 == step.children.len(), width);
        }
        if !last {
            out.push("│".into());
        }
    }
    out.join("\n")
}

/// An arrow operator, long forms included, at the start of the text.
static ARROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<?(?:-{2,}>|-{3,}|={2,}>|={3,}|-\.+->|-\.+-)").unwrap());

/// A line with its long arrows ("--->", "----", "-..->", "===>") made the
/// short ones the crate reads: it takes "--->" as "-->" and the rest of the
/// dashes as the next node's label ("> B"), or misses a "|label|" after it.
/// Text in quotes, shapes and |labels| is left alone.
fn canonical_arrows(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let (mut quote, mut pipe, mut depth) = (false, false, 0usize);
    let mut at = 0;
    while let Some(c) = line[at..].chars().next() {
        let top = !quote && !pipe && depth == 0;
        if top
            && matches!(c, '-' | '=' | '<')
            && let Some(found) = ARROW.find(&line[at..])
        {
            let arrow = found.as_str();
            let (head, op) = arrow.strip_prefix('<').map_or(("", arrow), |op| ("<", op));
            let short = if op.ends_with('>') {
                if op.starts_with('=') {
                    "==>"
                } else if op.contains('.') {
                    "-.->"
                } else {
                    "-->"
                }
            } else if op.starts_with('=') {
                "==="
            } else if op.contains('.') {
                "-.-"
            } else {
                "---"
            };
            out.push_str(head);
            out.push_str(short);
            at += arrow.len();
            continue;
        }
        match c {
            '"' => quote = !quote,
            '(' | '[' | '{' if !quote && !pipe => depth += 1,
            ')' | ']' | '}' if !quote && !pipe => depth = depth.saturating_sub(1),
            '|' if !quote && depth == 0 => pipe = !pipe,
            _ => {}
        }
        out.push(c);
        at += c.len_utf8();
    }
    out
}

/// The root of a flowchart that is a tree, or why it is not one.
fn tree_root(graph: &Graph) -> Result<usize, String> {
    if !graph.subgraphs.is_empty() {
        return Err("not a tree: it has subgraphs".into());
    }
    if graph
        .edges
        .iter()
        .any(|edge| edge.label.as_deref().is_some_and(|l| !l.trim().is_empty()))
    {
        return Err("not a tree: an edge has a label".into());
    }
    if graph.edges.iter().any(|edge| edge.start != EdgeEndpoint::None) {
        return Err("not a tree: an edge points both ways".into());
    }
    let index = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let mut parents = vec![0usize; graph.nodes.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        let (Some(from), Some(to)) = (index(&edge.from), index(&edge.to)) else {
            return Err("not a tree: an edge joins an unknown node".into());
        };
        parents[to] += 1;
        if parents[to] > 1 {
            return Err(format!(
                "not a tree: {} has two parents",
                short(&tidy(&graph.nodes[to].label))
            ));
        }
        children[from].push(to);
    }
    let roots: Vec<usize> = (0..graph.nodes.len()).filter(|&at| parents[at] == 0).collect();
    let root = match roots.as_slice() {
        [root] => *root,
        [] => return Err("not a tree: it has a cycle".into()),
        _ => return Err(format!("not a tree: it has {} roots", roots.len())),
    };
    // One root and one parent each still allows a ring of nodes apart from
    // the root's tree: every node must be reached from the root.
    let mut seen = vec![false; graph.nodes.len()];
    let mut stack = vec![root];
    while let Some(at) = stack.pop() {
        if !std::mem::replace(&mut seen[at], true) {
            stack.extend(&children[at]);
        }
    }
    if seen.contains(&false) {
        return Err("not a tree: it has a cycle".into());
    }
    if children.iter().all(|kids| kids.len() < 2) {
        return Err("not a tree: it never branches".into());
    }
    Ok(root)
}

/// The tree drawn whole when it fits, else drawn again at the width (long
/// labels cut with '…'); refused when even that does not fit or would leave
/// a label nothing but the '…', or when it is too tall.
fn fit(render: impl Fn(Option<usize>) -> String, max_columns: usize, noun: &str) -> Result<Drawing, String> {
    let whole = lines(&render(None));
    // The hooks refuse a line of more UTF-16 units than this, whatever its
    // width (combining marks, joined emoji): it is refused here, by name.
    let too_long = |drawn: &[String]| drawn.iter().any(|line| line.encode_utf16().count() > MAX_LINE_UNITS);
    let refused = || format!("{noun} has a label too long to draw");
    // Counted on the drawing itself: the hooks refuse more rows anyway.
    if whole.len() > MAX_ROWS {
        return Err(format!("{noun} is {} rows tall, limit {MAX_ROWS}", whole.len()));
    }
    let wide = widest(&whole);
    if wide <= max_columns {
        if too_long(&whole) {
            return Err(refused());
        }
        return Ok(Drawing {
            lines: whole,
            columns: wide,
        });
    }
    let cut = lines(&render(Some(max_columns)));
    let columns = widest(&cut);
    let emptied = cut
        .iter()
        .any(|line| line.ends_with("── …") || line.trim_end() == "│ … │" || line == "● …" || line == "●");
    if columns > max_columns || emptied {
        return Err(format!("{noun} is {wide} columns wide, {max_columns} available"));
    }
    if too_long(&cut) {
        return Err(refused());
    }
    Ok(Drawing { lines: cut, columns })
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(|line| line.trim_end().to_string()).collect()
}

fn widest(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0)
}

fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(source: &str) -> Vec<String> {
        draw(source, 255).expect("drawn").lines
    }

    fn refusal(source: &str) -> String {
        draw(source, 255).expect_err("refused")
    }

    #[test]
    fn a_mindmap_draws_as_an_outline_under_its_root() {
        let lines = drawn("mindmap\n  root((主题))\n    A\n      A1\n    B");
        assert_eq!(
            lines,
            [
                "╭──────╮",
                "│ 主题 │",
                "╰───┬──╯",
                "    │",
                "    ├── A",
                "    │   └── A1",
                "    └── B"
            ]
        );
    }

    #[test]
    fn mindmap_labels_lose_quotes_breaks_and_class_lines() {
        let lines = drawn("mindmap\n  root\n    id[\"a <br/> b\"]\n    :::urgent\n    `c`");
        assert_eq!(lines[4], "    ├── a b");
        assert_eq!(lines[5], "    └── c");
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn a_label_of_more_units_than_the_hooks_take_is_refused() {
        let label = format!("a{}", "\u{301}".repeat(1100));
        let refused = refusal(&format!("mindmap\n  root\n    {label}\n    b"));
        assert_eq!(refused, "mindmap has a label too long to draw");
        // A long label of plain letters is cut to the width instead.
        let long = drawn(&format!("mindmap\n  root\n    {}\n    b", "x".repeat(1100)));
        assert!(long.iter().all(|line| line.chars().count() <= 255), "{long:?}");
    }

    #[test]
    fn a_second_mindmap_root_is_refused() {
        assert_eq!(refusal("mindmap\n  A\n    B\n  C"), "mindmap has more than one root");
    }

    #[test]
    fn a_tree_flowchart_draws_as_its_mindmap() {
        let flow = drawn("graph TD\n  R[Root] --> A[\"f(x) 的定义\"]\n  R --> B\n  A-->A1((leaf)) & A2\n");
        let mind = drawn("mindmap\n  Root\n    a[\"f(x) 的定义\"]\n      leaf\n      A2\n    B");
        assert_eq!(flow, mind);
    }

    #[test]
    fn a_chain_written_from_the_root_draws_as_steps_with_their_branches() {
        // The chain is the sequence the author wrote; B's deep branch is
        // longer than the chain and still hangs off B.
        let lines = drawn(
            "flowchart TD\n  A[Start] --> B --> C --> D\n  A --> A1\n  B --> B1 & B2\n  B1 --> B11\n  B11 --> B111",
        );
        assert_eq!(
            lines,
            [
                "● Start",
                "│ └── A1",
                "│",
                "● B",
                "│ ├── B1",
                "│ │   └── B11",
                "│ │       └── B111",
                "│ └── B2",
                "│",
                "● C",
                "│",
                "● D",
            ]
        );
    }

    #[test]
    fn only_a_chain_from_the_root_makes_steps() {
        // A chain further down, a short one, two of them (a hierarchy
        // written as paths), edges written one by one, or statements that
        // share a line draw as the outline they always did.
        for source in [
            "graph TD\n  R --> A\n  A --> B --> C --> D\n  R --> E",
            "graph TD\n  R --> A --> B\n  A --> C",
            "graph TD\n  R --> A\n  A --> B\n  A --> C",
            "graph TD\n  R --> A & B\n  A --> C",
            "flowchart TD\n  Root --> Cat1 --> Sub1 --> Item1\n  Cat1 --> Item2\n  Root --> Cat2 --> Sub2 --> Item3",
            "graph TD; R --> A; A --> B; B --> C\n  A --> D",
            "graph TD\n  R --> A; A --> B; B --> C\n  A --> D",
        ] {
            assert!(drawn(source)[0].starts_with('╭'), "{source}");
        }
    }

    #[test]
    fn a_chain_after_the_header_on_its_line_is_steps() {
        let lines = drawn("graph TD; R --> A --> B --> C; A --> D");
        assert_eq!(lines, ["● R", "│", "● A", "│ └── D", "│", "● B", "│", "● C"]);
    }

    #[test]
    fn steps_are_cut_to_fit_and_refused_when_nothing_is_left() {
        let source =
            "graph TD\n  A[a long first step] --> B[a long second step] --> C --> E\n  A --> D[a long branch label]";
        let cut = draw(source, 16).expect("cut to fit");
        assert!(cut.columns <= 16, "{:?}", cut.lines);
        assert_eq!(cut.lines[0], "● a long first…");
        assert!(cut.lines[1].ends_with('…'), "{:?}", cut.lines);
        assert_eq!(
            draw(source, 6).expect_err("too narrow"),
            "tree is 25 columns wide, 6 available"
        );
    }

    #[test]
    fn the_root_is_the_node_without_a_parent() {
        let lines = drawn("flowchart LR\n  B --> C & D\n  A --> B");
        assert_eq!(lines[1], "│ A │");
    }

    #[test]
    fn flowcharts_that_are_not_trees_are_refused() {
        let cases = [
            ("graph TD\n A --> C\n B --> C", "not a tree: C has two parents"),
            ("graph TD\n A --> B\n A --> B", "not a tree: B has two parents"),
            (
                "graph TD\n A --> B\n B --> C\n C --> B",
                "not a tree: B has two parents",
            ),
            ("graph TD\n A --> B\n B --> A", "not a tree: it has a cycle"),
            ("graph TD\n A --> B\n C", "not a tree: it has 2 roots"),
            ("graph LR\n A --> B --> C --> D", "not a tree: it never branches"),
            ("graph TD\n A -->|yes| B", "not a tree: an edge has a label"),
            ("graph TD\n A -- yes --> B", "not a tree: an edge has a label"),
            ("graph TD\n A <--> B", "not a tree: an edge points both ways"),
            ("graph TD\n subgraph S\n A --> B\n end", "not a tree: it has subgraphs"),
            (
                "sequenceDiagram\n A->>B: hi",
                "not a tree: sequenceDiagram diagrams are not drawn as trees",
            ),
        ];
        for (source, why) in cases {
            assert_eq!(refusal(source), why, "{source}");
        }
    }

    #[test]
    fn long_arrows_are_read_as_arrows() {
        // The crate alone takes "--->" as "-->" and draws the spare '>' into
        // the next label, and misses the label after it.
        let short = drawn("graph TD\n A --> B\n A --> C\n A --> D\n A --- E");
        for source in [
            "graph TD\n A ---> B\n A ==> C\n A -.-> D\n A ---- E",
            "graph TD\n A--->B\n A====>C\n A-..->D\n A----E",
        ] {
            assert_eq!(drawn(source), short, "{source}");
        }
        assert_eq!(
            refusal("graph TD\n A --->|yes| B\n A --> C"),
            "not a tree: an edge has a label"
        );
        // Inside a label an arrow is text.
        assert_eq!(drawn("graph TD\n A --> B[\"x ---> y\"]\n A --> C")[4], "  ├── x ---> y");
    }

    #[test]
    fn a_markdown_string_over_lines_is_one_label() {
        let lines = drawn("mindmap\n  root\n    a[\"`line one\n  line two`\"]\n    b");
        assert_eq!(lines[4..], ["    ├── line one line two", "    └── b"]);
        assert_eq!(
            refusal("mindmap\n  root\n    a[\"`never closed\n    b"),
            "a markdown string (\"`...`\") never closes"
        );
    }

    #[test]
    fn a_ring_beside_the_root_is_a_cycle() {
        // One root, one parent each, yet C and D are never reached.
        assert_eq!(
            refusal("graph TD\n A --> B\n C --> D\n D --> C"),
            "not a tree: it has a cycle"
        );
    }

    #[test]
    fn long_labels_are_cut_to_fit_and_deep_trees_are_refused() {
        let source = "mindmap\n  root\n    a long label that does not fit";
        let cut = draw(source, 20).expect("cut to fit");
        assert_eq!(cut.columns, 20);
        assert!(cut.lines.iter().all(|line| UnicodeWidthStr::width(line.as_str()) <= 20));
        assert!(cut.lines[4].ends_with('…'), "{:?}", cut.lines);
        assert_eq!(
            draw(source, 8).expect_err("too narrow"),
            "mindmap is 38 columns wide, 8 available"
        );
    }

    #[test]
    fn more_than_eighty_rows_are_refused() {
        let mut source = String::from("graph TD\n");
        for i in 0..80 {
            source.push_str(&format!("  R --> N{i}\n"));
        }
        assert_eq!(refusal(&source), "tree is 84 rows tall, limit 80");
    }

    #[test]
    fn front_matter_and_comments_are_skipped() {
        let lines = drawn("---\ntitle: x\n---\n%% note\nmindmap\n  root\n    %% skip\n    A");
        assert_eq!(lines.last().map(String::as_str), Some("    └── A"));
    }
}
