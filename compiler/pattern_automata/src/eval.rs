//! Concrete runtime [`Value`], linear reference matcher [`eval_linear`],
//! and compiled decision-DAG executor [`eval_dag`].

use crate::bitvec::{BitValue, slice_to_value};
use crate::decision::{Binding, DagNode, DecisionDag, Place, PlaceStep, SwitchKind};
use crate::pattern::{
    BitSegmentPattern, CharInterval, ErrorTag, IntLit, MatchArm, Pattern, RangeBound,
    VariantPayloadPattern, VariantTag,
};
use crate::sexpr;
use std::collections::BTreeMap;
use std::fmt;

/// Runtime enum variant payload.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnumValuePayload {
    Unit,
    Tuple(Vec<Value>),
    Record(BTreeMap<String, Value>),
}

impl fmt::Display for EnumValuePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("unit"),
            Self::Tuple(elems) => {
                f.write_str("(tuple")?;
                for v in elems {
                    write!(f, " {v}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, v) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {v})")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Concrete runtime scrutinee value for reference and DAG evaluation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Bool(bool),
    Char(char),
    Int(IntLit),
    OptionalNull,
    OptionalPresent(Box<Value>),
    ErrorOk(Box<Value>),
    ErrorErr {
        tag: ErrorTag,
        inner: Box<Value>,
    },
    Enum {
        tag: VariantTag,
        payload: EnumValuePayload,
    },
    Tuple(Vec<Value>),
    Record(BTreeMap<String, Value>),
    List(Vec<Value>),
    BitVec(BitValue),
}

impl Value {
    /// Construct a signed integer value.
    pub fn signed(value: i128) -> Self {
        Self::Int(IntLit::Signed(value))
    }

    /// Construct an unsigned integer value.
    pub fn unsigned(value: u128) -> Self {
        Self::Int(IntLit::Unsigned(value))
    }

    /// Construct `?inner`.
    pub fn present(inner: Value) -> Self {
        Self::OptionalPresent(Box::new(inner))
    }

    /// Construct `ok(inner)`.
    pub fn ok(inner: Value) -> Self {
        Self::ErrorOk(Box::new(inner))
    }

    /// Construct `err(tag, inner)`.
    pub fn err(tag: impl Into<String>, inner: Value) -> Self {
        Self::ErrorErr {
            tag: ErrorTag::new(tag),
            inner: Box::new(inner),
        }
    }

    /// Construct a unit enum value.
    pub fn unit_variant(tag: VariantTag) -> Self {
        Self::Enum {
            tag,
            payload: EnumValuePayload::Unit,
        }
    }

    /// Construct a tuple-payload enum value.
    pub fn tuple_variant(tag: VariantTag, elems: Vec<Value>) -> Self {
        Self::Enum {
            tag,
            payload: EnumValuePayload::Tuple(elems),
        }
    }

    /// Construct a record-payload enum value.
    pub fn record_variant(tag: VariantTag, fields: BTreeMap<String, Value>) -> Self {
        Self::Enum {
            tag,
            payload: EnumValuePayload::Record(fields),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::Enum { tag, payload } => write!(f, "(variant {tag} {payload})"),
            Self::Tuple(elems) => {
                f.write_str("(tuple")?;
                for v in elems {
                    write!(f, " {v}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, v) in fields {
                    f.write_str(" (")?;
                    sexpr::write_quoted_str(f, name)?;
                    write!(f, " {v})")?;
                }
                f.write_str(")")
            }
            Self::List(elems) => {
                f.write_str("(list")?;
                for v in elems {
                    write!(f, " {v}")?;
                }
                f.write_str(")")
            }
            Self::BitVec(bv) => write!(f, "{bv}"),
        }
    }
}

/// Structured error returned when evaluating a [`DecisionDag`] against an incompatible [`Value`]
/// or malformed DAG structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    InvalidNodeId {
        id: usize,
        node_count: usize,
    },
    IndexOutOfBounds {
        context: &'static str,
        index: usize,
        len: usize,
    },
    MissingRecordField {
        field: String,
    },
    InvalidBitSlice {
        lo: u32,
        hi: u32,
        width: u32,
    },
    InvalidBitSliceConversion {
        lo: u32,
        hi: u32,
    },
    InvalidPlaceProjection {
        step: String,
        value: String,
    },
    UnexpectedSwitchValue {
        expected: &'static str,
        found: String,
    },
    MissingSwitchCase {
        kind: &'static str,
        detail: String,
    },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNodeId { id, node_count } => write!(
                f,
                "(eval-error invalid-node-id (id {id}) (node-count {node_count}))"
            ),
            Self::IndexOutOfBounds {
                context,
                index,
                len,
            } => write!(
                f,
                "(eval-error index-out-of-bounds (context {context}) (index {index}) (len {len}))"
            ),
            Self::MissingRecordField { field } => {
                f.write_str("(eval-error missing-record-field ")?;
                sexpr::write_quoted_str(f, field)?;
                f.write_str(")")
            }
            Self::InvalidBitSlice { lo, hi, width } => write!(
                f,
                "(eval-error invalid-bit-slice (lo {lo}) (hi {hi}) (width {width}))"
            ),
            Self::InvalidBitSliceConversion { lo, hi } => write!(
                f,
                "(eval-error invalid-bit-slice-conversion (lo {lo}) (hi {hi}))"
            ),
            Self::InvalidPlaceProjection { step, value } => {
                f.write_str("(eval-error invalid-place-projection (step ")?;
                sexpr::write_quoted_str(f, step)?;
                f.write_str(") (value ")?;
                sexpr::write_quoted_str(f, value)?;
                f.write_str("))")
            }
            Self::UnexpectedSwitchValue { expected, found } => {
                f.write_str("(eval-error unexpected-switch-value (expected ")?;
                sexpr::write_quoted_str(f, expected)?;
                f.write_str(") (found ")?;
                sexpr::write_quoted_str(f, found)?;
                f.write_str("))")
            }
            Self::MissingSwitchCase { kind, detail } => {
                f.write_str("(eval-error missing-switch-case (kind ")?;
                sexpr::write_quoted_str(f, kind)?;
                f.write_str(") (detail ")?;
                sexpr::write_quoted_str(f, detail)?;
                f.write_str("))")
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// Successful match outcome yielding the matched `arm_index` and exported bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchOutcome {
    pub arm_index: usize,
    pub bindings: BTreeMap<String, Value>,
}

impl fmt::Display for MatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(match-outcome (arm {}) (bindings", self.arm_index)?;
        for (name, val) in &self.bindings {
            f.write_str(" (")?;
            sexpr::write_quoted_str(f, name)?;
            write!(f, " {val})")?;
        }
        f.write_str("))")
    }
}

/// Linear arm-by-arm reference matcher preserving exact source order and guard fallthrough.
pub fn eval_linear<F>(arms: &[MatchArm], value: &Value, mut guard: F) -> Option<MatchOutcome>
where
    F: FnMut(usize, &BTreeMap<String, Value>) -> bool,
{
    for arm in arms {
        if let Some(bindings) = match_pattern(&arm.pattern, value) {
            if !arm.has_guard || guard(arm.arm_index, &bindings) {
                return Some(MatchOutcome {
                    arm_index: arm.arm_index,
                    bindings,
                });
            }
        }
    }
    None
}

fn match_pattern(pattern: &Pattern, value: &Value) -> Option<BTreeMap<String, Value>> {
    let mut bindings = BTreeMap::new();
    if match_into(pattern, value, &mut bindings) {
        Some(bindings)
    } else {
        None
    }
}

fn match_into(pattern: &Pattern, value: &Value, out: &mut BTreeMap<String, Value>) -> bool {
    match pattern {
        Pattern::Wild => true,
        Pattern::Bind { name, subpattern } => {
            if match_into(subpattern, value, out) {
                out.insert(name.clone(), value.clone());
                true
            } else {
                false
            }
        }
        Pattern::Bool(expected) => matches!(value, Value::Bool(actual) if actual == expected),
        Pattern::Char(expected) => matches!(value, Value::Char(actual) if actual == expected),
        Pattern::CharRange { start, end } => {
            let Value::Char(ch) = value else {
                return false;
            };
            let intervals = CharInterval::from_bounds(*start, *end);
            intervals.iter().any(|iv| iv.contains(*ch))
        }
        Pattern::Int(expected) => {
            matches!(value, Value::Int(actual) if actual.to_finite() == expected.to_finite())
        }
        Pattern::IntRange { start, end } => {
            let Value::Int(actual) = value else {
                return false;
            };
            let val = actual.to_finite();
            let ge_start = match start {
                RangeBound::Unbounded => true,
                RangeBound::Inclusive(lo) => val >= lo.to_finite(),
                RangeBound::Exclusive(lo) => val >= lo.to_finite().add_one(),
            };
            let le_end = match end {
                RangeBound::Unbounded => true,
                RangeBound::Inclusive(hi) => val <= hi.to_finite(),
                RangeBound::Exclusive(hi) => val <= hi.to_finite().sub_one(),
            };
            ge_start && le_end
        }
        Pattern::OptionalNull => matches!(value, Value::OptionalNull),
        Pattern::OptionalPresent(inner_pat) => match value {
            Value::OptionalPresent(inner_val) => match_into(inner_pat, inner_val, out),
            _ => false,
        },
        Pattern::ErrorOk(inner_pat) => match value {
            Value::ErrorOk(inner_val) => match_into(inner_pat, inner_val, out),
            _ => false,
        },
        Pattern::ErrorErr { tag, inner } => match value {
            Value::ErrorErr {
                tag: v_tag,
                inner: v_inner,
            } => tag.as_ref().is_none_or(|t| t == v_tag) && match_into(inner, v_inner, out),
            _ => false,
        },
        Pattern::EnumVariant { tag, payload } => match value {
            Value::Enum {
                tag: v_tag,
                payload: v_payload,
            } if v_tag == tag => match (payload, v_payload) {
                (VariantPayloadPattern::Unit, EnumValuePayload::Unit) => true,
                (VariantPayloadPattern::Tuple(ps), EnumValuePayload::Tuple(vs)) => {
                    ps.len() == vs.len()
                        && ps.iter().zip(vs.iter()).all(|(p, v)| match_into(p, v, out))
                }
                (VariantPayloadPattern::Record(pf), EnumValuePayload::Record(vf)) => pf
                    .iter()
                    .all(|(k, p)| vf.get(k).is_some_and(|v| match_into(p, v, out))),
                _ => false,
            },
            _ => false,
        },
        Pattern::Tuple(ps) => match value {
            Value::Tuple(vs) => {
                ps.len() == vs.len() && ps.iter().zip(vs.iter()).all(|(p, v)| match_into(p, v, out))
            }
            _ => false,
        },
        Pattern::Record(pf) => match value {
            Value::Record(vf) => pf
                .iter()
                .all(|(k, p)| vf.get(k).is_some_and(|v| match_into(p, v, out))),
            _ => false,
        },
        Pattern::List { prefix, rest } => match value {
            Value::List(vs) => match rest {
                None => {
                    vs.len() == prefix.len()
                        && prefix
                            .iter()
                            .zip(vs.iter())
                            .all(|(p, v)| match_into(p, v, out))
                }
                Some(rest_pat) => {
                    if vs.len() < prefix.len() {
                        return false;
                    }
                    if !prefix
                        .iter()
                        .zip(vs[..prefix.len()].iter())
                        .all(|(p, v)| match_into(p, v, out))
                    {
                        return false;
                    }
                    let tail = Value::List(vs[prefix.len()..].to_vec());
                    match_into(rest_pat, &tail, out)
                }
            },
            _ => false,
        },
        Pattern::BitVec(bv_pat) => match value {
            Value::BitVec(bv) => {
                let expected = bv_pat.computed_width();
                if bv.width() != expected || bv_pat.total_width.is_some_and(|w| w != bv.width()) {
                    return false;
                }
                let mut hi = bv.width();
                for seg in &bv_pat.segments {
                    let w = seg.width();
                    let Some(lo) = hi.checked_sub(w) else {
                        return false;
                    };
                    let slice = bv.slice(lo, hi);
                    hi = lo;
                    match seg {
                        BitSegmentPattern::Fixed {
                            value: expected_bits,
                            ..
                        } => {
                            if slice != *expected_bits {
                                return false;
                            }
                        }
                        BitSegmentPattern::Typed { ty, pattern, .. } => {
                            let Some(slice_val) = slice_to_value(&slice, ty) else {
                                return false;
                            };
                            if !match_into(pattern, &slice_val, out) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        },
        Pattern::Or(alts) => {
            for alt in alts {
                let mut branch_bindings = out.clone();
                if match_into(alt, value, &mut branch_bindings) {
                    *out = branch_bindings;
                    return true;
                }
            }
            false
        }
    }
}

/// Compiled decision-DAG executor matching [`eval_linear`] across all patterns and guards.
pub fn eval_dag<F>(
    dag: &DecisionDag,
    value: &Value,
    mut guard: F,
) -> Result<Option<MatchOutcome>, EvalError>
where
    F: FnMut(usize, &BTreeMap<String, Value>) -> bool,
{
    let mut current = dag.root;
    let mut arm_env: BTreeMap<usize, BTreeMap<String, Value>> = BTreeMap::new();

    loop {
        let node = dag.nodes.get(current.0).ok_or(EvalError::InvalidNodeId {
            id: current.0,
            node_count: dag.nodes.len(),
        })?;
        match node {
            DagNode::Fail => return Ok(None),
            DagNode::Leaf {
                arm_index,
                bindings,
            } => {
                let mut final_bindings = arm_env.remove(arm_index).unwrap_or_default();
                eval_bindings_into(value, bindings, &mut final_bindings)?;
                return Ok(Some(MatchOutcome {
                    arm_index: *arm_index,
                    bindings: final_bindings,
                }));
            }
            DagNode::Bind {
                arm_index,
                bindings,
                next,
            } => {
                let entry = arm_env.entry(*arm_index).or_default();
                eval_bindings_into(value, bindings, entry)?;
                current = *next;
            }
            DagNode::GuardCheck {
                guard_id,
                bindings,
                on_pass,
                on_fail,
            } => {
                let mut guard_bindings = arm_env.get(guard_id).cloned().unwrap_or_default();
                eval_bindings_into(value, bindings, &mut guard_bindings)?;
                if guard(*guard_id, &guard_bindings) {
                    current = *on_pass;
                } else {
                    arm_env.remove(guard_id);
                    current = *on_fail;
                }
            }
            DagNode::Switch { place, kind } => {
                let target = project_place(value, place)?;
                current = select_switch_target(&target, kind)?;
            }
        }
    }
}

fn eval_bindings_into(
    root: &Value,
    bindings: &[Binding],
    out: &mut BTreeMap<String, Value>,
) -> Result<(), EvalError> {
    for binding in bindings {
        let val = project_place(root, &binding.place)?;
        out.insert(binding.name.clone(), val);
    }
    Ok(())
}

fn project_place(root: &Value, place: &Place) -> Result<Value, EvalError> {
    let mut cur = root.clone();
    for step in &place.steps {
        cur = match (cur, step) {
            (Value::Tuple(mut elems), PlaceStep::TupleField(idx)) => {
                if *idx >= elems.len() {
                    return Err(EvalError::IndexOutOfBounds {
                        context: "tuple-field",
                        index: *idx,
                        len: elems.len(),
                    });
                }
                elems.swap_remove(*idx)
            }
            (Value::Record(mut fields), PlaceStep::RecordField(name)) => fields
                .remove(name)
                .ok_or_else(|| EvalError::MissingRecordField {
                    field: name.clone(),
                })?,
            (Value::OptionalPresent(inner), PlaceStep::OptionalInner) => *inner,
            (Value::ErrorOk(inner), PlaceStep::ErrorOkInner) => *inner,
            (Value::ErrorErr { inner, .. }, PlaceStep::ErrorErrInner) => *inner,
            (
                Value::Enum {
                    payload: EnumValuePayload::Tuple(mut elems),
                    ..
                },
                PlaceStep::VariantTupleField { index, .. },
            ) => {
                if *index >= elems.len() {
                    return Err(EvalError::IndexOutOfBounds {
                        context: "variant-tuple-field",
                        index: *index,
                        len: elems.len(),
                    });
                }
                elems.swap_remove(*index)
            }
            (
                Value::Enum {
                    payload: EnumValuePayload::Record(mut fields),
                    ..
                },
                PlaceStep::VariantRecordField { field, .. },
            ) => fields
                .remove(field)
                .ok_or_else(|| EvalError::MissingRecordField {
                    field: field.clone(),
                })?,
            (Value::List(mut elems), PlaceStep::ListElem(idx)) => {
                if *idx >= elems.len() {
                    return Err(EvalError::IndexOutOfBounds {
                        context: "list-elem",
                        index: *idx,
                        len: elems.len(),
                    });
                }
                elems.swap_remove(*idx)
            }
            (Value::List(elems), PlaceStep::ListRest(from)) => {
                let rest = elems.get(*from..).ok_or(EvalError::IndexOutOfBounds {
                    context: "list-rest",
                    index: *from,
                    len: elems.len(),
                })?;
                Value::List(rest.to_vec())
            }
            (Value::BitVec(bv), PlaceStep::BitSlice { lo, hi, ty }) => {
                if *lo > *hi || *hi > bv.width() {
                    return Err(EvalError::InvalidBitSlice {
                        lo: *lo,
                        hi: *hi,
                        width: bv.width(),
                    });
                }
                let slice = bv.slice(*lo, *hi);
                slice_to_value(&slice, ty)
                    .ok_or(EvalError::InvalidBitSliceConversion { lo: *lo, hi: *hi })?
            }
            (val, step) => {
                return Err(EvalError::InvalidPlaceProjection {
                    step: step.to_string(),
                    value: val.to_string(),
                });
            }
        };
    }
    Ok(cur)
}

fn select_switch_target(
    target: &Value,
    kind: &SwitchKind,
) -> Result<crate::decision::NodeId, EvalError> {
    match kind {
        SwitchKind::Bool { on_false, on_true } => match target {
            Value::Bool(false) => Ok(*on_false),
            Value::Bool(true) => Ok(*on_true),
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "bool",
                found: other.to_string(),
            }),
        },
        SwitchKind::Optional {
            on_null,
            on_present,
        } => match target {
            Value::OptionalNull => Ok(*on_null),
            Value::OptionalPresent(_) => Ok(*on_present),
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "optional",
                found: other.to_string(),
            }),
        },
        SwitchKind::Error {
            on_ok,
            cases,
            default_err,
        } => match target {
            Value::ErrorOk(_) => Ok(*on_ok),
            Value::ErrorErr { tag, .. } => {
                cases.get(tag).copied().or(*default_err).ok_or_else(|| {
                    EvalError::MissingSwitchCase {
                        kind: "error",
                        detail: tag.to_string(),
                    }
                })
            }
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "error",
                found: other.to_string(),
            }),
        },
        SwitchKind::Enum { cases, default } => {
            match target {
                Value::Enum { tag, .. } => cases.get(tag).copied().or(*default).ok_or_else(|| {
                    EvalError::MissingSwitchCase {
                        kind: "enum",
                        detail: tag.to_string(),
                    }
                }),
                other => Err(EvalError::UnexpectedSwitchValue {
                    expected: "enum",
                    found: other.to_string(),
                }),
            }
        }
        SwitchKind::Int { cases, default } => match target {
            Value::Int(lit) => {
                let val = lit.to_finite();
                // Binary search sorted disjoint intervals.
                let idx = cases.partition_point(|(iv, _)| match iv.hi {
                    crate::pattern::UpperPoint::Finite(hi) => hi < val,
                    crate::pattern::UpperPoint::PosInf => false,
                });
                if idx < cases.len() && cases[idx].0.contains_finite(val) {
                    Ok(cases[idx].1)
                } else {
                    default.ok_or_else(|| EvalError::MissingSwitchCase {
                        kind: "int",
                        detail: lit.to_string(),
                    })
                }
            }
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "int",
                found: other.to_string(),
            }),
        },
        SwitchKind::Char { cases, default } => match target {
            Value::Char(ch) => {
                let idx = cases.partition_point(|(iv, _)| iv.hi < *ch);
                if idx < cases.len() && cases[idx].0.contains(*ch) {
                    Ok(cases[idx].1)
                } else {
                    default.ok_or_else(|| EvalError::MissingSwitchCase {
                        kind: "char",
                        detail: ch.to_string(),
                    })
                }
            }
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "char",
                found: other.to_string(),
            }),
        },
        SwitchKind::ListLen { cases, default } => match target {
            Value::List(elems) => {
                let len = elems.len();
                let idx =
                    cases.partition_point(|(iv, _)| iv.max_inclusive.is_some_and(|max| max < len));
                if idx < cases.len() && cases[idx].0.contains(len) {
                    Ok(cases[idx].1)
                } else {
                    default.ok_or_else(|| EvalError::MissingSwitchCase {
                        kind: "list-len",
                        detail: len.to_string(),
                    })
                }
            }
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "list",
                found: other.to_string(),
            }),
        },
        SwitchKind::BitSlice {
            lo,
            hi,
            cases,
            default,
        } => match target {
            Value::BitVec(bv) => {
                if *lo > *hi || *hi > bv.width() {
                    return Err(EvalError::InvalidBitSlice {
                        lo: *lo,
                        hi: *hi,
                        width: bv.width(),
                    });
                }
                let slice = bv.slice(*lo, *hi);
                let idx = cases.partition_point(|(iv, _)| iv.hi < slice);
                if idx < cases.len() && cases[idx].0.contains(&slice) {
                    Ok(cases[idx].1)
                } else {
                    default.ok_or_else(|| EvalError::MissingSwitchCase {
                        kind: "bit-slice",
                        detail: slice.to_string(),
                    })
                }
            }
            other => Err(EvalError::UnexpectedSwitchValue {
                expected: "bitvec",
                found: other.to_string(),
            }),
        },
    }
}
