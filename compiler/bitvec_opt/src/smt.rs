//! Self-contained bit-blasting Tseitin CNF encoder and bounded CDCL SAT verifier.
//!
//! Verifies candidate equivalence and definedness under domain constraints:
//! `P(x) => defined(candidate, x) && candidate(x) == reference(x)`.
//!
//! Half-open slice ranges `[start .. start + width)` are translated cleanly
//! without off-by-one errors (and converted to inclusive bounds `[hi, lo]` when
//! formatting SMT-LIB indices for positive widths).

use std::collections::BTreeMap;
use std::fmt::{self, Display, Write};

use crate::baseline::BitValue;
use crate::ir::{
    CmpOp, DomainConstraint, DomainTerm, ExtractStart, InputIdentity, IrError, Node, Region,
};
use crate::sexpr::Quoted;
use crate::synth::{TargetOp, TargetProgram};

/// Step and conflict budget bounding SAT solving and CEGIS synthesis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_steps: u64,
    pub steps_used: u64,
    pub max_conflicts: u64,
    pub conflicts_used: u64,
}

impl Budget {
    /// Creates a bounded verification budget with matching step and conflict caps.
    pub fn new(max_steps: u64) -> Self {
        Self {
            max_steps,
            steps_used: 0,
            max_conflicts: max_steps,
            conflicts_used: 0,
        }
    }

    /// Creates a budget with explicit step and conflict limits.
    pub fn with_limits(max_steps: u64, max_conflicts: u64) -> Self {
        Self {
            max_steps,
            steps_used: 0,
            max_conflicts,
            conflicts_used: 0,
        }
    }

    /// Consumes `steps` from the remaining budget, returning `false` when exhausted.
    pub fn consume_steps(&mut self, steps: u64) -> bool {
        self.steps_used = self.steps_used.saturating_add(steps);
        self.steps_used <= self.max_steps
    }

    /// Records a solver conflict, returning `false` when `max_conflicts` is exceeded.
    pub fn consume_conflict(&mut self) -> bool {
        self.conflicts_used = self.conflicts_used.saturating_add(1);
        self.conflicts_used <= self.max_conflicts
    }

    /// Returns `true` if either step or conflict limit has been reached.
    pub fn is_exhausted(&self) -> bool {
        self.steps_used > self.max_steps || self.conflicts_used > self.max_conflicts
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::with_limits(500_000, 50_000)
    }
}

impl Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(budget (steps {}/{}) (conflicts {}/{}))",
            self.steps_used, self.max_steps, self.conflicts_used, self.max_conflicts
        )
    }
}

/// Reason a candidate was refuted by the verifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CounterexampleReason {
    /// Candidate produced different bits from the reference on a valid domain input.
    ValueMismatch,
    /// Candidate executed an undefined operation (e.g. out-of-range shift or OOB slice).
    UndefinedCandidateExecution(String),
    /// Candidate altered or dropped a required [`BehaviorBoundary`](crate::ir::BehaviorBoundary) event.
    BoundaryContractMismatch,
}

impl Display for CounterexampleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValueMismatch => f.write_str("(reason value-mismatch)"),
            Self::UndefinedCandidateExecution(detail) => {
                write!(f, "(reason undefined-candidate {})", Quoted(detail))
            }
            Self::BoundaryContractMismatch => f.write_str("(reason boundary-contract-mismatch)"),
        }
    }
}

/// Concrete counterexample model returned by SAT refutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counterexample {
    pub inputs: BTreeMap<InputIdentity, BitValue>,
    pub reason: CounterexampleReason,
}

impl Display for Counterexample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(counterexample ")?;
        self.reason.fmt(f)?;
        f.write_str(" (model (")?;
        for (idx, (id, val)) in self.inputs.iter().enumerate() {
            if idx != 0 {
                f.write_char(' ')?;
            }
            write!(f, "(assign {id} {val})")?;
        }
        f.write_str(")))")
    }
}

/// Certificate emitted when UNSAT proves full equivalence over `P(x)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofCertificate {
    pub solver: &'static str,
    pub vars: usize,
    pub clauses: usize,
    pub steps_used: u64,
    pub conflicts: u64,
    pub summary: String,
}

impl Display for ProofCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(proof-certificate (solver {}) (vars {}) (clauses {}) (steps {}) (conflicts {}) (summary {}))",
            Quoted(self.solver),
            self.vars,
            self.clauses,
            self.steps_used,
            self.conflicts,
            Quoted(&self.summary)
        )
    }
}

/// Result of bounded formal verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofOutcome {
    Equivalent(ProofCertificate),
    Counterexample(Counterexample),
    Unknown(Budget),
}

impl Display for ProofOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Equivalent(cert) => write!(f, "(proof-equivalent {cert})"),
            Self::Counterexample(cex) => write!(f, "(proof-counterexample {cex})"),
            Self::Unknown(budget) => write!(f, "(proof-unknown {budget})"),
        }
    }
}

/// Structural errors encountered prior to SAT verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    Ir(IrError),
    WidthMismatch { reference: u32, candidate: u32 },
}

impl From<IrError> for VerifyError {
    fn from(value: IrError) -> Self {
        Self::Ir(value)
    }
}

impl Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ir(err) => write!(f, "(verify-error ir {err})"),
            Self::WidthMismatch {
                reference,
                candidate,
            } => write!(
                f,
                "(verify-error width-mismatch (reference {reference}) (candidate {candidate}))"
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Converts Flurry's half-open slice `[start .. start + width)` to SMT-LIB's
/// inclusive `(hi, lo)` extraction bounds (`None` for 0-width slices).
pub fn half_open_to_inclusive_extract(start: u32, width: u32) -> Option<(u32, u32)> {
    if width == 0 {
        None
    } else {
        Some((start + width - 1, start))
    }
}

/// SAT literal encoded as `(var << 1) | sign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lit(u32);

impl Lit {
    /// Positive literal for variable `var`.
    pub fn pos(var: usize) -> Self {
        Self((var as u32) << 1)
    }

    /// Negative literal for variable `var`.
    pub fn neg(var: usize) -> Self {
        Self(((var as u32) << 1) | 1)
    }

    /// Variable index.
    pub fn var(self) -> usize {
        (self.0 >> 1) as usize
    }

    /// Polarity (`true` = positive, `false` = negated).
    pub fn polarity(self) -> bool {
        (self.0 & 1) == 0
    }

    /// Logical negation.
    pub fn not(self) -> Self {
        Self(self.0 ^ 1)
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// Raw SAT solver result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SatResult {
    Sat(Vec<bool>),
    Unsat,
    Unknown(Budget),
}

fn eval_lit(values: &[Option<bool>], lit: Lit) -> Option<bool> {
    values[lit.var()].map(|v| if lit.polarity() { v } else { !v })
}

fn bump_var_activity(activity: &mut [f64], var_inc: &mut f64, var: usize) {
    activity[var] += *var_inc;
    if activity[var] > 1e100 {
        for act in activity.iter_mut() {
            *act *= 1e-100;
        }
        *var_inc *= 1e-100;
    }
}

/// Self-contained CDCL SAT solver with 2-watched literals, 1-UIP learning,
/// phase saving, VSIDS-like activity heuristic, and bounded step budget.
#[derive(Clone, Debug)]
pub struct CdclSolver {
    num_vars: usize,
    clauses: Vec<Vec<Lit>>,
    watches: Vec<Vec<usize>>,
    values: Vec<Option<bool>>,
    levels: Vec<u32>,
    reasons: Vec<Option<usize>>,
    phases: Vec<bool>,
    activity: Vec<f64>,
    var_inc: f64,
    trail: Vec<Lit>,
    trail_lim: Vec<usize>,
    qhead: usize,
    seen: Vec<bool>,
    unsat: bool,
}

impl Default for CdclSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CdclSolver {
    /// Creates an empty CDCL solver instance.
    pub fn new() -> Self {
        Self {
            num_vars: 0,
            clauses: Vec::new(),
            watches: Vec::new(),
            values: Vec::new(),
            levels: Vec::new(),
            reasons: Vec::new(),
            phases: Vec::new(),
            activity: Vec::new(),
            var_inc: 1.0,
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            seen: Vec::new(),
            unsat: false,
        }
    }

    /// Allocates a fresh propositional variable and returns its index.
    pub fn new_var(&mut self) -> usize {
        let v = self.num_vars;
        self.num_vars += 1;
        self.watches.push(Vec::new());
        self.watches.push(Vec::new());
        self.values.push(None);
        self.levels.push(0);
        self.reasons.push(None);
        self.phases.push(false);
        self.activity.push(0.0);
        self.seen.push(false);
        v
    }

    /// Number of allocated propositional variables.
    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// Number of clauses currently in the database.
    pub fn num_clauses(&self) -> usize {
        self.clauses.len()
    }

    /// Adds a CNF clause to the solver.
    pub fn add_clause(&mut self, raw: &[Lit]) {
        if self.unsat {
            return;
        }
        let mut clause: Vec<Lit> = raw.to_vec();
        clause.sort();
        clause.dedup();
        for pair in clause.windows(2) {
            if pair[0].var() == pair[1].var() {
                // Contains both x and !x -> tautology.
                return;
            }
        }
        // Drop literals known false at decision level 0; skip satisfied clauses.
        let mut filtered = Vec::with_capacity(clause.len());
        for lit in clause {
            match self.lit_value(lit) {
                Some(true) if self.levels[lit.var()] == 0 => return,
                Some(false) if self.levels[lit.var()] == 0 => continue,
                _ => filtered.push(lit),
            }
        }
        match filtered.len() {
            0 => self.unsat = true,
            1 => {
                if !self.enqueue(filtered[0], None) {
                    self.unsat = true;
                }
            }
            _ => {
                let cid = self.clauses.len();
                self.watches[filtered[0].index()].push(cid);
                self.watches[filtered[1].index()].push(cid);
                self.clauses.push(filtered);
            }
        }
    }

    fn lit_value(&self, lit: Lit) -> Option<bool> {
        eval_lit(&self.values, lit)
    }

    fn current_level(&self) -> u32 {
        self.trail_lim.len() as u32
    }

    fn enqueue(&mut self, lit: Lit, reason: Option<usize>) -> bool {
        let v = lit.var();
        let sign = lit.polarity();
        match self.values[v] {
            Some(cur) => cur == sign,
            None => {
                self.values[v] = Some(sign);
                self.levels[v] = self.current_level();
                self.reasons[v] = reason;
                self.phases[v] = sign;
                self.trail.push(lit);
                true
            }
        }
    }

    fn decay_activity(&mut self) {
        self.var_inc *= 1.05;
    }

    fn propagate(&mut self, budget: &mut Budget) -> Result<Option<usize>, ()> {
        while self.qhead < self.trail.len() {
            if !budget.consume_steps(1) {
                return Err(());
            }
            let p = self.trail[self.qhead];
            self.qhead += 1;
            let false_lit = p.not();
            let mut list = std::mem::take(&mut self.watches[false_lit.index()]);
            let mut idx = 0;

            while idx < list.len() {
                let cid = list[idx];
                let clause = &mut self.clauses[cid];
                if clause[0] == false_lit {
                    clause.swap(0, 1);
                }
                debug_assert_eq!(clause[1], false_lit);

                let first = clause[0];
                if eval_lit(&self.values, first) == Some(true) {
                    idx += 1;
                    continue;
                }

                let mut replacement = None;
                for k in 2..clause.len() {
                    if eval_lit(&self.values, clause[k]) != Some(false) {
                        replacement = Some(k);
                        break;
                    }
                }

                if let Some(k) = replacement {
                    clause.swap(1, k);
                    let new_watch = clause[1];
                    list.swap_remove(idx);
                    self.watches[new_watch.index()].push(cid);
                    continue;
                }

                if eval_lit(&self.values, first) == Some(false) {
                    self.watches[false_lit.index()].extend(list);
                    return Ok(Some(cid));
                }

                let ok = self.enqueue(first, Some(cid));
                debug_assert!(ok);
                idx += 1;
            }
            self.watches[false_lit.index()].extend(list);
        }
        Ok(None)
    }

    fn analyze(&mut self, confl_cid: usize) -> (Vec<Lit>, u32) {
        let cur_level = self.current_level();
        let mut learned = vec![Lit(0)];
        let mut counter = 0usize;
        let mut cid = confl_cid;
        let mut p: Option<Lit> = None;
        let mut trail_idx = self.trail.len();

        loop {
            let clause_len = self.clauses[cid].len();
            for j in 0..clause_len {
                let q = self.clauses[cid][j];
                if Some(q) == p {
                    continue;
                }
                let v = q.var();
                if !self.seen[v] && self.levels[v] > 0 {
                    self.seen[v] = true;
                    bump_var_activity(&mut self.activity, &mut self.var_inc, v);
                    if self.levels[v] == cur_level {
                        counter += 1;
                    } else {
                        learned.push(q);
                    }
                }
            }

            let next_lit = loop {
                trail_idx -= 1;
                let cand = self.trail[trail_idx];
                if self.seen[cand.var()] {
                    break cand;
                }
            };
            p = Some(next_lit);
            self.seen[next_lit.var()] = false;
            counter -= 1;
            if counter == 0 {
                break;
            }
            cid = self.reasons[next_lit.var()].expect("non-decision literal must have reason");
        }

        let uip = p.expect("1-UIP literal must exist");
        learned[0] = uip.not();
        for &lit in &learned[1..] {
            self.seen[lit.var()] = false;
        }

        let bt_level = if learned.len() == 1 {
            0
        } else {
            let mut max_i = 1;
            let mut max_lvl = self.levels[learned[1].var()];
            for i in 2..learned.len() {
                let lvl = self.levels[learned[i].var()];
                if lvl > max_lvl {
                    max_lvl = lvl;
                    max_i = i;
                }
            }
            learned.swap(1, max_i);
            max_lvl
        };
        self.decay_activity();
        (learned, bt_level)
    }

    fn backtrack_to(&mut self, target_level: u32) {
        while self.current_level() > target_level {
            let lim = self.trail_lim.pop().expect("trail_lim non-empty");
            while self.trail.len() > lim {
                let lit = self.trail.pop().expect("trail matches lim");
                let v = lit.var();
                self.phases[v] = self.values[v].unwrap_or(false);
                self.values[v] = None;
                self.reasons[v] = None;
            }
        }
        self.qhead = self.trail.len();
    }

    fn pick_branch_var(&self) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for v in 0..self.num_vars {
            if self.values[v].is_none() {
                let act = self.activity[v];
                if best.is_none_or(|(_, b_act)| act > b_act) {
                    best = Some((v, act));
                }
            }
        }
        best.map(|(v, _)| v)
    }

    /// Solves the added CNF formula within `budget`.
    pub fn solve(&mut self, budget: &mut Budget) -> SatResult {
        if self.unsat {
            return SatResult::Unsat;
        }
        let mut restart_limit = 256u64;
        let mut local_conflicts = 0u64;

        loop {
            match self.propagate(budget) {
                Err(()) => return SatResult::Unknown(*budget),
                Ok(Some(confl_cid)) => {
                    if !budget.consume_conflict() {
                        return SatResult::Unknown(*budget);
                    }
                    if self.current_level() == 0 {
                        self.unsat = true;
                        return SatResult::Unsat;
                    }
                    local_conflicts += 1;
                    let (learned, bt_level) = self.analyze(confl_cid);
                    self.backtrack_to(bt_level);
                    if learned.len() == 1 {
                        if !self.enqueue(learned[0], None) {
                            self.unsat = true;
                            return SatResult::Unsat;
                        }
                    } else {
                        let cid = self.clauses.len();
                        let assert_lit = learned[0];
                        self.watches[learned[0].index()].push(cid);
                        self.watches[learned[1].index()].push(cid);
                        self.clauses.push(learned);
                        let ok = self.enqueue(assert_lit, Some(cid));
                        debug_assert!(ok);
                    }
                }
                Ok(None) => {
                    if local_conflicts >= restart_limit {
                        local_conflicts = 0;
                        restart_limit = restart_limit.saturating_mul(3) / 2;
                        self.backtrack_to(0);
                    }
                    let Some(var) = self.pick_branch_var() else {
                        let model = self.values.iter().map(|val| val.unwrap_or(false)).collect();
                        return SatResult::Sat(model);
                    };
                    if !budget.consume_steps(1) {
                        return SatResult::Unknown(*budget);
                    }
                    self.trail_lim.push(self.trail.len());
                    let dec_lit = if self.phases[var] {
                        Lit::pos(var)
                    } else {
                        Lit::neg(var)
                    };
                    let ok = self.enqueue(dec_lit, None);
                    debug_assert!(ok);
                }
            }
        }
    }
}

/// Bit-blasting Tseitin CNF encoder from BitVec IR/target programs to SAT.
pub struct BitBlastEncoder {
    solver: CdclSolver,
    true_lit: Lit,
    false_lit: Lit,
    inputs: BTreeMap<InputIdentity, Vec<Lit>>,
}

impl Default for BitBlastEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl BitBlastEncoder {
    /// Creates an encoder with fixed `true` and `false` constant literals.
    pub fn new() -> Self {
        let mut solver = CdclSolver::new();
        let tv = solver.new_var();
        let true_lit = Lit::pos(tv);
        let false_lit = Lit::neg(tv);
        solver.add_clause(&[true_lit]);
        Self {
            solver,
            true_lit,
            false_lit,
            inputs: BTreeMap::new(),
        }
    }

    /// Returns the fixed boolean-true literal.
    pub fn true_lit(&self) -> Lit {
        self.true_lit
    }

    /// Returns the fixed boolean-false literal.
    pub fn false_lit(&self) -> Lit {
        self.false_lit
    }

    /// Allocates or retrieves the SAT bit vector for `identity` (LSB at index 0).
    pub fn input_bits(
        &mut self,
        identity: &InputIdentity,
        width: u32,
    ) -> Result<Vec<Lit>, IrError> {
        if let Some(existing) = self.inputs.get(identity) {
            if existing.len() != width as usize {
                return Err(IrError::ConflictingInputWidth {
                    identity: identity.clone(),
                    expected: existing.len() as u32,
                    actual: width,
                });
            }
            return Ok(existing.clone());
        }
        let mut lits = Vec::with_capacity(width as usize);
        for _ in 0..width {
            lits.push(Lit::pos(self.solver.new_var()));
        }
        self.inputs.insert(identity.clone(), lits.clone());
        Ok(lits)
    }

    /// Converts a constant `BitValue` into constant `Lit`s.
    pub fn const_bits(&self, bits: &BitValue) -> Vec<Lit> {
        (0..bits.width())
            .map(|i| {
                if bits.bit(i).unwrap_or(false) {
                    self.true_lit
                } else {
                    self.false_lit
                }
            })
            .collect()
    }

    /// Tseitin NOT gate (zero clauses).
    pub fn mk_not(&self, a: Lit) -> Lit {
        a.not()
    }

    /// Tseitin AND gate with constant folding.
    pub fn mk_and(&mut self, a: Lit, b: Lit) -> Lit {
        if a == self.false_lit || b == self.false_lit || a == b.not() {
            return self.false_lit;
        }
        if a == self.true_lit || a == b {
            return b;
        }
        if b == self.true_lit {
            return a;
        }
        let z = Lit::pos(self.solver.new_var());
        self.solver.add_clause(&[z.not(), a]);
        self.solver.add_clause(&[z.not(), b]);
        self.solver.add_clause(&[z, a.not(), b.not()]);
        z
    }

    /// Tseitin OR gate with constant folding.
    pub fn mk_or(&mut self, a: Lit, b: Lit) -> Lit {
        if a == self.true_lit || b == self.true_lit || a == b.not() {
            return self.true_lit;
        }
        if a == self.false_lit || a == b {
            return b;
        }
        if b == self.false_lit {
            return a;
        }
        let z = Lit::pos(self.solver.new_var());
        self.solver.add_clause(&[z, a.not()]);
        self.solver.add_clause(&[z, b.not()]);
        self.solver.add_clause(&[z.not(), a, b]);
        z
    }

    /// Tseitin XOR gate with constant folding.
    pub fn mk_xor(&mut self, a: Lit, b: Lit) -> Lit {
        if a == b {
            return self.false_lit;
        }
        if a == b.not() {
            return self.true_lit;
        }
        if a == self.false_lit {
            return b;
        }
        if a == self.true_lit {
            return b.not();
        }
        if b == self.false_lit {
            return a;
        }
        if b == self.true_lit {
            return a.not();
        }
        let z = Lit::pos(self.solver.new_var());
        self.solver.add_clause(&[z.not(), a.not(), b.not()]);
        self.solver.add_clause(&[z.not(), a, b]);
        self.solver.add_clause(&[z, a.not(), b]);
        self.solver.add_clause(&[z, a, b.not()]);
        z
    }

    /// Tseitin ITE mux (`if cond then t else e`).
    pub fn mk_ite(&mut self, cond: Lit, t: Lit, e: Lit) -> Lit {
        if cond == self.true_lit || t == e {
            return t;
        }
        if cond == self.false_lit {
            return e;
        }
        if t == self.true_lit && e == self.false_lit {
            return cond;
        }
        if t == self.false_lit && e == self.true_lit {
            return cond.not();
        }
        if t == self.true_lit {
            return self.mk_or(cond, e);
        }
        if t == self.false_lit {
            return self.mk_and(cond.not(), e);
        }
        if e == self.false_lit {
            return self.mk_and(cond, t);
        }
        if e == self.true_lit {
            return self.mk_or(cond.not(), t);
        }
        let z = Lit::pos(self.solver.new_var());
        self.solver.add_clause(&[cond.not(), t.not(), z]);
        self.solver.add_clause(&[cond.not(), t, z.not()]);
        self.solver.add_clause(&[cond, e.not(), z]);
        self.solver.add_clause(&[cond, e, z.not()]);
        z
    }

    /// Conjunction over a slice of literals (`true_lit` if empty).
    pub fn mk_all_and(&mut self, lits: &[Lit]) -> Lit {
        let mut acc = self.true_lit;
        for &l in lits {
            acc = self.mk_and(acc, l);
        }
        acc
    }

    /// Disjunction over a slice of literals (`false_lit` if empty).
    pub fn mk_any_or(&mut self, lits: &[Lit]) -> Lit {
        let mut acc = self.false_lit;
        for &l in lits {
            acc = self.mk_or(acc, l);
        }
        acc
    }

    /// Encodes exact bit-vector equality (`lhs == rhs`, `true_lit` for empty 0-bit vectors).
    pub fn encode_bv_eq(&mut self, lhs: &[Lit], rhs: &[Lit]) -> Lit {
        debug_assert_eq!(lhs.len(), rhs.len());
        let mut eqs = Vec::with_capacity(lhs.len());
        for (&a, &b) in lhs.iter().zip(rhs) {
            let x = self.mk_xor(a, b);
            eqs.push(x.not());
        }
        self.mk_all_and(&eqs)
    }

    /// Encodes arbitrary-width unsigned less-than (`lhs < rhs`, zero-extending shorter operand).
    pub fn encode_bv_ult(&mut self, lhs: &[Lit], rhs: &[Lit]) -> Lit {
        let max_len = lhs.len().max(rhs.len());
        let mut lt = self.false_lit;
        for i in 0..max_len {
            let a = lhs.get(i).copied().unwrap_or(self.false_lit);
            let b = rhs.get(i).copied().unwrap_or(self.false_lit);
            let diff = self.mk_xor(a, b);
            lt = self.mk_ite(diff, b, lt);
        }
        lt
    }

    /// Encodes unsigned comparison (`lhs op rhs`).
    pub fn encode_bv_cmp(&mut self, lhs: &[Lit], op: CmpOp, rhs: &[Lit]) -> Lit {
        match op {
            CmpOp::Eq => self.encode_bv_eq(lhs, rhs),
            CmpOp::Ne => self.encode_bv_eq(lhs, rhs).not(),
            CmpOp::Ult => self.encode_bv_ult(lhs, rhs),
            CmpOp::Ule => self.encode_bv_ult(rhs, lhs).not(),
            CmpOp::Ugt => self.encode_bv_ult(rhs, lhs),
            CmpOp::Uge => self.encode_bv_ult(lhs, rhs).not(),
        }
    }

    /// Encodes a [`DomainTerm`] to SAT bits.
    pub fn encode_domain_term(&mut self, term: &DomainTerm) -> Result<Vec<Lit>, IrError> {
        match term {
            DomainTerm::Input { identity, width } => self.input_bits(identity, *width),
            DomainTerm::Constant(bits) => Ok(self.const_bits(bits)),
            DomainTerm::Extract { term, start, width } => {
                let bits = self.encode_domain_term(term)?;
                let start_idx = *start as usize;
                let end_idx = start_idx + (*width as usize);
                if end_idx > bits.len() {
                    return Err(IrError::SliceOutOfBounds {
                        input_width: bits.len() as u32,
                        start: *start,
                        slice_width: *width,
                    });
                }
                Ok(bits[start_idx..end_idx].to_vec())
            }
            DomainTerm::Concat(hi, lo) => {
                let hi_bits = self.encode_domain_term(hi)?;
                let lo_bits = self.encode_domain_term(lo)?;
                let mut out = Vec::with_capacity(lo_bits.len() + hi_bits.len());
                out.extend_from_slice(&lo_bits);
                out.extend_from_slice(&hi_bits);
                Ok(out)
            }
            DomainTerm::And(l, r) => {
                let lb = self.encode_domain_term(l)?;
                let rb = self.encode_domain_term(r)?;
                Ok(lb
                    .iter()
                    .zip(&rb)
                    .map(|(&a, &b)| self.mk_and(a, b))
                    .collect())
            }
            DomainTerm::Or(l, r) => {
                let lb = self.encode_domain_term(l)?;
                let rb = self.encode_domain_term(r)?;
                Ok(lb
                    .iter()
                    .zip(&rb)
                    .map(|(&a, &b)| self.mk_or(a, b))
                    .collect())
            }
            DomainTerm::Not(inner) => {
                let ib = self.encode_domain_term(inner)?;
                Ok(ib.into_iter().map(|l| self.mk_not(l)).collect())
            }
            DomainTerm::ZeroExtend { term, width } => {
                let mut ib = self.encode_domain_term(term)?;
                ib.resize(*width as usize, self.false_lit);
                Ok(ib)
            }
        }
    }

    /// Encodes a [`DomainConstraint`] predicate `P(x)` into a single SAT literal.
    pub fn encode_domain(&mut self, domain: &DomainConstraint) -> Result<Lit, IrError> {
        domain.validate()?;
        match domain {
            DomainConstraint::True => Ok(self.true_lit),
            DomainConstraint::False => Ok(self.false_lit),
            DomainConstraint::UnsignedCmp { lhs, op, rhs } => {
                let lb = self.encode_domain_term(lhs)?;
                let rb = self.encode_domain_term(rhs)?;
                Ok(self.encode_bv_cmp(&lb, *op, &rb))
            }
            DomainConstraint::InSet { term, allowed } => {
                let tb = self.encode_domain_term(term)?;
                let mut opts = Vec::with_capacity(allowed.len());
                for val in allowed {
                    let cb = self.const_bits(val);
                    opts.push(self.encode_bv_eq(&tb, &cb));
                }
                Ok(self.mk_any_or(&opts))
            }
            DomainConstraint::And(items) => {
                let mut lits = Vec::with_capacity(items.len());
                for item in items {
                    lits.push(self.encode_domain(item)?);
                }
                Ok(self.mk_all_and(&lits))
            }
            DomainConstraint::Or(items) => {
                let mut lits = Vec::with_capacity(items.len());
                for item in items {
                    lits.push(self.encode_domain(item)?);
                }
                Ok(self.mk_any_or(&lits))
            }
            DomainConstraint::Not(inner) => {
                let l = self.encode_domain(inner)?;
                Ok(l.not())
            }
        }
    }

    /// Encodes a dynamic slice `[idx .. idx + width)` via a barrel shifter and
    /// computes its definedness predicate (`idx + width <= in_bits.len()`).
    pub fn encode_dynamic_extract(
        &mut self,
        in_bits: &[Lit],
        idx_bits: &[Lit],
        width: u32,
    ) -> (Vec<Lit>, Lit) {
        let in_w = in_bits.len() as u32;
        if width > in_w {
            return (vec![self.false_lit; width as usize], self.false_lit);
        }
        let max_start = u64::from(in_w - width);
        let max_bv = BitValue::from_u64(idx_bits.len().max(64) as u32, max_start)
            .expect("u32 fits in 64 bits");
        let max_lits = self.const_bits(&max_bv);
        let defined = self.encode_bv_cmp(idx_bits, CmpOp::Ule, &max_lits);
        if width == 0 {
            return (Vec::new(), defined);
        }

        let mut cur = in_bits.to_vec();
        let n = cur.len();
        for (b, &sel) in idx_bits.iter().enumerate() {
            if b >= usize::BITS as usize {
                break;
            }
            let step = 1usize << b;
            if step > n {
                break;
            }
            let mut next = Vec::with_capacity(n);
            for i in 0..n {
                let shifted = if i + step < n {
                    cur[i + step]
                } else {
                    self.false_lit
                };
                next.push(self.mk_ite(sel, shifted, cur[i]));
            }
            cur = next;
        }
        cur.truncate(width as usize);
        (cur, defined)
    }

    /// Encodes dynamic left shift (`defined` iff `shift < width`).
    pub fn encode_dynamic_shl(&mut self, bits: &[Lit], shift_bits: &[Lit]) -> (Vec<Lit>, Lit) {
        let w = bits.len();
        if w == 0 {
            return (Vec::new(), self.false_lit);
        }
        let width_bv = BitValue::from_u64(shift_bits.len().max(64) as u32, w as u64)
            .expect("usize width fits in u64");
        let width_lits = self.const_bits(&width_bv);
        let defined = self.encode_bv_ult(shift_bits, &width_lits);

        let mut cur = bits.to_vec();
        for (b, &sel) in shift_bits.iter().enumerate() {
            if b >= usize::BITS as usize {
                break;
            }
            let step = 1usize << b;
            if step >= w {
                break;
            }
            let mut next = Vec::with_capacity(w);
            for i in 0..w {
                let shifted = if i >= step {
                    cur[i - step]
                } else {
                    self.false_lit
                };
                next.push(self.mk_ite(sel, shifted, cur[i]));
            }
            cur = next;
        }
        (cur, defined)
    }

    /// Encodes dynamic logical right shift (`defined` iff `shift < width`).
    pub fn encode_dynamic_lshr(&mut self, bits: &[Lit], shift_bits: &[Lit]) -> (Vec<Lit>, Lit) {
        let w = bits.len();
        if w == 0 {
            return (Vec::new(), self.false_lit);
        }
        let width_bv = BitValue::from_u64(shift_bits.len().max(64) as u32, w as u64)
            .expect("usize width fits in u64");
        let width_lits = self.const_bits(&width_bv);
        let defined = self.encode_bv_ult(shift_bits, &width_lits);

        let mut cur = bits.to_vec();
        for (b, &sel) in shift_bits.iter().enumerate() {
            if b >= usize::BITS as usize {
                break;
            }
            let step = 1usize << b;
            if step >= w {
                break;
            }
            let mut next = Vec::with_capacity(w);
            for i in 0..w {
                let shifted = if i + step < w {
                    cur[i + step]
                } else {
                    self.false_lit
                };
                next.push(self.mk_ite(sel, shifted, cur[i]));
            }
            cur = next;
        }
        (cur, defined)
    }

    /// Encodes dynamic rotate right (`defined` iff `shift < width`).
    pub fn encode_dynamic_rotr(&mut self, bits: &[Lit], shift_bits: &[Lit]) -> (Vec<Lit>, Lit) {
        let w = bits.len();
        if w == 0 {
            return (Vec::new(), self.false_lit);
        }
        let width_bv = BitValue::from_u64(shift_bits.len().max(64) as u32, w as u64)
            .expect("usize width fits in u64");
        let width_lits = self.const_bits(&width_bv);
        let defined = self.encode_bv_ult(shift_bits, &width_lits);

        let mut cur = bits.to_vec();
        for (b, &sel) in shift_bits.iter().enumerate() {
            if b >= usize::BITS as usize {
                break;
            }
            let raw_step = 1usize << b;
            if raw_step >= w {
                break;
            }
            let step = raw_step % w;
            let mut next = Vec::with_capacity(w);
            for i in 0..w {
                let rotated = cur[(i + step) % w];
                next.push(self.mk_ite(sel, rotated, cur[i]));
            }
            cur = next;
        }
        (cur, defined)
    }

    /// Encodes dynamic rotate left (`defined` iff `shift < width`).
    pub fn encode_dynamic_rotl(&mut self, bits: &[Lit], shift_bits: &[Lit]) -> (Vec<Lit>, Lit) {
        let w = bits.len();
        if w == 0 {
            return (Vec::new(), self.false_lit);
        }
        let width_bv = BitValue::from_u64(shift_bits.len().max(64) as u32, w as u64)
            .expect("usize width fits in u64");
        let width_lits = self.const_bits(&width_bv);
        let defined = self.encode_bv_ult(shift_bits, &width_lits);

        let mut cur = bits.to_vec();
        for (b, &sel) in shift_bits.iter().enumerate() {
            if b >= usize::BITS as usize {
                break;
            }
            let raw_step = 1usize << b;
            if raw_step >= w {
                break;
            }
            let step = raw_step % w;
            let mut next = Vec::with_capacity(w);
            for i in 0..w {
                let rotated = cur[(i + w - step) % w];
                next.push(self.mk_ite(sel, rotated, cur[i]));
            }
            cur = next;
        }
        (cur, defined)
    }

    /// Bit-blasts an entire [`Region`] DAG, returning `(root_bits, defined_lit)`.
    pub fn encode_region(&mut self, region: &Region) -> Result<(Vec<Lit>, Lit), IrError> {
        region.validate()?;
        for (id, &w) in &region.inputs {
            self.input_bits(id, w)?;
        }

        let mut node_bits: Vec<Vec<Lit>> = Vec::with_capacity(region.nodes.len());
        let mut defined_all = Vec::new();

        for node in &region.nodes {
            let bits = match node {
                Node::Input { width, identity } => self.input_bits(identity, *width)?,
                Node::Constant { bits, .. } => self.const_bits(bits),
                Node::Concat(hi, lo) => {
                    let mut out = node_bits[lo.0].clone();
                    out.extend_from_slice(&node_bits[hi.0]);
                    out
                }
                Node::Extract {
                    input,
                    start,
                    width,
                } => match start {
                    ExtractStart::Const(lo) => {
                        if let Some((hi_incl, lo_incl)) =
                            half_open_to_inclusive_extract(*lo, *width)
                        {
                            node_bits[input.0][(lo_incl as usize)..=(hi_incl as usize)].to_vec()
                        } else {
                            Vec::new()
                        }
                    }
                    ExtractStart::Dynamic(idx_node) => {
                        let (out, def) = self.encode_dynamic_extract(
                            &node_bits[input.0],
                            &node_bits[idx_node.0],
                            *width,
                        );
                        defined_all.push(def);
                        out
                    }
                },
                Node::And(l, r) => node_bits[l.0]
                    .iter()
                    .zip(&node_bits[r.0])
                    .map(|(&a, &b)| self.mk_and(a, b))
                    .collect(),
                Node::Or(l, r) => node_bits[l.0]
                    .iter()
                    .zip(&node_bits[r.0])
                    .map(|(&a, &b)| self.mk_or(a, b))
                    .collect(),
                Node::Not(inner) => node_bits[inner.0].iter().map(|&l| self.mk_not(l)).collect(),
                Node::Equal(l, r) => {
                    let eq = self.encode_bv_eq(&node_bits[l.0], &node_bits[r.0]);
                    vec![eq]
                }
                Node::ZeroExtend { input, width } => {
                    let mut out = node_bits[input.0].clone();
                    out.resize(*width as usize, self.false_lit);
                    out
                }
            };
            node_bits.push(bits);
        }

        let def_lit = self.mk_all_and(&defined_all);
        Ok((node_bits[region.root.0].clone(), def_lit))
    }

    /// Bit-blasts a candidate [`TargetProgram`], returning `(root_bits, defined_lit)`.
    pub fn encode_target_program(
        &mut self,
        program: &TargetProgram,
    ) -> Result<(Vec<Lit>, Lit), IrError> {
        program.validate()?;
        for (id, &w) in &program.declared_inputs {
            self.input_bits(id, w)?;
        }

        let mut reg_bits: Vec<Vec<Lit>> = vec![Vec::new(); program.instructions.len()];
        let mut defined_all = Vec::new();

        for inst in &program.instructions {
            let bits = match &inst.op {
                TargetOp::ZeroWidthEmpty { .. } => Vec::new(),
                TargetOp::Const { bits, .. } => self.const_bits(bits),
                TargetOp::Input { width, identity } => self.input_bits(identity, *width)?,
                TargetOp::Copy { src, .. } => reg_bits[src.0].clone(),
                TargetOp::And { lhs, rhs, .. } => reg_bits[lhs.0]
                    .iter()
                    .zip(&reg_bits[rhs.0])
                    .map(|(&a, &b)| self.mk_and(a, b))
                    .collect(),
                TargetOp::Or { lhs, rhs, .. } => reg_bits[lhs.0]
                    .iter()
                    .zip(&reg_bits[rhs.0])
                    .map(|(&a, &b)| self.mk_or(a, b))
                    .collect(),
                TargetOp::Xor { lhs, rhs, .. } => reg_bits[lhs.0]
                    .iter()
                    .zip(&reg_bits[rhs.0])
                    .map(|(&a, &b)| self.mk_xor(a, b))
                    .collect(),
                TargetOp::Not { src, .. } => {
                    reg_bits[src.0].iter().map(|&l| self.mk_not(l)).collect()
                }
                TargetOp::AndNot { lhs, inverted, .. } => reg_bits[lhs.0]
                    .iter()
                    .zip(&reg_bits[inverted.0])
                    .map(|(&a, &b)| self.mk_and(a, self.mk_not(b)))
                    .collect(),
                TargetOp::OrNot { lhs, inverted, .. } => reg_bits[lhs.0]
                    .iter()
                    .zip(&reg_bits[inverted.0])
                    .map(|(&a, &b)| self.mk_or(a, self.mk_not(b)))
                    .collect(),
                TargetOp::Equal { lhs, rhs, .. } => {
                    vec![self.encode_bv_eq(&reg_bits[lhs.0], &reg_bits[rhs.0])]
                }
                TargetOp::Concat { high, low, .. } => {
                    let mut out = reg_bits[low.0].clone();
                    out.extend_from_slice(&reg_bits[high.0]);
                    out
                }
                TargetOp::ExtractConst {
                    input,
                    start,
                    width,
                } => {
                    if let Some((hi_incl, lo_incl)) = half_open_to_inclusive_extract(*start, *width)
                    {
                        reg_bits[input.0][(lo_incl as usize)..=(hi_incl as usize)].to_vec()
                    } else {
                        Vec::new()
                    }
                }
                TargetOp::ExtractDynamic {
                    input,
                    start,
                    width,
                    ..
                } => {
                    let (out, def) =
                        self.encode_dynamic_extract(&reg_bits[input.0], &reg_bits[start.0], *width);
                    defined_all.push(def);
                    out
                }
                TargetOp::ZeroExtend {
                    input,
                    target_width,
                } => {
                    let mut out = reg_bits[input.0].clone();
                    out.resize(*target_width as usize, self.false_lit);
                    out
                }
                TargetOp::Bswap16 { input } => encode_bswap_bits(&reg_bits[input.0], 2),
                TargetOp::Bswap32 { input } => encode_bswap_bits(&reg_bits[input.0], 4),
                TargetOp::Bswap64 { input } => encode_bswap_bits(&reg_bits[input.0], 8),
                TargetOp::RotrConst { input, amount, .. } => {
                    let src = &reg_bits[input.0];
                    let w = src.len();
                    if w <= 1 {
                        src.clone()
                    } else {
                        let k = (*amount as usize) % w;
                        (0..w).map(|i| src[(i + k) % w]).collect()
                    }
                }
                TargetOp::RotlConst { input, amount, .. } => {
                    let src = &reg_bits[input.0];
                    let w = src.len();
                    if w <= 1 {
                        src.clone()
                    } else {
                        let k = (*amount as usize) % w;
                        (0..w).map(|i| src[(i + w - k) % w]).collect()
                    }
                }
                TargetOp::ShlConst { input, amount, .. } => {
                    let src = &reg_bits[input.0];
                    let w = src.len();
                    if w == 0 || (*amount as usize) >= w {
                        defined_all.push(self.false_lit);
                        vec![self.false_lit; w]
                    } else {
                        let amt = *amount as usize;
                        (0..w)
                            .map(|i| {
                                if i < amt {
                                    self.false_lit
                                } else {
                                    src[i - amt]
                                }
                            })
                            .collect()
                    }
                }
                TargetOp::LshrConst { input, amount, .. } => {
                    let src = &reg_bits[input.0];
                    let w = src.len();
                    if w == 0 || (*amount as usize) >= w {
                        defined_all.push(self.false_lit);
                        vec![self.false_lit; w]
                    } else {
                        let amt = *amount as usize;
                        (0..w)
                            .map(|i| {
                                if i + amt >= w {
                                    self.false_lit
                                } else {
                                    src[i + amt]
                                }
                            })
                            .collect()
                    }
                }
                TargetOp::Shl { input, shift, .. } => {
                    let (out, def) =
                        self.encode_dynamic_shl(&reg_bits[input.0], &reg_bits[shift.0]);
                    defined_all.push(def);
                    out
                }
                TargetOp::Lshr { input, shift, .. } => {
                    let (out, def) =
                        self.encode_dynamic_lshr(&reg_bits[input.0], &reg_bits[shift.0]);
                    defined_all.push(def);
                    out
                }
                TargetOp::Rotr { input, shift, .. } => {
                    let (out, def) =
                        self.encode_dynamic_rotr(&reg_bits[input.0], &reg_bits[shift.0]);
                    defined_all.push(def);
                    out
                }
                TargetOp::Rotl { input, shift, .. } => {
                    let (out, def) =
                        self.encode_dynamic_rotl(&reg_bits[input.0], &reg_bits[shift.0]);
                    defined_all.push(def);
                    out
                }
            };
            reg_bits[inst.dst.0] = bits;
        }

        let def_lit = self.mk_all_and(&defined_all);
        Ok((reg_bits[program.root.0].clone(), def_lit))
    }

    fn solve_sat(mut self, query_lit: Lit, budget: &mut Budget) -> SatResult {
        self.solver.add_clause(&[query_lit]);
        self.solver.solve(budget)
    }
}

fn encode_bswap_bits(src: &[Lit], num_bytes: usize) -> Vec<Lit> {
    let mut out = Vec::with_capacity(num_bytes * 8);
    // Low byte moves to high byte; original high byte becomes lowest byte.
    for b in (0..num_bytes).rev() {
        let start = b * 8;
        out.extend_from_slice(&src[start..start + 8]);
    }
    out
}

fn decode_model_from_inputs(
    inputs: &BTreeMap<InputIdentity, Vec<Lit>>,
    assignment: &[bool],
) -> BTreeMap<InputIdentity, BitValue> {
    let mut model = BTreeMap::new();
    for (id, lits) in inputs {
        let bits: Vec<bool> = lits
            .iter()
            .map(|lit| {
                let val = assignment[lit.var()];
                if lit.polarity() { val } else { !val }
            })
            .collect();
        model.insert(id.clone(), BitValue::from_bits_lsb(&bits));
    }
    model
}

/// Verifies `P(x) => defined(candidate, x) && candidate(x) == reference(x)`
/// as well as strict [`BehaviorBoundary`](crate::ir::BehaviorBoundary) preservation.
pub fn verify_target_program(
    reference: &Region,
    candidate: &TargetProgram,
    budget: &mut Budget,
) -> Result<ProofOutcome, VerifyError> {
    reference.validate()?;
    candidate.validate()?;

    let ref_w = reference.output_kind.bit_width();
    if ref_w != candidate.output_width {
        return Err(VerifyError::WidthMismatch {
            reference: ref_w,
            candidate: candidate.output_width,
        });
    }

    if !reference.boundary.matches_contract(&candidate.boundary) {
        return Ok(ProofOutcome::Counterexample(Counterexample {
            inputs: BTreeMap::new(),
            reason: CounterexampleReason::BoundaryContractMismatch,
        }));
    }

    let mut enc = BitBlastEncoder::new();
    let domain_lit = enc.encode_domain(&reference.domain)?;
    let (ref_bits, _ref_def) = enc.encode_region(reference)?;
    let (cand_bits, cand_def) = enc.encode_target_program(candidate)?;

    if !budget.consume_steps(1) {
        return Ok(ProofOutcome::Unknown(*budget));
    }

    let undef_lit = cand_def.not();
    let eq_lit = enc.encode_bv_eq(&ref_bits, &cand_bits);
    let diff_lit = eq_lit.not();
    let bad_or = enc.mk_or(undef_lit, diff_lit);
    let query = enc.mk_and(domain_lit, bad_or);

    let vars = enc.solver.num_vars();
    let clauses = enc.solver.num_clauses();
    let input_map = enc.inputs.clone();
    match enc.solve_sat(query, budget) {
        SatResult::Unsat => Ok(ProofOutcome::Equivalent(ProofCertificate {
            solver: "bitvec-cdcl-tseitin-v1",
            vars,
            clauses,
            steps_used: budget.steps_used,
            conflicts: budget.conflicts_used,
            summary: format!("proved {ref_w}-bit target equivalence over P(x)"),
        })),
        SatResult::Unknown(b) => Ok(ProofOutcome::Unknown(b)),
        SatResult::Sat(assignment) => {
            let model = decode_model_from_inputs(&input_map, &assignment);
            let reason = match candidate.eval(&model) {
                Err(crate::baseline::EvalError::UndefinedTargetOperation(msg)) => {
                    CounterexampleReason::UndefinedCandidateExecution(msg)
                }
                Err(crate::baseline::EvalError::DynamicSliceOutOfBounds {
                    start_detail, ..
                }) => CounterexampleReason::UndefinedCandidateExecution(format!(
                    "dynamic slice out of bounds at {start_detail}"
                )),
                _ => CounterexampleReason::ValueMismatch,
            };
            Ok(ProofOutcome::Counterexample(Counterexample {
                inputs: model,
                reason,
            }))
        }
    }
}

/// Verifies that two [`Region`]s compute identical bit outputs under `reference.domain`
/// and preserve the [`BehaviorBoundary`](crate::ir::BehaviorBoundary).
pub fn verify_region_equivalence(
    reference: &Region,
    candidate: &Region,
    budget: &mut Budget,
) -> Result<ProofOutcome, VerifyError> {
    reference.validate()?;
    candidate.validate()?;
    let ref_w = reference.output_kind.bit_width();
    let cand_w = candidate.output_kind.bit_width();
    if ref_w != cand_w {
        return Err(VerifyError::WidthMismatch {
            reference: ref_w,
            candidate: cand_w,
        });
    }
    if !reference.boundary.matches_contract(&candidate.boundary) {
        return Ok(ProofOutcome::Counterexample(Counterexample {
            inputs: BTreeMap::new(),
            reason: CounterexampleReason::BoundaryContractMismatch,
        }));
    }

    let mut enc = BitBlastEncoder::new();
    let domain_lit = enc.encode_domain(&reference.domain)?;
    let (ref_bits, _) = enc.encode_region(reference)?;
    let (cand_bits, cand_def) = enc.encode_region(candidate)?;

    let eq_lit = enc.encode_bv_eq(&ref_bits, &cand_bits);
    let bad = enc.mk_or(cand_def.not(), eq_lit.not());
    let query = enc.mk_and(domain_lit, bad);

    let vars = enc.solver.num_vars();
    let clauses = enc.solver.num_clauses();
    let input_map = enc.inputs.clone();
    match enc.solve_sat(query, budget) {
        SatResult::Unsat => Ok(ProofOutcome::Equivalent(ProofCertificate {
            solver: "bitvec-cdcl-tseitin-v1",
            vars,
            clauses,
            steps_used: budget.steps_used,
            conflicts: budget.conflicts_used,
            summary: format!("proved {ref_w}-bit region equivalence over P(x)"),
        })),
        SatResult::Unknown(b) => Ok(ProofOutcome::Unknown(b)),
        SatResult::Sat(assignment) => {
            let model = decode_model_from_inputs(&input_map, &assignment);
            Ok(ProofOutcome::Counterexample(Counterexample {
                inputs: model,
                reason: CounterexampleReason::ValueMismatch,
            }))
        }
    }
}

/// Finds a concrete input assignment satisfying `region.domain` (`P(x)`), if any exists.
pub fn find_satisfying_inputs(
    region: &Region,
    budget: &mut Budget,
) -> Result<Option<BTreeMap<InputIdentity, BitValue>>, VerifyError> {
    region.validate()?;
    let mut enc = BitBlastEncoder::new();
    for (id, &w) in &region.inputs {
        enc.input_bits(id, w)?;
    }
    let dom = enc.encode_domain(&region.domain)?;
    let input_map = enc.inputs.clone();
    match enc.solve_sat(dom, budget) {
        SatResult::Sat(assignment) => Ok(Some(decode_model_from_inputs(&input_map, &assignment))),
        SatResult::Unsat | SatResult::Unknown(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cdcl_pigeonhole_4_3_unsat_and_3sat_sat() {
        // Pigeonhole principle: 4 pigeons into 3 holes is UNSAT and exercises
        // multi-level conflict analysis and 1-UIP clause learning.
        let mut solver = CdclSolver::new();
        let mut vars = [[0usize; 3]; 4];
        for p in &mut vars {
            for h in p {
                *h = solver.new_var();
            }
        }
        for p in &vars {
            solver.add_clause(&[Lit::pos(p[0]), Lit::pos(p[1]), Lit::pos(p[2])]);
        }
        for h in 0..3 {
            for p1 in 0..4 {
                for p2 in (p1 + 1)..4 {
                    solver.add_clause(&[Lit::neg(vars[p1][h]), Lit::neg(vars[p2][h])]);
                }
            }
        }
        let mut budget = Budget::default();
        assert_eq!(solver.solve(&mut budget), SatResult::Unsat);
        assert!(budget.conflicts_used > 0);

        // Satisfiable 3-SAT formula
        let mut s2 = CdclSolver::new();
        let x0 = s2.new_var();
        let x1 = s2.new_var();
        let x2 = s2.new_var();
        let clauses = [
            [Lit::pos(x0), Lit::pos(x1), Lit::neg(x2)],
            [Lit::neg(x0), Lit::pos(x1), Lit::pos(x2)],
            [Lit::pos(x0), Lit::neg(x1), Lit::pos(x2)],
            [Lit::neg(x0), Lit::neg(x1), Lit::neg(x2)],
        ];
        for c in &clauses {
            s2.add_clause(c);
        }
        let mut b2 = Budget::default();
        match s2.solve(&mut b2) {
            SatResult::Sat(model) => {
                for c in &clauses {
                    let sat = c.iter().any(|lit| {
                        let v = model[lit.var()];
                        if lit.polarity() { v } else { !v }
                    });
                    assert!(sat, "SAT assignment violated clause");
                }
            }
            other => panic!("expected SAT model, got {other:?}"),
        }
    }
}
