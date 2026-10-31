//! The parser against Obsidian 1.14.5's own parse trees.
//!
//! The CDP run captured the tree Obsidian built for every statement it was
//! given (`tools/obsidian-probe/bases-expr.mjs`, raw run in
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json`): patching
//! `evaluateFormula` on the prototype hands back what `MQ.parse` produced, and
//! a parsed statement is plain JSON. `obsidian_trees.tsv` is every one of
//! those that parsed, rendered to a one-line shape, and `obsidian_rejects.txt`
//! is every one Obsidian marked `invalid`.
//!
//! This is the ratchet: it holds the parser to a recorded tree per statement
//! rather than to the precedence table in the specification, which is itself
//! only a reading of these trees. Where the two disagree the trees win.

use silica_bases::{parse, BinaryOp, Expr, Literal};

/// The same one-line rendering the fixtures were generated in. Structure only:
/// what it has to capture is which operand bound to which operator.
fn shape(e: &Expr) -> String {
    match e {
        Expr::Primitive(Literal::Number(n)) => format!("{n}"),
        Expr::Primitive(Literal::String(s)) => format!("{s:?}"),
        Expr::Primitive(Literal::Bool(b)) => b.to_string(),
        Expr::Primitive(Literal::Null) => "null".into(),
        Expr::Ident(id) => id.clone(),
        Expr::Array(items) => format!(
            "[{}]",
            items.iter().map(shape).collect::<Vec<_>>().join(", ")
        ),
        Expr::Regexp { pattern, flags } => format!("/{pattern}/{flags}"),
        Expr::Paren(inner) => format!("({})", shape(inner)),
        Expr::Binary { op, left, right } => {
            format!("({} {} {})", shape(left), op.as_str(), shape(right))
        }
        Expr::Negate(e) => format!("(- {})", shape(e)),
        Expr::Not(e) => format!("(! {})", shape(e)),
        Expr::Function {
            name,
            subject,
            args,
        } => {
            let args = args.iter().map(shape).collect::<Vec<_>>().join(", ");
            match subject {
                Some(s) => format!("{}.{name}({args})", shape(s)),
                None => format!("{name}({args})"),
            }
        }
        Expr::ObjectAccess { object, index } => format!("{}.{index}", shape(object)),
        Expr::ArrayAccess { array, index } => format!("{}[{}]", shape(array), shape(index)),
        Expr::Invalid { value, .. } => format!("<invalid {value:?}>"),
    }
}

/// `BinaryOp::as_str` has to spell the operator the way Obsidian's tree does,
/// or every shape below would differ for a reason that is not the parser's.
#[test]
fn the_operator_spellings_are_obsidians() {
    for (op, text) in [
        (BinaryOp::Add, "+"),
        (BinaryOp::Sub, "-"),
        (BinaryOp::Mul, "*"),
        (BinaryOp::Div, "/"),
        (BinaryOp::Rem, "%"),
        (BinaryOp::Lt, "<"),
        (BinaryOp::Le, "<="),
        (BinaryOp::Gt, ">"),
        (BinaryOp::Ge, ">="),
        (BinaryOp::Eq, "=="),
        (BinaryOp::Ne, "!="),
        (BinaryOp::And, "&&"),
        (BinaryOp::Or, "||"),
    ] {
        assert_eq!(op.as_str(), text);
    }
}

#[test]
fn every_statement_parses_to_the_tree_obsidian_built() {
    let fixture = include_str!("obsidian_trees.tsv");
    let mut checked = 0;
    let mut wrong = Vec::new();
    for line in fixture.lines() {
        let Some((source, expected)) = line.split_once('\t') else {
            continue;
        };
        match parse(source) {
            Ok(tree) => {
                let got = shape(&tree);
                if got != expected {
                    wrong.push(format!(
                        "  {source}\n    obsidian: {expected}\n    silica:   {got}"
                    ));
                }
            }
            Err(e) => wrong.push(format!(
                "  {source}\n    obsidian: {expected}\n    silica:   did not parse: {e}"
            )),
        }
        checked += 1;
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked} statements parsed differently to Obsidian:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    // A floor, so that a fixture that stops being read fails here rather than
    // passing while checking nothing.
    assert!(checked >= 277, "only {checked} trees were compared");
}

#[test]
fn every_statement_obsidian_refused_is_refused_here() {
    let fixture = include_str!("obsidian_rejects.txt");
    let mut checked = 0;
    let mut accepted = Vec::new();
    for source in fixture.lines() {
        if source.is_empty() {
            continue;
        }
        if let Ok(tree) = parse(source) {
            accepted.push(format!("  {source}  ->  {}", shape(&tree)));
        }
        checked += 1;
    }
    assert!(
        accepted.is_empty(),
        "{} of {checked} statements parsed here that Obsidian rejected:\n{}",
        accepted.len(),
        accepted.join("\n")
    );
    assert!(checked >= 32, "only {checked} rejections were compared");
}
