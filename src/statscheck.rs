//! `--stats-check <cases.json> <out.json>` evaluates the statistics against a
//! file of cases so an external reference can check every number.
//!
//! The statistics are the part of this program most likely to be subtly wrong
//! and least likely to look wrong, so they are checked against scipy rather
//! than trusted.

use crate::core::abstats as st;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Case {
    id: String,
    a: Vec<f64>,
    b: Vec<f64>,
}

#[derive(Serialize)]
struct Out {
    id: String,
    mwu_u: f64,
    mwu_p_exact: f64,
    mwu_p_asymptotic: f64,
    mwu_exact_used: bool,
    mwu_rank_biserial: f64,
    mwu_prob_superiority: f64,
    wsr_statistic: f64,
    wsr_p_exact: f64,
    wsr_p_asymptotic: f64,
    wsr_p_pratt_exact: f64,
    hl_unpaired: f64,
    hl_unpaired_lo: f64,
    hl_unpaired_hi: f64,
    hl_unpaired_level: f64,
    hl_unpaired_exact: bool,
    hl_paired: f64,
    hl_paired_lo: f64,
    hl_paired_hi: f64,
    hl_paired_exact: bool,
    median_a: f64,
    q1_a: f64,
    q3_a: f64,
    mad_a: f64,
    perm_p: f64,
    perm_exhaustive: bool,
}

pub fn run(input: &str, output: &str) {
    let text = match std::fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {input}: {e}");
            return;
        }
    };
    let cases: Vec<Case> = match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bad cases file: {e}");
            return;
        }
    };

    let mut out = Vec::new();
    for c in &cases {
        let mwu_exact = st::mann_whitney_u(&c.a, &c.b, st::MwuMethod::Exact);
        let mwu_asym = st::mann_whitney_u(&c.a, &c.b, st::MwuMethod::Asymptotic);
        let mwu_auto = st::mann_whitney_u(&c.a, &c.b, st::MwuMethod::Auto);

        let d: Vec<f64> = c.a.iter().zip(&c.b).map(|(x, y)| x - y).collect();
        let wsr_exact =
            st::wilcoxon_signed_rank(&d, st::ZeroMethod::Wilcox, false, st::WsrMethod::Exact);
        let wsr_asym =
            st::wilcoxon_signed_rank(&d, st::ZeroMethod::Wilcox, false, st::WsrMethod::Asymptotic);
        let wsr_pratt =
            st::wilcoxon_signed_rank(&d, st::ZeroMethod::Pratt, false, st::WsrMethod::Exact);

        let hlu = st::hodges_lehmann_shift_ci(&c.a, &c.b, 0.05);
        let hlp = st::hodges_lehmann_paired_ci(&d, 0.05);
        let sa = st::robust_summary(&c.a);
        let perm = st::perm_test(&c.a, &c.b, st::diff_of_medians, 20_000, 12345);

        out.push(Out {
            id: c.id.clone(),
            mwu_u: mwu_auto.u_a,
            mwu_p_exact: mwu_exact.p_two_sided,
            mwu_p_asymptotic: mwu_asym.p_two_sided,
            mwu_exact_used: mwu_auto.exact,
            mwu_rank_biserial: mwu_auto.rank_biserial,
            mwu_prob_superiority: mwu_auto.prob_superiority,
            wsr_statistic: wsr_exact.statistic,
            wsr_p_exact: wsr_exact.p_two_sided,
            wsr_p_asymptotic: wsr_asym.p_two_sided,
            wsr_p_pratt_exact: wsr_pratt.p_two_sided,
            hl_unpaired: hlu.estimate,
            hl_unpaired_lo: hlu.lo,
            hl_unpaired_hi: hlu.hi,
            hl_unpaired_level: hlu.achieved_level,
            hl_unpaired_exact: hlu.exact,
            hl_paired: hlp.estimate,
            hl_paired_lo: hlp.lo,
            hl_paired_hi: hlp.hi,
            hl_paired_exact: hlp.exact,
            median_a: sa.median,
            q1_a: sa.q1,
            q3_a: sa.q3,
            mad_a: sa.mad,
            perm_p: perm.p_two_sided,
            perm_exhaustive: perm.exhaustive,
        });
    }

    match serde_json::to_string_pretty(&out) {
        Ok(j) => {
            if let Err(e) = std::fs::write(output, j) {
                eprintln!("cannot write {output}: {e}");
            } else {
                println!("wrote {} result(s) to {output}", out.len());
            }
        }
        Err(e) => eprintln!("serialise failed: {e}"),
    }
}
