use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rand::Rng;
use regex::Regex;

use crate::grammar::{Grammar, Rule};

// ---------------------------------------------------------------------------
// TerminalKind + classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalKind {
    Identifier,
    DecimalNumber,
    HexNumber,
    StringLit,
    Whitespace,
    Unknown,
}

pub fn default_candidates(kind: TerminalKind) -> &'static [&'static str] {
    match kind {
        TerminalKind::Identifier => &[
            "foo", "bar", "baz", "qux", "x", "y", "z", "tmp", "val", "item", "a", "b", "c", "arg",
            "ret",
        ],
        TerminalKind::DecimalNumber => &["0", "1", "2", "42", "100", "255", "1000"],
        TerminalKind::HexNumber => &["0x0", "0x1", "0xFF", "0xDEAD", "0x10", "0xCAFE"],
        TerminalKind::StringLit => &["\"hello\"", "\"\"", "\"test\"", "\"foo bar\"", "\"x\""],
        TerminalKind::Whitespace => &[],
        TerminalKind::Unknown => &[],
    }
}

/// Classify a regex pattern string by inspecting its text shape.
pub fn classify_pattern(pattern: &str) -> TerminalKind {
    // Strip leading noise: ^, (, ()?
    let stripped = strip_prefix_noise(pattern);

    // Whitespace: exact matches
    let ws_patterns = [
        "\\s", "\\n", "\\r", "\\t", "\\r\\n", "[\\s]", "[\\n]", "[\\r]", "[\\t]", "\\s+", "[\\s]+",
    ];
    if ws_patterns.contains(&pattern) {
        return TerminalKind::Whitespace;
    }

    // HexNumber: starts with 0[xX] or 0x
    if stripped.starts_with("0[xX]") || stripped.starts_with("0x") || stripped.starts_with("0X") {
        return TerminalKind::HexNumber;
    }

    // StringLit: starts with quote or prefixed quote
    if stripped.starts_with('"')
        || stripped.starts_with('\'')
        || stripped.starts_with("b\"")
        || stripped.starts_with("x\"")
        || stripped.starts_with("r\"")
        || stripped.starts_with("r#\"")
    {
        return TerminalKind::StringLit;
    }

    // Identifier: starts with char class containing letters
    if stripped.starts_with("[a-zA-Z_")
        || stripped.starts_with("[_a-zA-Z")
        || stripped.starts_with("[a-zA-Z")
        || stripped.starts_with("[a-z_")
        || stripped.starts_with("[_a-z")
        || stripped.starts_with("[a-z")
        || stripped.starts_with("[A-Z")
        || stripped.starts_with("[A-Za-z")
    {
        return TerminalKind::Identifier;
    }

    // DecimalNumber: starts with digit class or digit literal
    if stripped.starts_with("[0-9")
        || stripped.starts_with("\\d")
        || stripped.starts_with('0')
        || stripped.starts_with('1')
        || stripped.starts_with('2')
        || stripped.starts_with('3')
        || stripped.starts_with('4')
        || stripped.starts_with('5')
        || stripped.starts_with('6')
        || stripped.starts_with('7')
        || stripped.starts_with('8')
        || stripped.starts_with('9')
    {
        return TerminalKind::DecimalNumber;
    }

    TerminalKind::Unknown
}

fn strip_prefix_noise(pattern: &str) -> &str {
    let mut s = pattern;
    // Strip leading ^
    s = s.strip_prefix('^').unwrap_or(s);
    // Strip leading (
    if s.starts_with('(') && !s.starts_with("(?") {
        s = &s[1..];
    }
    // Strip leading )?
    if s.starts_with(")?") {
        s = &s[2..];
    }
    s
}

// ---------------------------------------------------------------------------
// JS → Rust regex normalization
// ---------------------------------------------------------------------------

fn normalize_js_regex(pattern: &str) -> String {
    let mut s = pattern.to_string();

    // Strip lookaheads (?=...) and (?!...)
    // Strip lookbehinds (?<=...) and (?<!...)
    // Simple approach: remove (?=...), (?!...), (?<=...), (?<!...) groups
    // This is approximate but handles common cases.
    loop {
        let before = s.clone();
        s = strip_lookaround(&s);
        if s == before {
            break;
        }
    }

    // Strip backreferences \1, \2, etc.
    let backref_re = Regex::new(r"\\[1-9]").unwrap();
    s = backref_re.replace_all(&s, "").to_string();

    s
}

fn strip_lookaround(s: &str) -> String {
    // Find (?= or (?! or (?<= or (?<!
    let prefixes = ["(?=", "(?!", "(?<=", "(?<!"];
    for prefix in prefixes {
        if let Some(start) = s.find(prefix) {
            let after = &s[start + prefix.len()..];
            if let Some(depth_end) = find_matching_paren(after) {
                let mut result = String::from(&s[..start]);
                result.push_str(&after[depth_end + 1..]);
                return result;
            }
        }
    }
    s.to_string()
}

fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth: usize = 0;
    let mut escaped = false;
    for (i, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Candidate validation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ValidationOutcome {
    Validated(Vec<String>),
    CompilationFailed(String),
}

pub fn validate_candidates(candidates: &[&str], pattern: &str) -> ValidationOutcome {
    let normalized = normalize_js_regex(pattern);
    let anchored = format!("^(?:{})$", normalized);

    match Regex::new(&anchored) {
        Ok(re) => {
            let valid: Vec<String> = candidates
                .iter()
                .filter(|c| re.is_match(c))
                .map(|c| c.to_string())
                .collect();
            ValidationOutcome::Validated(valid)
        }
        Err(e) => ValidationOutcome::CompilationFailed(format!(
            "regex compilation failed for '{}' (normalized: '{}'): {}",
            pattern, normalized, e
        )),
    }
}

// ---------------------------------------------------------------------------
// rand_regex fallback
// ---------------------------------------------------------------------------

pub fn generate_from_regex(pattern: &str, count: usize, rng: &mut impl Rng) -> Vec<String> {
    let normalized = normalize_js_regex(pattern);
    match rand_regex::Regex::compile(&normalized, 5) {
        Ok(regex_gen) => {
            let mut results = Vec::with_capacity(count);
            for _ in 0..count {
                let sample: String = rng.sample(&regex_gen);
                results.push(sample);
            }
            results
        }
        Err(e) => {
            eprintln!(
                "[tsgen] warning: rand_regex failed for '{}': {}",
                pattern, e
            );
            vec!["UNKNOWN".into()]
        }
    }
}

// ---------------------------------------------------------------------------
// HarvestPool — values extracted from real source files
// ---------------------------------------------------------------------------

pub struct HarvestPool {
    pools: HashMap<TerminalKind, Vec<String>>,
}

impl HarvestPool {
    /// Scan a directory recursively, extract terminal values from source files.
    pub fn from_directory(
        dir: &Path,
        ext: Option<&str>,
        keywords: &HashSet<String>,
    ) -> Result<Self> {
        let mut pools: HashMap<TerminalKind, HashSet<String>> = HashMap::new();

        let id_re = Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]{2,}").unwrap();
        let hex_re = Regex::new(r"0x[0-9a-fA-F_]+").unwrap();
        let dec_re = Regex::new(r"\b[0-9][0-9_]*\b").unwrap();
        let str_re = Regex::new(r#""([^"\\]|\\.)*""#).unwrap();

        walk_dir(dir, ext, &mut |contents| {
            // Hex before decimal (0xFF would match decimal too).
            for m in hex_re.find_iter(contents) {
                pools
                    .entry(TerminalKind::HexNumber)
                    .or_default()
                    .insert(m.as_str().to_string());
            }
            for m in dec_re.find_iter(contents) {
                let s = m.as_str();
                // Skip if it's part of a hex literal (already captured).
                if !s.starts_with("0x") && !s.starts_with("0X") {
                    pools
                        .entry(TerminalKind::DecimalNumber)
                        .or_default()
                        .insert(s.to_string());
                }
            }
            for m in str_re.find_iter(contents) {
                pools
                    .entry(TerminalKind::StringLit)
                    .or_default()
                    .insert(m.as_str().to_string());
            }
            for m in id_re.find_iter(contents) {
                let s = m.as_str();
                if !keywords.contains(s) {
                    pools
                        .entry(TerminalKind::Identifier)
                        .or_default()
                        .insert(s.to_string());
                }
            }
        })?;

        // Convert HashSet → Vec for random access.
        let pools = pools
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect::<Vec<_>>()))
            .collect();

        Ok(HarvestPool { pools })
    }

    /// Load a newline-delimited word list straight into the Identifier pool.
    /// Skips blank lines and lines starting with '#'. No regex filtering.
    pub fn from_dict_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read dict file {}: {}", path.display(), e))?;
        let mut ids: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if seen.insert(line.to_string()) {
                ids.push(line.to_string());
            }
        }
        let mut pools = HashMap::new();
        if !ids.is_empty() {
            pools.insert(TerminalKind::Identifier, ids);
        }
        Ok(HarvestPool { pools })
    }

    /// Merge another pool into this one.
    pub fn merge(&mut self, other: HarvestPool) {
        for (kind, values) in other.pools {
            let entry = self.pools.entry(kind).or_default();
            let existing: HashSet<String> = entry.iter().cloned().collect();
            for v in values {
                if !existing.contains(&v) {
                    entry.push(v);
                }
            }
        }
    }

    /// Pick a random value of the given kind, if available.
    pub fn get_random(&self, kind: TerminalKind, rng: &mut impl Rng) -> Option<&str> {
        self.pools.get(&kind).and_then(|v| {
            if v.is_empty() {
                None
            } else {
                Some(v[rng.gen_range(0..v.len())].as_str())
            }
        })
    }

    pub fn dump(&self) {
        for (kind, values) in &self.pools {
            println!(
                "  {:?}: {} values (sample: {:?})",
                kind,
                values.len(),
                &values[..values.len().min(10)]
            );
        }
    }
}

fn walk_dir(dir: &Path, filter: Option<&str>, callback: &mut dyn FnMut(&str)) -> Result<()> {
    if !dir.is_dir() {
        anyhow::bail!("{} is not a directory", dir.display());
    }

    let glob_matcher = match filter {
        Some(f) if f.contains('*') || f.contains('?') || f.contains('[') => Some(
            globset::GlobBuilder::new(f)
                .literal_separator(true)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid glob '{}': {}", f, e))?
                .compile_matcher(),
        ),
        _ => None,
    };

    walk_dir_inner(dir, filter, &glob_matcher, callback)
}

fn walk_dir_inner(
    dir: &Path,
    filter: Option<&str>,
    glob_matcher: &Option<globset::GlobMatcher>,
    callback: &mut dyn FnMut(&str),
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir_inner(&path, filter, glob_matcher, callback)?;
        } else if path.is_file() {
            let matches = if let Some(matcher) = glob_matcher {
                // Glob pattern — match against filename.
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                matcher.is_match(filename)
            } else if let Some(e) = filter {
                // Extension filter.
                path.extension().and_then(|x| x.to_str()) == Some(e)
            } else {
                true
            };
            if matches && let Ok(contents) = std::fs::read_to_string(&path) {
                callback(&contents);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TerminalDictionary
// ---------------------------------------------------------------------------

pub struct TerminalDictionary {
    entries: HashMap<String, Vec<String>>,
    harvest_pool: Option<HarvestPool>,
    harvest_weight: f64,
}

impl TerminalDictionary {
    pub fn from_grammar(
        grammar: &Grammar,
        rng: &mut impl Rng,
        harvest_pool: Option<HarvestPool>,
        harvest_weight: f64,
    ) -> Self {
        let mut patterns = Vec::new();
        for rule in grammar.rules.values() {
            collect_patterns(rule, &mut patterns);
        }
        patterns.sort();
        patterns.dedup();

        let mut entries = HashMap::new();

        for pattern in patterns {
            let kind = classify_pattern(&pattern);

            if kind == TerminalKind::Whitespace {
                // Whitespace patterns: store a single space, never called in practice.
                entries.insert(pattern, vec![" ".into()]);
                continue;
            }

            let raw_candidates = default_candidates(kind);

            // Keyword filtering for identifiers.
            let filtered: Vec<&str> = if kind == TerminalKind::Identifier {
                raw_candidates
                    .iter()
                    .copied()
                    .filter(|c| !grammar.is_keyword(c))
                    .collect()
            } else {
                raw_candidates.to_vec()
            };

            let outcome = validate_candidates(&filtered, &pattern);

            let valid = match outcome {
                ValidationOutcome::Validated(v) if !v.is_empty() => v,
                ValidationOutcome::CompilationFailed(msg) => {
                    // Regex didn't compile — trust the classifier.
                    eprintln!(
                        "[tsgen] warning: validation skipped for '{}': {}",
                        pattern, msg
                    );
                    filtered.iter().map(|s| s.to_string()).collect()
                }
                ValidationOutcome::Validated(_) => {
                    // Regex compiled but no candidates matched — try rand_regex.
                    let generated = generate_from_regex(&pattern, 10, rng);
                    if !generated.is_empty() && generated[0] != "UNKNOWN" {
                        generated
                    } else {
                        // Last resort: raw classified candidates.
                        let fallback: Vec<String> =
                            raw_candidates.iter().map(|s| s.to_string()).collect();
                        if !fallback.is_empty() {
                            fallback
                        } else {
                            vec!["UNKNOWN".into()]
                        }
                    }
                }
            };

            entries.insert(pattern, valid);
        }

        TerminalDictionary {
            entries,
            harvest_pool,
            harvest_weight,
        }
    }

    pub fn dump(&self) {
        let mut patterns: Vec<&String> = self.entries.keys().collect();
        patterns.sort();
        for pattern in patterns {
            let vals = &self.entries[pattern];
            println!("  '{}' → {:?}", pattern, vals);
        }
    }

    pub fn get(&self, pattern: &str, rng: &mut impl Rng) -> &str {
        // Roll dice: harvest or regular?
        if let Some(pool) = &self.harvest_pool
            && self.harvest_weight > 0.0
            && rng.gen_bool(self.harvest_weight.min(1.0))
        {
            let kind = classify_pattern(pattern);
            if let Some(val) = pool.get_random(kind, rng) {
                return val;
            }
        }
        // Regular candidates.
        match self.entries.get(pattern) {
            Some(candidates) if !candidates.is_empty() => {
                let idx = rng.gen_range(0..candidates.len());
                &candidates[idx]
            }
            _ => "UNKNOWN",
        }
    }
}

fn collect_patterns(rule: &Rule, patterns: &mut Vec<String>) {
    match rule {
        Rule::Pattern { value } => {
            patterns.push(value.clone());
        }
        Rule::Seq { members } | Rule::Choice { members, .. } => {
            for m in members {
                collect_patterns(m, patterns);
            }
        }
        Rule::Repeat { content }
        | Rule::Repeat1 { content }
        | Rule::Prec { content }
        | Rule::PrecLeft { content }
        | Rule::PrecRight { content }
        | Rule::PrecDynamic { content }
        | Rule::Field { content, .. }
        | Rule::Token { content }
        | Rule::ImmediateToken { content }
        | Rule::Alias { content } => {
            collect_patterns(content, patterns);
        }
        Rule::Symbol { .. } | Rule::Str { .. } | Rule::Blank => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    // -- classify_pattern --

    #[test]
    fn classify_identifier() {
        assert_eq!(
            classify_pattern("[a-zA-Z_][0-9a-zA-Z_]*"),
            TerminalKind::Identifier
        );
    }

    #[test]
    fn classify_decimal() {
        assert_eq!(classify_pattern("[0-9]+"), TerminalKind::DecimalNumber);
        assert_eq!(
            classify_pattern("[0-9][0-9_]*"),
            TerminalKind::DecimalNumber
        );
    }

    #[test]
    fn classify_hex() {
        assert_eq!(classify_pattern("0x[a-fA-F0-9_]+"), TerminalKind::HexNumber);
    }

    #[test]
    fn classify_string() {
        assert_eq!(
            classify_pattern(r#""(\\.|[^\\"])*""#),
            TerminalKind::StringLit
        );
        assert_eq!(
            classify_pattern(r#"b"(\\.|[^\\"])*""#),
            TerminalKind::StringLit
        );
    }

    #[test]
    fn classify_whitespace() {
        assert_eq!(classify_pattern(r"\s"), TerminalKind::Whitespace);
    }

    #[test]
    fn classify_unknown_block_comment() {
        assert_eq!(
            classify_pattern(r"[^*]*\*+([^/*][^*]*\*+)*"),
            TerminalKind::Unknown
        );
    }

    #[test]
    fn classify_unknown_prefixed_hex() {
        // A hex literal wrapped in a prefix character must not be
        // misclassified as HexNumber — the classifier should bail to Unknown.
        assert_eq!(classify_pattern("@(0x[a-fA-F0-9]+)"), TerminalKind::Unknown);
    }

    // -- validate_candidates --

    #[test]
    fn validate_identifiers() {
        match validate_candidates(&["foo", "bar"], "[a-zA-Z_][0-9a-zA-Z_]*") {
            ValidationOutcome::Validated(v) => {
                assert!(v.contains(&"foo".to_string()));
                assert!(v.contains(&"bar".to_string()));
            }
            _ => panic!("expected Validated"),
        }
    }

    #[test]
    fn dict_from_test_grammar() {
        let path = std::path::Path::new("testdata/test-lang/src/grammar.json");
        if !path.exists() {
            eprintln!("skipping: testdata not generated");
            return;
        }
        let grammar = crate::grammar::Grammar::from_json(path).unwrap();
        let mut rng = StdRng::seed_from_u64(42);
        let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);

        // Identifiers should never return keywords.
        for _ in 0..50 {
            let id = dict.get("[a-zA-Z_][a-zA-Z0-9_]*", &mut rng);
            assert!(
                !grammar.is_keyword(id),
                "identifier dict returned keyword: {}",
                id
            );
        }
    }
}
