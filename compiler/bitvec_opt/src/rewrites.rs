//! Verified algebraic rewrite rules and idiom recognizers with explicit certificates.
//!
//! Every simplification attaches an explicit [`Justification`] certificate
//! explaining the algebraic axiom or derivation. Pure rewrites strictly preserve
//! [`BehaviorBoundary`](crate::ir::BehaviorBoundary) events, checks, and cleanups.

use std::collections::BTreeMap;
use std::fmt::{self, Display};

use crate::ir::{BitValue, ExtractStart, IrError, Node, NodeId, Region};
use crate::sexpr::{Quoted, write_list};
use crate::zero_width::compact_reachable;

/// Formal justification certificate attached to a rewrite or recognized idiom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Justification {
    Axiom {
        rule: &'static str,
        theorem: &'static str,
        detail: String,
    },
    Derived {
        rule: &'static str,
        premises: Vec<Justification>,
        detail: String,
    },
    SmtVerified {
        solver: &'static str,
        summary: String,
    },
}

impl Display for Justification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Axiom {
                rule,
                theorem,
                detail,
            } => write!(
                f,
                "(axiom (rule {}) (theorem {}) (detail {}))",
                Quoted(rule),
                Quoted(theorem),
                Quoted(detail)
            ),
            Self::Derived {
                rule,
                premises,
                detail,
            } => {
                write!(f, "(derived (rule {}) (premises ", Quoted(rule))?;
                write_list(f, premises)?;
                write!(f, ") (detail {}))", Quoted(detail))
            }
            Self::SmtVerified { solver, summary } => write!(
                f,
                "(smt-verified (solver {}) (summary {}))",
                Quoted(solver),
                Quoted(summary)
            ),
        }
    }
}

/// Record of one algebraic rewrite step performed on the pure DAG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteStep {
    pub rule: &'static str,
    pub original_node: NodeId,
    pub rewritten_node: NodeId,
    pub justification: Justification,
}

impl Display for RewriteStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(rewrite-step (rule {}) (from {}) (to {}) (justification {}))",
            Quoted(self.rule),
            self.original_node,
            self.rewritten_node,
            self.justification
        )
    }
}

/// Target-level algebraic idiom recognized in a pure BitVec DAG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Xor {
        lhs: NodeId,
        rhs: NodeId,
        width: u32,
    },
    Bswap16 {
        input: NodeId,
    },
    Bswap32 {
        input: NodeId,
    },
    Bswap64 {
        input: NodeId,
    },
    RotateRight {
        input: NodeId,
        rotr_amount: u32,
        rotl_amount: u32,
        width: u32,
    },
    ShlConst {
        input: NodeId,
        amount: u32,
        width: u32,
    },
    LshrConst {
        input: NodeId,
        amount: u32,
        width: u32,
    },
    AndNot {
        lhs: NodeId,
        inverted: NodeId,
        width: u32,
    },
    OrNot {
        lhs: NodeId,
        inverted: NodeId,
        width: u32,
    },
}

impl Display for PatternKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xor { lhs, rhs, width } => {
                write!(f, "(pattern-xor {lhs} {rhs} (width {width}))")
            }
            Self::Bswap16 { input } => write!(f, "(pattern-bswap16 {input})"),
            Self::Bswap32 { input } => write!(f, "(pattern-bswap32 {input})"),
            Self::Bswap64 { input } => write!(f, "(pattern-bswap64 {input})"),
            Self::RotateRight {
                input,
                rotr_amount,
                rotl_amount,
                width,
            } => write!(
                f,
                "(pattern-rotate {input} (rotr {rotr_amount}) (rotl {rotl_amount}) (width {width}))"
            ),
            Self::ShlConst {
                input,
                amount,
                width,
            } => write!(
                f,
                "(pattern-shl-const {input} (amount {amount}) (width {width}))"
            ),
            Self::LshrConst {
                input,
                amount,
                width,
            } => write!(
                f,
                "(pattern-lshr-const {input} (amount {amount}) (width {width}))"
            ),
            Self::AndNot {
                lhs,
                inverted,
                width,
            } => write!(f, "(pattern-andn {lhs} {inverted} (width {width}))"),
            Self::OrNot {
                lhs,
                inverted,
                width,
            } => write!(f, "(pattern-orn {lhs} {inverted} (width {width}))"),
        }
    }
}

/// Recognized pattern at a specific DAG node paired with its justification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecognizedPattern {
    pub node: NodeId,
    pub kind: PatternKind,
    pub justification: Justification,
}

impl Display for RecognizedPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(recognized-pattern (node {}) (kind {}) (justification {}))",
            self.node, self.kind, self.justification
        )
    }
}

/// Output of the algebraic rewrite pass on a [`Region`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteResult {
    pub region: Region,
    pub steps: Vec<RewriteStep>,
    pub patterns: Vec<RecognizedPattern>,
}

impl Display for RewriteResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(rewrite-result (steps ")?;
        write_list(f, &self.steps)?;
        f.write_str(") (patterns ")?;
        write_list(f, &self.patterns)?;
        write!(f, ") (region {}))", self.region)
    }
}

/// Applies verified core algebraic rewrites to fixpoint on `region`.
///
/// Simplifications strictly preserve `region.boundary` (`inputs`, `checks`,
/// `cleanups`) and `region.domain`, and never merge distinct `InputIdentity`
/// memory versions.
pub fn simplify_region(region: &Region) -> Result<RewriteResult, IrError> {
    region.validate()?;
    let mut current = region.clone();
    let mut all_steps = Vec::new();
    const MAX_PASSES: usize = 16;

    for _ in 0..MAX_PASSES {
        let (next_region, mut pass_steps) = simplify_once(&current)?;
        if pass_steps.is_empty() {
            current = next_region;
            break;
        }
        all_steps.append(&mut pass_steps);
        current = next_region;
    }

    let patterns = recognize_patterns(&current)?;
    Ok(RewriteResult {
        region: current,
        steps: all_steps,
        patterns,
    })
}

fn simplify_once(region: &Region) -> Result<(Region, Vec<RewriteStep>), IrError> {
    let mut builder = DagRewriter::new();
    let mut remap = vec![NodeId(0); region.nodes.len()];
    let mut steps = Vec::new();

    for (idx, node) in region.nodes.iter().enumerate() {
        let orig_id = NodeId(idx);
        let rewritten = match node {
            Node::Input { width, identity } => builder.intern(
                Node::Input {
                    width: *width,
                    identity: identity.clone(),
                },
                *width,
            ),
            Node::Constant { width, bits } => builder.intern(
                Node::Constant {
                    width: *width,
                    bits: bits.clone(),
                },
                *width,
            ),
            Node::Not(inner) => {
                let x = remap[inner.0];
                let w = builder.width(x);
                match builder.node(x).clone() {
                    Node::Constant { bits, .. } => {
                        let folded = bits.bit_not();
                        let res = builder.constant(folded);
                        steps.push(RewriteStep {
                            rule: "const-fold-not",
                            original_node: orig_id,
                            rewritten_node: res,
                            justification: Justification::Axiom {
                                rule: "const-fold-not",
                                theorem: "not(C) = bit_not(C)",
                                detail: format!("folded {w}-bit constant inversion"),
                            },
                        });
                        res
                    }
                    Node::Not(inner2) => {
                        steps.push(RewriteStep {
                            rule: "not-not-involution",
                            original_node: orig_id,
                            rewritten_node: inner2,
                            justification: Justification::Axiom {
                                rule: "not-not-involution",
                                theorem: "not(not(x)) = x",
                                detail: format!("double negation on {w}-bit value"),
                            },
                        });
                        inner2
                    }
                    _ => builder.intern(Node::Not(x), w),
                }
            }
            Node::And(l, r) => {
                let a = remap[l.0];
                let b = remap[r.0];
                let w = builder.width(a);
                if a == b {
                    steps.push(RewriteStep {
                        rule: "and-idempotent",
                        original_node: orig_id,
                        rewritten_node: a,
                        justification: Justification::Axiom {
                            rule: "and-idempotent",
                            theorem: "x & x = x",
                            detail: format!("idempotent conjunction on {w}-bit value"),
                        },
                    });
                    a
                } else if let (Node::Constant { bits: lb, .. }, Node::Constant { bits: rb, .. }) =
                    (builder.node(a).clone(), builder.node(b).clone())
                {
                    let res = builder.constant(lb.bit_and(&rb)?);
                    steps.push(RewriteStep {
                        rule: "const-fold-and",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "const-fold-and",
                            theorem: "C1 & C2 = bit_and(C1, C2)",
                            detail: format!("folded {w}-bit constant conjunction"),
                        },
                    });
                    res
                } else if is_zero_const(builder.node(a)) || is_zero_const(builder.node(b)) {
                    let res = builder.constant(BitValue::zeros(w));
                    steps.push(RewriteStep {
                        rule: "and-zero-annihilator",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "and-zero-annihilator",
                            theorem: "x & 0 = 0 (boundary inputs preserved)",
                            detail: format!("annihilated {w}-bit conjunction with zero"),
                        },
                    });
                    res
                } else if is_ones_const(builder.node(a)) {
                    steps.push(RewriteStep {
                        rule: "and-ones-identity",
                        original_node: orig_id,
                        rewritten_node: b,
                        justification: Justification::Axiom {
                            rule: "and-ones-identity",
                            theorem: "all_ones & x = x",
                            detail: format!("identity conjunction on {w}-bit value"),
                        },
                    });
                    b
                } else if is_ones_const(builder.node(b)) {
                    steps.push(RewriteStep {
                        rule: "and-ones-identity",
                        original_node: orig_id,
                        rewritten_node: a,
                        justification: Justification::Axiom {
                            rule: "and-ones-identity",
                            theorem: "x & all_ones = x",
                            detail: format!("identity conjunction on {w}-bit value"),
                        },
                    });
                    a
                } else if is_complement_pair(builder.nodes(), a, b) {
                    let res = builder.constant(BitValue::zeros(w));
                    steps.push(RewriteStep {
                        rule: "and-complement-zero",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "and-complement-zero",
                            theorem: "x & not(x) = 0",
                            detail: format!("complementary conjunction on {w}-bit value"),
                        },
                    });
                    res
                } else if let (Node::Not(na), Node::Not(nb)) =
                    (builder.node(a).clone(), builder.node(b).clone())
                {
                    let inner_or = builder.intern(Node::Or(na, nb), w);
                    let res = builder.intern(Node::Not(inner_or), w);
                    steps.push(RewriteStep {
                        rule: "de-morgan-and-of-nots",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "de-morgan-and-of-nots",
                            theorem: "not(a) & not(b) = not(a | b)",
                            detail: format!("De Morgan reduction on {w}-bit value"),
                        },
                    });
                    res
                } else {
                    builder.intern(Node::And(a, b), w)
                }
            }
            Node::Or(l, r) => {
                let a = remap[l.0];
                let b = remap[r.0];
                let w = builder.width(a);
                if a == b {
                    steps.push(RewriteStep {
                        rule: "or-idempotent",
                        original_node: orig_id,
                        rewritten_node: a,
                        justification: Justification::Axiom {
                            rule: "or-idempotent",
                            theorem: "x | x = x",
                            detail: format!("idempotent disjunction on {w}-bit value"),
                        },
                    });
                    a
                } else if let (Node::Constant { bits: lb, .. }, Node::Constant { bits: rb, .. }) =
                    (builder.node(a).clone(), builder.node(b).clone())
                {
                    let res = builder.constant(lb.bit_or(&rb)?);
                    steps.push(RewriteStep {
                        rule: "const-fold-or",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "const-fold-or",
                            theorem: "C1 | C2 = bit_or(C1, C2)",
                            detail: format!("folded {w}-bit constant disjunction"),
                        },
                    });
                    res
                } else if is_ones_const(builder.node(a)) || is_ones_const(builder.node(b)) {
                    let res = builder.constant(BitValue::ones(w));
                    steps.push(RewriteStep {
                        rule: "or-ones-annihilator",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "or-ones-annihilator",
                            theorem: "x | all_ones = all_ones (boundary inputs preserved)",
                            detail: format!("annihilated {w}-bit disjunction with all-ones"),
                        },
                    });
                    res
                } else if is_zero_const(builder.node(a)) {
                    steps.push(RewriteStep {
                        rule: "or-zero-identity",
                        original_node: orig_id,
                        rewritten_node: b,
                        justification: Justification::Axiom {
                            rule: "or-zero-identity",
                            theorem: "0 | x = x",
                            detail: format!("identity disjunction on {w}-bit value"),
                        },
                    });
                    b
                } else if is_zero_const(builder.node(b)) {
                    steps.push(RewriteStep {
                        rule: "or-zero-identity",
                        original_node: orig_id,
                        rewritten_node: a,
                        justification: Justification::Axiom {
                            rule: "or-zero-identity",
                            theorem: "x | 0 = x",
                            detail: format!("identity disjunction on {w}-bit value"),
                        },
                    });
                    a
                } else if is_complement_pair(builder.nodes(), a, b) {
                    let res = builder.constant(BitValue::ones(w));
                    steps.push(RewriteStep {
                        rule: "or-complement-ones",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "or-complement-ones",
                            theorem: "x | not(x) = all_ones",
                            detail: format!("complementary disjunction on {w}-bit value"),
                        },
                    });
                    res
                } else if let (Node::Not(na), Node::Not(nb)) =
                    (builder.node(a).clone(), builder.node(b).clone())
                {
                    let inner_and = builder.intern(Node::And(na, nb), w);
                    let res = builder.intern(Node::Not(inner_and), w);
                    steps.push(RewriteStep {
                        rule: "de-morgan-or-of-nots",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "de-morgan-or-of-nots",
                            theorem: "not(a) | not(b) = not(a & b)",
                            detail: format!("De Morgan reduction on {w}-bit value"),
                        },
                    });
                    res
                } else {
                    builder.intern(Node::Or(a, b), w)
                }
            }
            Node::Equal(l, r) => {
                let a = remap[l.0];
                let b = remap[r.0];
                if a == b {
                    let res = builder.constant(BitValue::from_bool(true));
                    steps.push(RewriteStep {
                        rule: "equal-reflexive",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "equal-reflexive",
                            theorem: "x == x = true",
                            detail: String::from("reflexive equality on identical DAG node"),
                        },
                    });
                    res
                } else if let (Node::Constant { bits: lb, .. }, Node::Constant { bits: rb, .. }) =
                    (builder.node(a).clone(), builder.node(b).clone())
                {
                    let res = builder.constant(BitValue::from_bool(lb == rb));
                    steps.push(RewriteStep {
                        rule: "const-fold-equal",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "const-fold-equal",
                            theorem: "(C1 == C2) = bool_const(C1 == C2)",
                            detail: String::from("folded constant equality"),
                        },
                    });
                    res
                } else {
                    builder.intern(Node::Equal(a, b), 1)
                }
            }
            Node::Concat(hi, lo) => {
                let m_hi = remap[hi.0];
                let m_lo = remap[lo.0];
                let w_hi = builder.width(m_hi);
                let w_lo = builder.width(m_lo);
                if w_hi == 0 {
                    steps.push(RewriteStep {
                        rule: "concat-empty-high",
                        original_node: orig_id,
                        rewritten_node: m_lo,
                        justification: Justification::Axiom {
                            rule: "concat-empty-high",
                            theorem: "empty ++ x = x",
                            detail: String::from("eliminated 0-width high concat operand"),
                        },
                    });
                    m_lo
                } else if w_lo == 0 {
                    steps.push(RewriteStep {
                        rule: "concat-empty-low",
                        original_node: orig_id,
                        rewritten_node: m_hi,
                        justification: Justification::Axiom {
                            rule: "concat-empty-low",
                            theorem: "x ++ empty = x",
                            detail: String::from("eliminated 0-width low concat operand"),
                        },
                    });
                    m_hi
                } else if let (Node::Constant { bits: hb, .. }, Node::Constant { bits: lb, .. }) =
                    (builder.node(m_hi).clone(), builder.node(m_lo).clone())
                {
                    let res = builder.constant(hb.concat(&lb)?);
                    steps.push(RewriteStep {
                        rule: "const-fold-concat",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "const-fold-concat",
                            theorem: "C_hi ++ C_lo = concat(C_hi, C_lo)",
                            detail: format!("folded ({w_hi} ++ {w_lo})-bit constant concat"),
                        },
                    });
                    res
                } else if let (
                    Node::Extract {
                        input: x1,
                        start: ExtractStart::Const(s_hi),
                        width: w1,
                    },
                    Node::Extract {
                        input: x2,
                        start: ExtractStart::Const(s_lo),
                        width: w2,
                    },
                ) = (builder.node(m_hi).clone(), builder.node(m_lo).clone())
                    && x1 == x2
                    && s_hi == s_lo + w2
                {
                    let total_w = w1 + w2;
                    let src_w = builder.width(x1);
                    let res = if s_lo == 0 && total_w == src_w {
                        x1
                    } else {
                        builder.intern(
                            Node::Extract {
                                input: x1,
                                start: ExtractStart::Const(s_lo),
                                width: total_w,
                            },
                            total_w,
                        )
                    };
                    steps.push(RewriteStep {
                        rule: "concat-adjacent-slices",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "concat-adjacent-slices",
                            theorem: "x[b..c] ++ x[a..b] = x[a..c]",
                            detail: format!(
                                "coalesced adjacent slices [{s_lo}..{s_hi}) and [{s_hi}..{})",
                                s_hi + w1
                            ),
                        },
                    });
                    res
                } else {
                    builder.intern(Node::Concat(m_hi, m_lo), w_hi + w_lo)
                }
            }
            Node::Extract {
                input,
                start,
                width,
            } => {
                let m_in = remap[input.0];
                let in_w = builder.width(m_in);
                match start {
                    ExtractStart::Const(lo) => {
                        if *lo == 0 && *width == in_w {
                            steps.push(RewriteStep {
                                rule: "extract-full-width",
                                original_node: orig_id,
                                rewritten_node: m_in,
                                justification: Justification::Axiom {
                                    rule: "extract-full-width",
                                    theorem: "x[0..n] = x when width(x) == n",
                                    detail: format!("eliminated identity {in_w}-bit extract"),
                                },
                            });
                            m_in
                        } else if let Node::Constant { bits, .. } = builder.node(m_in).clone() {
                            let res = builder.constant(bits.extract(*lo, *width)?);
                            steps.push(RewriteStep {
                                rule: "const-fold-extract",
                                original_node: orig_id,
                                rewritten_node: res,
                                justification: Justification::Axiom {
                                    rule: "const-fold-extract",
                                    theorem: "C[lo..lo+w] = extract(C, lo, w)",
                                    detail: format!(
                                        "folded constant slice [{lo}..{})",
                                        *lo + *width
                                    ),
                                },
                            });
                            res
                        } else if let Node::Extract {
                            input: inner,
                            start: ExtractStart::Const(inner_lo),
                            ..
                        } = builder.node(m_in).clone()
                        {
                            let combined_lo = inner_lo + *lo;
                            let inner_w = builder.width(inner);
                            let res = if combined_lo == 0 && *width == inner_w {
                                inner
                            } else {
                                builder.intern(
                                    Node::Extract {
                                        input: inner,
                                        start: ExtractStart::Const(combined_lo),
                                        width: *width,
                                    },
                                    *width,
                                )
                            };
                            steps.push(RewriteStep {
                                rule: "extract-coalesce",
                                original_node: orig_id,
                                rewritten_node: res,
                                justification: Justification::Axiom {
                                    rule: "extract-coalesce",
                                    theorem: "(x[a..b])[c..d] = x[a+c..a+d]",
                                    detail: format!(
                                        "coalesced nested slice to [{combined_lo}..{})",
                                        combined_lo + *width
                                    ),
                                },
                            });
                            res
                        } else if let Node::Concat(hi, c_lo) = builder.node(m_in).clone() {
                            let w_lo = builder.width(c_lo);
                            let end = *lo + *width;
                            if end <= w_lo {
                                let res = if *lo == 0 && *width == w_lo {
                                    c_lo
                                } else {
                                    builder.intern(
                                        Node::Extract {
                                            input: c_lo,
                                            start: ExtractStart::Const(*lo),
                                            width: *width,
                                        },
                                        *width,
                                    )
                                };
                                steps.push(RewriteStep {
                                    rule: "concat-extract-low",
                                    original_node: orig_id,
                                    rewritten_node: res,
                                    justification: Justification::Axiom {
                                        rule: "concat-extract-low",
                                        theorem: "(a ++ b)[lo..hi] = b[lo..hi] when hi <= width(b)",
                                        detail: format!(
                                            "bypassed high concat operand for [{lo}..{end})"
                                        ),
                                    },
                                });
                                res
                            } else if *lo >= w_lo {
                                let shifted_lo = *lo - w_lo;
                                let w_hi = builder.width(hi);
                                let res = if shifted_lo == 0 && *width == w_hi {
                                    hi
                                } else {
                                    builder.intern(
                                        Node::Extract {
                                            input: hi,
                                            start: ExtractStart::Const(shifted_lo),
                                            width: *width,
                                        },
                                        *width,
                                    )
                                };
                                steps.push(RewriteStep {
                                    rule: "concat-extract-high",
                                    original_node: orig_id,
                                    rewritten_node: res,
                                    justification: Justification::Axiom {
                                        rule: "concat-extract-high",
                                        theorem:
                                            "(a ++ b)[lo..hi] = a[lo-wb..hi-wb] when lo >= width(b)",
                                        detail: format!("bypassed low concat operand for [{lo}..{end})"),
                                    },
                                });
                                res
                            } else {
                                builder.intern(
                                    Node::Extract {
                                        input: m_in,
                                        start: ExtractStart::Const(*lo),
                                        width: *width,
                                    },
                                    *width,
                                )
                            }
                        } else if let Node::ZeroExtend { input: inner, .. } =
                            builder.node(m_in).clone()
                        {
                            let inner_w = builder.width(inner);
                            let end = *lo + *width;
                            if end <= inner_w {
                                let res = if *lo == 0 && *width == inner_w {
                                    inner
                                } else {
                                    builder.intern(
                                        Node::Extract {
                                            input: inner,
                                            start: ExtractStart::Const(*lo),
                                            width: *width,
                                        },
                                        *width,
                                    )
                                };
                                steps.push(RewriteStep {
                                    rule: "zext-extract-low",
                                    original_node: orig_id,
                                    rewritten_node: res,
                                    justification: Justification::Axiom {
                                        rule: "zext-extract-low",
                                        theorem: "zext(x)[lo..hi] = x[lo..hi] when hi <= width(x)",
                                        detail: format!(
                                            "eliminated zero-extend below slice [{lo}..{end})"
                                        ),
                                    },
                                });
                                res
                            } else if *lo >= inner_w {
                                let res = builder.constant(BitValue::zeros(*width));
                                steps.push(RewriteStep {
                                    rule: "zext-extract-padding",
                                    original_node: orig_id,
                                    rewritten_node: res,
                                    justification: Justification::Axiom {
                                        rule: "zext-extract-padding",
                                        theorem: "zext(x)[lo..hi] = 0 when lo >= width(x)",
                                        detail: format!(
                                            "folded high zero-extension slice [{lo}..{end}) to 0"
                                        ),
                                    },
                                });
                                res
                            } else {
                                builder.intern(
                                    Node::Extract {
                                        input: m_in,
                                        start: ExtractStart::Const(*lo),
                                        width: *width,
                                    },
                                    *width,
                                )
                            }
                        } else {
                            builder.intern(
                                Node::Extract {
                                    input: m_in,
                                    start: ExtractStart::Const(*lo),
                                    width: *width,
                                },
                                *width,
                            )
                        }
                    }
                    ExtractStart::Dynamic(idx_node) => {
                        let m_idx = remap[idx_node.0];
                        if let Node::Constant { bits, .. } = builder.node(m_idx).clone()
                            && let Some(const_lo) = bits.to_u64()
                            && let Ok(const_lo_u32) = u32::try_from(const_lo)
                            && const_lo_u32.checked_add(*width).is_some_and(|e| e <= in_w)
                        {
                            let res = builder.intern(
                                Node::Extract {
                                    input: m_in,
                                    start: ExtractStart::Const(const_lo_u32),
                                    width: *width,
                                },
                                *width,
                            );
                            steps.push(RewriteStep {
                                rule: "extract-dynamic-const-start",
                                original_node: orig_id,
                                rewritten_node: res,
                                justification: Justification::Axiom {
                                    rule: "extract-dynamic-const-start",
                                    theorem: "x[const_k .. const_k + w] = static_extract(x, k, w)",
                                    detail: format!(
                                        "specialized constant dynamic start {const_lo_u32}"
                                    ),
                                },
                            });
                            res
                        } else {
                            builder.intern(
                                Node::Extract {
                                    input: m_in,
                                    start: ExtractStart::Dynamic(m_idx),
                                    width: *width,
                                },
                                *width,
                            )
                        }
                    }
                }
            }
            Node::ZeroExtend { input, width } => {
                let m_in = remap[input.0];
                let in_w = builder.width(m_in);
                if *width == in_w {
                    steps.push(RewriteStep {
                        rule: "zext-same-width",
                        original_node: orig_id,
                        rewritten_node: m_in,
                        justification: Justification::Axiom {
                            rule: "zext-same-width",
                            theorem: "zext(x, width(x)) = x",
                            detail: format!("eliminated no-op zero-extend of {in_w}-bit value"),
                        },
                    });
                    m_in
                } else if let Node::Constant { bits, .. } = builder.node(m_in).clone() {
                    let res = builder.constant(bits.zero_extend(*width)?);
                    steps.push(RewriteStep {
                        rule: "const-fold-zext",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "const-fold-zext",
                            theorem: "zext(C, w) = zero_extend(C, w)",
                            detail: format!("folded {in_w}->{width} constant zero-extend"),
                        },
                    });
                    res
                } else if let Node::ZeroExtend { input: inner, .. } = builder.node(m_in).clone() {
                    let res = builder.intern(
                        Node::ZeroExtend {
                            input: inner,
                            width: *width,
                        },
                        *width,
                    );
                    steps.push(RewriteStep {
                        rule: "zext-chain",
                        original_node: orig_id,
                        rewritten_node: res,
                        justification: Justification::Axiom {
                            rule: "zext-chain",
                            theorem: "zext(zext(x, w1), w2) = zext(x, w2)",
                            detail: format!("collapsed chained zero-extension to {width} bits"),
                        },
                    });
                    res
                } else {
                    builder.intern(
                        Node::ZeroExtend {
                            input: m_in,
                            width: *width,
                        },
                        *width,
                    )
                }
            }
        };
        remap[idx] = rewritten;
    }

    let next_root = remap[region.root.0];
    let compacted = compact_reachable(
        &region.inputs,
        &region.domain,
        builder.nodes(),
        next_root,
        region.output_kind,
        region.boundary.clone(),
    )?;
    compacted.validate()?;
    Ok((compacted, steps))
}

/// Recognizes target-level idioms (`xor`, `bswap16/32/64`, `rotr`/`rotl`, `shl`, `lshr`, `andn`, `orn`)
/// across all nodes in a validated [`Region`].
pub fn recognize_patterns(region: &Region) -> Result<Vec<RecognizedPattern>, IrError> {
    region.validate()?;
    let widths = region.node_widths()?;
    let mut found = Vec::new();

    for (idx, node) in region.nodes.iter().enumerate() {
        let nid = NodeId(idx);
        let w = widths[idx];

        if let Some((lhs, rhs)) = match_xor_pattern(&region.nodes, node) {
            found.push(RecognizedPattern {
                node: nid,
                kind: PatternKind::Xor { lhs, rhs, width: w },
                justification: Justification::Axiom {
                    rule: "xor-pattern",
                    theorem: "((a & !b) | (!a & b)) = xor(a, b)",
                    detail: format!("matched {w}-bit XOR synthesis idiom"),
                },
            });
        }

        if let Some((lhs, inverted)) = match_and_not_pattern(&region.nodes, node) {
            found.push(RecognizedPattern {
                node: nid,
                kind: PatternKind::AndNot {
                    lhs,
                    inverted,
                    width: w,
                },
                justification: Justification::Axiom {
                    rule: "and-not-pattern",
                    theorem: "a & not(b) = andn(a, b)",
                    detail: format!("matched {w}-bit and-not idiom"),
                },
            });
        }

        if let Some((lhs, inverted)) = match_or_not_pattern(&region.nodes, node) {
            found.push(RecognizedPattern {
                node: nid,
                kind: PatternKind::OrNot {
                    lhs,
                    inverted,
                    width: w,
                },
                justification: Justification::Axiom {
                    rule: "or-not-pattern",
                    theorem: "a | not(b) = orn(a, b)",
                    detail: format!("matched {w}-bit or-not idiom"),
                },
            });
        }

        if let Some(bswap_kind) = match_bswap_pattern(&region.nodes, &widths, nid) {
            let (rule, theorem, detail) = match bswap_kind {
                PatternKind::Bswap16 { .. } => (
                    "bswap16-pattern",
                    "bits[0..8] ++ bits[8..16] = bswap16(bits)",
                    "matched 16-bit byte swap",
                ),
                PatternKind::Bswap32 { .. } => (
                    "bswap32-pattern",
                    "bits[0..8] ++ bits[8..16] ++ bits[16..24] ++ bits[24..32] = bswap32(bits)",
                    "matched 32-bit byte swap",
                ),
                PatternKind::Bswap64 { .. } => (
                    "bswap64-pattern",
                    "reversed 8-byte concat = bswap64(bits)",
                    "matched 64-bit byte swap",
                ),
                _ => unreachable!(),
            };
            found.push(RecognizedPattern {
                node: nid,
                kind: bswap_kind,
                justification: Justification::Axiom {
                    rule,
                    theorem,
                    detail: String::from(detail),
                },
            });
        }

        if let Some(rot_kind) = match_rotate_pattern(&region.nodes, &widths, nid) {
            let PatternKind::RotateRight {
                rotr_amount,
                rotl_amount,
                width,
                ..
            } = rot_kind
            else {
                unreachable!();
            };
            found.push(RecognizedPattern {
                node: nid,
                kind: rot_kind,
                justification: Justification::Axiom {
                    rule: "rotate-pattern",
                    theorem: "x[0..k] ++ x[k..n] = rotr(x, k) = rotl(x, n - k)",
                    detail: format!(
                        "matched {width}-bit rotate (rotr {rotr_amount} / rotl {rotl_amount})"
                    ),
                },
            });
        }

        if let Some(shift_kind) = match_shift_pattern(&region.nodes, &widths, nid) {
            let (rule, theorem, detail) = match &shift_kind {
                PatternKind::ShlConst { amount, width, .. } => (
                    "shl-const-pattern",
                    "x[0..n-k] ++ 0_k = shl(x, k)",
                    format!("matched {width}-bit logical shift left by {amount}"),
                ),
                PatternKind::LshrConst { amount, width, .. } => (
                    "lshr-const-pattern",
                    "zext(x[k..n], n) = lshr(x, k)",
                    format!("matched {width}-bit logical shift right by {amount}"),
                ),
                _ => unreachable!(),
            };
            found.push(RecognizedPattern {
                node: nid,
                kind: shift_kind,
                justification: Justification::Axiom {
                    rule,
                    theorem,
                    detail,
                },
            });
        }
    }

    Ok(found)
}

fn match_xor_pattern(nodes: &[Node], node: &Node) -> Option<(NodeId, NodeId)> {
    match node {
        // ((a & !b) | (!a & b)) with arbitrary operand orderings.
        Node::Or(l, r) => {
            let (a1, neg_b1) = match_and_with_not(nodes, *l)?;
            let (a2, neg_b2) = match_and_with_not(nodes, *r)?;
            if a1 == neg_b2 && a2 == neg_b1 {
                let (lhs, rhs) = if a1 <= a2 { (a1, a2) } else { (a2, a1) };
                return Some((lhs, rhs));
            }
            None
        }
        // ((a | b) & !(a & b)) or ((a | b) & (!a | !b))
        Node::And(l, r) => {
            if let Some(pair) = match_or_and_nand(nodes, *l, *r) {
                return Some(pair);
            }
            match_or_and_nand(nodes, *r, *l)
        }
        _ => None,
    }
}

fn match_or_and_nand(nodes: &[Node], or_id: NodeId, other_id: NodeId) -> Option<(NodeId, NodeId)> {
    let Node::Or(a, b) = nodes[or_id.0] else {
        return None;
    };
    match nodes[other_id.0] {
        Node::Not(inner) => {
            let Node::And(x, y) = nodes[inner.0] else {
                return None;
            };
            if (a == x && b == y) || (a == y && b == x) {
                let (lhs, rhs) = if a <= b { (a, b) } else { (b, a) };
                Some((lhs, rhs))
            } else {
                None
            }
        }
        Node::Or(x, y) => {
            let (Node::Not(nx), Node::Not(ny)) = (nodes[x.0].clone(), nodes[y.0].clone()) else {
                return None;
            };
            if (a == nx && b == ny) || (a == ny && b == nx) {
                let (lhs, rhs) = if a <= b { (a, b) } else { (b, a) };
                Some((lhs, rhs))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn match_and_with_not(nodes: &[Node], id: NodeId) -> Option<(NodeId, NodeId)> {
    let Node::And(l, r) = nodes[id.0] else {
        return None;
    };
    if let Node::Not(inner) = nodes[r.0] {
        return Some((l, inner));
    }
    if let Node::Not(inner) = nodes[l.0] {
        return Some((r, inner));
    }
    None
}

fn match_and_not_pattern(nodes: &[Node], node: &Node) -> Option<(NodeId, NodeId)> {
    let Node::And(l, r) = node else {
        return None;
    };
    if let Node::Not(inv) = nodes[r.0] {
        return Some((*l, inv));
    }
    if let Node::Not(inv) = nodes[l.0] {
        return Some((*r, inv));
    }
    None
}

fn match_or_not_pattern(nodes: &[Node], node: &Node) -> Option<(NodeId, NodeId)> {
    let Node::Or(l, r) = node else {
        return None;
    };
    if let Node::Not(inv) = nodes[r.0] {
        return Some((*l, inv));
    }
    if let Node::Not(inv) = nodes[l.0] {
        return Some((*r, inv));
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConcatPiece {
    Slice {
        input: NodeId,
        start: u32,
        width: u32,
    },
    ZeroConst {
        width: u32,
    },
    Other,
}

/// Flattens a `Concat` tree from high bits to low bits and coalesces adjacent slices.
fn flatten_concat_pieces(nodes: &[Node], widths: &[u32], id: NodeId) -> Vec<ConcatPiece> {
    let mut raw = Vec::new();
    collect_concat_pieces(nodes, widths, id, &mut raw);
    let mut coalesced: Vec<ConcatPiece> = Vec::with_capacity(raw.len());
    for piece in raw {
        match (coalesced.last_mut(), &piece) {
            (
                Some(ConcatPiece::Slice {
                    input: prev_in,
                    start: prev_start,
                    width: prev_w,
                }),
                ConcatPiece::Slice {
                    input: cur_in,
                    start: cur_start,
                    width: cur_w,
                },
            ) if *prev_in == *cur_in && *prev_start == cur_start + cur_w => {
                // High piece ends at prev_start; low piece covers [cur_start .. prev_start].
                *prev_start = *cur_start;
                *prev_w += *cur_w;
            }
            (
                Some(ConcatPiece::ZeroConst { width: prev_w }),
                ConcatPiece::ZeroConst { width: cur_w },
            ) => {
                *prev_w += *cur_w;
            }
            _ => coalesced.push(piece),
        }
    }
    coalesced
}

fn collect_concat_pieces(nodes: &[Node], widths: &[u32], id: NodeId, out: &mut Vec<ConcatPiece>) {
    match &nodes[id.0] {
        Node::Concat(hi, lo) => {
            collect_concat_pieces(nodes, widths, *hi, out);
            collect_concat_pieces(nodes, widths, *lo, out);
        }
        Node::ZeroExtend { input, width } => {
            let in_w = widths[input.0];
            let pad = width.saturating_sub(in_w);
            if pad > 0 {
                out.push(ConcatPiece::ZeroConst { width: pad });
            }
            collect_concat_pieces(nodes, widths, *input, out);
        }
        Node::Extract {
            input,
            start: ExtractStart::Const(start),
            width,
        } => {
            if *width > 0 {
                out.push(ConcatPiece::Slice {
                    input: *input,
                    start: *start,
                    width: *width,
                });
            }
        }
        Node::Constant { width, bits } if *width > 0 && bits.is_zero() => {
            out.push(ConcatPiece::ZeroConst { width: *width });
        }
        _ => {
            let w = widths[id.0];
            if w > 0 {
                out.push(ConcatPiece::Slice {
                    input: id,
                    start: 0,
                    width: w,
                });
            } else {
                out.push(ConcatPiece::Other);
            }
        }
    }
}

fn match_bswap_pattern(nodes: &[Node], widths: &[u32], id: NodeId) -> Option<PatternKind> {
    let Node::Concat(_, _) = nodes[id.0] else {
        return None;
    };
    let w = widths[id.0];
    if !matches!(w, 16 | 32 | 64) {
        return None;
    }
    let pieces = flatten_concat_pieces(nodes, widths, id);
    let expected_bytes = (w / 8) as usize;
    if pieces.len() != expected_bytes {
        return None;
    }
    let ConcatPiece::Slice {
        input: target_input,
        ..
    } = pieces[0]
    else {
        return None;
    };
    if widths[target_input.0] != w {
        return None;
    }
    for (idx, piece) in pieces.iter().enumerate() {
        let ConcatPiece::Slice {
            input,
            start,
            width: 8,
        } = piece
        else {
            return None;
        };
        if *input != target_input || *start != (idx as u32) * 8 {
            return None;
        }
    }
    match w {
        16 => Some(PatternKind::Bswap16 {
            input: target_input,
        }),
        32 => Some(PatternKind::Bswap32 {
            input: target_input,
        }),
        64 => Some(PatternKind::Bswap64 {
            input: target_input,
        }),
        _ => None,
    }
}

fn match_rotate_pattern(nodes: &[Node], widths: &[u32], id: NodeId) -> Option<PatternKind> {
    let Node::Concat(_, _) = nodes[id.0] else {
        return None;
    };
    let w = widths[id.0];
    if w <= 1 {
        return None;
    }
    let pieces = flatten_concat_pieces(nodes, widths, id);
    if pieces.len() != 2 {
        return None;
    }
    let (
        ConcatPiece::Slice {
            input: in_hi,
            start: start_hi,
            width: k,
        },
        ConcatPiece::Slice {
            input: in_lo,
            start: start_lo,
            width: rest,
        },
    ) = (&pieces[0], &pieces[1])
    else {
        return None;
    };
    if *in_hi != *in_lo || widths[in_hi.0] != w {
        return None;
    }
    if *start_hi == 0 && *start_lo == *k && *k > 0 && *k + *rest == w {
        return Some(PatternKind::RotateRight {
            input: *in_hi,
            rotr_amount: *k,
            rotl_amount: w - *k,
            width: w,
        });
    }
    None
}

fn match_shift_pattern(nodes: &[Node], widths: &[u32], id: NodeId) -> Option<PatternKind> {
    match &nodes[id.0] {
        Node::Concat(_, _) | Node::ZeroExtend { .. } => {}
        _ => return None,
    }
    let w = widths[id.0];
    if w <= 1 {
        return None;
    }
    let pieces = flatten_concat_pieces(nodes, widths, id);
    if pieces.len() != 2 {
        return None;
    }
    match (&pieces[0], &pieces[1]) {
        (
            ConcatPiece::Slice {
                input,
                start: 0,
                width: kept,
            },
            ConcatPiece::ZeroConst { width: k },
        ) if widths[input.0] == w && *k > 0 && *kept + *k == w => Some(PatternKind::ShlConst {
            input: *input,
            amount: *k,
            width: w,
        }),
        (
            ConcatPiece::ZeroConst { width: k },
            ConcatPiece::Slice {
                input,
                start,
                width: kept,
            },
        ) if widths[input.0] == w && *k > 0 && *start == *k && *kept + *k == w => {
            Some(PatternKind::LshrConst {
                input: *input,
                amount: *k,
                width: w,
            })
        }
        _ => None,
    }
}

fn is_zero_const(node: &Node) -> bool {
    matches!(node, Node::Constant { bits, .. } if bits.is_zero())
}

fn is_ones_const(node: &Node) -> bool {
    matches!(node, Node::Constant { bits, .. } if bits.is_all_ones())
}

fn is_complement_pair(nodes: &[Node], a: NodeId, b: NodeId) -> bool {
    matches!(nodes[a.0], Node::Not(x) if x == b) || matches!(nodes[b.0], Node::Not(x) if x == a)
}

#[derive(Clone, Debug, Default)]
struct DagRewriter {
    nodes: Vec<Node>,
    widths: Vec<u32>,
    intern: BTreeMap<Node, NodeId>,
}

impl DagRewriter {
    fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, node: Node, width: u32) -> NodeId {
        if let Some(&existing) = self.intern.get(&node) {
            return existing;
        }
        let id = NodeId(self.nodes.len());
        self.intern.insert(node.clone(), id);
        self.nodes.push(node);
        self.widths.push(width);
        id
    }

    fn constant(&mut self, bits: BitValue) -> NodeId {
        let width = bits.width();
        self.intern(Node::Constant { width, bits }, width)
    }

    fn width(&self, id: NodeId) -> u32 {
        self.widths[id.0]
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    fn nodes(&self) -> &[Node] {
        &self.nodes
    }
}
