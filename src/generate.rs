use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::coverage::{CoverageMap, ValidCoverageMap, collect_num_alternatives};
use crate::expand::{ExpandCtx, cleanup_whitespace, generate};
use crate::grammar::Grammar;
use crate::terminal::TerminalDictionary;
use crate::validate::Validator;

// ---------------------------------------------------------------------------
// Config + Result
// ---------------------------------------------------------------------------

pub struct GeneratorConfig {
    pub max_depth: usize,
    pub max_repeat: usize,
    pub count: usize,
    pub coverage_target: f64,
    pub max_attempts: usize,
    pub seed: u64,
    pub complexity_bias: f64,
    pub top_level_rules: Vec<String>,
    pub unicode: bool,
}

pub struct GenerationResult {
    pub programs: Vec<String>,
    pub exploration_coverage: f64,
    pub valid_coverage: f64,
    pub total_attempts: usize,
    pub valid_count: usize,
    pub discarded_count: usize,
    pub duplicate_count: usize,
    pub exploration_covered: usize,
    pub exploration_total: usize,
    pub valid_covered: usize,
    pub valid_total: usize,
}

// ---------------------------------------------------------------------------
// Main generation loop
// ---------------------------------------------------------------------------

pub fn run_generation(
    grammar: &Grammar,
    min_depths: &HashMap<String, usize>,
    terminal_dict: &TerminalDictionary,
    mut validator: Option<&mut Validator>,
    config: &GeneratorConfig,
    cleanup: bool,
) -> GenerationResult {
    let mut rng = StdRng::seed_from_u64(config.seed);
    let alts = collect_num_alternatives(grammar);
    let mut exploration_cov = CoverageMap::new(grammar.total_choices, &alts);
    let mut valid_cov = ValidCoverageMap::new(grammar.total_choices, &alts);
    let mut seen: HashSet<u64> = HashSet::new();
    let mut programs: Vec<String> = Vec::new();
    let mut total_attempts = 0usize;
    let mut discarded_count = 0usize;
    let mut duplicate_count = 0usize;

    while (programs.len() < config.count || valid_cov.coverage_ratio() < config.coverage_target)
        && total_attempts < config.max_attempts
    {
        total_attempts += 1;

        let mut ctx = ExpandCtx {
            grammar,
            min_depths,
            terminal_dict,
            coverage: &mut exploration_cov,
            rng: &mut rng,
            max_depth: config.max_depth,
            max_repeat: config.max_repeat,
            in_token: false,
            choice_log: Vec::new(),
            complexity_bias: config.complexity_bias,
            top_level_rules: config.top_level_rules.clone(),
        };
        let mut prog = generate(&mut ctx);
        let choice_log = std::mem::take(&mut ctx.choice_log);

        // ASCII sanitize (default: strip non-ASCII).
        if !config.unicode {
            prog = sanitize_ascii(&prog);
        }

        if cleanup {
            prog = cleanup_whitespace(&prog);
        }

        let hash = hash_string(&prog);
        if seen.contains(&hash) {
            duplicate_count += 1;
            continue;
        }

        if let Some(ref mut v) = validator {
            let result = v.validate(&prog);
            if result.has_errors {
                discarded_count += 1;
                continue;
            }
        }

        // Program accepted — replay choices into valid coverage.
        valid_cov.replay(&choice_log);
        seen.insert(hash);
        programs.push(prog);

        if programs.len().is_multiple_of(10) || programs.len() == config.count {
            eprint!(
                "\r[tsgen] Generating... {}/{} (valid cov: {:.1}%, explored: {:.1}%, valid: {}, discarded: {}, dupes: {})",
                programs.len(),
                config.count,
                valid_cov.coverage_ratio() * 100.0,
                exploration_cov.coverage_ratio() * 100.0,
                programs.len(),
                discarded_count,
                duplicate_count,
            );
        }
    }
    eprintln!(); // newline after progress

    GenerationResult {
        valid_count: programs.len(),
        programs,
        exploration_coverage: exploration_cov.coverage_ratio(),
        valid_coverage: valid_cov.coverage_ratio(),
        total_attempts,
        discarded_count,
        duplicate_count,
        exploration_covered: exploration_cov.covered_count(),
        exploration_total: exploration_cov.total_alternatives(),
        valid_covered: valid_cov.covered_count(),
        valid_total: valid_cov.total_alternatives(),
    }
}

/// Replace non-ASCII chars with 'z'. Keeps string/char literals valid.
fn sanitize_ascii(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c } else { 'z' })
        .collect()
}

fn hash_string(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Corpus output
// ---------------------------------------------------------------------------

pub fn write_corpus(result: &GenerationResult, output_dir: &Path, extension: &str) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;
    let width = (result.programs.len() as f64).log10() as usize + 1;
    let width = width.max(4);

    for (i, prog) in result.programs.iter().enumerate() {
        let filename = format!("gen_{:0>width$}{}", i + 1, extension, width = width);
        let path = output_dir.join(filename);
        let mut f = std::fs::File::create(&path)?;
        f.write_all(prog.as_bytes())?;
        f.write_all(b"\n")?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

pub fn print_summary(result: &GenerationResult) {
    eprintln!("[tsgen] Done. Generated {} programs.", result.valid_count);
    eprintln!(
        "[tsgen]   Valid coverage:       {:.1}% ({}/{})",
        result.valid_coverage * 100.0,
        result.valid_covered,
        result.valid_total,
    );
    eprintln!(
        "[tsgen]   Exploration coverage: {:.1}% ({}/{})",
        result.exploration_coverage * 100.0,
        result.exploration_covered,
        result.exploration_total,
    );
    eprintln!(
        "[tsgen]   Attempts: {}. Discarded: {}. Duplicates: {}.",
        result.total_attempts, result.discarded_count, result.duplicate_count,
    );
}
