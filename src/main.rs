use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rand::SeedableRng;
use rand::rngs::StdRng;

use tsgen::coverage::collect_num_alternatives;
use tsgen::depth::compute_min_depths;
use tsgen::generate::{GeneratorConfig, print_summary, run_generation, write_corpus};
use tsgen::grammar::Grammar;
use tsgen::terminal::{HarvestPool, TerminalDictionary};
use tsgen::validate::Validator;

#[derive(Parser)]
#[command(name = "tsgen", about = "Tree-sitter grammar-based program generator")]
struct Cli {
    /// Path to grammar.json
    #[arg(long)]
    grammar: PathBuf,

    /// Path to compiled parser .so for validation (optional)
    #[arg(long)]
    parser: Option<PathBuf>,

    /// Number of programs to generate
    #[arg(long, default_value = "100")]
    count: usize,

    /// Maximum tree depth
    #[arg(long, default_value = "15")]
    max_depth: usize,

    /// Maximum repetition count for REPEAT rules
    #[arg(long, default_value = "5")]
    max_repeat: usize,

    /// Random seed
    #[arg(long, default_value = "0")]
    seed: u64,

    /// Stop when valid coverage reaches this ratio
    #[arg(long, default_value = "0.95")]
    coverage_target: f64,

    /// Output directory for generated corpus
    #[arg(long, default_value = "corpus")]
    output_dir: PathBuf,

    /// File extension for generated files
    #[arg(long, default_value = ".txt")]
    ext: String,

    /// Disable post-processing whitespace cleanup
    #[arg(long)]
    no_cleanup: bool,

    /// Dump loaded grammar rules and min-depths, then exit
    #[arg(long)]
    dump_grammar: bool,

    /// Generate programs to stdout without writing files
    #[arg(long)]
    dry_run: bool,

    /// Maximum total attempts before giving up
    #[arg(long, default_value = "10000")]
    max_attempts: usize,

    /// Override root rule: only generate these rules at the top level (repeatable).
    /// E.g. --top-level-rule _declaration_statement to skip expression statements at file scope.
    #[arg(long)]
    top_level_rule: Vec<String>,

    /// Complexity bias for CHOICE selection (0.0 = uniform, 1.0 = strongly prefer complex alternatives).
    /// At low tree depth, alternatives with higher min-depth get proportionally more weight,
    /// producing structurally richer programs. The bias fades near max-depth.
    #[arg(long, default_value = "0.0")]
    complexity_bias: f64,

    /// Directory of real source files to harvest terminal values from (repeatable)
    #[arg(long)]
    harvest_dir: Vec<std::path::PathBuf>,

    /// Probability of using a harvested value vs generated (0.0-1.0)
    #[arg(long, default_value = "0.5")]
    harvest_weight: f64,

    /// File extension filter for harvested files (e.g. ".rs")
    #[arg(long)]
    harvest_ext: Option<String>,

    /// Newline-delimited dict of identifiers to use as seed names (repeatable).
    /// Blank lines and '#' comments are skipped. Merges with --harvest-dir pools.
    #[arg(long)]
    dict: Vec<PathBuf>,

    /// Allow unicode characters in generated programs (default: ASCII only)
    #[arg(long)]
    unicode: bool,
}

fn main() -> Result<()> {
    let args = Cli::parse();

    eprintln!("[tsgen] Loading grammar: {}", args.grammar.display());
    let grammar = Grammar::from_json(&args.grammar)?;
    eprintln!(
        "[tsgen] Grammar '{}': {} rules, {} total choices",
        grammar.name,
        grammar.rules.len(),
        grammar.total_choices,
    );

    let min_depths = compute_min_depths(&grammar);

    if args.dump_grammar {
        let num_alts = collect_num_alternatives(&grammar);
        println!("Grammar: {}", grammar.name);
        println!("Root rule: {}", grammar.root_rule_name());
        println!("Total choices: {}", grammar.total_choices);
        println!("Extras: {:?}", grammar.extras);
        println!("Keywords: {:?}", grammar.keywords);
        println!();
        println!("Rules and min-depths:");
        for name in grammar.all_rule_names() {
            let depth = min_depths
                .get(name)
                .map_or("unreachable".to_string(), |d| d.to_string());
            println!("  {} → depth {}", name, depth);
        }
        println!();
        println!("Choice registry ({} choices):", num_alts.len());
        for (id, n) in num_alts.iter().enumerate() {
            println!("  choice {} → {} alternatives", id, n);
        }

        let mut rng = StdRng::seed_from_u64(args.seed);
        let harvest = build_harvest_pool(
            &args.harvest_dir,
            args.harvest_ext.as_deref(),
            &args.dict,
            &grammar,
        );
        let dict =
            TerminalDictionary::from_grammar(&grammar, &mut rng, harvest, args.harvest_weight);
        println!();
        println!("Terminal dictionary:");
        dict.dump();

        return Ok(());
    }

    let mut rng = StdRng::seed_from_u64(args.seed);
    let harvest = build_harvest_pool(
        &args.harvest_dir,
        args.harvest_ext.as_deref(),
        &args.dict,
        &grammar,
    );
    let dict = TerminalDictionary::from_grammar(&grammar, &mut rng, harvest, args.harvest_weight);

    let language_fn = format!("tree_sitter_{}", grammar.name);
    let mut validator = match &args.parser {
        Some(parser_path) => {
            eprintln!(
                "[tsgen] Loading parser: {} (fn: {})",
                parser_path.display(),
                language_fn
            );
            Some(Validator::from_shared_lib(parser_path, &language_fn)?)
        }
        None => {
            eprintln!("[tsgen] No parser provided — skipping validation");
            None
        }
    };

    let config = GeneratorConfig {
        max_depth: args.max_depth,
        max_repeat: args.max_repeat,
        count: args.count,
        coverage_target: args.coverage_target,
        max_attempts: args.max_attempts,
        seed: args.seed,
        complexity_bias: args.complexity_bias,
        top_level_rules: args.top_level_rule.clone(),
        unicode: args.unicode,
    };

    let result = run_generation(
        &grammar,
        &min_depths,
        &dict,
        validator.as_mut(),
        &config,
        !args.no_cleanup,
    );

    if args.dry_run {
        for (i, prog) in result.programs.iter().enumerate() {
            println!("--- program {} ---", i + 1);
            println!("{}", prog);
            println!();
        }
    } else {
        write_corpus(&result, &args.output_dir, &args.ext)?;
        eprintln!(
            "[tsgen] Wrote {} files to {}",
            result.valid_count,
            args.output_dir.display()
        );
    }

    print_summary(&result);

    Ok(())
}

fn build_harvest_pool(
    dirs: &[PathBuf],
    ext: Option<&str>,
    dicts: &[PathBuf],
    grammar: &Grammar,
) -> Option<HarvestPool> {
    if dirs.is_empty() && dicts.is_empty() {
        return None;
    }
    let mut pool: Option<HarvestPool> = None;
    for dir in dirs {
        eprintln!("[tsgen] Harvesting from: {}", dir.display());
        match HarvestPool::from_directory(dir, ext, grammar.keywords()) {
            Ok(p) => match &mut pool {
                Some(existing) => existing.merge(p),
                None => pool = Some(p),
            },
            Err(e) => {
                eprintln!(
                    "[tsgen] warning: harvest failed for {}: {}",
                    dir.display(),
                    e
                );
            }
        }
    }
    for dict_path in dicts {
        eprintln!("[tsgen] Loading dict: {}", dict_path.display());
        match HarvestPool::from_dict_file(dict_path) {
            Ok(p) => match &mut pool {
                Some(existing) => existing.merge(p),
                None => pool = Some(p),
            },
            Err(e) => {
                eprintln!(
                    "[tsgen] warning: dict load failed for {}: {}",
                    dict_path.display(),
                    e
                );
            }
        }
    }
    if let Some(ref p) = pool {
        eprintln!("[tsgen] Harvest pool:");
        p.dump();
    }
    pool
}
