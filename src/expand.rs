use std::collections::HashMap;

use std::sync::OnceLock;

use rand::Rng;
use rand::rngs::StdRng;
use regex::Regex;

use crate::coverage::CoverageMap;
use crate::depth::inline_min_depth;
use crate::grammar::{ChoiceId, Grammar, Rule};
use crate::terminal::TerminalDictionary;

// ---------------------------------------------------------------------------
// Generation context
// ---------------------------------------------------------------------------

pub struct ExpandCtx<'a> {
    pub grammar: &'a Grammar,
    pub min_depths: &'a HashMap<String, usize>,
    pub terminal_dict: &'a TerminalDictionary,
    pub coverage: &'a mut CoverageMap,
    pub rng: &'a mut StdRng,
    pub max_depth: usize,
    pub max_repeat: usize,
    pub in_token: bool,
    pub choice_log: Vec<(ChoiceId, usize)>,
    /// Complexity bias strength (0.0 = uniform, 1.0 = strongly prefer high min_depth alternatives).
    /// At low depth, alternatives with higher min_depth get proportionally more weight.
    /// The bias fades as depth approaches max_depth, converging to uniform near the leaves.
    pub complexity_bias: f64,
    /// Override: instead of expanding the root rule, generate sequences of these rules.
    pub top_level_rules: Vec<String>,
}

// ---------------------------------------------------------------------------
// Core expand
// ---------------------------------------------------------------------------

pub fn expand(rule: &Rule, depth: usize, ctx: &mut ExpandCtx) -> String {
    match rule {
        Rule::Str { value } => value.clone(),
        Rule::Pattern { value } => ctx.terminal_dict.get(value, ctx.rng).to_string(),
        Rule::Blank => String::new(),

        Rule::Seq { members } => {
            let parts: Vec<String> = members
                .iter()
                .map(|m| expand(m, depth, ctx))
                .filter(|s| !s.is_empty())
                .collect();
            if ctx.in_token {
                parts.join("")
            } else {
                parts.join(" ")
            }
        }

        Rule::Choice { members, choice_id } => {
            let remaining = ctx.max_depth.saturating_sub(depth);

            // Filter by depth eligibility.
            let eligible: Vec<(usize, &Rule)> = members
                .iter()
                .enumerate()
                .filter(|(_, m)| inline_min_depth(m, ctx.min_depths) <= remaining)
                .collect();

            let (picked_idx, picked_rule) = if eligible.is_empty() {
                // Safety: pick the member with lowest min_depth.
                let (idx, rule) = members
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, m)| inline_min_depth(m, ctx.min_depths))
                    .unwrap();
                (idx, rule)
            } else {
                // Coverage-biased selection: prefer uncovered alternatives.
                let eligible_indices: Vec<usize> = eligible.iter().map(|(i, _)| *i).collect();
                let uncovered = ctx.coverage.uncovered_alts(*choice_id);
                let uncovered_eligible: Vec<usize> = uncovered
                    .iter()
                    .filter(|i| eligible_indices.contains(i))
                    .copied()
                    .collect();

                // Select the candidate pool: uncovered first, then all eligible.
                let pool = if !uncovered_eligible.is_empty() {
                    &uncovered_eligible
                } else {
                    &eligible_indices
                };

                let picked_idx = if ctx.complexity_bias > 0.0 && ctx.max_depth > 0 {
                    // Complexity-biased weighted selection.
                    // Budget ratio: 1.0 at root, 0.0 at max_depth.
                    let budget =
                        (ctx.max_depth.saturating_sub(depth)) as f64 / ctx.max_depth as f64;
                    // Scale bias by remaining budget — full bias at root, none at leaves.
                    let effective_bias = ctx.complexity_bias * budget;

                    // Weight each candidate by its min_depth (complexity proxy).
                    let weights: Vec<f64> = pool
                        .iter()
                        .map(|&idx| {
                            let md = inline_min_depth(&members[idx], ctx.min_depths) as f64;
                            1.0 + effective_bias * md
                        })
                        .collect();
                    weighted_pick(pool, &weights, ctx.rng)
                } else {
                    // Uniform random.
                    pool[ctx.rng.gen_range(0..pool.len())]
                };

                (picked_idx, &members[picked_idx])
            };

            ctx.coverage.record(*choice_id, picked_idx);
            ctx.choice_log.push((*choice_id, picked_idx));

            expand(picked_rule, depth, ctx)
        }

        Rule::Repeat { content } => {
            let remaining = ctx.max_depth.saturating_sub(depth);
            if inline_min_depth(content, ctx.min_depths) > remaining {
                return String::new();
            }
            let cap = 1.max(ctx.max_repeat.saturating_sub(depth));
            let count = ctx.rng.gen_range(0..=cap);
            let parts: Vec<String> = (0..count)
                .map(|_| expand(content, depth + 1, ctx))
                .filter(|s| !s.is_empty())
                .collect();
            if ctx.in_token {
                parts.join("")
            } else {
                parts.join(" ")
            }
        }

        Rule::Repeat1 { content } => {
            let remaining = ctx.max_depth.saturating_sub(depth);
            if inline_min_depth(content, ctx.min_depths) > remaining {
                eprintln!(
                    "[tsgen] warning: Repeat1 at depth {} exceeds max_depth {}, forcing 1 attempt",
                    depth, ctx.max_depth
                );
            }
            let cap = 1.max(ctx.max_repeat.saturating_sub(depth));
            let count = ctx.rng.gen_range(1..=cap);
            let parts: Vec<String> = (0..count)
                .map(|_| expand(content, depth + 1, ctx))
                .filter(|s| !s.is_empty())
                .collect();
            if parts.is_empty() {
                // Repeat1 must produce at least something; force one expansion.
                expand(content, depth + 1, ctx)
            } else if ctx.in_token {
                parts.join("")
            } else {
                parts.join(" ")
            }
        }

        Rule::Field { content, .. }
        | Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Alias { content } => expand(content, depth, ctx),

        Rule::Token { content } | Rule::ImmediateToken { content } => {
            let prev = ctx.in_token;
            ctx.in_token = true;
            let result = expand(content, depth, ctx);
            ctx.in_token = prev;
            result
        }

        Rule::Symbol { name } => {
            if ctx.grammar.is_extra(name) {
                return String::new();
            }
            match ctx.grammar.get_rule(name) {
                Some(rule) => expand(rule, depth + 1, ctx),
                None => {
                    eprintln!("[tsgen] error: missing rule '{}'", name);
                    format!("<MISSING:{}>", name)
                }
            }
        }
    }
}

/// Weighted random selection: pick from `candidates` proportional to `weights`.
fn weighted_pick(candidates: &[usize], weights: &[f64], rng: &mut impl Rng) -> usize {
    let total: f64 = weights.iter().sum();
    let mut r = rng.gen_range(0.0..total);
    for (i, &w) in weights.iter().enumerate() {
        r -= w;
        if r <= 0.0 {
            return candidates[i];
        }
    }
    // Fallback (floating point edge case).
    *candidates.last().unwrap()
}

// ---------------------------------------------------------------------------
// Top-level generate
// ---------------------------------------------------------------------------

pub fn generate(ctx: &mut ExpandCtx) -> String {
    if ctx.top_level_rules.is_empty() {
        let root_name = ctx.grammar.root_rule_name();
        let root_rule = ctx.grammar.get_rule(root_name).unwrap();
        expand(root_rule, 0, ctx)
    } else {
        // Override: generate a sequence of the specified top-level rules.
        let cap = 1.max(ctx.max_repeat);
        let count = ctx.rng.gen_range(1..=cap);
        let parts: Vec<String> = (0..count)
            .filter_map(|_| {
                let name = &ctx.top_level_rules[ctx.rng.gen_range(0..ctx.top_level_rules.len())];
                match ctx.grammar.get_rule(name) {
                    Some(rule) => {
                        let s = expand(rule, 1, ctx);
                        if s.is_empty() { None } else { Some(s) }
                    }
                    None => {
                        eprintln!(
                            "[tsgen] error: top-level rule '{}' not found in grammar",
                            name
                        );
                        None
                    }
                }
            })
            .collect();
        parts.join(" ")
    }
}

// ---------------------------------------------------------------------------
// Whitespace cleanup
// ---------------------------------------------------------------------------

fn cleanup_regexes() -> &'static [Regex; 6] {
    static REGEXES: OnceLock<[Regex; 6]> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            Regex::new(r" {2,}").unwrap(),
            Regex::new(r" ([;\,\)\]\}])").unwrap(),
            Regex::new(r"([\(\[\{]) ").unwrap(),
            Regex::new(r"\{([^\n\}])").unwrap(),
            Regex::new(r";([^\n\s\}])").unwrap(),
            Regex::new(r"[ \t]+\n").unwrap(),
        ]
    })
}

pub fn cleanup_whitespace(source: &str) -> String {
    let re = cleanup_regexes();
    let mut s = source.to_string();

    s = re[0].replace_all(&s, " ").to_string();
    s = re[1].replace_all(&s, "$1").to_string();
    s = re[2].replace_all(&s, "$1").to_string();
    s = re[3].replace_all(&s, "{\n$1").to_string();
    s = re[4].replace_all(&s, ";\n$1").to_string();
    s = re[5].replace_all(&s, "\n").to_string();

    s.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{CoverageMap, collect_num_alternatives};
    use crate::depth::compute_min_depths;
    use crate::grammar::Grammar;
    use crate::terminal::TerminalDictionary;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn make_ctx<'a>(
        grammar: &'a Grammar,
        min_depths: &'a HashMap<String, usize>,
        dict: &'a TerminalDictionary,
        coverage: &'a mut CoverageMap,
        rng: &'a mut StdRng,
    ) -> ExpandCtx<'a> {
        ExpandCtx {
            grammar,
            min_depths,
            terminal_dict: dict,
            coverage,
            rng,
            max_depth: 15,
            max_repeat: 5,
            in_token: false,
            choice_log: Vec::new(),
            complexity_bias: 0.0,
            top_level_rules: Vec::new(),
        }
    }

    #[test]
    fn expand_seq() {
        let rule = Rule::Seq {
            members: vec![
                Rule::Str {
                    value: "let".into(),
                },
                Rule::Str { value: "x".into() },
                Rule::Str { value: "=".into() },
                Rule::Str { value: "1".into() },
            ],
        };
        let grammar = load_test_grammar();
        let depths = compute_min_depths(&grammar);
        let mut rng = StdRng::seed_from_u64(0);
        let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);
        let alts = collect_num_alternatives(&grammar);
        let mut cov = CoverageMap::new(grammar.total_choices, &alts);
        let mut rng2 = StdRng::seed_from_u64(0);
        let mut ctx = make_ctx(&grammar, &depths, &dict, &mut cov, &mut rng2);
        assert_eq!(expand(&rule, 0, &mut ctx), "let x = 1");
    }

    #[test]
    fn expand_seq_filters_blanks() {
        let rule = Rule::Seq {
            members: vec![
                Rule::Str { value: "a".into() },
                Rule::Blank,
                Rule::Str { value: "b".into() },
            ],
        };
        let grammar = load_test_grammar();
        let depths = compute_min_depths(&grammar);
        let mut rng = StdRng::seed_from_u64(0);
        let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);
        let alts = collect_num_alternatives(&grammar);
        let mut cov = CoverageMap::new(grammar.total_choices, &alts);
        let mut rng2 = StdRng::seed_from_u64(0);
        let mut ctx = make_ctx(&grammar, &depths, &dict, &mut cov, &mut rng2);
        assert_eq!(expand(&rule, 0, &mut ctx), "a b");
    }

    #[test]
    fn expand_token_no_spaces() {
        let rule = Rule::Token {
            content: Box::new(Rule::Seq {
                members: vec![
                    Rule::Str { value: "//".into() },
                    Rule::Str {
                        value: "comment".into(),
                    },
                ],
            }),
        };
        let grammar = load_test_grammar();
        let depths = compute_min_depths(&grammar);
        let mut rng = StdRng::seed_from_u64(0);
        let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);
        let alts = collect_num_alternatives(&grammar);
        let mut cov = CoverageMap::new(grammar.total_choices, &alts);
        let mut rng2 = StdRng::seed_from_u64(0);
        let mut ctx = make_ctx(&grammar, &depths, &dict, &mut cov, &mut rng2);
        assert_eq!(expand(&rule, 0, &mut ctx), "//comment");
    }

    #[test]
    fn generate_100_no_panics() {
        let grammar = load_test_grammar();
        let depths = compute_min_depths(&grammar);
        let mut rng = StdRng::seed_from_u64(42);
        let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);
        let alts = collect_num_alternatives(&grammar);
        let mut cov = CoverageMap::new(grammar.total_choices, &alts);
        let mut rng2 = StdRng::seed_from_u64(42);

        for _ in 0..100 {
            let mut ctx = make_ctx(&grammar, &depths, &dict, &mut cov, &mut rng2);
            ctx.choice_log.clear();
            let _ = generate(&mut ctx);
        }
        assert!(
            cov.coverage_ratio() > 0.0,
            "expected some coverage after 100 generations"
        );
    }

    #[test]
    fn cleanup_collapses_spaces() {
        assert_eq!(cleanup_whitespace("let  foo  =  1 ;"), "let foo = 1;");
    }

    #[test]
    fn cleanup_brace_newlines() {
        let cleaned = cleanup_whitespace("{ x ; }");
        assert!(
            cleaned.contains("{\n"),
            "expected newline after {{: {}",
            cleaned
        );
    }

    // -- Helper --

    fn load_test_grammar() -> Grammar {
        let path = std::path::Path::new("testdata/test-lang/src/grammar.json");
        Grammar::from_json(path).expect("testdata/test-lang/src/grammar.json must exist")
    }
}
