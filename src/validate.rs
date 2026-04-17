use std::path::Path;

use anyhow::{Context, Result};

pub struct ValidationResult {
    pub has_errors: bool,
    pub error_nodes: usize,
}

pub struct Validator {
    parser: tree_sitter::Parser,
    // Keep the library alive so the language function pointer remains valid.
    _library: libloading::Library,
}

impl Validator {
    /// Load a compiled parser .so and create a Validator.
    /// `language_fn` is the C function name, e.g. "tree_sitter_test_lang".
    pub fn from_shared_lib(path: &Path, language_fn: &str) -> Result<Self> {
        // SAFETY: we trust the .so to contain a valid tree-sitter language function.
        unsafe {
            let lib = libloading::Library::new(path)
                .with_context(|| format!("failed to load parser library: {}", path.display()))?;

            let func: libloading::Symbol<unsafe extern "C" fn() -> *const std::ffi::c_void> =
                lib.get(language_fn.as_bytes()).with_context(|| {
                    format!("symbol '{}' not found in {}", language_fn, path.display())
                })?;

            let raw_lang = func();
            let language = tree_sitter::Language::from_raw(raw_lang as *const _);

            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&language)
                .with_context(|| "failed to set parser language")?;

            Ok(Validator {
                parser,
                _library: lib,
            })
        }
    }

    /// Parse the source and return validation statistics.
    pub fn validate(&mut self, source: &str) -> ValidationResult {
        let tree = self
            .parser
            .parse(source, None)
            .expect("parser returned None");
        let root = tree.root_node();
        let mut error_nodes = 0usize;
        count_errors(root, &mut error_nodes);
        ValidationResult {
            has_errors: root.has_error(),
            error_nodes,
        }
    }
}

fn count_errors(node: tree_sitter::Node, errors: &mut usize) {
    if node.is_error() {
        *errors += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_errors(child, errors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validator() -> Option<Validator> {
        let so_path = std::path::Path::new("testdata/test-lang/parser.so");
        if !so_path.exists() {
            eprintln!("skipping: parser.so not found");
            return None;
        }
        Some(Validator::from_shared_lib(so_path, "tree_sitter_test_lang").unwrap())
    }

    #[test]
    fn validate_generated_programs() {
        let Some(mut validator) = make_validator() else {
            return;
        };

        let grammar_path = std::path::Path::new("testdata/test-lang/src/grammar.json");
        if !grammar_path.exists() {
            return;
        }

        let grammar = crate::grammar::Grammar::from_json(grammar_path).unwrap();
        let depths = crate::depth::compute_min_depths(&grammar);
        let mut rng: rand::rngs::StdRng = rand::SeedableRng::seed_from_u64(42);
        let dict = crate::terminal::TerminalDictionary::from_grammar(&grammar, &mut rng, None, 0.0);
        let alts = crate::coverage::collect_num_alternatives(&grammar);
        let mut cov = crate::coverage::CoverageMap::new(grammar.total_choices, &alts);
        let mut rng2: rand::rngs::StdRng = rand::SeedableRng::seed_from_u64(99);

        let mut valid_count = 0usize;
        let total = 50;

        for _ in 0..total {
            let mut ctx = crate::expand::ExpandCtx {
                grammar: &grammar,
                min_depths: &depths,
                terminal_dict: &dict,
                coverage: &mut cov,
                rng: &mut rng2,
                max_depth: 10,
                max_repeat: 3,
                in_token: false,
                choice_log: Vec::new(),
                complexity_bias: 0.0,
                top_level_rules: Vec::new(),
            };
            let prog = crate::expand::generate(&mut ctx);
            let cleaned = crate::expand::cleanup_whitespace(&prog);
            let result = validator.validate(&cleaned);
            if !result.has_errors {
                valid_count += 1;
            }
        }

        let valid_pct = valid_count as f64 / total as f64 * 100.0;
        eprintln!(
            "[validate_generated] {}/{} valid ({:.1}%)",
            valid_count, total, valid_pct
        );
        assert!(
            valid_count as f64 / total as f64 >= 0.30,
            "expected >= 30% valid programs, got {:.1}%",
            valid_pct
        );
    }
}
