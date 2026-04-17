use std::collections::HashSet;

use crate::grammar::{ChoiceId, Grammar, Rule};

// ---------------------------------------------------------------------------
// CoverageMap — exploration map (all attempts)
// ---------------------------------------------------------------------------

pub struct CoverageMap {
    covered: Vec<HashSet<usize>>,
    num_alternatives: Vec<usize>,
    total_alternatives: usize,
}

impl CoverageMap {
    pub fn new(total_choices: usize, num_alternatives: &[usize]) -> Self {
        let covered = vec![HashSet::new(); total_choices];
        let total_alternatives: usize = num_alternatives.iter().sum();
        CoverageMap {
            covered,
            num_alternatives: num_alternatives.to_vec(),
            total_alternatives,
        }
    }

    pub fn record(&mut self, choice_id: ChoiceId, alt_index: usize) {
        if choice_id < self.covered.len() {
            self.covered[choice_id].insert(alt_index);
        }
    }

    pub fn coverage_ratio(&self) -> f64 {
        if self.total_alternatives == 0 {
            return 1.0;
        }
        self.covered_count() as f64 / self.total_alternatives as f64
    }

    pub fn covered_count(&self) -> usize {
        self.covered.iter().map(|s| s.len()).sum()
    }

    pub fn total_alternatives(&self) -> usize {
        self.total_alternatives
    }

    /// Returns alternative indices NOT yet covered for the given choice.
    pub fn uncovered_alts(&self, choice_id: ChoiceId) -> Vec<usize> {
        if choice_id >= self.covered.len() {
            return Vec::new();
        }
        let n = self.num_alternatives.get(choice_id).copied().unwrap_or(0);
        let covered = &self.covered[choice_id];
        (0..n).filter(|i| !covered.contains(i)).collect()
    }
}

// ---------------------------------------------------------------------------
// ValidCoverageMap — valid programs only
// ---------------------------------------------------------------------------

pub struct ValidCoverageMap {
    covered: Vec<HashSet<usize>>,
    total_alternatives: usize,
}

impl ValidCoverageMap {
    pub fn new(total_choices: usize, num_alternatives: &[usize]) -> Self {
        let covered = vec![HashSet::new(); total_choices];
        let total_alternatives: usize = num_alternatives.iter().sum();
        ValidCoverageMap {
            covered,
            total_alternatives,
        }
    }

    pub fn record(&mut self, choice_id: ChoiceId, alt_index: usize) {
        if choice_id < self.covered.len() {
            self.covered[choice_id].insert(alt_index);
        }
    }

    pub fn coverage_ratio(&self) -> f64 {
        if self.total_alternatives == 0 {
            return 1.0;
        }
        self.covered_count() as f64 / self.total_alternatives as f64
    }

    pub fn covered_count(&self) -> usize {
        self.covered.iter().map(|s| s.len()).sum()
    }

    pub fn total_alternatives(&self) -> usize {
        self.total_alternatives
    }

    /// Replay a choice log from a validated program.
    pub fn replay(&mut self, choice_log: &[(ChoiceId, usize)]) {
        for &(choice_id, alt_index) in choice_log {
            self.record(choice_id, alt_index);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn collect_num_alternatives(grammar: &Grammar) -> Vec<usize> {
    let mut alts = vec![0usize; grammar.total_choices];
    for rule in grammar.rules.values() {
        collect_alts_recursive(rule, &mut alts);
    }
    alts
}

fn collect_alts_recursive(rule: &Rule, alts: &mut [usize]) {
    match rule {
        Rule::Choice { members, choice_id } => {
            alts[*choice_id] = members.len();
            for m in members {
                collect_alts_recursive(m, alts);
            }
        }
        Rule::Seq { members } => {
            for m in members {
                collect_alts_recursive(m, alts);
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
            collect_alts_recursive(content, alts);
        }
        Rule::Symbol { .. } | Rule::Str { .. } | Rule::Pattern { .. } | Rule::Blank => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncovered_alts_basic() {
        let num_alts = vec![3, 2]; // choice 0 has 3 alts, choice 1 has 2
        let mut cov = CoverageMap::new(2, &num_alts);
        assert_eq!(cov.uncovered_alts(0), vec![0, 1, 2]);
        cov.record(0, 1);
        assert_eq!(cov.uncovered_alts(0), vec![0, 2]);
        cov.record(0, 0);
        cov.record(0, 2);
        assert!(cov.uncovered_alts(0).is_empty());
    }

    #[test]
    fn coverage_ratio_increases() {
        let num_alts = vec![4, 2]; // total 6
        let mut cov = CoverageMap::new(2, &num_alts);
        assert_eq!(cov.coverage_ratio(), 0.0);
        cov.record(0, 0);
        assert!(cov.coverage_ratio() > 0.0);
        cov.record(0, 1);
        cov.record(0, 2);
        cov.record(0, 3);
        cov.record(1, 0);
        cov.record(1, 1);
        assert!((cov.coverage_ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn valid_coverage_replay() {
        let num_alts = vec![3];
        let mut vcov = ValidCoverageMap::new(1, &num_alts);
        assert_eq!(vcov.coverage_ratio(), 0.0);
        let log = vec![(0, 1), (0, 2)];
        vcov.replay(&log);
        assert_eq!(vcov.covered_count(), 2);
    }
}
