use std::collections::HashMap;

use crate::grammar::{Grammar, Rule};

/// Compute min_depth for a single Rule node given already-known depths.
/// Returns None when a needed symbol's depth isn't known yet.
pub fn min_depth_for_rule(rule: &Rule, known: &HashMap<String, usize>) -> Option<usize> {
    match rule {
        Rule::Str { .. } | Rule::Pattern { .. } | Rule::Blank => Some(0),

        Rule::Symbol { name } => known.get(name.as_str()).map(|d| d + 1),

        Rule::Seq { members } => {
            let mut max = 0usize;
            for m in members {
                match min_depth_for_rule(m, known) {
                    Some(d) => max = max.max(d),
                    None => return None,
                }
            }
            Some(max)
        }

        Rule::Choice { members, .. } => {
            let mut best: Option<usize> = None;
            for m in members {
                if let Some(d) = min_depth_for_rule(m, known) {
                    best = Some(best.map_or(d, |b: usize| b.min(d)));
                }
            }
            best
        }

        Rule::Repeat { .. } => Some(0),

        Rule::Repeat1 { content } => min_depth_for_rule(content, known),

        Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Field { content, .. }
        | Rule::Token { content }
        | Rule::ImmediateToken { content }
        | Rule::Alias { content } => min_depth_for_rule(content, known),
    }
}

/// Fixed-point iteration: compute min_depth for all named rules in the grammar.
pub fn compute_min_depths(grammar: &Grammar) -> HashMap<String, usize> {
    let mut known: HashMap<String, usize> = HashMap::new();

    loop {
        let mut changed = false;
        for (name, rule) in &grammar.rules {
            if let Some(d) = min_depth_for_rule(rule, &known) {
                let entry = known.get(name.as_str());
                if entry.is_none() || *entry.unwrap() > d {
                    known.insert(name.clone(), d);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for name in grammar.all_rule_names() {
        if !known.contains_key(name) {
            eprintln!(
                "[tsgen] warning: rule '{}' is unreachable (infinite min-depth)",
                name
            );
        }
    }

    known
}

/// Compute min_depth for an arbitrary rule subtree (post-convergence).
/// Unknown symbols are treated as usize::MAX (unreachable).
/// Uses saturating_add to prevent overflow panics.
pub fn inline_min_depth(rule: &Rule, depths: &HashMap<String, usize>) -> usize {
    match rule {
        Rule::Str { .. } | Rule::Pattern { .. } | Rule::Blank => 0,

        Rule::Symbol { name } => depths
            .get(name.as_str())
            .map_or(usize::MAX, |d| d.saturating_add(1)),

        Rule::Seq { members } => {
            let mut max = 0usize;
            for m in members {
                let d = inline_min_depth(m, depths);
                max = max.max(d);
            }
            max
        }

        Rule::Choice { members, .. } => {
            let mut best = usize::MAX;
            for m in members {
                let d = inline_min_depth(m, depths);
                best = best.min(d);
            }
            best
        }

        Rule::Repeat { .. } => 0,

        Rule::Repeat1 { content } => inline_min_depth(content, depths),

        Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Field { content, .. }
        | Rule::Token { content }
        | Rule::ImmediateToken { content }
        | Rule::Alias { content } => inline_min_depth(content, depths),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn depth_leaves() {
        let empty = HashMap::new();
        assert_eq!(
            min_depth_for_rule(
                &Rule::Str {
                    value: "let".into()
                },
                &empty
            ),
            Some(0)
        );
        assert_eq!(
            min_depth_for_rule(
                &Rule::Pattern {
                    value: "[0-9]+".into()
                },
                &empty
            ),
            Some(0)
        );
        assert_eq!(min_depth_for_rule(&Rule::Blank, &empty), Some(0));
    }

    #[test]
    fn depth_symbol_known() {
        let mut known = HashMap::new();
        known.insert("x".into(), 0);
        assert_eq!(
            min_depth_for_rule(&Rule::Symbol { name: "x".into() }, &known),
            Some(1)
        );
    }

    #[test]
    fn depth_symbol_unknown() {
        assert_eq!(
            min_depth_for_rule(&Rule::Symbol { name: "x".into() }, &HashMap::new()),
            None
        );
    }

    #[test]
    fn depth_seq() {
        let mut known = HashMap::new();
        known.insert("x".into(), 0);
        let rule = Rule::Seq {
            members: vec![
                Rule::Str { value: ";".into() },
                Rule::Symbol { name: "x".into() },
            ],
        };
        assert_eq!(min_depth_for_rule(&rule, &known), Some(1));
    }

    #[test]
    fn depth_seq_unknown_member() {
        let rule = Rule::Seq {
            members: vec![
                Rule::Str { value: ";".into() },
                Rule::Symbol {
                    name: "unknown".into(),
                },
            ],
        };
        assert_eq!(min_depth_for_rule(&rule, &HashMap::new()), None);
    }

    #[test]
    fn depth_choice_partial_known() {
        let rule = Rule::Choice {
            members: vec![
                Rule::Symbol { name: "a".into() },
                Rule::Str { value: "b".into() },
            ],
            choice_id: 0,
        };
        // a is unknown, but "b" is Str → 0
        assert_eq!(min_depth_for_rule(&rule, &HashMap::new()), Some(0));
    }

    #[test]
    fn depth_choice_all_unknown() {
        let rule = Rule::Choice {
            members: vec![
                Rule::Symbol { name: "a".into() },
                Rule::Symbol { name: "b".into() },
            ],
            choice_id: 0,
        };
        assert_eq!(min_depth_for_rule(&rule, &HashMap::new()), None);
    }

    #[test]
    fn depth_repeat() {
        let rule = Rule::Repeat {
            content: Box::new(Rule::Symbol {
                name: "whatever".into(),
            }),
        };
        assert_eq!(min_depth_for_rule(&rule, &HashMap::new()), Some(0));
    }

    #[test]
    fn depth_repeat1() {
        let mut known = HashMap::new();
        known.insert("x".into(), 2);
        let rule = Rule::Repeat1 {
            content: Box::new(Rule::Symbol { name: "x".into() }),
        };
        assert_eq!(min_depth_for_rule(&rule, &known), Some(3));
    }

    #[test]
    fn depth_passthrough() {
        let rule = Rule::Field {
            name: "f".into(),
            content: Box::new(Rule::Str { value: "hi".into() }),
        };
        assert_eq!(min_depth_for_rule(&rule, &HashMap::new()), Some(0));
    }

    #[test]
    fn depth_prec_left() {
        let mut known = HashMap::new();
        known.insert("expr".into(), 1);
        let rule = Rule::PrecLeft {
            content: Box::new(Rule::Symbol {
                name: "expr".into(),
            }),
        };
        assert_eq!(min_depth_for_rule(&rule, &known), Some(2));
    }

    // -- compute_min_depths integration test --

    #[test]
    fn compute_depths_test_grammar() {
        let path = std::path::Path::new("testdata/test-lang/src/grammar.json");
        if !path.exists() {
            eprintln!("skipping: testdata not generated");
            return;
        }
        let grammar = Grammar::from_json(path).unwrap();
        let depths = compute_min_depths(&grammar);

        assert_eq!(depths["number"], 0);
        assert_eq!(depths["identifier"], 0);
        assert_eq!(depths["program"], 0); // REPEAT → 0
        assert_eq!(depths["expression"], 1); // CHOICE → number (SYMBOL depth 0 + 1)
        assert!(depths["binary_expr"] > 1);
        assert!(depths["let_stmt"] > 1);
        assert_eq!(depths["line_comment"], 0); // TOKEN(SEQ(STRING, PATTERN)) → 0
    }

    // -- inline_min_depth tests --

    #[test]
    fn inline_depth_unknown_symbol() {
        let depths = HashMap::new();
        let rule = Rule::Symbol {
            name: "nonexistent".into(),
        };
        assert_eq!(inline_min_depth(&rule, &depths), usize::MAX);
    }

    #[test]
    fn inline_depth_saturating() {
        let mut depths = HashMap::new();
        depths.insert("x".into(), usize::MAX - 1);
        let rule = Rule::Symbol { name: "x".into() };
        // Should saturate to usize::MAX, not panic.
        assert_eq!(inline_min_depth(&rule, &depths), usize::MAX);
    }

    #[test]
    fn inline_depth_seq_with_known() {
        let mut depths = HashMap::new();
        depths.insert("expression".into(), 1);
        let rule = Rule::Seq {
            members: vec![
                Rule::Str { value: ";".into() },
                Rule::Symbol {
                    name: "expression".into(),
                },
            ],
        };
        assert_eq!(inline_min_depth(&rule, &depths), 2);
    }
}
