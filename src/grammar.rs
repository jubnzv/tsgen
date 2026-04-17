use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub type ChoiceId = usize;

#[derive(Debug, Clone)]
pub enum Rule {
    Seq {
        members: Vec<Rule>,
    },
    Choice {
        members: Vec<Rule>,
        choice_id: ChoiceId,
    },
    Repeat {
        content: Box<Rule>,
    },
    Repeat1 {
        content: Box<Rule>,
    },
    Symbol {
        name: String,
    },
    Str {
        value: String,
    },
    Pattern {
        value: String,
    },
    Blank,
    Field {
        name: String,
        content: Box<Rule>,
    },
    Prec {
        content: Box<Rule>,
    },
    PrecLeft {
        content: Box<Rule>,
    },
    PrecRight {
        content: Box<Rule>,
    },
    PrecDynamic {
        content: Box<Rule>,
    },
    Token {
        content: Box<Rule>,
    },
    ImmediateToken {
        content: Box<Rule>,
    },
    Alias {
        content: Box<Rule>,
    },
}

// ---------------------------------------------------------------------------
// Grammar struct
// ---------------------------------------------------------------------------

pub struct Grammar {
    pub name: String,
    pub rules: IndexMap<String, Rule>,
    pub extras: HashSet<String>,
    pub keywords: HashSet<String>,
    pub total_choices: usize,
}

impl Grammar {
    pub fn from_json(path: &Path) -> Result<Grammar> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read grammar file: {}", path.display()))?;
        let raw: GrammarJson = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse grammar JSON: {}", path.display()))?;

        // Convert RawRule → Rule for each entry, preserving order.
        let mut rules: IndexMap<String, Rule> = raw
            .rules
            .into_iter()
            .map(|(name, raw_rule)| (name, Rule::from(raw_rule)))
            .collect();

        let total_choices = assign_choice_ids(&mut rules);

        // Collect extras: only SYMBOL entries get added (PATTERN extras like \s are implicit).
        let mut extras = HashSet::new();
        for extra in &raw.extras {
            if let RawRule::Symbol { name } = extra {
                extras.insert(name.clone());
            }
        }

        let mut keywords = HashSet::new();
        for rule in rules.values() {
            collect_keywords(rule, &mut keywords);
        }

        Ok(Grammar {
            name: raw.name,
            rules,
            extras,
            keywords,
            total_choices,
        })
    }

    pub fn root_rule_name(&self) -> &str {
        self.rules.keys().next().expect("grammar has no rules")
    }

    pub fn get_rule(&self, name: &str) -> Option<&Rule> {
        self.rules.get(name)
    }

    pub fn is_extra(&self, name: &str) -> bool {
        self.extras.contains(name)
    }

    pub fn is_keyword(&self, value: &str) -> bool {
        self.keywords.contains(value)
    }

    pub fn keywords(&self) -> &HashSet<String> {
        &self.keywords
    }

    pub fn all_rule_names(&self) -> impl Iterator<Item = &str> {
        self.rules.keys().map(|s| s.as_str())
    }
}

// ---------------------------------------------------------------------------
// Deserialization (intermediate types)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GrammarJson {
    name: String,
    rules: IndexMap<String, RawRule>,
    #[serde(default)]
    extras: Vec<RawRule>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum RawRule {
    #[serde(rename = "SEQ")]
    Seq { members: Vec<RawRule> },
    #[serde(rename = "CHOICE")]
    Choice { members: Vec<RawRule> },
    #[serde(rename = "REPEAT")]
    Repeat { content: Box<RawRule> },
    #[serde(rename = "REPEAT1")]
    Repeat1 { content: Box<RawRule> },
    #[serde(rename = "SYMBOL")]
    Symbol { name: String },
    #[serde(rename = "STRING")]
    Str { value: String },
    #[serde(rename = "PATTERN")]
    Pattern { value: String },
    #[serde(rename = "BLANK")]
    Blank {},
    #[serde(rename = "FIELD")]
    Field { name: String, content: Box<RawRule> },
    #[serde(rename = "PREC")]
    Prec { content: Box<RawRule> },
    #[serde(rename = "PREC_LEFT")]
    PrecLeft { content: Box<RawRule> },
    #[serde(rename = "PREC_RIGHT")]
    PrecRight { content: Box<RawRule> },
    #[serde(rename = "PREC_DYNAMIC")]
    PrecDynamic { content: Box<RawRule> },
    #[serde(rename = "TOKEN")]
    Token { content: Box<RawRule> },
    #[serde(rename = "IMMEDIATE_TOKEN")]
    ImmediateToken { content: Box<RawRule> },
    #[serde(rename = "ALIAS")]
    Alias { content: Box<RawRule> },
}

// ---------------------------------------------------------------------------
// RawRule → Rule conversion
// ---------------------------------------------------------------------------

impl From<RawRule> for Rule {
    fn from(raw: RawRule) -> Self {
        match raw {
            RawRule::Seq { members } => Rule::Seq {
                members: members.into_iter().map(Rule::from).collect(),
            },
            RawRule::Choice { members } => Rule::Choice {
                members: members.into_iter().map(Rule::from).collect(),
                choice_id: 0, // placeholder — assigned in post-deserialization walk
            },
            RawRule::Repeat { content } => Rule::Repeat {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::Repeat1 { content } => Rule::Repeat1 {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::Symbol { name } => Rule::Symbol { name },
            RawRule::Str { value } => Rule::Str { value },
            RawRule::Pattern { value } => Rule::Pattern { value },
            RawRule::Blank {} => Rule::Blank,
            RawRule::Field { name, content } => Rule::Field {
                name,
                content: Box::new(Rule::from(*content)),
            },
            RawRule::Prec { content } => Rule::Prec {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::PrecLeft { content } => Rule::PrecLeft {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::PrecRight { content } => Rule::PrecRight {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::PrecDynamic { content } => Rule::PrecDynamic {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::Token { content } => Rule::Token {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::ImmediateToken { content } => Rule::ImmediateToken {
                content: Box::new(Rule::from(*content)),
            },
            RawRule::Alias { content } => Rule::Alias {
                content: Box::new(Rule::from(*content)),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ChoiceId assignment (post-deserialization walk)
// ---------------------------------------------------------------------------

/// Assigns sequential ChoiceIds to all Choice nodes across the grammar.
/// Returns the total number of Choice nodes found.
pub fn assign_choice_ids(rules: &mut IndexMap<String, Rule>) -> usize {
    let mut counter: usize = 0;
    for rule in rules.values_mut() {
        assign_choice_ids_recursive(rule, &mut counter);
    }
    counter
}

fn assign_choice_ids_recursive(rule: &mut Rule, counter: &mut usize) {
    match rule {
        Rule::Choice { members, choice_id } => {
            *choice_id = *counter;
            *counter += 1;
            for member in members {
                assign_choice_ids_recursive(member, counter);
            }
        }
        Rule::Seq { members } => {
            for member in members {
                assign_choice_ids_recursive(member, counter);
            }
        }
        Rule::Repeat { content }
        | Rule::Repeat1 { content }
        | Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Token { content }
        | Rule::ImmediateToken { content }
        | Rule::Alias { content }
        | Rule::Field { content, .. } => {
            assign_choice_ids_recursive(content, counter);
        }
        Rule::Symbol { .. } | Rule::Str { .. } | Rule::Pattern { .. } | Rule::Blank => {}
    }
}

// ---------------------------------------------------------------------------
// Keyword collection
// ---------------------------------------------------------------------------

fn collect_keywords(rule: &Rule, keywords: &mut HashSet<String>) {
    match rule {
        Rule::Str { value } => {
            keywords.insert(value.clone());
        }
        Rule::Seq { members } | Rule::Choice { members, .. } => {
            for member in members {
                collect_keywords(member, keywords);
            }
        }
        Rule::Repeat { content }
        | Rule::Repeat1 { content }
        | Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Token { content }
        | Rule::ImmediateToken { content }
        | Rule::Alias { content }
        | Rule::Field { content, .. } => {
            collect_keywords(content, keywords);
        }
        Rule::Symbol { .. } | Rule::Pattern { .. } | Rule::Blank => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_all_choice_ids(rules: &IndexMap<String, Rule>) -> Vec<ChoiceId> {
        fn walk(rule: &Rule, ids: &mut Vec<ChoiceId>) {
            match rule {
                Rule::Choice { members, choice_id } => {
                    ids.push(*choice_id);
                    for m in members {
                        walk(m, ids);
                    }
                }
                Rule::Seq { members } => {
                    for m in members {
                        walk(m, ids);
                    }
                }
                Rule::Repeat { content }
                | Rule::Repeat1 { content }
                | Rule::Prec { content }
                | Rule::PrecLeft { content }
                | Rule::PrecRight { content }
                | Rule::PrecDynamic { content }
                | Rule::Token { content }
                | Rule::ImmediateToken { content }
                | Rule::Alias { content }
                | Rule::Field { content, .. } => walk(content, ids),
                Rule::Symbol { .. } | Rule::Str { .. } | Rule::Pattern { .. } | Rule::Blank => {}
            }
        }
        let mut ids = Vec::new();
        for rule in rules.values() {
            walk(rule, &mut ids);
        }
        ids
    }

    // -- Deserialization tests (one per variant) --

    fn deser(json: &str) -> Rule {
        let raw: RawRule = serde_json::from_str(json).unwrap();
        Rule::from(raw)
    }

    #[test]
    fn deser_blank() {
        // BLANK is the one variant that doesn't map 1:1 from JSON: `{}` object → unit variant.
        let rule = deser(r#"{"type":"BLANK"}"#);
        assert!(matches!(rule, Rule::Blank));
    }

    #[test]
    fn deser_choice_placeholder_id() {
        // choice_id isn't in the JSON — it gets assigned in a post-deserialization walk,
        // so fresh choices must come out with the placeholder 0.
        let rule = deser(
            r#"{"type":"CHOICE","members":[{"type":"STRING","value":"a"},{"type":"STRING","value":"b"}]}"#,
        );
        match rule {
            Rule::Choice { members, choice_id } => {
                assert_eq!(members.len(), 2);
                assert_eq!(choice_id, 0);
            }
            _ => panic!("expected Choice"),
        }
    }

    // -- Grammar loading tests --

    #[test]
    fn load_test_grammar() {
        let path = std::path::Path::new("testdata/test-lang/src/grammar.json");
        if !path.exists() {
            eprintln!("skipping: testdata not yet generated");
            return;
        }
        let grammar = Grammar::from_json(path).unwrap();
        assert_eq!(grammar.name, "test_lang");
        assert_eq!(grammar.root_rule_name(), "program");
        assert!(grammar.rules.len() > 5);
        assert!(grammar.is_extra("line_comment"));
        assert!(grammar.is_keyword("let"));
        assert!(grammar.is_keyword("if"));
        assert!(grammar.is_keyword("else"));
        assert!(grammar.is_keyword("="));
        assert!(grammar.is_keyword(";"));
        assert!(!grammar.is_keyword("foo"));

        let ids = collect_all_choice_ids(&grammar.rules);
        assert!(!ids.is_empty());
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "choice_ids must be unique");
        assert_eq!(
            sorted,
            (0..sorted.len()).collect::<Vec<_>>(),
            "choice_ids must be sequential"
        );
        assert_eq!(grammar.total_choices, ids.len());
    }
}
