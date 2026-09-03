//! Blinded, interleaved A/B comparison of two mouse settings.
//!
//! The problem this solves: the difference between two firmware settings is
//! usually much smaller than the variation in how well you happen to click on a
//! given attempt. Doing all the A trials and then all the B trials lets fatigue,
//! warm-up and mood land entirely on one condition. Seeing a running score lets
//! the result you are hoping for change how hard you try.
//!
//! So trials alternate, results are withheld until the run finishes, and the
//! order within each pair is counterbalanced: pair one runs A then B, pair two
//! runs B then A, and so on. Alternating removes drift across the session;
//! counterbalancing removes any advantage that comes simply from going second
//! within a pair, which strict A-then-B alternation would leave confounded with
//! the setting itself.

use crate::core::abstats::{self, AbReport};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// Clicks per second: which setting lets you click faster.
    Rate,
    /// Registered actuations under deliberately weak input: which setting
    /// registers presses the other misses entirely. A rate test cannot answer
    /// this, because a press that never registers is not a slow press.
    WeakInput,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Rate => "click rate",
            Variant::WeakInput => "actuations under weak input",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Variant::Rate => "CPS",
            Variant::WeakInput => "clicks",
        }
    }

    pub fn instruction(self) -> &'static str {
        match self {
            Variant::Rate => {
                "Click as fast as you can hold for the whole trial. Consistency matters more \
                 than a burst at the start."
            }
            Variant::WeakInput => {
                "Press as lightly as you can while still trying to actuate, at a steady \
                 rhythm of about two per second. The question is how many of your attempts \
                 the mouse registers at all, so do not press harder to score better."
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Condition {
    A,
    B,
}

impl Condition {
}

#[derive(Clone, Copy, Debug)]
pub struct Trial {
    /// Position in the run, from zero.
    pub index: usize,
    pub condition: Condition,
    /// Which pair this trial belongs to.
    pub pair: usize,
    /// Clicks per second, or registered actuations, depending on the variant.
    pub value: f64,
    pub presses: usize,
    pub duration_s: f64,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub label_a: String,
    pub label_b: String,
    pub variant: Variant,
    pub trial_seconds: f64,
    pub pairs: usize,
    pub button: Option<u8>,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            label_a: String::new(),
            label_b: String::new(),
            variant: Variant::Rate,
            trial_seconds: 10.0,
            pairs: 8,
            button: None,
        }
    }
}

impl Plan {
    pub fn total_trials(&self) -> usize {
        self.pairs * 2
    }

    /// Condition for trial `index`, counterbalanced across pairs.
    ///
    /// Pair 0 runs A then B, pair 1 runs B then A, and so on, so that going
    /// second within a pair is not always the same setting.
    pub fn condition_at(&self, index: usize) -> Condition {
        let pair = index / 2;
        let first_is_a = pair % 2 == 0;
        let second = index % 2 == 1;
        match (first_is_a, second) {
            (true, false) | (false, true) => Condition::A,
            _ => Condition::B,
        }
    }

    pub fn label(&self, c: Condition) -> &str {
        match c {
            Condition::A => &self.label_a,
            Condition::B => &self.label_b,
        }
    }

    pub fn ready(&self) -> bool {
        !self.label_a.trim().is_empty()
            && !self.label_b.trim().is_empty()
            && self.label_a.trim() != self.label_b.trim()
            && self.pairs >= 3
    }
}

#[derive(Clone, Debug)]
pub struct Run {
    pub plan: Plan,
    pub trials: Vec<Trial>,
}

impl Run {
    /// Values paired by pair index, so the paired test compares like with like.
    pub fn paired(&self) -> (Vec<f64>, Vec<f64>) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let pairs = self.trials.iter().map(|t| t.pair).max().map(|m| m + 1).unwrap_or(0);
        for p in 0..pairs {
            let av = self
                .trials
                .iter()
                .find(|t| t.pair == p && t.condition == Condition::A)
                .map(|t| t.value);
            let bv = self
                .trials
                .iter()
                .find(|t| t.pair == p && t.condition == Condition::B)
                .map(|t| t.value);
            if let (Some(x), Some(y)) = (av, bv) {
                a.push(x);
                b.push(y);
            }
        }
        (a, b)
    }

    pub fn analyse(&self, alpha: f64) -> AbReport {
        let (a, b) = self.paired();
        abstats::analyse(&a, &b, true, alpha)
    }

    /// Every trial, in run order, as comma-separated rows.
    pub fn to_csv(&self) -> String {
        let mut s = String::from(
            "trial_index,pair,condition,condition_label,variant,duration_s,presses,value,unit\n",
        );
        for t in &self.trials {
            let c = match t.condition {
                Condition::A => "A",
                Condition::B => "B",
            };
            s.push_str(&format!(
                "{},{},{},{},{},{:.3},{},{:.6},{}\n",
                t.index,
                t.pair,
                c,
                csv_escape(self.plan.label(t.condition)),
                self.plan.variant.label(),
                t.duration_s,
                t.presses,
                t.value,
                self.plan.variant.unit(),
            ));
        }
        s
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(pairs: usize) -> Plan {
        Plan {
            label_a: "800 Hz".into(),
            label_b: "1000 Hz".into(),
            pairs,
            ..Default::default()
        }
    }

    #[test]
    fn trials_are_interleaved_not_batched() {
        // Counterbalancing means a condition repeats across each pair boundary
        // (A B B A A B ...), so strict alternation is not the property to check.
        // What matters is that the run never becomes a block of one setting.
        let p = plan(8);
        let seq: Vec<Condition> = (0..p.total_trials()).map(|i| p.condition_at(i)).collect();
        let mut run_len = 1;
        for w in seq.windows(2) {
            run_len = if w[0] == w[1] { run_len + 1 } else { 1 };
            assert!(
                run_len <= 2,
                "{run_len} trials of the same setting in a row: {seq:?}"
            );
        }
        let a_count = seq.iter().filter(|c| **c == Condition::A).count();
        assert_eq!(a_count, seq.len() / 2, "the two settings must get equal trials");
    }

    #[test]
    fn each_pair_contains_one_of_each_condition() {
        let p = plan(8);
        for pair in 0..p.pairs {
            let first = p.condition_at(pair * 2);
            let second = p.condition_at(pair * 2 + 1);
            assert_ne!(first, second, "pair {pair} does not contain both conditions");
        }
    }

    #[test]
    fn within_pair_order_is_counterbalanced() {
        // If A always went first, any advantage from going second would be
        // indistinguishable from the setting under test.
        let p = plan(8);
        let a_first = (0..p.pairs)
            .filter(|&pair| p.condition_at(pair * 2) == Condition::A)
            .count();
        assert_eq!(
            a_first,
            p.pairs / 2,
            "A went first in {a_first} of {} pairs; the order is not balanced",
            p.pairs
        );
    }

    #[test]
    fn a_plan_needs_two_distinct_labels_and_enough_pairs() {
        let mut p = plan(8);
        assert!(p.ready());
        p.label_b = p.label_a.clone();
        assert!(!p.ready(), "identical labels should not be runnable");
        p.label_b = "1000 Hz".into();
        p.pairs = 2;
        assert!(!p.ready(), "two pairs cannot support a conclusion");
    }

    #[test]
    fn pairing_matches_values_by_pair_not_by_position() {
        let p = plan(3);
        let mut run = Run {
            plan: p.clone(),
            trials: Vec::new(),
        };
        // Values chosen so a positional pairing would give a different answer.
        let vals = [1.0, 2.0, 30.0, 20.0, 5.0, 6.0];
        for (i, v) in vals.iter().enumerate() {
            run.trials.push(Trial {
                index: i,
                condition: p.condition_at(i),
                pair: i / 2,
                value: *v,
                presses: 0,
                duration_s: 10.0,
            });
        }
        let (a, b) = run.paired();
        // Pair 0 is A then B; pair 1 is B then A; pair 2 is A then B.
        assert_eq!(a, vec![1.0, 20.0, 5.0]);
        assert_eq!(b, vec![2.0, 30.0, 6.0]);
    }

    #[test]
    fn export_contains_every_trial_in_order() {
        let p = plan(3);
        let mut run = Run { plan: p.clone(), trials: Vec::new() };
        for i in 0..6 {
            run.trials.push(Trial {
                index: i,
                condition: p.condition_at(i),
                pair: i / 2,
                value: i as f64,
                presses: i,
                duration_s: 10.0,
            });
        }
        let csv = run.to_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 7, "header plus six trials");
        assert!(lines[1].starts_with("0,0,A,800 Hz,"));
        assert!(lines[3].starts_with("2,1,B,1000 Hz,"));
    }

    #[test]
    fn a_label_containing_a_comma_does_not_break_the_export() {
        let mut p = plan(3);
        p.label_a = "800 Hz, low".into();
        let run = Run {
            plan: p.clone(),
            trials: vec![Trial {
                index: 0,
                condition: Condition::A,
                pair: 0,
                value: 1.0,
                presses: 1,
                duration_s: 5.0,
            }],
        };
        let csv = run.to_csv();
        assert!(csv.contains("\"800 Hz, low\""), "{csv}");
        assert_eq!(csv.lines().nth(1).unwrap().split(',').count(), 10);
    }
}
