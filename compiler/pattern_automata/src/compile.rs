//! Maranget-style pattern matrix compiler producing compact hash-consed decision DAGs,
//! exact arm reachability, guard-aware fallthrough, and concrete exhaustiveness witnesses.

use crate::bitvec::{
    self, AlignedBitCell, BitColumnRow, BitInterval, BitValue, first_uncovered_bit_value,
    partition_bit_intervals,
};
use crate::decision::{
    Binding, DagBuilder, DagNode, DecisionDag, LenInterval, NodeId, Place, PlaceStep, SwitchKind,
    coalesce_bit_cases, coalesce_char_cases, coalesce_int_cases, coalesce_len_cases,
};
use crate::pattern::{
    CharInterval, CompileBudget, CompileError, EnumType, ErrorRowType, ErrorTag, IntInterval,
    IntLit, IntType, MatchArm, Pattern, PatternMatrix, RangeBound, TypeDesc, VariantPayloadPattern,
    VariantPayloadType, VariantTag,
};
use crate::sexpr;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Payload for an enum counterexample witness.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WitnessPayload {
    Unit,
    Tuple(Vec<MissingWitness>),
    Record(BTreeMap<String, MissingWitness>),
}

impl fmt::Display for WitnessPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("unit"),
            Self::Tuple(elems) => {
                f.write_str("(tuple")?;
                for w in elems {
                    write!(f, " {w}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, w) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {w})")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Concrete counterexample value witnessing an unmatched pattern case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MissingWitness {
    Wild,
    Bool(bool),
    Char(char),
    Int(IntLit),
    OptionalNull,
    OptionalPresent(Box<MissingWitness>),
    ErrorOk(Box<MissingWitness>),
    ErrorErr {
        tag: ErrorTag,
        inner: Box<MissingWitness>,
    },
    ErrorOtherTag {
        inner: Box<MissingWitness>,
    },
    EnumVariant {
        tag: VariantTag,
        payload: WitnessPayload,
    },
    EnumOtherVariant,
    Tuple(Vec<MissingWitness>),
    Record(BTreeMap<String, MissingWitness>),
    List(Vec<MissingWitness>),
    EndlessList(Vec<MissingWitness>),
    BitVec(BitValue),
    Opaque,
}

impl MissingWitness {
    /// Convert a concrete witness into a runtime [`crate::eval::Value`] when all tags are known.
    ///
    /// Returns `None` for open-universe placeholders (`EnumOtherVariant`, `ErrorOtherTag`, `Opaque`, `Wild`).
    pub fn to_value(&self) -> Option<crate::eval::Value> {
        use crate::eval::{EnumValuePayload, Value};
        Some(match self {
            Self::Wild | Self::Opaque | Self::EnumOtherVariant | Self::ErrorOtherTag { .. } => {
                return None;
            }
            Self::Bool(b) => Value::Bool(*b),
            Self::Char(c) => Value::Char(*c),
            Self::Int(lit) => Value::Int(*lit),
            Self::OptionalNull => Value::OptionalNull,
            Self::OptionalPresent(inner) => Value::OptionalPresent(Box::new(inner.to_value()?)),
            Self::ErrorOk(inner) => Value::ErrorOk(Box::new(inner.to_value()?)),
            Self::ErrorErr { tag, inner } => Value::ErrorErr {
                tag: tag.clone(),
                inner: Box::new(inner.to_value()?),
            },
            Self::EnumVariant { tag, payload } => Value::Enum {
                tag: tag.clone(),
                payload: match payload {
                    WitnessPayload::Unit => EnumValuePayload::Unit,
                    WitnessPayload::Tuple(elems) => EnumValuePayload::Tuple(
                        elems
                            .iter()
                            .map(Self::to_value)
                            .collect::<Option<Vec<_>>>()?,
                    ),
                    WitnessPayload::Record(fields) => EnumValuePayload::Record(
                        fields
                            .iter()
                            .map(|(k, v)| Some((k.clone(), v.to_value()?)))
                            .collect::<Option<BTreeMap<_, _>>>()?,
                    ),
                },
            },
            Self::Tuple(elems) => Value::Tuple(
                elems
                    .iter()
                    .map(Self::to_value)
                    .collect::<Option<Vec<_>>>()?,
            ),
            Self::Record(fields) => Value::Record(
                fields
                    .iter()
                    .map(|(k, v)| Some((k.clone(), v.to_value()?)))
                    .collect::<Option<BTreeMap<_, _>>>()?,
            ),
            Self::List(elems) | Self::EndlessList(elems) => Value::List(
                elems
                    .iter()
                    .map(Self::to_value)
                    .collect::<Option<Vec<_>>>()?,
            ),
            Self::BitVec(bv) => Value::BitVec(bv.clone()),
        })
    }
}

impl fmt::Display for MissingWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wild => f.write_str("_"),
            Self::Bool(b) => write!(f, "(bool {b})"),
            Self::Char(c) => {
                f.write_str("(char ")?;
                sexpr::write_quoted_char(f, *c)?;
                f.write_str(")")
            }
            Self::Int(lit) => write!(f, "{lit}"),
            Self::OptionalNull => f.write_str("(null)"),
            Self::OptionalPresent(inner) => write!(f, "(present {inner})"),
            Self::ErrorOk(inner) => write!(f, "(ok {inner})"),
            Self::ErrorErr { tag, inner } => write!(f, "(err {tag} {inner})"),
            Self::ErrorOtherTag { inner } => write!(f, "(err-other {inner})"),
            Self::EnumVariant { tag, payload } => write!(f, "(variant {tag} {payload})"),
            Self::EnumOtherVariant => f.write_str("(enum-other)"),
            Self::Tuple(elems) => {
                f.write_str("(tuple")?;
                for w in elems {
                    write!(f, " {w}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, w) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {w})")?;
                }
                f.write_str(")")
            }
            Self::List(elems) => {
                f.write_str("(list")?;
                for w in elems {
                    write!(f, " {w}")?;
                }
                f.write_str(")")
            }
            Self::EndlessList(elems) => {
                f.write_str("(endless-list")?;
                for w in elems {
                    write!(f, " {w}")?;
                }
                f.write_str(")")
            }
            Self::BitVec(bv) => write!(f, "{bv}"),
            Self::Opaque => f.write_str("(opaque)"),
        }
    }
}

/// Compilation telemetry and sharing statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileStats {
    pub steps_used: usize,
    pub allocated_nodes: usize,
    pub final_nodes: usize,
    pub memo_hits: usize,
}

impl fmt::Display for CompileStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(compile-stats (steps {}) (allocated-nodes {}) (final-nodes {}) (memo-hits {}))",
            self.steps_used, self.allocated_nodes, self.final_nodes, self.memo_hits
        )
    }
}

/// Complete output from compiling a [`PatternMatrix`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileOutput {
    pub dag: DecisionDag,
    pub unreachable_arms: Vec<usize>,
    pub missing_witnesses: Vec<MissingWitness>,
    pub is_exhaustive: bool,
    pub stats: CompileStats,
}

impl fmt::Display for CompileOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(compile-output (exhaustive {}) (unreachable",
            self.is_exhaustive
        )?;
        for idx in &self.unreachable_arms {
            write!(f, " {idx}")?;
        }
        f.write_str(") (missing")?;
        for witness in &self.missing_witnesses {
            write!(f, " {witness}")?;
        }
        write!(f, ") {} {})", self.stats, self.dag)
    }
}

/// Compile a [`PatternMatrix`] with the default [`CompileBudget`].
pub fn compile(matrix: &PatternMatrix) -> Result<CompileOutput, CompileError> {
    compile_with_budget(matrix, CompileBudget::default())
}

/// Compile a [`PatternMatrix`] using an explicit [`CompileBudget`].
pub fn compile_with_budget(
    matrix: &PatternMatrix,
    budget: CompileBudget,
) -> Result<CompileOutput, CompileError> {
    matrix.validate()?;

    let mut rows = Vec::with_capacity(matrix.arms.len());
    for (arm_pos, arm) in matrix.arms.iter().enumerate() {
        rows.push(Row {
            arm_pos,
            arm_index: arm.arm_index,
            has_guard: arm.has_guard,
            pats: vec![ColPattern::Pattern(arm.pattern.clone())],
            bindings: Vec::new(),
        });
    }

    let initial_cols = vec![Column {
        place: Place::root(),
        domain: ColDomain::Typed(matrix.scrutinee_type.clone()),
    }];

    let mut compiler = Compiler {
        builder: DagBuilder::new(budget),
        memo: BTreeMap::new(),
        memo_hits: 0,
    };

    let root = compiler.compile_matrix(initial_cols, rows)?;
    let steps_used = compiler.builder.steps_used();
    let allocated_nodes = compiler.builder.allocated_nodes();
    let memo_hits = compiler.memo_hits;
    let dag = compiler.builder.finalize(root);

    let reachable_set = dag.reachable_arms();
    let unreachable_arms: Vec<usize> = matrix
        .arms
        .iter()
        .filter(|arm| !reachable_set.contains(&arm.arm_index))
        .map(|arm| arm.arm_index)
        .collect();

    let is_exhaustive = !dag.has_fail();
    let missing_witnesses = if is_exhaustive {
        Vec::new()
    } else {
        collect_missing_witnesses(&dag, &matrix.scrutinee_type, budget.max_witnesses)
    };

    let final_nodes = dag.node_count();
    Ok(CompileOutput {
        dag,
        unreachable_arms,
        missing_witnesses,
        is_exhaustive,
        stats: CompileStats {
            steps_used,
            allocated_nodes,
            final_nodes,
            memo_hits,
        },
    })
}

/// Convenience helper compiling `(scrutinee_type, arms)` directly.
pub fn compile_arms(
    scrutinee_type: TypeDesc,
    arms: Vec<MatchArm>,
    budget: CompileBudget,
) -> Result<CompileOutput, CompileError> {
    let matrix = PatternMatrix::new(scrutinee_type, arms)?;
    compile_with_budget(&matrix, budget)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ColDomain {
    Typed(TypeDesc),
    AlignedBitSlice { parent: Place, lo: u32, hi: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Column {
    place: Place,
    domain: ColDomain,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ColPattern {
    Pattern(Pattern),
    BitSlice(AlignedBitCell),
}

impl ColPattern {
    fn is_wild(&self) -> bool {
        match self {
            Self::Pattern(pat) => is_pattern_wild(pat),
            Self::BitSlice(cell) => matches!(cell, AlignedBitCell::Wild),
        }
    }
}

fn is_pattern_wild(pat: &Pattern) -> bool {
    match pat {
        Pattern::Wild => true,
        Pattern::Bind { .. } => false,
        Pattern::Or(alts) => {
            !alts.is_empty() && !pattern_has_any_bindings(pat) && alts.iter().any(is_pattern_wild)
        }
        _ => false,
    }
}

fn pattern_has_any_bindings(pat: &Pattern) -> bool {
    match pat {
        Pattern::Wild
        | Pattern::Bool(_)
        | Pattern::Char(_)
        | Pattern::CharRange { .. }
        | Pattern::Int(_)
        | Pattern::IntRange { .. }
        | Pattern::OptionalNull => false,
        Pattern::Bind { .. } => true,
        Pattern::OptionalPresent(inner)
        | Pattern::ErrorOk(inner)
        | Pattern::ErrorErr { inner, .. } => pattern_has_any_bindings(inner),
        Pattern::EnumVariant { payload, .. } => match payload {
            VariantPayloadPattern::Unit => false,
            VariantPayloadPattern::Tuple(pats) => pats.iter().any(pattern_has_any_bindings),
            VariantPayloadPattern::Record(fields) => fields.values().any(pattern_has_any_bindings),
        },
        Pattern::Tuple(pats) | Pattern::Or(pats) => pats.iter().any(pattern_has_any_bindings),
        Pattern::Record(fields) => fields.values().any(pattern_has_any_bindings),
        Pattern::List { prefix, rest } => {
            prefix.iter().any(pattern_has_any_bindings)
                || rest.as_deref().is_some_and(pattern_has_any_bindings)
        }
        Pattern::BitVec(bv) => bv.segments.iter().any(|seg| match seg {
            crate::pattern::BitSegmentPattern::Fixed { .. } => false,
            crate::pattern::BitSegmentPattern::Typed { pattern, .. } => {
                pattern_has_any_bindings(pattern)
            }
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    arm_pos: usize,
    arm_index: usize,
    has_guard: bool,
    pats: Vec<ColPattern>,
    bindings: Vec<Binding>,
}

type MatrixKey = (Vec<Column>, Vec<Row>);

struct Compiler {
    builder: DagBuilder,
    memo: BTreeMap<MatrixKey, NodeId>,
    memo_hits: usize,
}

impl Compiler {
    fn compile_matrix(
        &mut self,
        cols: Vec<Column>,
        mut rows: Vec<Row>,
    ) -> Result<NodeId, CompileError> {
        self.builder.charge_step()?;

        if rows.is_empty() {
            // An uninhabited column (such as a zero-variant closed enum or product containing one)
            // cannot produce a runtime value reaching this point.
            if let Some(empty_place) = cols.iter().find_map(|c| match &c.domain {
                ColDomain::Typed(ty) => find_uninhabited_place(&c.place, ty),
                ColDomain::AlignedBitSlice { .. } => None,
            }) {
                return self.builder.switch(
                    empty_place,
                    SwitchKind::Enum {
                        cases: BTreeMap::new(),
                        default: None,
                    },
                );
            }
            return self.builder.fail();
        }

        // Peel outer bindings and normalize trivial wildcards across all rows/columns.
        normalize_rows(&cols, &mut rows);

        // Prune unreachable rows beneath an unguarded all-wild row, and drop duplicate
        // sub-rows belonging to an arm that already matched all-wild structurally.
        prune_subsumed_rows(&mut rows);

        // Drop columns that are `Wild` across every surviving row (after peeling any bindings).
        let cols = drop_all_wild_columns(cols, &mut rows);

        let key = (cols.clone(), rows.clone());
        if let Some(&cached) = self.memo.get(&key) {
            self.memo_hits += 1;
            return Ok(cached);
        }

        // Flush settled bindings into `DagNode::Bind` frames when a row is the unique active
        // sub-row of its `arm_index`, `arm_index` appears only once across `rows`, and further
        // structural tests remain. This maximizes DAG memoization sharing across `Or` branches
        // that bind identical names from different scrutinee sub-places.
        if rows[0].pats.iter().any(|p| !p.is_wild()) {
            let mut arm_index_counts: BTreeMap<usize, usize> = BTreeMap::new();
            for row in &rows {
                *arm_index_counts.entry(row.arm_index).or_insert(0) += 1;
            }
            let mut flushed = Vec::new();
            for row in &mut rows {
                if !row.bindings.is_empty() && arm_index_counts[&row.arm_index] == 1 {
                    let bindings = std::mem::take(&mut row.bindings);
                    flushed.push((row.arm_index, bindings));
                }
            }
            if !flushed.is_empty() {
                let mut inner = self.compile_matrix(cols, rows)?;
                for (arm_index, bindings) in flushed.into_iter().rev() {
                    inner = self.builder.bind(arm_index, bindings, inner)?;
                }
                self.memo.insert(key, inner);
                return Ok(inner);
            }
        }

        // Case 2: First row has all irrefutable patterns.
        if rows[0].pats.iter().all(ColPattern::is_wild) {
            let first = &rows[0];
            let node = if !first.has_guard {
                self.builder.leaf(first.arm_index, first.bindings.clone())?
            } else {
                let on_pass = self.builder.leaf(first.arm_index, first.bindings.clone())?;
                // Failing an arm's outer guard must skip ALL remaining `Or` sub-rows of that arm.
                let rest: Vec<Row> = rows[1..]
                    .iter()
                    .filter(|r| r.arm_pos != first.arm_pos)
                    .cloned()
                    .collect();
                let on_fail = self.compile_matrix(cols, rest)?;
                self.builder.guard_check(
                    first.arm_index,
                    first.bindings.clone(),
                    on_pass,
                    on_fail,
                )?
            };
            self.memo.insert(key, node);
            return Ok(node);
        }

        // Case 3: Select discriminating column using Maranget's necessity + small-branching heuristic.
        let col_idx = select_column(&cols, &rows);
        let node = self.specialize_column(cols, rows, col_idx)?;
        self.memo.insert(key, node);
        Ok(node)
    }

    fn specialize_column(
        &mut self,
        cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
    ) -> Result<NodeId, CompileError> {
        let col = cols[col_idx].clone();
        match &col.domain {
            ColDomain::AlignedBitSlice { parent, lo, hi } => {
                self.specialize_aligned_bitslice(cols, rows, col_idx, parent.clone(), *lo, *hi)
            }
            ColDomain::Typed(ty) => match ty.clone() {
                TypeDesc::Tuple(elem_tys) => {
                    self.specialize_tuple(cols, rows, col_idx, &col.place, &elem_tys)
                }
                TypeDesc::Record(field_tys) => {
                    self.specialize_record(cols, rows, col_idx, &col.place, &field_tys)
                }
                TypeDesc::Bool => self.specialize_bool(cols, rows, col_idx, col.place),
                TypeDesc::Optional(inner_ty) => {
                    self.specialize_optional(cols, rows, col_idx, col.place, *inner_ty)
                }
                TypeDesc::ErrorRow(row_ty) => {
                    self.specialize_error_row(cols, rows, col_idx, col.place, &row_ty)
                }
                TypeDesc::Enum(enum_ty) => {
                    self.specialize_enum(cols, rows, col_idx, col.place, &enum_ty)
                }
                TypeDesc::Int(int_ty) => {
                    self.specialize_int(cols, rows, col_idx, col.place, int_ty)
                }
                TypeDesc::Char => self.specialize_char(cols, rows, col_idx, col.place),
                TypeDesc::List(elem_ty) => {
                    self.specialize_list(cols, rows, col_idx, col.place, *elem_ty)
                }
                TypeDesc::EndlessList(elem_ty) => {
                    self.specialize_endless_list(cols, rows, col_idx, col.place, *elem_ty)
                }
                TypeDesc::BitVec(width) => {
                    self.specialize_bitvec(cols, rows, col_idx, col.place, width)
                }
                TypeDesc::Any => self.specialize_any(cols, rows, col_idx),
            },
        }
    }

    fn specialize_tuple(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: &Place,
        elem_tys: &[TypeDesc],
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);
        let new_cols: Vec<Column> = elem_tys
            .iter()
            .enumerate()
            .map(|(i, ty)| Column {
                place: place.tuple_field(i),
                domain: ColDomain::Typed(ty.clone()),
            })
            .collect();
        cols.splice(col_idx..=col_idx, new_cols);

        let mut next_rows = Vec::with_capacity(expanded.len());
        for mut row in expanded {
            let pat = match row.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            let replacement: Vec<ColPattern> = match pat {
                Pattern::Wild => vec![ColPattern::Pattern(Pattern::Wild); elem_tys.len()],
                Pattern::Tuple(sub_pats) => sub_pats.into_iter().map(ColPattern::Pattern).collect(),
                other => {
                    return Err(CompileError::TypeMismatch {
                        expected: "tuple".to_string(),
                        found: other.to_string(),
                    });
                }
            };
            row.pats.splice(col_idx..col_idx, replacement);
            next_rows.push(row);
        }

        self.compile_matrix(cols, next_rows)
    }

    fn specialize_record(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: &Place,
        field_tys: &BTreeMap<String, TypeDesc>,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);
        let field_names: Vec<String> = field_tys.keys().cloned().collect();
        let new_cols: Vec<Column> = field_tys
            .iter()
            .map(|(name, ty)| Column {
                place: place.record_field(name),
                domain: ColDomain::Typed(ty.clone()),
            })
            .collect();
        cols.splice(col_idx..=col_idx, new_cols);

        let mut next_rows = Vec::with_capacity(expanded.len());
        for mut row in expanded {
            let pat = match row.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            let replacement: Vec<ColPattern> = match pat {
                Pattern::Wild => vec![ColPattern::Pattern(Pattern::Wild); field_names.len()],
                Pattern::Record(mut fields) => field_names
                    .iter()
                    .map(|name| ColPattern::Pattern(fields.remove(name).unwrap_or(Pattern::Wild)))
                    .collect(),
                other => {
                    return Err(CompileError::TypeMismatch {
                        expected: "record".to_string(),
                        found: other.to_string(),
                    });
                }
            };
            row.pats.splice(col_idx..col_idx, replacement);
            next_rows.push(row);
        }

        self.compile_matrix(cols, next_rows)
    }

    fn specialize_bool(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);
        cols.remove(col_idx);

        let mut false_rows = Vec::new();
        let mut true_rows = Vec::new();
        for mut row in expanded {
            let pat = match row.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            match pat {
                Pattern::Wild => {
                    false_rows.push(row.clone());
                    true_rows.push(row);
                }
                Pattern::Bool(false) => false_rows.push(row),
                Pattern::Bool(true) => true_rows.push(row),
                other => {
                    return Err(CompileError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: other.to_string(),
                    });
                }
            }
        }

        let on_false = self.compile_matrix(cols.clone(), false_rows)?;
        let on_true = self.compile_matrix(cols, true_rows)?;
        self.builder
            .switch(place, SwitchKind::Bool { on_false, on_true })
    }

    fn specialize_optional(
        &mut self,
        cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        inner_ty: TypeDesc,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);

        let mut null_cols = cols.clone();
        null_cols.remove(col_idx);
        let mut present_cols = cols;
        present_cols[col_idx] = Column {
            place: place.optional_inner(),
            domain: ColDomain::Typed(inner_ty),
        };

        let mut null_rows = Vec::new();
        let mut present_rows = Vec::new();

        for row in expanded {
            let mut r_clone = row.clone();
            let pat = match r_clone.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            match pat {
                Pattern::Wild => {
                    null_rows.push(r_clone);
                    let mut p_row = row;
                    p_row.pats[col_idx] = ColPattern::Pattern(Pattern::Wild);
                    present_rows.push(p_row);
                }
                Pattern::OptionalNull => null_rows.push(r_clone),
                Pattern::OptionalPresent(inner) => {
                    let mut p_row = row;
                    p_row.pats[col_idx] = ColPattern::Pattern(*inner);
                    present_rows.push(p_row);
                }
                other => {
                    return Err(CompileError::TypeMismatch {
                        expected: "optional".to_string(),
                        found: other.to_string(),
                    });
                }
            }
        }

        let on_null = self.compile_matrix(null_cols, null_rows)?;
        let on_present = self.compile_matrix(present_cols, present_rows)?;
        self.builder.switch(
            place,
            SwitchKind::Optional {
                on_null,
                on_present,
            },
        )
    }

    fn specialize_error_row(
        &mut self,
        cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        row_ty: &ErrorRowType,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);

        let mut all_tags: BTreeMap<ErrorTag, TypeDesc> = row_ty.error_tags.clone();
        for row in &expanded {
            if let ColPattern::Pattern(Pattern::ErrorErr { tag: Some(t), .. }) = &row.pats[col_idx]
            {
                if !all_tags.contains_key(t) {
                    if row_ty.is_open {
                        all_tags.insert(t.clone(), TypeDesc::Any);
                    } else {
                        return Err(CompileError::UnknownErrorTag { tag: t.clone() });
                    }
                }
            }
        }

        let mut ok_cols = cols.clone();
        ok_cols[col_idx] = Column {
            place: place.error_ok_inner(),
            domain: ColDomain::Typed((*row_ty.ok_type).clone()),
        };
        let mut ok_rows = Vec::new();
        for mut row in expanded.clone() {
            match std::mem::replace(&mut row.pats[col_idx], ColPattern::Pattern(Pattern::Wild)) {
                ColPattern::Pattern(Pattern::Wild) => ok_rows.push(row),
                ColPattern::Pattern(Pattern::ErrorOk(inner)) => {
                    row.pats[col_idx] = ColPattern::Pattern(*inner);
                    ok_rows.push(row);
                }
                ColPattern::Pattern(Pattern::ErrorErr { .. }) => {}
                ColPattern::Pattern(other) => {
                    return Err(CompileError::TypeMismatch {
                        expected: "error-row".to_string(),
                        found: other.to_string(),
                    });
                }
                ColPattern::BitSlice(_) => unreachable!(),
            }
        }
        let on_ok = self.compile_matrix(ok_cols, ok_rows)?;

        let mut cases = BTreeMap::new();
        for (tag, payload_ty) in &all_tags {
            let mut tag_cols = cols.clone();
            tag_cols[col_idx] = Column {
                place: place.error_err_inner(),
                domain: ColDomain::Typed(payload_ty.clone()),
            };
            let mut tag_rows = Vec::new();
            for mut row in expanded.clone() {
                match std::mem::replace(&mut row.pats[col_idx], ColPattern::Pattern(Pattern::Wild))
                {
                    ColPattern::Pattern(Pattern::Wild) => tag_rows.push(row),
                    ColPattern::Pattern(Pattern::ErrorErr {
                        tag: Some(t),
                        inner,
                    }) if t == *tag => {
                        row.pats[col_idx] = ColPattern::Pattern(*inner);
                        tag_rows.push(row);
                    }
                    ColPattern::Pattern(Pattern::ErrorErr { tag: None, inner }) => {
                        row.pats[col_idx] = ColPattern::Pattern(*inner);
                        tag_rows.push(row);
                    }
                    ColPattern::Pattern(Pattern::ErrorOk(_) | Pattern::ErrorErr { .. }) => {}
                    ColPattern::Pattern(other) => {
                        return Err(CompileError::TypeMismatch {
                            expected: "error-row".to_string(),
                            found: other.to_string(),
                        });
                    }
                    ColPattern::BitSlice(_) => unreachable!(),
                }
            }
            let target = self.compile_matrix(tag_cols, tag_rows)?;
            cases.insert(tag.clone(), target);
        }

        let default_err = if row_ty.is_open {
            let mut def_cols = cols;
            def_cols[col_idx] = Column {
                place: place.error_err_inner(),
                domain: ColDomain::Typed(TypeDesc::Any),
            };
            let mut def_rows = Vec::new();
            for mut row in expanded {
                match std::mem::replace(&mut row.pats[col_idx], ColPattern::Pattern(Pattern::Wild))
                {
                    ColPattern::Pattern(Pattern::Wild) => def_rows.push(row),
                    ColPattern::Pattern(Pattern::ErrorErr { tag: None, inner }) => {
                        row.pats[col_idx] = ColPattern::Pattern(*inner);
                        def_rows.push(row);
                    }
                    ColPattern::Pattern(Pattern::ErrorOk(_) | Pattern::ErrorErr { .. }) => {}
                    ColPattern::Pattern(other) => {
                        return Err(CompileError::TypeMismatch {
                            expected: "error-row".to_string(),
                            found: other.to_string(),
                        });
                    }
                    ColPattern::BitSlice(_) => unreachable!(),
                }
            }
            Some(self.compile_matrix(def_cols, def_rows)?)
        } else {
            None
        };

        self.builder.switch(
            place,
            SwitchKind::Error {
                on_ok,
                cases,
                default_err,
            },
        )
    }

    fn specialize_enum(
        &mut self,
        cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        enum_ty: &EnumType,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);

        let mut all_variants: BTreeMap<VariantTag, VariantPayloadType> = enum_ty
            .variants
            .iter()
            .map(|(k, v)| (k.clone(), v.payload.clone()))
            .collect();

        for row in &expanded {
            if let ColPattern::Pattern(Pattern::EnumVariant { tag, payload }) = &row.pats[col_idx]
                && !all_variants.contains_key(tag)
            {
                if !enum_ty.is_open {
                    return Err(CompileError::UnknownVariant {
                        enum_name: enum_ty.name.clone(),
                        tag: tag.clone(),
                    });
                }
                let inferred = match payload {
                    VariantPayloadPattern::Unit => VariantPayloadType::Unit,
                    VariantPayloadPattern::Tuple(ps) => {
                        VariantPayloadType::Tuple(vec![TypeDesc::Any; ps.len()])
                    }
                    VariantPayloadPattern::Record(fs) => VariantPayloadType::Record(
                        fs.keys().map(|k| (k.clone(), TypeDesc::Any)).collect(),
                    ),
                };
                all_variants.insert(tag.clone(), inferred);
            }
        }

        let mut cases = BTreeMap::new();
        for (tag, payload_ty) in &all_variants {
            let mut sub_cols = cols.clone();
            let (payload_cols, field_names): (Vec<Column>, Vec<String>) = match payload_ty {
                VariantPayloadType::Unit => (Vec::new(), Vec::new()),
                VariantPayloadType::Tuple(tys) => (
                    tys.iter()
                        .enumerate()
                        .map(|(i, t)| Column {
                            place: place.variant_tuple_field(tag.clone(), i),
                            domain: ColDomain::Typed(t.clone()),
                        })
                        .collect(),
                    Vec::new(),
                ),
                VariantPayloadType::Record(fields) => (
                    fields
                        .iter()
                        .map(|(name, t)| Column {
                            place: place.variant_record_field(tag.clone(), name),
                            domain: ColDomain::Typed(t.clone()),
                        })
                        .collect(),
                    fields.keys().cloned().collect(),
                ),
            };
            let arity = payload_cols.len();
            sub_cols.splice(col_idx..=col_idx, payload_cols);

            let mut sub_rows = Vec::new();
            for mut row in expanded.clone() {
                let pat = match row.pats.remove(col_idx) {
                    ColPattern::Pattern(p) => p,
                    ColPattern::BitSlice(_) => unreachable!(),
                };
                match pat {
                    Pattern::Wild => {
                        row.pats.splice(
                            col_idx..col_idx,
                            vec![ColPattern::Pattern(Pattern::Wild); arity],
                        );
                        sub_rows.push(row);
                    }
                    Pattern::EnumVariant {
                        tag: ptag,
                        payload: ppayload,
                    } => {
                        if ptag != *tag {
                            continue;
                        }
                        let replacement: Vec<ColPattern> = match (ppayload, payload_ty) {
                            (VariantPayloadPattern::Unit, VariantPayloadType::Unit) => Vec::new(),
                            (VariantPayloadPattern::Tuple(ps), VariantPayloadType::Tuple(_)) => {
                                ps.into_iter().map(ColPattern::Pattern).collect()
                            }
                            (
                                VariantPayloadPattern::Record(mut fs),
                                VariantPayloadType::Record(_),
                            ) => field_names
                                .iter()
                                .map(|name| {
                                    ColPattern::Pattern(fs.remove(name).unwrap_or(Pattern::Wild))
                                })
                                .collect(),
                            (found_p, expected_t) => {
                                return Err(CompileError::TypeMismatch {
                                    expected: expected_t.to_string(),
                                    found: found_p.to_string(),
                                });
                            }
                        };
                        row.pats.splice(col_idx..col_idx, replacement);
                        sub_rows.push(row);
                    }
                    other => {
                        return Err(CompileError::TypeMismatch {
                            expected: enum_ty.to_string(),
                            found: other.to_string(),
                        });
                    }
                }
            }

            let child = self.compile_matrix(sub_cols, sub_rows)?;
            cases.insert(tag.clone(), child);
        }

        let default = if enum_ty.is_open {
            let mut def_cols = cols;
            def_cols.remove(col_idx);
            let mut def_rows = Vec::new();
            for mut row in expanded {
                match row.pats.remove(col_idx) {
                    ColPattern::Pattern(Pattern::Wild) => def_rows.push(row),
                    ColPattern::Pattern(Pattern::EnumVariant { .. }) => {}
                    ColPattern::Pattern(other) => {
                        return Err(CompileError::TypeMismatch {
                            expected: enum_ty.to_string(),
                            found: other.to_string(),
                        });
                    }
                    ColPattern::BitSlice(_) => unreachable!(),
                }
            }
            Some(self.compile_matrix(def_cols, def_rows)?)
        } else {
            None
        };

        self.builder
            .switch(place, SwitchKind::Enum { cases, default })
    }

    fn specialize_int(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        int_ty: IntType,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col_for_intervals(&cols, rows, col_idx);
        cols.remove(col_idx);
        let domain = IntInterval::domain_for(int_ty);

        let mut row_specs: Vec<(Row, Option<Vec<IntInterval>>)> =
            Vec::with_capacity(expanded.len());
        let mut all_intervals = Vec::new();

        for mut row in expanded {
            let pat = match row.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            match extract_int_intervals(&pat, domain)? {
                None => row_specs.push((row, None)),
                Some(ivs) => {
                    all_intervals.extend(ivs.iter().copied());
                    row_specs.push((row, Some(ivs)));
                }
            }
        }

        let partition = IntInterval::partition(domain, &all_intervals);
        let mut explicit_cases: Vec<(IntInterval, NodeId)> = Vec::new();
        let mut gap_target: Option<NodeId> = None;

        for elem_iv in partition {
            let mut matched_explicit = false;
            let mut sub_rows = Vec::new();
            for (row, opt_ivs) in &row_specs {
                match opt_ivs {
                    None => sub_rows.push(row.clone()),
                    Some(ivs) => {
                        if ivs.iter().any(|r| r.lo <= elem_iv.lo && r.hi >= elem_iv.hi) {
                            matched_explicit = true;
                            sub_rows.push(row.clone());
                        }
                    }
                }
            }
            let target = self.compile_matrix(cols.clone(), sub_rows)?;
            if matched_explicit {
                explicit_cases.push((elem_iv, target));
            } else {
                // All gap intervals share the exact same `Wild`-only row subset and thus
                // the same `NodeId`.
                debug_assert!(gap_target.is_none() || gap_target == Some(target));
                gap_target = Some(target);
            }
        }

        let cases = coalesce_int_cases(explicit_cases);
        self.builder.switch(
            place,
            SwitchKind::Int {
                cases,
                default: gap_target,
            },
        )
    }

    fn specialize_char(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col_for_intervals(&cols, rows, col_idx);
        cols.remove(col_idx);

        let mut row_specs: Vec<(Row, Option<Vec<CharInterval>>)> =
            Vec::with_capacity(expanded.len());
        let mut all_intervals = Vec::new();

        for mut row in expanded {
            let pat = match row.pats.remove(col_idx) {
                ColPattern::Pattern(p) => p,
                ColPattern::BitSlice(_) => unreachable!(),
            };
            match extract_char_intervals(&pat)? {
                None => row_specs.push((row, None)),
                Some(ivs) => {
                    all_intervals.extend(ivs.iter().copied());
                    row_specs.push((row, Some(ivs)));
                }
            }
        }

        let partition = CharInterval::partition_all(&all_intervals);
        let mut explicit_cases: Vec<(CharInterval, NodeId)> = Vec::new();
        let mut gap_target: Option<NodeId> = None;

        for elem_iv in partition {
            let mut matched_explicit = false;
            let mut sub_rows = Vec::new();
            for (row, opt_ivs) in &row_specs {
                match opt_ivs {
                    None => sub_rows.push(row.clone()),
                    Some(ivs) => {
                        if ivs.iter().any(|r| r.lo <= elem_iv.lo && r.hi >= elem_iv.hi) {
                            matched_explicit = true;
                            sub_rows.push(row.clone());
                        }
                    }
                }
            }
            let target = self.compile_matrix(cols.clone(), sub_rows)?;
            if matched_explicit {
                explicit_cases.push((elem_iv, target));
            } else {
                debug_assert!(gap_target.is_none() || gap_target == Some(target));
                gap_target = Some(target);
            }
        }

        let cases = coalesce_char_cases(explicit_cases);
        self.builder.switch(
            place,
            SwitchKind::Char {
                cases,
                default: gap_target,
            },
        )
    }

    fn specialize_list(
        &mut self,
        cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        elem_ty: TypeDesc,
    ) -> Result<NodeId, CompileError> {
        // Expand top-level `Or` and peel outer `Bind`s before normalizing list patterns so
        // alternatives like `([1] as xs) or ([2] as xs)` preserve bindings across branches.
        let expanded = expand_or_at_col(&cols, rows, col_idx);

        let mut normalized_rows = Vec::with_capacity(expanded.len());
        for row in expanded {
            normalize_list_row(row, col_idx, &place, &mut normalized_rows)?;
        }

        #[derive(Clone)]
        enum ListShape {
            Wild,
            Exact {
                prefix: Vec<Pattern>,
                rest_binds: Vec<(String, usize)>,
            },
            AtLeast {
                prefix: Vec<Pattern>,
                rest_binds: Vec<(String, usize)>,
            },
        }

        let mut breakpoints = BTreeSet::new();
        breakpoints.insert(0usize);
        let mut shapes: Vec<(Row, ListShape)> = Vec::with_capacity(normalized_rows.len());

        for norm in normalized_rows {
            let row = norm.row;
            let rest_binds = norm.rest_binds;
            let shape = match &row.pats[col_idx] {
                ColPattern::Pattern(Pattern::Wild) => ListShape::Wild,
                ColPattern::Pattern(Pattern::List { prefix, rest }) => {
                    if let Some(rest_pat) = rest {
                        debug_assert!(matches!(rest_pat.as_ref(), Pattern::Wild));
                        breakpoints.insert(prefix.len());
                        ListShape::AtLeast {
                            prefix: prefix.clone(),
                            rest_binds,
                        }
                    } else {
                        breakpoints.insert(prefix.len());
                        breakpoints.insert(prefix.len() + 1);
                        ListShape::Exact {
                            prefix: prefix.clone(),
                            rest_binds,
                        }
                    }
                }
                ColPattern::Pattern(other) => {
                    return Err(CompileError::TypeMismatch {
                        expected: "list".to_string(),
                        found: other.to_string(),
                    });
                }
                ColPattern::BitSlice(_) => unreachable!(),
            };
            shapes.push((row, shape));
        }

        let ordered_pts: Vec<usize> = breakpoints.into_iter().collect();
        let mut len_cases: Vec<(LenInterval, NodeId)> = Vec::new();

        for (idx, &start) in ordered_pts.iter().enumerate() {
            let max_inclusive = if idx + 1 < ordered_pts.len() {
                Some(ordered_pts[idx + 1] - 1)
            } else {
                None
            };
            let len_iv = LenInterval {
                min: start,
                max_inclusive,
            };

            let is_exact = max_inclusive == Some(start);
            let mut active_rows = Vec::new();
            let mut required_elems = if is_exact { start } else { 0usize };

            for (row, shape) in &shapes {
                match shape {
                    ListShape::Wild => active_rows.push((row.clone(), shape)),
                    ListShape::Exact { prefix, .. } if is_exact && prefix.len() == start => {
                        active_rows.push((row.clone(), shape));
                    }
                    ListShape::AtLeast { prefix, .. } if prefix.len() <= start => {
                        required_elems = required_elems.max(prefix.len());
                        active_rows.push((row.clone(), shape));
                    }
                    _ => {}
                }
            }

            let mut sub_cols = cols.clone();
            let elem_cols: Vec<Column> = (0..required_elems)
                .map(|i| Column {
                    place: place.list_elem(i),
                    domain: ColDomain::Typed(elem_ty.clone()),
                })
                .collect();
            sub_cols.splice(col_idx..=col_idx, elem_cols);

            let mut sub_rows = Vec::with_capacity(active_rows.len());
            for (mut row, shape) in active_rows {
                row.pats.remove(col_idx);
                let replacement: Vec<ColPattern> = match shape {
                    ListShape::Wild => {
                        vec![ColPattern::Pattern(Pattern::Wild); required_elems]
                    }
                    ListShape::Exact { prefix, rest_binds } => {
                        for (bind_name, offset) in rest_binds {
                            row.bindings.push(Binding {
                                name: bind_name.clone(),
                                place: place.list_rest(*offset),
                            });
                        }
                        prefix.iter().cloned().map(ColPattern::Pattern).collect()
                    }
                    ListShape::AtLeast { prefix, rest_binds } => {
                        for (bind_name, offset) in rest_binds {
                            row.bindings.push(Binding {
                                name: bind_name.clone(),
                                place: place.list_rest(*offset),
                            });
                        }
                        let mut pats: Vec<ColPattern> =
                            prefix.iter().cloned().map(ColPattern::Pattern).collect();
                        while pats.len() < required_elems {
                            pats.push(ColPattern::Pattern(Pattern::Wild));
                        }
                        pats
                    }
                };
                row.pats.splice(col_idx..col_idx, replacement);
                sub_rows.push(row);
            }

            let target = self.compile_matrix(sub_cols, sub_rows)?;
            len_cases.push((len_iv, target));
        }

        let cases = coalesce_len_cases(len_cases);
        self.builder.switch(
            place,
            SwitchKind::ListLen {
                cases,
                default: None,
            },
        )
    }

    fn specialize_endless_list(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        elem_ty: TypeDesc,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);

        let mut normalized_rows = Vec::with_capacity(expanded.len());
        for row in expanded {
            normalize_list_row(row, col_idx, &place, &mut normalized_rows)?;
        }

        enum EndlessShape {
            Wild,
            AtLeast {
                prefix: Vec<Pattern>,
                rest_binds: Vec<(String, usize)>,
            },
        }

        let mut required_elems = 0usize;
        let mut shapes: Vec<(Row, EndlessShape)> = Vec::with_capacity(normalized_rows.len());

        for norm in normalized_rows {
            let row = norm.row;
            let rest_binds = norm.rest_binds;
            let shape = match &row.pats[col_idx] {
                ColPattern::Pattern(Pattern::Wild) => EndlessShape::Wild,
                ColPattern::Pattern(Pattern::List { prefix, rest }) => {
                    let Some(rest_pat) = rest else {
                        return Err(CompileError::TypeMismatch {
                            expected: "endless list pattern with trailing rest".to_string(),
                            found: "fixed-length list pattern".to_string(),
                        });
                    };
                    debug_assert!(matches!(rest_pat.as_ref(), Pattern::Wild));
                    required_elems = required_elems.max(prefix.len());
                    EndlessShape::AtLeast {
                        prefix: prefix.clone(),
                        rest_binds,
                    }
                }
                ColPattern::Pattern(other) => {
                    return Err(CompileError::TypeMismatch {
                        expected: "endless list".to_string(),
                        found: other.to_string(),
                    });
                }
                ColPattern::BitSlice(_) => unreachable!(),
            };
            shapes.push((row, shape));
        }

        let elem_cols: Vec<Column> = (0..required_elems)
            .map(|i| Column {
                place: place.list_elem(i),
                domain: ColDomain::Typed(elem_ty.clone()),
            })
            .collect();
        cols.splice(col_idx..=col_idx, elem_cols);

        let mut next_rows = Vec::with_capacity(shapes.len());
        for (mut row, shape) in shapes {
            row.pats.remove(col_idx);
            let replacement: Vec<ColPattern> = match shape {
                EndlessShape::Wild => {
                    vec![ColPattern::Pattern(Pattern::Wild); required_elems]
                }
                EndlessShape::AtLeast { prefix, rest_binds } => {
                    for (bind_name, offset) in rest_binds {
                        row.bindings.push(Binding {
                            name: bind_name,
                            place: place.list_rest(offset),
                        });
                    }
                    let mut pats: Vec<ColPattern> =
                        prefix.into_iter().map(ColPattern::Pattern).collect();
                    while pats.len() < required_elems {
                        pats.push(ColPattern::Pattern(Pattern::Wild));
                    }
                    pats
                }
            };
            row.pats.splice(col_idx..col_idx, replacement);
            next_rows.push(row);
        }

        self.compile_matrix(cols, next_rows)
    }

    fn specialize_bitvec(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        place: Place,
        width: u32,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);
        let mut bit_rows = Vec::with_capacity(expanded.len());

        for row in &expanded {
            match &row.pats[col_idx] {
                ColPattern::Pattern(Pattern::Wild) => bit_rows.push(BitColumnRow::Wild),
                ColPattern::Pattern(Pattern::BitVec(bv)) => {
                    bit_rows.push(BitColumnRow::BitVec(bv.clone()))
                }
                ColPattern::Pattern(other) => {
                    return Err(CompileError::TypeMismatch {
                        expected: format!("BitVec<{width}>"),
                        found: other.to_string(),
                    });
                }
                ColPattern::BitSlice(_) => unreachable!(),
            }
        }

        let aligned = bitvec::align_bitvec_column(width, &place, &bit_rows)?;

        let new_cols: Vec<Column> = aligned
            .intervals
            .iter()
            .map(|iv| Column {
                place: place.bit_slice(iv.lo, iv.hi, iv.ty.clone()),
                domain: ColDomain::AlignedBitSlice {
                    parent: place.clone(),
                    lo: iv.lo,
                    hi: iv.hi,
                },
            })
            .collect();
        cols.splice(col_idx..=col_idx, new_cols);

        let mut next_rows = Vec::with_capacity(aligned.rows.len());
        for a_row in aligned.rows {
            let mut row = expanded[a_row.source_row_idx].clone();
            row.bindings.extend(a_row.bindings);
            row.pats.remove(col_idx);
            let replacement: Vec<ColPattern> =
                a_row.cells.into_iter().map(ColPattern::BitSlice).collect();
            row.pats.splice(col_idx..col_idx, replacement);
            next_rows.push(row);
        }

        self.compile_matrix(cols, next_rows)
    }

    fn specialize_aligned_bitslice(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
        parent: Place,
        lo: u32,
        hi: u32,
    ) -> Result<NodeId, CompileError> {
        // Check if any row contains an unsplit opaque typed subpattern on this interval.
        if let Some(opaque_ty) = rows.iter().find_map(|r| match &r.pats[col_idx] {
            ColPattern::BitSlice(AlignedBitCell::Opaque { ty, .. }) => Some(ty.clone()),
            _ => None,
        }) {
            cols[col_idx] = Column {
                place: parent.bit_slice(lo, hi, opaque_ty.clone()),
                domain: ColDomain::Typed(opaque_ty),
            };
            let converted_rows: Vec<Row> = rows
                .into_iter()
                .map(|mut r| {
                    let next_pat = match &r.pats[col_idx] {
                        ColPattern::BitSlice(AlignedBitCell::Wild) => Pattern::Wild,
                        ColPattern::BitSlice(AlignedBitCell::Opaque { pattern, .. }) => {
                            pattern.clone()
                        }
                        ColPattern::BitSlice(AlignedBitCell::BitRanges(_)) => {
                            unreachable!("opaque intervals do not mix with split ranges")
                        }
                        ColPattern::Pattern(p) => p.clone(),
                    };
                    r.pats[col_idx] = ColPattern::Pattern(next_pat);
                    r
                })
                .collect();
            return self.compile_matrix(cols, converted_rows);
        }

        cols.remove(col_idx);
        let slice_width = hi - lo;

        let mut all_intervals = Vec::new();
        let mut row_specs: Vec<(Row, Option<Vec<BitInterval>>)> = Vec::with_capacity(rows.len());

        for mut row in rows {
            match row.pats.remove(col_idx) {
                ColPattern::BitSlice(AlignedBitCell::Wild) => row_specs.push((row, None)),
                ColPattern::BitSlice(AlignedBitCell::BitRanges(ivs)) => {
                    all_intervals.extend(ivs.iter().cloned());
                    row_specs.push((row, Some(ivs)));
                }
                ColPattern::BitSlice(AlignedBitCell::Opaque { .. }) | ColPattern::Pattern(_) => {
                    unreachable!()
                }
            }
        }

        let partition = partition_bit_intervals(slice_width, &all_intervals);
        let mut explicit_cases: Vec<(BitInterval, NodeId)> = Vec::new();
        let mut gap_target: Option<NodeId> = None;

        for elem_iv in partition {
            let mut matched_explicit = false;
            let mut sub_rows = Vec::new();
            for (row, opt_ivs) in &row_specs {
                match opt_ivs {
                    None => sub_rows.push(row.clone()),
                    Some(ivs) => {
                        if ivs.iter().any(|r| r.lo <= elem_iv.lo && r.hi >= elem_iv.hi) {
                            matched_explicit = true;
                            sub_rows.push(row.clone());
                        }
                    }
                }
            }
            let target = self.compile_matrix(cols.clone(), sub_rows)?;
            if matched_explicit {
                explicit_cases.push((elem_iv, target));
            } else {
                debug_assert!(gap_target.is_none() || gap_target == Some(target));
                gap_target = Some(target);
            }
        }

        let cases = coalesce_bit_cases(explicit_cases);
        self.builder.switch(
            parent,
            SwitchKind::BitSlice {
                lo,
                hi,
                cases,
                default: gap_target,
            },
        )
    }

    fn specialize_any(
        &mut self,
        mut cols: Vec<Column>,
        rows: Vec<Row>,
        col_idx: usize,
    ) -> Result<NodeId, CompileError> {
        let expanded = expand_or_at_col(&cols, rows, col_idx);
        cols.remove(col_idx);
        let mut next_rows = Vec::with_capacity(expanded.len());
        for mut row in expanded {
            match row.pats.remove(col_idx) {
                ColPattern::Pattern(Pattern::Wild) => next_rows.push(row),
                ColPattern::Pattern(other) => {
                    return Err(CompileError::TypeMismatch {
                        expected: "any".to_string(),
                        found: other.to_string(),
                    });
                }
                ColPattern::BitSlice(_) => unreachable!(),
            }
        }
        self.compile_matrix(cols, next_rows)
    }
}

fn find_uninhabited_place(place: &Place, ty: &TypeDesc) -> Option<Place> {
    match ty {
        TypeDesc::Enum(en) if !en.is_open && en.variants.is_empty() => Some(place.clone()),
        TypeDesc::Tuple(elems) => elems
            .iter()
            .enumerate()
            .find_map(|(i, elem_ty)| find_uninhabited_place(&place.tuple_field(i), elem_ty)),
        TypeDesc::Record(fields) => fields.iter().find_map(|(name, field_ty)| {
            find_uninhabited_place(&place.record_field(name), field_ty)
        }),
        _ => None,
    }
}

fn normalize_rows(cols: &[Column], rows: &mut [Row]) {
    for row in rows {
        for (col_idx, col) in cols.iter().enumerate() {
            if let ColPattern::Pattern(pat) = &mut row.pats[col_idx] {
                peel_outer_bindings_in_place(pat, &col.place, &mut row.bindings);
                simplify_trivial_or_in_place(pat);
            }
        }
    }
}

fn drop_all_wild_columns(cols: Vec<Column>, rows: &mut [Row]) -> Vec<Column> {
    let keep: Vec<bool> = (0..cols.len())
        .map(|idx| rows.iter().any(|r| !r.pats[idx].is_wild()))
        .collect();
    if keep.iter().all(|&k| k) {
        return cols;
    }
    let filtered_cols: Vec<Column> = cols
        .into_iter()
        .zip(keep.iter())
        .filter_map(|(c, &k)| k.then_some(c))
        .collect();
    for row in rows {
        let mut idx = 0usize;
        row.pats.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }
    filtered_cols
}

fn peel_outer_bindings_in_place(pat: &mut Pattern, place: &Place, bindings: &mut Vec<Binding>) {
    while let Pattern::Bind { name, subpattern } = pat {
        bindings.push(Binding {
            name: name.clone(),
            place: place.clone(),
        });
        let inner = std::mem::replace(subpattern.as_mut(), Pattern::Wild);
        *pat = inner;
    }
}

fn simplify_trivial_or_in_place(pat: &mut Pattern) {
    if let Pattern::Or(alts) = pat {
        for alt in alts.iter_mut() {
            simplify_trivial_or_in_place(alt);
        }
        let mut flattened = Vec::with_capacity(alts.len());
        for alt in alts.drain(..) {
            match alt {
                Pattern::Or(inner) => flattened.extend(inner),
                other => flattened.push(other),
            }
        }
        *alts = flattened;
        if alts.len() == 1 {
            *pat = alts.remove(0);
            simplify_trivial_or_in_place(pat);
        } else if !alts.iter().any(pattern_has_any_bindings) && alts.iter().any(is_pattern_wild) {
            *pat = Pattern::Wild;
        }
    }
}

fn prune_subsumed_rows(rows: &mut Vec<Row>) {
    let mut seen_all_wild_arms = BTreeSet::new();
    let mut keep = Vec::with_capacity(rows.len());

    for row in rows.drain(..) {
        if seen_all_wild_arms.contains(&row.arm_pos) {
            continue;
        }
        let all_wild = row.pats.iter().all(ColPattern::is_wild);
        let stop_all = all_wild && !row.has_guard;
        if all_wild {
            seen_all_wild_arms.insert(row.arm_pos);
        }
        keep.push(row);
        if stop_all {
            break;
        }
    }

    *rows = keep;
}

fn expand_or_at_col(cols: &[Column], rows: Vec<Row>, col_idx: usize) -> Vec<Row> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut stack = vec![row];
        while let Some(mut cur) = stack.pop() {
            if let ColPattern::Pattern(pat) = &mut cur.pats[col_idx] {
                peel_outer_bindings_in_place(pat, &cols[col_idx].place, &mut cur.bindings);
                simplify_trivial_or_in_place(pat);
            }
            if let ColPattern::Pattern(Pattern::Or(alts)) = cur.pats[col_idx].clone() {
                for alt in alts.into_iter().rev() {
                    let mut cloned = cur.clone();
                    cloned.pats[col_idx] = ColPattern::Pattern(alt);
                    stack.push(cloned);
                }
            } else {
                out.push(cur);
            }
        }
    }
    out
}

/// Variant of `expand_or_at_col` for integer/char intervals: when an `Or` branch is a pure
/// interval/literal with no inner `Bind`s, keep the `Or` intact on the single row so
/// interval extraction handles `(1 or 2 or 3)` without row duplication.
fn expand_or_at_col_for_intervals(cols: &[Column], rows: Vec<Row>, col_idx: usize) -> Vec<Row> {
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        if let ColPattern::Pattern(pat) = &mut row.pats[col_idx] {
            peel_outer_bindings_in_place(pat, &cols[col_idx].place, &mut row.bindings);
            simplify_trivial_or_in_place(pat);
        }
        if let ColPattern::Pattern(pat) = &row.pats[col_idx]
            && pattern_has_nested_bindings(pat)
        {
            out.extend(expand_or_at_col(cols, vec![row], col_idx));
        } else {
            out.push(row);
        }
    }
    out
}

fn pattern_has_nested_bindings(pat: &Pattern) -> bool {
    match pat {
        Pattern::Bind { .. } => true,
        Pattern::Or(alts) => alts.iter().any(pattern_has_nested_bindings),
        _ => false,
    }
}

fn extract_int_intervals(
    pat: &Pattern,
    domain: IntInterval,
) -> Result<Option<Vec<IntInterval>>, CompileError> {
    match pat {
        Pattern::Wild => Ok(None),
        Pattern::Int(lit) => Ok(Some(
            IntInterval::from_bounds(
                RangeBound::Inclusive(*lit),
                RangeBound::Inclusive(*lit),
                domain,
            )
            .into_iter()
            .collect(),
        )),
        Pattern::IntRange { start, end } => Ok(Some(
            IntInterval::from_bounds(*start, *end, domain)
                .into_iter()
                .collect(),
        )),
        Pattern::Or(alts) => {
            let mut merged = Vec::new();
            for alt in alts {
                match extract_int_intervals(alt, domain)? {
                    None => return Ok(None),
                    Some(ivs) => merged.extend(ivs),
                }
            }
            Ok(Some(IntInterval::merge(&merged)))
        }
        other => Err(CompileError::TypeMismatch {
            expected: "int".to_string(),
            found: other.to_string(),
        }),
    }
}

fn extract_char_intervals(pat: &Pattern) -> Result<Option<Vec<CharInterval>>, CompileError> {
    match pat {
        Pattern::Wild => Ok(None),
        Pattern::Char(ch) => Ok(Some(vec![CharInterval { lo: *ch, hi: *ch }])),
        Pattern::CharRange { start, end } => Ok(Some(CharInterval::from_bounds(*start, *end))),
        Pattern::Or(alts) => {
            let mut merged = Vec::new();
            for alt in alts {
                match extract_char_intervals(alt)? {
                    None => return Ok(None),
                    Some(ivs) => merged.extend(ivs),
                }
            }
            Ok(Some(merged))
        }
        other => Err(CompileError::TypeMismatch {
            expected: "char".to_string(),
            found: other.to_string(),
        }),
    }
}

struct NormalizedListRow {
    row: Row,
    rest_binds: Vec<(String, usize)>,
}

fn normalize_list_row(
    mut row: Row,
    col_idx: usize,
    place: &Place,
    out: &mut Vec<NormalizedListRow>,
) -> Result<(), CompileError> {
    let pat = match std::mem::replace(&mut row.pats[col_idx], ColPattern::Pattern(Pattern::Wild)) {
        ColPattern::Pattern(p) => p,
        ColPattern::BitSlice(_) => unreachable!(),
    };
    normalize_list_pattern_step(pat, row, col_idx, place, Vec::new(), out)
}

fn normalize_list_pattern_step(
    pat: Pattern,
    mut row: Row,
    col_idx: usize,
    place: &Place,
    mut rest_binds: Vec<(String, usize)>,
    out: &mut Vec<NormalizedListRow>,
) -> Result<(), CompileError> {
    match pat {
        Pattern::Wild => {
            row.pats[col_idx] = ColPattern::Pattern(Pattern::Wild);
            out.push(NormalizedListRow { row, rest_binds });
            Ok(())
        }
        Pattern::Bind { name, subpattern } => {
            row.bindings.push(Binding {
                name,
                place: place.clone(),
            });
            normalize_list_pattern_step(*subpattern, row, col_idx, place, rest_binds, out)
        }
        Pattern::Or(alts) => {
            for alt in alts {
                normalize_list_pattern_step(
                    alt,
                    row.clone(),
                    col_idx,
                    place,
                    rest_binds.clone(),
                    out,
                )?;
            }
            Ok(())
        }
        Pattern::List { mut prefix, rest } => {
            let Some(rest_box) = rest else {
                row.pats[col_idx] = ColPattern::Pattern(Pattern::List { prefix, rest: None });
                out.push(NormalizedListRow { row, rest_binds });
                return Ok(());
            };
            match *rest_box {
                Pattern::Wild => {
                    row.pats[col_idx] = ColPattern::Pattern(Pattern::List {
                        prefix,
                        rest: Some(Box::new(Pattern::Wild)),
                    });
                    out.push(NormalizedListRow { row, rest_binds });
                    Ok(())
                }
                Pattern::Bind { name, subpattern } => {
                    rest_binds.push((name, prefix.len()));
                    let next_rest = if is_pattern_wild(&subpattern) {
                        Pattern::Wild
                    } else {
                        *subpattern
                    };
                    normalize_list_pattern_step(
                        Pattern::List {
                            prefix,
                            rest: Some(Box::new(next_rest)),
                        },
                        row,
                        col_idx,
                        place,
                        rest_binds,
                        out,
                    )
                }
                Pattern::List {
                    prefix: p2,
                    rest: r2,
                } => {
                    prefix.extend(p2);
                    normalize_list_pattern_step(
                        Pattern::List { prefix, rest: r2 },
                        row,
                        col_idx,
                        place,
                        rest_binds,
                        out,
                    )
                }
                Pattern::Or(alts) => {
                    if is_pattern_wild(&Pattern::Or(alts.clone())) {
                        row.pats[col_idx] = ColPattern::Pattern(Pattern::List {
                            prefix,
                            rest: Some(Box::new(Pattern::Wild)),
                        });
                        out.push(NormalizedListRow { row, rest_binds });
                        return Ok(());
                    }
                    for alt in alts {
                        normalize_list_pattern_step(
                            Pattern::List {
                                prefix: prefix.clone(),
                                rest: Some(Box::new(alt)),
                            },
                            row.clone(),
                            col_idx,
                            place,
                            rest_binds.clone(),
                            out,
                        )?;
                    }
                    Ok(())
                }
                other => Err(CompileError::TypeMismatch {
                    expected: "wild or identifier rest pattern".to_string(),
                    found: other.to_string(),
                }),
            }
        }
        other => Err(CompileError::TypeMismatch {
            expected: "list".to_string(),
            found: other.to_string(),
        }),
    }
}

/// Select the best discriminating column using Maranget's necessity + small-branching heuristic.
fn select_column(cols: &[Column], rows: &[Row]) -> usize {
    // Immediate zero-cost win: unpack any Tuple/Record column that introduces no `Or` splits.
    for (idx, col) in cols.iter().enumerate() {
        if matches!(
            col.domain,
            ColDomain::Typed(TypeDesc::Tuple(_) | TypeDesc::Record(_))
        ) && rows.iter().all(|r| match &r.pats[idx] {
            ColPattern::Pattern(Pattern::Or(_)) => false,
            ColPattern::Pattern(_) => true,
            ColPattern::BitSlice(_) => false,
        }) {
            return idx;
        }
    }

    let mut best_idx = 0usize;
    let mut best_score = (isize::MIN, 0usize, 0usize, isize::MIN, usize::MAX);

    for (idx, col) in cols.iter().enumerate() {
        if rows[0].pats[idx].is_wild() {
            continue;
        }
        // Strongly prefer columns that require no `Or` row-splitting before expanding fresh `Or`s.
        let or_extra_rows: isize = rows
            .iter()
            .map(|r| match &r.pats[idx] {
                ColPattern::Pattern(Pattern::Or(alts)) if !is_interval_column(&col.domain) => {
                    alts.len().saturating_sub(1) as isize
                }
                _ => 0,
            })
            .sum();
        let prefix_needed = rows.iter().take_while(|r| !r.pats[idx].is_wild()).count();
        let total_needed = rows.iter().filter(|r| !r.pats[idx].is_wild()).count();
        let neg_branching = -estimated_branching(col, rows, idx);
        let score = (
            -or_extra_rows,
            prefix_needed,
            total_needed,
            neg_branching,
            usize::MAX - idx,
        );
        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
    }

    best_idx
}

fn is_interval_column(domain: &ColDomain) -> bool {
    matches!(
        domain,
        ColDomain::Typed(TypeDesc::Int(_) | TypeDesc::Char) | ColDomain::AlignedBitSlice { .. }
    )
}

fn estimated_branching(col: &Column, rows: &[Row], col_idx: usize) -> isize {
    match &col.domain {
        ColDomain::Typed(TypeDesc::Tuple(_) | TypeDesc::Record(_)) => 1,
        ColDomain::Typed(TypeDesc::Bool | TypeDesc::Optional(_)) => 2,
        ColDomain::Typed(TypeDesc::ErrorRow(r)) => 1 + (r.error_tags.len() as isize),
        ColDomain::Typed(TypeDesc::Enum(e)) => (e.variants.len().max(1)) as isize,
        ColDomain::Typed(TypeDesc::BitVec(w)) => {
            if *w <= 4 {
                1isize << *w
            } else {
                16
            }
        }
        ColDomain::AlignedBitSlice { lo, hi, .. } => {
            let w = hi - lo;
            let distinct: BTreeSet<&AlignedBitCell> = rows
                .iter()
                .filter_map(|r| match &r.pats[col_idx] {
                    ColPattern::BitSlice(c) if !matches!(c, AlignedBitCell::Wild) => Some(c),
                    _ => None,
                })
                .collect();
            if w <= 3 {
                (1isize << w).min((distinct.len() as isize) + 1)
            } else {
                (distinct.len() as isize) + 1
            }
        }
        ColDomain::Typed(
            TypeDesc::Int(_) | TypeDesc::Char | TypeDesc::List(_) | TypeDesc::EndlessList(_),
        ) => {
            let count = rows.iter().filter(|r| !r.pats[col_idx].is_wild()).count();
            (count as isize) + 1
        }
        ColDomain::Typed(TypeDesc::Any) => 1,
    }
}

#[derive(Clone, Debug)]
enum WitnessConstraint {
    Bool(bool),
    Int(IntLit),
    Char(char),
    OptionalNull,
    OptionalPresent,
    ErrorOk,
    ErrorErr(ErrorTag),
    ErrorOtherTag,
    EnumVariant(VariantTag),
    EnumOtherVariant,
    ListLen(usize),
    BitSlices(u32, BTreeMap<(u32, u32), BitValue>),
}

fn collect_missing_witnesses(
    dag: &DecisionDag,
    root_ty: &TypeDesc,
    max_witnesses: usize,
) -> Vec<MissingWitness> {
    if max_witnesses == 0 {
        return Vec::new();
    }
    let can_fail = compute_can_fail(dag);
    if !can_fail[dag.root.0] {
        return Vec::new();
    }

    let mut witnesses = Vec::new();
    let mut seen = BTreeSet::new();
    let mut constraints: BTreeMap<Place, WitnessConstraint> = BTreeMap::new();
    let mut known_types: BTreeMap<Place, TypeDesc> = BTreeMap::new();
    known_types.insert(Place::root(), root_ty.clone());
    let mut search_budget = max_witnesses.saturating_mul(128).max(256);

    collect_witness_dfs(
        dag,
        dag.root,
        root_ty,
        &can_fail,
        &mut constraints,
        &mut known_types,
        &mut witnesses,
        &mut seen,
        max_witnesses,
        &mut search_budget,
    );

    witnesses
}

fn compute_can_fail(dag: &DecisionDag) -> Vec<bool> {
    let mut can_fail = vec![false; dag.nodes.len()];
    for (idx, node) in dag.nodes.iter().enumerate() {
        if matches!(node, DagNode::Fail) {
            can_fail[idx] = true;
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (idx, node) in dag.nodes.iter().enumerate() {
            if can_fail[idx] {
                continue;
            }
            if node.children().into_iter().any(|c| can_fail[c.0]) {
                can_fail[idx] = true;
                changed = true;
            }
        }
    }
    can_fail
}

#[allow(clippy::too_many_arguments)]
fn collect_witness_dfs(
    dag: &DecisionDag,
    node_id: NodeId,
    root_ty: &TypeDesc,
    can_fail: &[bool],
    constraints: &mut BTreeMap<Place, WitnessConstraint>,
    known_types: &mut BTreeMap<Place, TypeDesc>,
    witnesses: &mut Vec<MissingWitness>,
    seen: &mut BTreeSet<MissingWitness>,
    max_witnesses: usize,
    search_budget: &mut usize,
) {
    if witnesses.len() >= max_witnesses || *search_budget == 0 || !can_fail[node_id.0] {
        return;
    }
    *search_budget -= 1;

    match dag.node(node_id) {
        DagNode::Fail => {
            let w = materialize_witness(root_ty, &Place::root(), constraints, known_types);
            if seen.insert(w.clone()) {
                witnesses.push(w);
            }
        }
        DagNode::Leaf { .. } => {}
        DagNode::Bind { next, .. } => {
            collect_witness_dfs(
                dag,
                *next,
                root_ty,
                can_fail,
                constraints,
                known_types,
                witnesses,
                seen,
                max_witnesses,
                search_budget,
            );
        }
        DagNode::GuardCheck {
            on_pass, on_fail, ..
        } => {
            collect_witness_dfs(
                dag,
                *on_fail,
                root_ty,
                can_fail,
                constraints,
                known_types,
                witnesses,
                seen,
                max_witnesses,
                search_budget,
            );
            collect_witness_dfs(
                dag,
                *on_pass,
                root_ty,
                can_fail,
                constraints,
                known_types,
                witnesses,
                seen,
                max_witnesses,
                search_budget,
            );
        }
        DagNode::Switch { place, kind } => {
            let place_ty = resolve_place_type(root_ty, place, known_types);
            match kind {
                SwitchKind::Bool { on_false, on_true } => {
                    for (b, child) in [(false, *on_false), (true, *on_true)] {
                        if can_fail[child.0] {
                            let prev =
                                constraints.insert(place.clone(), WitnessConstraint::Bool(b));
                            collect_witness_dfs(
                                dag,
                                child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                }
                SwitchKind::Optional {
                    on_null,
                    on_present,
                } => {
                    if can_fail[on_null.0] {
                        let prev =
                            constraints.insert(place.clone(), WitnessConstraint::OptionalNull);
                        collect_witness_dfs(
                            dag,
                            *on_null,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                    if can_fail[on_present.0] {
                        let prev =
                            constraints.insert(place.clone(), WitnessConstraint::OptionalPresent);
                        collect_witness_dfs(
                            dag,
                            *on_present,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                }
                SwitchKind::Error {
                    on_ok,
                    cases,
                    default_err,
                } => {
                    if can_fail[on_ok.0] {
                        let prev = constraints.insert(place.clone(), WitnessConstraint::ErrorOk);
                        collect_witness_dfs(
                            dag,
                            *on_ok,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                    for (tag, &child) in cases {
                        if can_fail[child.0] {
                            let err_place = place.error_err_inner();
                            let prev_ty = if let TypeDesc::ErrorRow(ref row_ty) = place_ty
                                && let Some(inner_ty) = row_ty.error_tags.get(tag)
                            {
                                known_types.insert(err_place.clone(), inner_ty.clone())
                            } else {
                                known_types.remove(&err_place)
                            };
                            let prev = constraints
                                .insert(place.clone(), WitnessConstraint::ErrorErr(tag.clone()));
                            collect_witness_dfs(
                                dag,
                                child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                            restore_known_type(known_types, &err_place, prev_ty);
                        }
                    }
                    if let Some(def) = default_err
                        && can_fail[def.0]
                    {
                        let prev =
                            constraints.insert(place.clone(), WitnessConstraint::ErrorOtherTag);
                        collect_witness_dfs(
                            dag,
                            *def,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                }
                SwitchKind::Enum { cases, default } => {
                    for (tag, &child) in cases {
                        if can_fail[child.0] {
                            let prev = constraints
                                .insert(place.clone(), WitnessConstraint::EnumVariant(tag.clone()));
                            collect_witness_dfs(
                                dag,
                                child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                    if let Some(def) = default
                        && can_fail[def.0]
                    {
                        let prev =
                            constraints.insert(place.clone(), WitnessConstraint::EnumOtherVariant);
                        collect_witness_dfs(
                            dag,
                            *def,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                }
                SwitchKind::Int { cases, default } => {
                    let (domain, prefer_signed) = match place_ty {
                        TypeDesc::Int(it) => (IntInterval::domain_for(it), it.is_signed()),
                        _ => (IntInterval::domain_for(IntType::OpenSigned), true),
                    };
                    if let Some(def) = default
                        && can_fail[def.0]
                    {
                        let covered: Vec<IntInterval> = cases.iter().map(|(iv, _)| *iv).collect();
                        if let Some(rep) =
                            IntInterval::first_uncovered(domain, &covered, prefer_signed)
                        {
                            let prev =
                                constraints.insert(place.clone(), WitnessConstraint::Int(rep));
                            collect_witness_dfs(
                                dag,
                                *def,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                    for (iv, child) in cases {
                        if can_fail[child.0] {
                            let rep = iv.representative(prefer_signed);
                            let prev =
                                constraints.insert(place.clone(), WitnessConstraint::Int(rep));
                            collect_witness_dfs(
                                dag,
                                *child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                }
                SwitchKind::Char { cases, default } => {
                    if let Some(def) = default
                        && can_fail[def.0]
                    {
                        let covered: Vec<CharInterval> = cases.iter().map(|(iv, _)| *iv).collect();
                        if let Some(rep) = CharInterval::first_uncovered(&covered) {
                            let prev =
                                constraints.insert(place.clone(), WitnessConstraint::Char(rep));
                            collect_witness_dfs(
                                dag,
                                *def,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                    for (iv, child) in cases {
                        if can_fail[child.0] {
                            let prev =
                                constraints.insert(place.clone(), WitnessConstraint::Char(iv.lo));
                            collect_witness_dfs(
                                dag,
                                *child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                }
                SwitchKind::ListLen { cases, default } => {
                    for (iv, child) in cases {
                        if can_fail[child.0] {
                            let prev = constraints
                                .insert(place.clone(), WitnessConstraint::ListLen(iv.min));
                            collect_witness_dfs(
                                dag,
                                *child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                    if let Some(def) = default
                        && can_fail[def.0]
                    {
                        let len = first_uncovered_list_len(cases);
                        let prev =
                            constraints.insert(place.clone(), WitnessConstraint::ListLen(len));
                        collect_witness_dfs(
                            dag,
                            *def,
                            root_ty,
                            can_fail,
                            constraints,
                            known_types,
                            witnesses,
                            seen,
                            max_witnesses,
                            search_budget,
                        );
                        restore_constraint(constraints, place, prev);
                    }
                }
                SwitchKind::BitSlice {
                    lo,
                    hi,
                    cases,
                    default,
                } => {
                    let parent_width = match place_ty {
                        TypeDesc::BitVec(w) => w,
                        _ => *hi,
                    };
                    let slice_width = *hi - *lo;
                    if let Some(def) = default
                        && can_fail[def.0]
                    {
                        let covered: Vec<BitInterval> =
                            cases.iter().map(|(iv, _)| iv.clone()).collect();
                        if let Some(rep) = first_uncovered_bit_value(slice_width, &covered) {
                            let prev = insert_bit_slice_constraint(
                                constraints,
                                place,
                                parent_width,
                                (*lo, *hi),
                                rep,
                            );
                            collect_witness_dfs(
                                dag,
                                *def,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                    for (iv, child) in cases {
                        if can_fail[child.0] {
                            let prev = insert_bit_slice_constraint(
                                constraints,
                                place,
                                parent_width,
                                (*lo, *hi),
                                iv.lo.clone(),
                            );
                            collect_witness_dfs(
                                dag,
                                *child,
                                root_ty,
                                can_fail,
                                constraints,
                                known_types,
                                witnesses,
                                seen,
                                max_witnesses,
                                search_budget,
                            );
                            restore_constraint(constraints, place, prev);
                        }
                    }
                }
            }
        }
    }
}

fn restore_known_type(
    known_types: &mut BTreeMap<Place, TypeDesc>,
    place: &Place,
    prev: Option<TypeDesc>,
) {
    match prev {
        Some(ty) => {
            known_types.insert(place.clone(), ty);
        }
        None => {
            known_types.remove(place);
        }
    }
}

fn first_uncovered_list_len(cases: &[(LenInterval, NodeId)]) -> usize {
    let mut candidate = 0usize;
    loop {
        if cases.iter().all(|(iv, _)| !iv.contains(candidate)) {
            return candidate;
        }
        candidate += 1;
    }
}

fn insert_bit_slice_constraint(
    constraints: &mut BTreeMap<Place, WitnessConstraint>,
    place: &Place,
    parent_width: u32,
    range: (u32, u32),
    value: BitValue,
) -> Option<WitnessConstraint> {
    let prev = constraints.get(place).cloned();
    let mut slices = match &prev {
        Some(WitnessConstraint::BitSlices(_, map)) => map.clone(),
        _ => BTreeMap::new(),
    };
    slices.insert(range, value);
    constraints.insert(
        place.clone(),
        WitnessConstraint::BitSlices(parent_width, slices),
    );
    prev
}

fn restore_constraint(
    constraints: &mut BTreeMap<Place, WitnessConstraint>,
    place: &Place,
    prev: Option<WitnessConstraint>,
) {
    match prev {
        Some(c) => {
            constraints.insert(place.clone(), c);
        }
        None => {
            constraints.remove(place);
        }
    }
}

fn resolve_place_type(
    root_ty: &TypeDesc,
    place: &Place,
    known_types: &BTreeMap<Place, TypeDesc>,
) -> TypeDesc {
    if let Some(known) = known_types.get(place) {
        return known.clone();
    }
    use crate::decision::PlaceStep;
    let mut cur = root_ty.clone();
    for (idx, step) in place.steps.iter().enumerate() {
        let prefix = Place {
            steps: place.steps[..=idx].to_vec(),
        };
        if let Some(known) = known_types.get(&prefix) {
            cur = known.clone();
            continue;
        }
        cur = match (cur, step) {
            (TypeDesc::Tuple(mut elems), PlaceStep::TupleField(i)) => elems.swap_remove(*i),
            (TypeDesc::Record(mut fields), PlaceStep::RecordField(name)) => {
                fields.remove(name).unwrap_or(TypeDesc::Any)
            }
            (TypeDesc::Optional(inner), PlaceStep::OptionalInner) => *inner,
            (TypeDesc::ErrorRow(row), PlaceStep::ErrorOkInner) => *row.ok_type,
            (TypeDesc::ErrorRow(_row), PlaceStep::ErrorErrInner) => TypeDesc::Any,
            (TypeDesc::Enum(mut en), PlaceStep::VariantTupleField { tag, index }) => {
                match en.variants.remove(tag).map(|s| s.payload) {
                    Some(VariantPayloadType::Tuple(mut ts)) if *index < ts.len() => {
                        ts.swap_remove(*index)
                    }
                    _ => TypeDesc::Any,
                }
            }
            (TypeDesc::Enum(mut en), PlaceStep::VariantRecordField { tag, field }) => {
                match en.variants.remove(tag).map(|s| s.payload) {
                    Some(VariantPayloadType::Record(mut fs)) => {
                        fs.remove(field).unwrap_or(TypeDesc::Any)
                    }
                    _ => TypeDesc::Any,
                }
            }
            (TypeDesc::List(elem), PlaceStep::ListElem(_)) => *elem,
            (TypeDesc::List(elem), PlaceStep::ListRest(_)) => TypeDesc::List(elem),
            (TypeDesc::EndlessList(elem), PlaceStep::ListElem(_)) => *elem,
            (TypeDesc::EndlessList(elem), PlaceStep::ListRest(_)) => TypeDesc::EndlessList(elem),
            (_, PlaceStep::BitSlice { ty, .. }) => ty.clone(),
            _ => TypeDesc::Any,
        };
    }
    cur
}

fn materialize_witness(
    ty: &TypeDesc,
    place: &Place,
    constraints: &BTreeMap<Place, WitnessConstraint>,
    known_types: &BTreeMap<Place, TypeDesc>,
) -> MissingWitness {
    match ty {
        TypeDesc::Tuple(elems) => MissingWitness::Tuple(
            elems
                .iter()
                .enumerate()
                .map(|(i, elem_ty)| {
                    materialize_witness(elem_ty, &place.tuple_field(i), constraints, known_types)
                })
                .collect(),
        ),
        TypeDesc::Record(fields) => MissingWitness::Record(
            fields
                .iter()
                .map(|(name, field_ty)| {
                    (
                        name.clone(),
                        materialize_witness(
                            field_ty,
                            &place.record_field(name),
                            constraints,
                            known_types,
                        ),
                    )
                })
                .collect(),
        ),
        TypeDesc::EndlessList(elem_ty) => {
            let mut max_elem: Option<usize> = None;
            for key in constraints.keys().chain(known_types.keys()) {
                if key.steps.len() > place.steps.len()
                    && key.steps[..place.steps.len()] == place.steps[..]
                    && let PlaceStep::ListElem(idx) = key.steps[place.steps.len()]
                {
                    max_elem = Some(max_elem.map_or(idx + 1, |cur| cur.max(idx + 1)));
                }
            }
            let len = max_elem.unwrap_or(0);
            MissingWitness::EndlessList(
                (0..len)
                    .map(|i| {
                        materialize_witness(elem_ty, &place.list_elem(i), constraints, known_types)
                    })
                    .collect(),
            )
        }
        TypeDesc::BitVec(width) => {
            let mut bv = BitValue::zero(*width);
            if let Some(WitnessConstraint::BitSlices(_, slices)) = constraints.get(place) {
                for (&(lo, _hi), val) in slices {
                    bv.splice_from(lo, val);
                }
            }
            MissingWitness::BitVec(bv)
        }
        _ => match constraints.get(place) {
            None => default_witness(ty),
            Some(WitnessConstraint::Bool(b)) => MissingWitness::Bool(*b),
            Some(WitnessConstraint::Int(lit)) => MissingWitness::Int(*lit),
            Some(WitnessConstraint::Char(ch)) => MissingWitness::Char(*ch),
            Some(WitnessConstraint::OptionalNull) => MissingWitness::OptionalNull,
            Some(WitnessConstraint::OptionalPresent) => {
                let inner_ty = match ty {
                    TypeDesc::Optional(inner) => inner.as_ref(),
                    _ => &TypeDesc::Any,
                };
                MissingWitness::OptionalPresent(Box::new(materialize_witness(
                    inner_ty,
                    &place.optional_inner(),
                    constraints,
                    known_types,
                )))
            }
            Some(WitnessConstraint::ErrorOk) => {
                let ok_ty = match ty {
                    TypeDesc::ErrorRow(row) => row.ok_type.as_ref(),
                    _ => &TypeDesc::Any,
                };
                MissingWitness::ErrorOk(Box::new(materialize_witness(
                    ok_ty,
                    &place.error_ok_inner(),
                    constraints,
                    known_types,
                )))
            }
            Some(WitnessConstraint::ErrorErr(tag)) => {
                let err_place = place.error_err_inner();
                let err_ty = known_types.get(&err_place).unwrap_or_else(|| match ty {
                    TypeDesc::ErrorRow(row) => row.error_tags.get(tag).unwrap_or(&TypeDesc::Any),
                    _ => &TypeDesc::Any,
                });
                MissingWitness::ErrorErr {
                    tag: tag.clone(),
                    inner: Box::new(materialize_witness(
                        err_ty,
                        &err_place,
                        constraints,
                        known_types,
                    )),
                }
            }
            Some(WitnessConstraint::ErrorOtherTag) => MissingWitness::ErrorOtherTag {
                inner: Box::new(materialize_witness(
                    &TypeDesc::Any,
                    &place.error_err_inner(),
                    constraints,
                    known_types,
                )),
            },
            Some(WitnessConstraint::EnumVariant(tag)) => {
                let payload = match ty {
                    TypeDesc::Enum(enum_ty) => match enum_ty.variants.get(tag) {
                        Some(schema) => match &schema.payload {
                            VariantPayloadType::Unit => WitnessPayload::Unit,
                            VariantPayloadType::Tuple(tys) => WitnessPayload::Tuple(
                                tys.iter()
                                    .enumerate()
                                    .map(|(i, t)| {
                                        materialize_witness(
                                            t,
                                            &place.variant_tuple_field(tag.clone(), i),
                                            constraints,
                                            known_types,
                                        )
                                    })
                                    .collect(),
                            ),
                            VariantPayloadType::Record(fields) => WitnessPayload::Record(
                                fields
                                    .iter()
                                    .map(|(name, t)| {
                                        (
                                            name.clone(),
                                            materialize_witness(
                                                t,
                                                &place.variant_record_field(tag.clone(), name),
                                                constraints,
                                                known_types,
                                            ),
                                        )
                                    })
                                    .collect(),
                            ),
                        },
                        None => WitnessPayload::Unit,
                    },
                    _ => WitnessPayload::Unit,
                };
                MissingWitness::EnumVariant {
                    tag: tag.clone(),
                    payload,
                }
            }
            Some(WitnessConstraint::EnumOtherVariant) => MissingWitness::EnumOtherVariant,
            Some(WitnessConstraint::ListLen(len)) => {
                let elem_ty = match ty {
                    TypeDesc::List(inner) => inner.as_ref(),
                    _ => &TypeDesc::Any,
                };
                MissingWitness::List(
                    (0..*len)
                        .map(|i| {
                            materialize_witness(
                                elem_ty,
                                &place.list_elem(i),
                                constraints,
                                known_types,
                            )
                        })
                        .collect(),
                )
            }
            Some(WitnessConstraint::BitSlices(width, slices)) => {
                let mut bv = BitValue::zero(*width);
                for (&(lo, _hi), val) in slices {
                    bv.splice_from(lo, val);
                }
                MissingWitness::BitVec(bv)
            }
        },
    }
}

fn default_witness(ty: &TypeDesc) -> MissingWitness {
    match ty {
        TypeDesc::Bool => MissingWitness::Bool(false),
        TypeDesc::Char => MissingWitness::Char('\0'),
        TypeDesc::Int(it) => {
            MissingWitness::Int(IntInterval::domain_for(*it).representative(it.is_signed()))
        }
        TypeDesc::Optional(_) => MissingWitness::OptionalNull,
        TypeDesc::ErrorRow(row) => MissingWitness::ErrorOk(Box::new(default_witness(&row.ok_type))),
        TypeDesc::Enum(en) => {
            if let Some((tag, schema)) = en.variants.iter().next() {
                let payload = match &schema.payload {
                    VariantPayloadType::Unit => WitnessPayload::Unit,
                    VariantPayloadType::Tuple(tys) => {
                        WitnessPayload::Tuple(tys.iter().map(default_witness).collect())
                    }
                    VariantPayloadType::Record(fs) => WitnessPayload::Record(
                        fs.iter()
                            .map(|(k, v)| (k.clone(), default_witness(v)))
                            .collect(),
                    ),
                };
                MissingWitness::EnumVariant {
                    tag: tag.clone(),
                    payload,
                }
            } else if en.is_open {
                MissingWitness::EnumOtherVariant
            } else {
                MissingWitness::Wild
            }
        }
        TypeDesc::Tuple(ts) => MissingWitness::Tuple(ts.iter().map(default_witness).collect()),
        TypeDesc::Record(fs) => MissingWitness::Record(
            fs.iter()
                .map(|(k, v)| (k.clone(), default_witness(v)))
                .collect(),
        ),
        TypeDesc::List(_) => MissingWitness::List(Vec::new()),
        TypeDesc::EndlessList(_) => MissingWitness::EndlessList(Vec::new()),
        TypeDesc::BitVec(w) => MissingWitness::BitVec(BitValue::zero(*w)),
        TypeDesc::Any => MissingWitness::Opaque,
    }
}
