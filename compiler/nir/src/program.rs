//! Owned typed expression trees with explicit declaration execution phases.
//! This stage preserves evaluation order;
//! it does not yet represent checked resource/effect obligations or executable IIR.
use crate::display::string;
use crate::{DeclarationId, Term};
use std::fmt;
use std::sync::Arc;

/// Relative selections of a writable bit view, in source order.
#[derive(Clone, Debug, PartialEq)]
pub enum BitSelection {
    Static {
        start: usize,
        end: usize,
    },
    Index(Box<Expr>),
    Window {
        start: Box<Expr>,
        end: Box<Expr>,
        width: usize,
    },
}
impl BitSelection {
    pub fn expressions(&self) -> impl Iterator<Item = &Expr> {
        match self {
            Self::Static { .. } => [None, None],
            Self::Index(index) => [Some(index.as_ref()), None],
            Self::Window { start, end, .. } => [Some(start.as_ref()), Some(end.as_ref())],
        }
        .into_iter()
        .flatten()
    }
    pub fn expressions_mut(&mut self) -> impl Iterator<Item = &mut Expr> {
        match self {
            Self::Static { .. } => [None, None],
            Self::Index(index) => [Some(index.as_mut()), None],
            Self::Window { start, end, .. } => [Some(start.as_mut()), Some(end.as_mut())],
        }
        .into_iter()
        .flatten()
    }
    pub fn output_width(&self, input: usize) -> Option<usize> {
        match self {
            Self::Static { start, end } => (start <= end && *end <= input).then(|| end - start),
            Self::Index(_) => Some(1),
            Self::Window { width, .. } => (*width <= input).then_some(*width),
        }
    }
}
impl fmt::Display for BitSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static { start, end } => write!(f, "(static (start {start}) (end {end}))"),
            Self::Index(index) => write!(f, "(index {index})"),
            Self::Window { start, end, width } => {
                write!(f, "(window (start {start}) (end {end}) (width {width}))")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalId {
    pub owner: u32,
    pub index: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRange {
    pub start: u32,
    pub end: u32,
}
impl SourceRange {
    /// Generated locations are IR identities, not offsets into a source file.
    /// Their origin and connection site live in the package's expansion table.
    pub fn generated(index: u32) -> Self {
        Self {
            start: u32::MAX,
            end: index,
        }
    }
    pub fn expansion_index(self) -> Option<u32> {
        (self.start == u32::MAX).then_some(self.end)
    }
    /// Recover a source location without erasing the generated IR identity.
    /// Missing entries and cycles are invalid provenance, not file offsets.
    pub fn original(self, expansions: &[ExpansionSource]) -> Option<Self> {
        let mut current = self;
        for _ in 0..=expansions.len() {
            match current.expansion_index() {
                Some(index) => current = expansions.get(index as usize)?.original,
                None => return (current.start <= current.end).then_some(current),
            }
        }
        None
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionSource {
    pub original: SourceRange,
    pub site: SourceRange,
}
impl fmt::Display for ExpansionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(expansion-origin (original {}) (site {}))",
            self.original, self.site
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    PendingInteger {
        spelling: String,
        negative: bool,
    },
    /// Definition-side exact spelling; must form after type substitution.
    PendingFloat {
        spelling: String,
        negative: bool,
    },
    BitVec {
        width: usize,
        words: Vec<u32>,
    },
    Float16(u16),
    BFloat16(u16),
    Float32(u32),
    Float64(u64),
    SignedInteger(i128),
    UnsignedInteger(u128),
    Bool(bool),
    Char(char),
    String(String),
    Symbol(String),
    Integer(String),
    Float(String),
}
impl Literal {
    /// Check the intrinsic domain and fixed-integer range without evaluating
    /// or parsing a literal. Spelling and bit payload admission remain separate.
    pub fn has_primitive_type(&self, ty: &Term) -> bool {
        self.has_primitive_type_in(ty, None)
    }

    /// Target-dependent literal admission requires an explicit layout. This
    /// does not rewrite the literal's type to a fixed-width primitive.
    pub fn has_primitive_type_in(
        &self,
        ty: &Term,
        target: Option<crate::target::TargetData>,
    ) -> bool {
        use crate::Primitive as P;
        let Term::Primitive(domain) = ty else {
            return false;
        };
        let integer_layout = || match target {
            Some(target) => target.integer_layout(*domain),
            None => domain.fixed_integer_layout(),
        };
        match self {
            Self::PendingFloat { .. } | Self::PendingInteger { .. } => false,
            Self::Bool(_) => *domain == P::Bool,
            Self::Char(_) => *domain == P::Char,
            Self::String(_) => *domain == P::Str,
            Self::Symbol(_) => *domain == P::Symbol,
            Self::Integer(_) => *domain == P::Integer,
            Self::Float(_) => *domain == P::Float,
            Self::Float16(_) => *domain == P::F16,
            Self::BFloat16(_) => *domain == P::Bf16,
            Self::Float32(_) => *domain == P::F32,
            Self::Float64(_) => *domain == P::F64,
            Self::BitVec { width, .. } => *domain == P::BitVec(*width),
            Self::SignedInteger(value) => integer_layout().is_some_and(|(signed, width)| {
                signed
                    && comptime_runtime::machine::MachineInteger::from_signed(*value, width as u16)
                        .is_ok()
            }),
            Self::UnsignedInteger(value) => integer_layout().is_some_and(|(signed, width)| {
                !signed
                    && comptime_runtime::machine::MachineInteger::from_unsigned(
                        *value,
                        width as u16,
                    )
                    .is_ok()
            }),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub ty: Term,
    pub source: SourceRange,
    pub kind: ExprKind,
}
/// Invocation-local evaluation order and bindings for head inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalFormInputPlan {
    pub order: Vec<usize>,
    pub bindings: Vec<LocalId>,
    /// Tuple producers, before the indicated position in `order`. Position
    /// `order.len()` preserves trailing empty packets as well.
    pub preparations: Vec<NormalFormPreparation>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NormalFormPreparation {
    pub before: usize,
    pub expression: Expr,
}
impl NormalFormPreparation {
    /// The only bindings exported by a packet producer belong to its outer
    /// tuple pattern. Producer-local temporaries never enter the input plan.
    pub fn local_bindings(&self) -> Option<Vec<LocalId>> {
        let ExprKind::LetPattern {
            pattern: Pattern::Tuple(fields),
            guard: None,
            otherwise: None,
            ..
        } = &self.expression.kind
        else {
            return None;
        };
        fields
            .iter()
            .map(|field| {
                let Pattern::Binding { binding, inner } = field else {
                    return None;
                };
                matches!(inner.as_ref(), Pattern::Wild).then_some(*binding)
            })
            .collect()
    }
}
impl NormalFormInputPlan {
    pub fn local_bindings(&self) -> Option<Vec<LocalId>> {
        let mut bindings = self.bindings.clone();
        for preparation in &self.preparations {
            bindings.extend(preparation.local_bindings()?);
        }
        Some(bindings)
    }
    pub fn valid(&self, count: usize) -> bool {
        self.order.len() == count
            && self.bindings.len() == count
            && self
                .order
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                == (0..count).collect()
            && self
                .bindings
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == count
            && self.preparations.iter().all(|p| p.before <= count)
            && self
                .preparations
                .windows(2)
                .all(|p| p[0].before <= p[1].before)
            && self.local_bindings().is_some_and(|bindings| {
                bindings
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == bindings.len()
            })
    }
}
impl fmt::Display for NormalFormInputPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-plan (order")?;
        for slot in &self.order {
            write!(f, " {slot}")?;
        }
        f.write_str(") (bindings")?;
        for binding in &self.bindings {
            write!(f, " {binding}")?;
        }
        f.write_str(") (preparations")?;
        for preparation in &self.preparations {
            write!(
                f,
                " (before {} {})",
                preparation.before, preparation.expression
            )?;
        }
        f.write_str("))")
    }
}
/// A head input may need its earlier Type inputs before literal conversion.
#[derive(Clone, Debug, PartialEq)]
pub enum NormalFormArgument {
    /// Collect expressions into one contextual comptime list in source order.
    TypeList {
        source: SourceRange,
        items: Vec<Self>,
    },
    Value(Expr),
    /// Expand a completed list of the collector's exact contextual list type.
    TypeListSpread(Expr),
    Unit {
        source: SourceRange,
    },
    Integer {
        source: SourceRange,
        spelling: String,
        negative: bool,
    },
    If {
        source: SourceRange,
        condition: Expr,
        then_input: Box<Self>,
        else_input: Box<Self>,
    },
    Block {
        source: SourceRange,
        statements: Vec<Expr>,
        result: Option<Box<Self>>,
    },
    Tuple {
        source: SourceRange,
        /// Trusted Tuple identity used if the contextual result is a Type.
        type_head: Option<DeclarationId>,
        items: Vec<Self>,
    },
    Match {
        source: SourceRange,
        value: Expr,
        arms: Vec<NormalMatchArm>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct NormalMatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: NormalFormArgument,
}
impl fmt::Display for NormalMatchArm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(contextual-match-arm (pattern {}) (guard", self.pattern)?;
        if let Some(guard) = &self.guard {
            write!(f, " {guard}")?;
        }
        write!(f, ") (body {}))", self.body)
    }
}
impl NormalFormArgument {
    pub fn expression(&self) -> Option<&Expr> {
        match self {
            Self::TypeListSpread(value) | Self::Value(value) => Some(value),
            _ => None,
        }
    }
    pub fn expression_mut(&mut self) -> Option<&mut Expr> {
        match self {
            Self::TypeListSpread(value) | Self::Value(value) => Some(value),
            _ => None,
        }
    }
    pub fn expressions(&self) -> Vec<&Expr> {
        let mut pending = vec![self];
        let mut result = Vec::new();
        while let Some(input) = pending.pop() {
            match input {
                Self::TypeListSpread(value) | Self::Value(value) => result.push(value),
                Self::Integer { .. } | Self::Unit { .. } => {}
                Self::If {
                    condition,
                    then_input,
                    else_input,
                    ..
                } => {
                    result.push(condition);
                    pending.push(else_input);
                    pending.push(then_input);
                }
                Self::TypeList { items, .. } | Self::Tuple { items, .. } => {
                    pending.extend(items.iter().rev())
                }
                Self::Match { value, arms, .. } => {
                    result.push(value);
                    for arm in arms.iter().rev() {
                        if let Some(guard) = &arm.guard {
                            result.push(guard);
                        }
                        pending.push(&arm.body);
                    }
                }
                Self::Block {
                    statements,
                    result: tail,
                    ..
                } => {
                    result.extend(statements.iter());
                    if let Some(tail) = tail {
                        pending.push(tail);
                    }
                }
            }
        }
        result
    }
    pub fn expressions_mut(&mut self) -> Vec<&mut Expr> {
        let mut pending = vec![self];
        let mut result = Vec::new();
        while let Some(input) = pending.pop() {
            match input {
                Self::TypeListSpread(value) | Self::Value(value) => result.push(value),
                Self::Integer { .. } | Self::Unit { .. } => {}
                Self::If {
                    condition,
                    then_input,
                    else_input,
                    ..
                } => {
                    result.push(condition);
                    pending.push(else_input);
                    pending.push(then_input);
                }
                Self::TypeList { items, .. } | Self::Tuple { items, .. } => {
                    pending.extend(items.iter_mut().rev())
                }
                Self::Match { value, arms, .. } => {
                    result.push(value);
                    for arm in arms.iter_mut().rev() {
                        if let Some(guard) = &mut arm.guard {
                            result.push(guard);
                        }
                        pending.push(&mut arm.body);
                    }
                }
                Self::Block {
                    statements,
                    result: tail,
                    ..
                } => {
                    result.extend(statements.iter_mut());
                    if let Some(tail) = tail {
                        pending.push(tail);
                    }
                }
            }
        }
        result
    }
    pub fn patterns(&self) -> Vec<&Pattern> {
        let mut pending = vec![self];
        let mut patterns = Vec::new();
        while let Some(input) = pending.pop() {
            match input {
                Self::TypeList { items, .. } | Self::Tuple { items, .. } => {
                    pending.extend(items.iter().rev())
                }
                Self::Match { arms, .. } => {
                    for arm in arms.iter().rev() {
                        patterns.push(&arm.pattern);
                        pending.push(&arm.body);
                    }
                }
                Self::If {
                    then_input,
                    else_input,
                    ..
                } => {
                    pending.push(else_input);
                    pending.push(then_input);
                }
                Self::Block {
                    result: Some(result),
                    ..
                } => pending.push(result),
                _ => {}
            }
        }
        patterns
    }
    pub fn patterns_mut(&mut self) -> Vec<&mut Pattern> {
        let mut pending = vec![self];
        let mut patterns = Vec::new();
        while let Some(input) = pending.pop() {
            match input {
                Self::TypeList { items, .. } | Self::Tuple { items, .. } => {
                    pending.extend(items.iter_mut().rev())
                }
                Self::Match { arms, .. } => {
                    for arm in arms.iter_mut().rev() {
                        patterns.push(&mut arm.pattern);
                        pending.push(&mut arm.body);
                    }
                }
                Self::If {
                    then_input,
                    else_input,
                    ..
                } => {
                    pending.push(else_input);
                    pending.push(then_input);
                }
                Self::Block {
                    result: Some(result),
                    ..
                } => pending.push(result),
                _ => {}
            }
        }
        patterns
    }
    pub fn is_diverging(&self) -> bool {
        enum Work<'a> {
            Input(&'a NormalFormArgument),
            All(usize),
            Any(usize),
        }
        let mut work = vec![Work::Input(self)];
        let mut results = Vec::new();
        let never = |e: &Expr| e.ty == Term::Primitive(crate::Primitive::Never);
        while let Some(next) = work.pop() {
            match next {
                Work::All(count) => {
                    let mut all = true;
                    for _ in 0..count {
                        all &= results.pop().unwrap();
                    }
                    results.push(all);
                }
                Work::Any(count) => {
                    let mut any = false;
                    for _ in 0..count {
                        any |= results.pop().unwrap();
                    }
                    results.push(any);
                }
                Work::Input(Self::TypeList { items, .. } | Self::Tuple { items, .. }) => {
                    work.push(Work::Any(items.len()));
                    work.extend(items.iter().rev().map(Work::Input));
                }
                Work::Input(Self::TypeListSpread(value) | Self::Value(value)) => {
                    results.push(never(value))
                }
                Work::Input(Self::Integer { .. } | Self::Unit { .. }) => results.push(false),
                Work::Input(Self::If {
                    condition,
                    then_input,
                    else_input,
                    ..
                }) => {
                    if never(condition) {
                        results.push(true);
                    } else {
                        work.push(Work::All(2));
                        work.push(Work::Input(else_input));
                        work.push(Work::Input(then_input));
                    }
                }
                Work::Input(Self::Match { value, arms, .. }) => {
                    if never(value) {
                        results.push(true);
                    } else {
                        work.push(Work::All(arms.len()));
                        for arm in arms.iter().rev() {
                            work.push(Work::Input(&arm.body));
                        }
                    }
                }
                Work::Input(Self::Block {
                    statements, result, ..
                }) => {
                    if statements.iter().any(never) {
                        results.push(true);
                    } else if let Some(result) = result {
                        work.push(Work::Input(result));
                    } else {
                        results.push(false);
                    }
                }
            }
        }
        results.pop().unwrap()
    }
    pub fn source(&self) -> SourceRange {
        match self {
            Self::TypeListSpread(value) | Self::Value(value) => value.source,
            Self::Unit { source }
            | Self::Integer { source, .. }
            | Self::If { source, .. }
            | Self::Block { source, .. }
            | Self::TypeList { source, .. }
            | Self::Tuple { source, .. }
            | Self::Match { source, .. } => *source,
        }
    }
}
impl fmt::Display for NormalFormArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeListSpread(value) => write!(f, "(type-list-spread {value})"),
            Self::Value(value) => write!(f, "{value}"),
            Self::If {
                source,
                condition,
                then_input,
                else_input,
            } => write!(
                f,
                "(contextual-if-input (source {source}) (condition {condition}) (then {then_input}) (else {else_input}))"
            ),
            Self::Block {
                source,
                statements,
                result,
            } => {
                write!(f, "(contextual-block-input (source {source}) (statements")?;
                for statement in statements {
                    write!(f, " {statement}")?;
                }
                f.write_str(") (result")?;
                if let Some(result) = result {
                    write!(f, " {result}")?;
                }
                f.write_str("))")
            }
            Self::TypeList { source, items } => {
                write!(f, "(type-list-input (source {source}) (items")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str("))")
            }
            Self::Tuple {
                source,
                type_head,
                items,
            } => {
                write!(f, "(contextual-tuple-input (source {source}) (type-head")?;
                if let Some(head) = type_head {
                    write!(f, " {head}")?;
                }
                f.write_str(") (items")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str("))")
            }
            Self::Match {
                source,
                value,
                arms,
            } => {
                write!(
                    f,
                    "(contextual-match-input (source {source}) (value {value}) (arms"
                )?;
                for arm in arms {
                    write!(f, " {arm}")?;
                }
                f.write_str("))")
            }
            Self::Unit { source } => write!(f, "(contextual-unit-input (source {source}))"),
            Self::Integer {
                source,
                spelling,
                negative,
            } => {
                write!(
                    f,
                    "(contextual-integer-input (source {source}) (negative {negative}) (spelling "
                )?;
                string(f, spelling)?;
                f.write_str("))")
            }
        }
    }
}

/// Exact comptime Integer operations; fixed-width operators use their protocols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerOp {
    Add,
    Subtract,
    Multiply,
    /// Explicit internal rounding rule; source operator selection is separate.
    DivideTrunc,
    RemainderTrunc,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
impl IntegerOp {
    /// Canonical scalar evaluation shared by interpretation and static identity
    /// recovery. Never proves a type relation or reduces an opaque head.
    pub fn evaluate(
        self,
        left: &comptime_runtime::Integer,
        right: &comptime_runtime::Integer,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Term, comptime_runtime::Error> {
        Ok(match self {
            Self::Add => Term::Integer(left.add(right, budget)?),
            Self::Subtract => Term::Integer(left.subtract(right, budget)?),
            Self::Multiply => Term::Integer(left.multiply(right, budget)?),
            Self::DivideTrunc => Term::Integer(left.div_rem_trunc(right, budget)?.0),
            Self::RemainderTrunc => Term::Integer(left.div_rem_trunc(right, budget)?.1),
            _ => {
                let size = left.bit_len().max(right.bit_len()).div_ceil(32);
                if size > budget.max_limbs {
                    return Err(comptime_runtime::Error::SizeLimit);
                }
                budget.consume(size.saturating_add(1))?;
                let order = left.cmp(right);
                Term::Bool(match self {
                    Self::Equal => order.is_eq(),
                    Self::NotEqual => !order.is_eq(),
                    Self::Less => order.is_lt(),
                    Self::LessEqual => !order.is_gt(),
                    Self::Greater => order.is_gt(),
                    Self::GreaterEqual => !order.is_lt(),
                    _ => unreachable!(),
                })
            }
        })
    }
    pub fn is_comparison(self) -> bool {
        !matches!(
            self,
            Self::Add | Self::Subtract | Self::Multiply | Self::DivideTrunc | Self::RemainderTrunc
        )
    }
}
impl fmt::Display for IntegerOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Multiply => "multiply",
            Self::DivideTrunc => "divide-trunc",
            Self::RemainderTrunc => "remainder-trunc",
            Self::Equal => "equal",
            Self::NotEqual => "not-equal",
            Self::Less => "less",
            Self::LessEqual => "less-equal",
            Self::Greater => "greater",
            Self::GreaterEqual => "greater-equal",
        })
    }
}
/// IEEE binary arithmetic; each node rounds at its operand precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}
impl FloatOp {
    pub fn evaluate_half(
        self,
        format: comptime_runtime::float16::Float16Format,
        left: u16,
        right: u16,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<u16, comptime_runtime::Error> {
        use comptime_runtime::float16::Float16Op as Op;
        format.arithmetic(
            match self {
                Self::Add => Op::Add,
                Self::Subtract => Op::Subtract,
                Self::Multiply => Op::Multiply,
                Self::Divide => Op::Divide,
            },
            left,
            right,
            budget,
        )
    }
    pub fn evaluate_f32(self, left: u32, right: u32) -> u32 {
        let (left, right) = (f32::from_bits(left), f32::from_bits(right));
        let value = match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
        };
        // Do not expose host-selected NaN payloads as invocation nondeterminism.
        if value.is_nan() {
            0x7fc00000
        } else {
            value.to_bits()
        }
    }
    pub fn evaluate_f64(self, left: u64, right: u64) -> u64 {
        let (left, right) = (f64::from_bits(left), f64::from_bits(right));
        let value = match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
        };
        if value.is_nan() {
            0x7ff8000000000000
        } else {
            value.to_bits()
        }
    }
}
impl fmt::Display for FloatOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Multiply => "multiply",
            Self::Divide => "divide",
        })
    }
}

/// A checked operation branch group with its delivery ABI. `fallback` indexes the outer
/// allowed row when coverage is partial; it is not a continuation value.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectHandler {
    pub operation: crate::effects::Operation,
    pub abi: crate::effects::Abi,
    pub signature: Term,
    pub arms: Vec<MatchArm>,
    pub fallback: Option<usize>,
}
impl fmt::Display for EffectHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-handler {} (abi {}) (signature {}) (fallback",
            self.operation, self.abi, self.signature
        )?;
        if let Some(slot) = self.fallback {
            write!(f, " {slot}")?;
        }
        f.write_str(") (arms")?;
        for arm in &self.arms {
            write!(f, " {arm}")?;
        }
        f.write_str("))")
    }
}

/// Functional editing of ordered heterogeneous Objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectEditKind {
    Concat,
    Append,
    Replace,
    Remove,
}
impl fmt::Display for ObjectEditKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Concat => "concat",
            Self::Append => "append",
            Self::Replace => "replace",
            Self::Remove => "remove",
        })
    }
}

/// Type formation only; target widths and value bounds are checked at invocation.
pub fn comptime_integer_conversion_valid(input: &Term, result: &Term) -> bool {
    use crate::Primitive;
    let integer = |ty: &Term| {
        matches!(ty, Term::Primitive(p)
        if *p == Primitive::Integer || p.fixed_integer_layout().is_some()
            || matches!(p, Primitive::Usize | Primitive::Isize))
    };
    integer(result) && (integer(input) || *input == Term::Primitive(Primitive::Never))
        || *input == Term::Primitive(Primitive::Never) && *result == *input
}

/// The result width is fixed even when the input representation needs a target.
pub fn integer_representation_valid(input: &Term, result: &Term) -> bool {
    use crate::Primitive as P;
    match input {
        Term::Primitive(P::Never) => *result == Term::Primitive(P::Never),
        Term::Primitive(p)
            if p.fixed_integer_layout().is_some() || matches!(p, P::Usize | P::Isize) =>
        {
            *result == Term::Primitive(P::U128)
        }
        _ => false,
    }
}

/// UTF-8 byte observations, independent of target pointer width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextQueryKind {
    ByteLength,
    Byte,
    SymbolByteLength,
    SymbolByte,
}
impl fmt::Display for TextQueryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ByteLength => "byte-length",
            Self::Byte => "byte",
            Self::SymbolByteLength => "symbol-byte-length",
            Self::SymbolByte => "symbol-byte",
        })
    }
}
impl TextQueryKind {
    pub fn is_length(self) -> bool {
        matches!(self, Self::ByteLength | Self::SymbolByteLength)
    }
    pub fn is_symbol(self) -> bool {
        matches!(self, Self::SymbolByteLength | Self::SymbolByte)
    }
    pub fn validate(self, packet: &Term, result: &Term) -> bool {
        use crate::Primitive as P;
        let Term::Tuple(inputs) = packet else {
            return false;
        };
        let count = if self.is_length() { 1 } else { 2 };
        if inputs.len() != count
            || inputs.iter().enumerate().any(|(i, ty)| {
                *ty != Term::Primitive(if i == 0 {
                    if self.is_symbol() { P::Symbol } else { P::Str }
                } else {
                    P::U64
                }) && *ty != Term::Primitive(P::Never)
            })
        {
            return false;
        }
        let expected = if inputs.contains(&Term::Primitive(P::Never)) {
            Term::Primitive(P::Never)
        } else if self.is_length() {
            Term::Primitive(P::U64)
        } else {
            Term::Optional(Box::new(Term::Primitive(P::U8)))
        };
        *result == expected
    }
}

/// Ordered Object observations. Indexed queries use a checked argument packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectQueryKind {
    Length,
    Key,
    EntryType,
    Value,
    NamedValue,
}
impl fmt::Display for ObjectQueryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "length",
            Self::Key => "key",
            Self::EntryType => "entry-type",
            Self::Value => "value",
            Self::NamedValue => "named-value",
        })
    }
}

/// Logical enum structure; declaration order never implies a discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnumStructureQuery {
    GroupPaths,
    VariantPaths,
    Shapes,
    FieldNames,
    FieldTypes,
}
impl fmt::Display for EnumStructureQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::GroupPaths => "group-paths",
            Self::VariantPaths => "variant-paths",
            Self::Shapes => "shapes",
            Self::FieldNames => "field-names",
            Self::FieldTypes => "field-types",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Preserve the entire operand as the payload of one new optional layer.
    OptionalPresent(Box<Expr>),
    /// Absence in the optional domain carried by Expr.ty.
    OptionalAbsent,
    /// Comptime construction of an optional Type from a Type-valued expression.
    OptionalType(Box<Expr>),
    /// Explicit result construction/transport, independent of effect continuations.
    ErrorConvert {
        conversion: crate::error_results::Conversion,
        value: Box<Expr>,
    },
    /// Early delivery to the lexically enclosing direct handler branch.
    EffectResume(Box<Expr>),
    EffectHandle {
        computation: Box<Expr>,
        handlers: Vec<EffectHandler>,
        outer: Vec<crate::effects::Operation>,
    },
    /// Discharge the source computation's leading occurrences into slots of
    /// the enclosing allowed row. Instantiating a generic result can leave a
    /// residual computation; those later occurrences are not consumed here.
    EffectRun {
        computation: Box<Expr>,
        slots: Vec<usize>,
    },
    /// A body checked in an explicit allowed effect environment. Its outer
    /// type retains this row and the body's result (including bottom coercion).
    EffectComputation {
        effects: Vec<crate::effects::Operation>,
        body: Box<Expr>,
    },
    /// A typed request into the enclosing computation environment. The slot
    /// identifies one occurrence, not a runtime handler activation.
    EffectRequest {
        operation: crate::effects::Operation,
        abi: crate::effects::Abi,
        allowed_index: usize,
        signature: Box<Term>,
        arguments: Vec<Expr>,
    },
    /// A checked type identity, never an instruction to execute its head.
    TypeValue(Term),
    /// A captured comptime value. Invocation substitutes its declaration identity;
    /// reading it never invokes or unfolds a normal-form head.
    ComptimeValue(Term),
    /// Origin-bound host metadata. Only the selected immutable descriptor is admitted.
    BuildMetadata {
        owner: u64,
        selector: Box<Expr>,
        entries: Vec<(String, u64, Option<Term>)>,
    },
    /// Completed immutable meta.List<Type> data, distinct from Type and tuple data.
    TypeListValue(Vec<Term>),
    /// Ordered comptime construction of immutable meta.List<T> data.
    /// The expression type carries the trusted head and homogeneous element domain.
    TypeList(Vec<Expr>),
    /// Checked comptime indexing of a compiler-bound meta.List<T> value.
    TypeListIndex(Box<Expr>, Box<Expr>),
    /// Length of a checked immutable comptime list.
    ListLength(Box<Expr>),
    StructFieldTypes(Box<Expr>),
    StructFieldNames(Box<Expr>),
    /// Compiler-private packet: definition name, field names, field Types.
    BuildStruct(Box<Expr>),
    /// Private packet: name, groups, variants, shapes, field names, field Types,
    /// and optionally a Symbol encoding list or fixed-bit encoding packet.
    BuildEnum(Box<Expr>),
    EnumStructure {
        subject: Box<Expr>,
        query: EnumStructureQuery,
    },
    /// Evaluate and freeze clause operands before creating a comptime callable.
    /// The callable signature is already known; the packet retains input domains.
    /// A trait member declaration retains its origin: packet inputs are Self,
    /// interface inputs, then member inputs, and dispatch uses the trait contract.
    ComptimeFunctionInstance {
        declaration: DeclarationId,
        subject: Box<Expr>,
    },
    ObjectEdit {
        subject: Box<Expr>,
        operation: ObjectEditKind,
    },
    ObjectQuery {
        subject: Box<Expr>,
        query: ObjectQueryKind,
    },
    /// Ordered concatenation preserving the complete List<T> identity.
    ListConcat(Box<Expr>, Box<Expr>),
    /// A value selected by an interface member. Its declaration identity remains
    /// abstract until implementation evidence fixes the value; defaults are not facts.
    AssociatedValue {
        interface: InterfaceInstance,
        member: DeclarationId,
        subject: Term,
    },
    /// Intrinsic BitVec constructor with a concrete comptime Integer operand.
    BitVecType(Box<Expr>),
    /// Decide type equality only with positive equality/disequality evidence.
    TypeEqual(Box<Expr>, Box<Expr>),
    /// Builtin scalar equality, including IEEE floats; never type equality.
    ScalarEqual(Box<Expr>, Box<Expr>),
    /// Internal runtime dispatch comparison of validated staged callable
    /// identities. This does not expose function equality in source syntax.
    TemplateEqual(Box<Expr>, Box<Expr>),
    ScalarLessEqual(Box<Expr>, Box<Expr>),
    ScalarLess(Box<Expr>, Box<Expr>),
    ScalarGreater(Box<Expr>, Box<Expr>),
    ScalarGreaterEqual(Box<Expr>, Box<Expr>),
    /// Evaluates Type-valued fields in source order and constructs a tuple type.
    TupleType(Vec<Expr>),
    NominalType {
        preparations: Vec<NormalFormPreparation>,
        /// Invocation-local input bindings, available to omitted defaults.
        bindings: Option<Vec<LocalId>>,
        /// Source evaluation order; None means declaration order.
        order: Option<Vec<usize>>,
        declaration: DeclarationId,
        arguments: Vec<NormalFormArgument>,
    },
    /// Comptime-only guard before expanding a transparent type alias.
    CheckTypeAlias {
        declaration: DeclarationId,
        preparations: Vec<NormalFormPreparation>,
        bindings: Option<Vec<LocalId>>,
        order: Option<Vec<usize>>,
        arguments: Vec<NormalFormArgument>,
        /// Execute after admission, specializing captured and own input types.
        body: Option<Box<Expr>>,
    },
    PointerType(Box<Expr>),
    /// Address a checked local/field place without delivering its contents.
    AddressOf(Box<Expr>),
    ReadPointer(Box<Expr>),
    PointerAnonymousField {
        pointer: Box<Expr>,
        name: String,
        index: usize,
    },
    PointerField {
        pointer: Box<Expr>,
        declaration: DeclarationId,
        index: usize,
    },
    WritePointer {
        pointer: Box<Expr>,
        value: Box<Expr>,
    },
    ClosureType {
        environment: Vec<Expr>,
        signature: Box<Expr>,
    },
    /// Construct a callable signature from computed parameter types.
    FunctionType {
        comptime: bool,
        /// One flag per input; its computed domain determines Type vs value sort.
        comptime_parameters: Vec<bool>,
        parameters: Vec<Expr>,
    },
    /// Replace a callable signature's result, preserving its invocation phase.
    FunctionResultType {
        signature: Box<Expr>,
        result: Box<Expr>,
    },
    /// Evaluate inputs and retain the opaque head; never invoke its body.
    NormalFormType {
        head: DeclarationId,
        arguments: Vec<NormalFormArgument>,
        plan: Option<NormalFormInputPlan>,
    },
    FloatNegate(Box<Expr>),
    FloatBinary {
        op: FloatOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    IntegerNegate(Box<Expr>),
    IntegerBinary {
        op: IntegerOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Literal(Literal),
    Local(LocalId),
    Function(DeclarationId),
    /// Definition-fixed interface/member source. Only the subject is replaced
    /// during instantiation; no competing member lookup occurs afterwards.
    TraitMethod {
        /// Method-local arguments, separate from enclosing interface arguments.
        method_arguments: Vec<Term>,
        interface_arguments: Vec<Term>,
        interface: DeclarationId,
        member: DeclarationId,
        subject: Term,
    },
    FunctionInstance {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    Closure {
        parameters: Vec<LocalId>,
        captures: Vec<Capture>,
        body: Arc<Expr>,
    },
    Unit,
    Variant {
        declaration: DeclarationId,
        index: usize,
        fields: Vec<Expr>,
    },
    Tuple(Vec<Expr>),
    /// Named structural construction; field labels and types are retained in ty.
    AnonymousRecord {
        fields: Vec<Expr>,
    },
    /// Ordered compile-time metadata. None denotes a positional entry;
    /// duplicate property labels remain distinct until an attribute merge.
    Object {
        names: Vec<Option<String>>,
        fields: Vec<Expr>,
    },
    Record {
        declaration: DeclarationId,
        fields: Vec<Expr>,
    },
    Field {
        value: Box<Expr>,
        declaration: DeclarationId,
        index: usize,
    },
    AnonymousField {
        value: Box<Expr>,
        name: String,
        index: usize,
    },
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Block {
        statements: Vec<Expr>,
        result: Option<Box<Expr>>,
    },
    /// Internal call preparation. Statements are fresh Let bindings or plain
    /// tuple LetPattern unpackings, evaluated in source order without creating
    /// full-expression temporary scopes.
    /// The result is a normalized call, variant construction, effect request or
    /// deterministically expanded call block; temporaries keep the surrounding
    /// expression lifetime. Unlike Block, this node is transparent to the
    /// result expression's TemporaryEnd boundary.
    CallPreparation {
        statements: Vec<Expr>,
        result: Option<Box<Expr>>,
    },
    Let {
        binding: LocalId,
        value: Box<Expr>,
    },
    LetPattern {
        pattern: Pattern,
        value: Box<Expr>,
        guard: Option<Box<Expr>>,
        otherwise: Option<Box<Expr>>,
    },
    BitWritePointer {
        pointer: Box<Expr>,
        selections: Vec<BitSelection>,
        value: Box<Expr>,
    },
    BitUpdate {
        place: Place,
        storage_type: Term,
        selections: Vec<BitSelection>,
        value: Box<Expr>,
    },
    BitAssign {
        place: Place,
        storage_type: Term,
        start: usize,
        end: usize,
        value: Box<Expr>,
    },
    Assign {
        place: Place,
        value: Box<Expr>,
    },
    Loop {
        identity: LocalId,
        body: Box<Expr>,
    },
    Break(LocalId),
    Continue(LocalId),
    /// Evaluate a legal program predicate once; failure ends the current path.
    /// Retained inline call/return proof obligations, also checked at execution.
    RequirePredicate(Box<Expr>),
    EnsurePredicate(Box<Expr>),
    CheckPredicate(Box<Expr>),
    /// Lexical unsafe audit boundary; ordinary body contracts still apply.
    UnsafeScope(Box<Expr>),
    /// Target-defined operation. Inputs remain in source evaluation order.
    /// The enclosing Expr carries the explicit output type and source range.
    Intrinsic {
        tag: crate::intrinsics::Tag,
        binding: Option<crate::intrinsics::Checked>,
        inputs: Vec<Expr>,
    },
    Discard(Box<Expr>),
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    Return(Option<Box<Expr>>),
    /// Type-directed application. A NormalFormFunction callee forms an opaque
    /// type at comptime; it never invokes the head's body. Both calling forms
    /// retain the callee and source-ordered inputs in the same traversal shape.
    Call {
        callee: Box<Expr>,
        /// Source-ordered explicit inputs followed by missing defaults in
        /// declaration order. Defaults use a fresh parameter binding frame;
        /// explicit inputs always use the caller's environment.
        arguments: Vec<CallArgument>,
        parameter_bindings: Vec<LocalId>,
    },
    /// Source-ordered value and runtime position; bounds require program evidence.
    BitIndex(Box<Expr>, Box<Expr>),
    /// Dynamic endpoints, evaluated after the value in source order.
    /// The result type fixes the width; program verification proves the difference.
    BitWindow {
        value: Box<Expr>,
        start: Box<Expr>,
        end: Box<Expr>,
    },
    BitSlice {
        value: Box<Expr>,
        start: usize,
        end: usize,
    },
    /// Zero-extension to BitVec/unsigned integers, or exact-width reinterpretation
    /// to fixed signed integers and IEEE floats. Other representations require
    /// separate construction/validity contracts and are not accepted here.
    BitConvert(Box<Expr>),
    /// Transfer a whole value across exactly one declared newtype representation edge.
    NewtypeConvert(Box<Expr>),
    BitEqual(Box<Expr>, Box<Expr>),
    /// Explicit representation snapshot; resource checking must prove Copy.
    Snapshot(Box<Expr>),
    TakeBits(Box<Expr>),
    SymbolName(Box<Expr>),
    /// Zero-extend integer representation bits to u128 using execution target facts.
    IntegerRepresentation(Box<Expr>),
    /// Exact comptime Integer/machine-integer conversion; never truncates.
    ComptimeIntegerConvert(Box<Expr>),
    /// UTF-8 observations; packet is (str) or (str, u64), in evaluation order.
    TextQuery {
        subject: Box<Expr>,
        query: TextQueryKind,
    },
    /// Observe an enum tag without transferring its payload.
    EnumName(Box<Expr>),
    Not(Box<Expr>),
    BitConcat(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
/// Layout family for a field in a program place. Anonymous records have no
/// declaration identity; their slot is interpreted in the root's checked type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FieldOwner {
    Nominal(DeclarationId),
    Anonymous,
}
impl fmt::Display for FieldOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nominal(id) => id.fmt(f),
            Self::Anonymous => f.write_str("(anonymous-record)"),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub root: LocalId,
    pub fields: Vec<(FieldOwner, usize)>,
}
impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(place (root {}) (fields", self.root)?;
        for (declaration, index) in &self.fields {
            write!(f, " (field {declaration} {index})")?;
        }
        f.write_str("))")
    }
}
impl Place {
    /// Locate a syntactic place; no expression evaluation is hidden here.
    pub fn is_root(&self) -> bool {
        self.fields.is_empty()
    }
}
/// Argument transport is distinct from the source type. Comptime references
/// carry invocation-local storage; they never introduce a pointer type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArgumentPassing {
    #[default]
    Value,
    ComptimeReference,
    /// Alias a let binding without granting whole-value replacement.
    ComptimeReferenceNoRebind,
}
impl fmt::Display for ArgumentPassing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Value => "value",
            Self::ComptimeReference => "comptime-reference",
            Self::ComptimeReferenceNoRebind => "comptime-reference-no-rebind",
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct CallArgument {
    pub passing: ArgumentPassing,
    pub parameter: usize,
    pub value: Expr,
    pub is_default: bool,
}
impl fmt::Display for CallArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(argument (parameter {}) (default {}) (passing {}) (value {}))",
            self.parameter, self.is_default, self.passing, self.value
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub binding: LocalId,
    pub ty: Term,
    /// None reads the lexical binding. Some restores a completed captured
    /// value, without looking up or overwriting a caller-local binding.
    pub value: Option<Box<Expr>>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    /// Match one existing optional layer and expose its whole payload.
    OptionalPresent(Box<Pattern>),
    /// Match absence at exactly this layer.
    OptionalAbsent,
    /// None selects success; Some selects a full error type identity.
    ErrorResult {
        label: Option<Term>,
        inner: Box<Pattern>,
    },
    /// Definition-only pattern-code evidence. Never executable or lowerable.
    CodeEvidence {
        outputs: Vec<(LocalId, Term)>,
    },
    /// Internal decomposition of the checked inputs of a nominal Type value.
    NominalType {
        declaration: DeclarationId,
        fields: Vec<Pattern>,
    },
    /// Read-only candidate-local alias of the current subject. The alias is
    /// available while matching `inner`, but is never a successful export.
    Observe {
        binding: LocalId,
        inner: Box<Pattern>,
    },
    Then {
        first: Box<Pattern>,
        value: Box<Expr>,
        next: Box<Pattern>,
    },
    Guard {
        pattern: Box<Pattern>,
        condition: Box<Expr>,
    },
    Remap {
        pattern: Box<Pattern>,
        bindings: Vec<(LocalId, LocalId)>,
    },
    AnonymousRecord {
        names: Vec<String>,
        fields: Vec<Pattern>,
    },
    Record {
        declaration: DeclarationId,
        fields: Vec<Pattern>,
    },
    Tuple(Vec<Pattern>),
    /// Observe fields of an already exposed tuple type, without unfolding heads.
    TypeTuple(Vec<Pattern>),
    NormalForm {
        head: DeclarationId,
        arguments: Vec<Pattern>,
    },
    Wild,
    Unit,
    Variant {
        declaration: DeclarationId,
        index: usize,
        fields: Vec<Pattern>,
    },
    Literal(Literal),
    Binding {
        binding: LocalId,
        inner: Box<Pattern>,
    },
    Or(Vec<Pattern>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}
impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OptionalPresent(inner) => write!(f, "(optional-present-pattern {inner})"),
            Self::OptionalAbsent => f.write_str("(optional-absent-pattern)"),
            Self::CodeEvidence { outputs } => {
                f.write_str("(pattern-code-evidence (outputs")?;
                for (id, ty) in outputs {
                    write!(f, " (output {id} {ty})")?;
                }
                f.write_str("))")
            }

            Self::Observe { binding, inner } => {
                write!(f, "(observe-pattern (subject {binding}) (inner {inner}))")
            }
            Self::Then { first, value, next } => {
                write!(f, "(and-is (first {first}) (value {value}) (next {next}))")
            }
            Self::Guard { pattern, condition } => {
                write!(f, "(guard-pattern {pattern} {condition})")
            }
            Self::Remap { pattern, bindings } => {
                write!(f, "(remap-pattern {pattern} (bindings")?;
                for (from, to) in bindings {
                    write!(f, " ({from} {to})")?;
                }
                f.write_str("))")
            }
            Self::AnonymousRecord { names, fields } => {
                f.write_str("(anonymous-record-pattern (names")?;
                for name in names {
                    f.write_str(" ")?;
                    crate::display::string(f, name)?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Record {
                declaration,
                fields,
            } => {
                write!(f, "(record-pattern {declaration} (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::NominalType {
                declaration,
                fields,
            } => {
                write!(
                    f,
                    "(nominal-type-pattern (declaration {declaration}) (inputs"
                )?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::NormalForm { head, arguments } => {
                write!(f, "(normal-form-pattern (head {head}) (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::TypeTuple(fields) => {
                f.write_str("(tuple-type-pattern")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")
            }
            Self::Tuple(fields) => {
                f.write_str("(tuple-pattern")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")
            }
            Self::ErrorResult { label, inner } => {
                f.write_str("(error-result-pattern (branch ")?;
                match label {
                    Some(label) => write!(f, "(error {label})")?,
                    None => f.write_str("success")?,
                };
                write!(f, ") (payload {inner}))")
            }
            Self::Wild => f.write_str("(wild)"),
            Self::Unit => f.write_str("(unit-pattern)"),
            Self::Variant {
                declaration,
                index,
                fields,
            } => {
                write!(f, "(variant-pattern {declaration} {index} (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Literal(value) => write!(f, "(constant {value})"),
            Self::Binding { binding, inner } => write!(f, "(binding {binding} {inner})"),
            Self::Or(patterns) => {
                f.write_str("(or-pattern")?;
                for pattern in patterns {
                    write!(f, " {pattern}")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for MatchArm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(arm (pattern {}) (guard", self.pattern)?;
        if let Some(guard) = &self.guard {
            write!(f, " {guard}")?;
        }
        write!(f, ") (body {}))", self.body)
    }
}
impl fmt::Display for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(capture (binding {}) (type {})", self.binding, self.ty)?;
        if let Some(value) = &self.value {
            write!(f, " (value {value})")?;
        }
        f.write_str(")")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FunctionPhase {
    #[default]
    Runtime,
    Comptime,
    /// Opaque identity: a checked definition, never an ordinary invocation entry.
    NormalForm,
}
impl fmt::Display for FunctionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Runtime => "runtime",
            Self::Comptime => "comptime",
            Self::NormalForm => "normal-form",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntrySnapshot {
    pub parameter: LocalId,
    pub binding: LocalId,
}
impl fmt::Display for EntrySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(entry-snapshot (parameter {}) (binding {}))",
            self.parameter, self.binding
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub phase: FunctionPhase,
    /// Source inlining intent; not mandatory expansion or a purity promise.
    pub inline: bool,
    /// Trusted source promise; never authorizes normal-form unfolding.
    pub pure: bool,
    /// Caller must establish an explicit unsafe boundary; independent of phase/purity.
    pub is_unsafe: bool,
    /// Checked program entry predicates; never sent to the type solver.
    pub preconditions: Vec<Expr>,
    pub postconditions: Vec<Expr>,
    pub result_binding: Option<LocalId>,
    pub entry_snapshots: Vec<EntrySnapshot>,
    pub capabilities: Vec<TraitRequirement>,
    pub requirements: Vec<TypePredicate>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub name: String,
    pub signature: Term,
    pub parameters: Vec<LocalId>,
    pub body: Expr,
    pub defaults: Vec<(usize, Expr)>,
}
impl Function {
    /// Value interface of this declaration; the stored signature remains its
    /// parameter/result telescope for instantiation and ABI consumers.
    pub fn callable_type(&self) -> Term {
        self.signature
            .clone()
            .with_comptime_phase(self.phase == FunctionPhase::Comptime)
            .with_unsafe_call_requirement(self.is_unsafe)
    }

    /// Specialize every type-bearing part of a function template together.
    /// Declaration metadata and local identities are retained: selecting which
    /// binders disappear, replacing value locals and erasing staged arguments
    /// belong to the caller's instantiation pass.
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        // Type-bearing payloads are constructed once. Cloning the complete
        // template here would duplicate and then discard its body, contracts,
        // and defaults before their specialized copies replace them.
        Ok(Self {
            phase: self.phase,
            inline: self.inline,
            pure: self.pure,
            is_unsafe: self.is_unsafe,
            result_binding: self.result_binding,
            entry_snapshots: self.entry_snapshots.clone(),
            type_parameters: self.type_parameters.clone(),
            declaration: self.declaration,
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            signature: self
                .signature
                .instantiate_parameters(parameters, arguments)?,
            requirements: self
                .requirements
                .iter()
                .map(|p| p.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            capabilities: self
                .capabilities
                .iter()
                .map(|p| p.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            body: self.body.instantiate_parameters(parameters, arguments)?,
            preconditions: self
                .preconditions
                .iter()
                .map(|e| e.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            postconditions: self
                .postconditions
                .iter()
                .map(|e| e.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            defaults: self
                .defaults
                .iter()
                .map(|(index, e)| {
                    e.instantiate_parameters(parameters, arguments)
                        .map(|e| (*index, e))
                })
                .collect::<Result<_, _>>()?,
        })
    }

    /// Specialize a declaration using completed domains while retaining its
    /// ordinary input frame in contracts, defaults and body annotations.
    /// Callers separately admit values and charge the template copy.
    pub fn instantiate_declaration_parameters(
        &self,
        domains: &[Term],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        // Type-only declarations cannot change binder sorts. Reuse their
        // strict substitution path without rescanning wide, unused Type inputs.
        if domains
            .iter()
            .all(|domain| *domain == Term::Primitive(crate::Primitive::Type))
        {
            *fuel = fuel
                .checked_sub(self.type_parameters.len())
                .ok_or(crate::type_computations::Error::Budget)?;
            let binders = crate::declaration_parameters(&self.type_parameters, domains)
                .map_err(crate::type_computations::Error::Scope)?;
            return self
                .instantiate_parameters(&binders, arguments)
                .map_err(crate::type_computations::Error::Scope);
        }
        let closed = crate::DeclarationSubstitution::bounded(
            &self.type_parameters,
            domains,
            arguments,
            &[],
            fuel,
        )?;
        let frames = match self.signature.callable_signature() {
            Term::Function { parameters, .. } => vec![parameters.iter().map(|p| p.sort).collect()],
            _ => vec![],
        };
        let scoped = crate::DeclarationSubstitution::bounded(
            &self.type_parameters,
            domains,
            arguments,
            &frames,
            fuel,
        )?;
        let specialized = (|| -> Result<Self, crate::TypeError> {
            let expression = |value: &Expr| {
                let mut value = value.clone();
                value.rewrite_types(&mut |ty| scoped.instantiate(ty))?;
                Ok::<_, crate::TypeError>(value)
            };
            Ok(Self {
                phase: self.phase,
                inline: self.inline,
                pure: self.pure,
                is_unsafe: self.is_unsafe,
                result_binding: self.result_binding,
                entry_snapshots: self.entry_snapshots.clone(),
                type_parameters: self.type_parameters.clone(),
                declaration: self.declaration,
                name: self.name.clone(),
                parameters: self.parameters.clone(),
                signature: closed.instantiate(&self.signature)?,
                requirements: self
                    .requirements
                    .iter()
                    .map(|p| scoped.instantiate_predicate(p))
                    .collect::<Result<_, _>>()?,
                capabilities: self
                    .capabilities
                    .iter()
                    .map(|capability| {
                        let TypePredicate::Trait(value) = scoped
                            .instantiate_predicate(&TypePredicate::Trait(capability.clone()))?
                        else {
                            unreachable!()
                        };
                        Ok(value)
                    })
                    .collect::<Result<_, crate::TypeError>>()?,
                body: expression(&self.body)?,
                preconditions: self
                    .preconditions
                    .iter()
                    .map(expression)
                    .collect::<Result<_, _>>()?,
                postconditions: self
                    .postconditions
                    .iter()
                    .map(expression)
                    .collect::<Result<_, _>>()?,
                defaults: self
                    .defaults
                    .iter()
                    .map(|(index, value)| expression(value).map(|value| (*index, value)))
                    .collect::<Result<_, _>>()?,
            })
        })();
        specialized.map_err(crate::type_computations::Error::Scope)
    }

    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| crate::DeclarationParameter {
                declaration,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InterfaceInstance {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
}
impl From<DeclarationId> for InterfaceInstance {
    fn from(declaration: DeclarationId) -> Self {
        Self {
            declaration,
            arguments: vec![],
        }
    }
}
impl InterfaceInstance {
    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters_in_scope(parameters, arguments, frames)?;
        Ok(Self {
            declaration: self.declaration,
            arguments: self
                .arguments
                .iter()
                .map(|ty| ty.instantiate_parameters_in_scope(parameters, arguments, frames))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| crate::DeclarationParameter {
                declaration,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }

    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        Ok(Self {
            declaration: self.declaration,
            arguments: self
                .arguments
                .iter()
                .map(|t| t.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
        })
    }
}
impl fmt::Display for InterfaceInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(interface-instance {} (arguments", self.declaration)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, "))")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitRequirement {
    pub interface_arguments: Vec<Term>,
    pub subject: Term,
    pub interface: DeclarationId,
}
impl TraitRequirement {
    /// Substitute declaration inputs while retaining ordinary caller bindings.
    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        Ok(Self {
            interface: self.interface,
            subject: self
                .subject
                .instantiate_parameters_in_scope(parameters, arguments, frames)?,
            interface_arguments: self
                .interface_arguments
                .iter()
                .map(|ty| ty.instantiate_parameters_in_scope(parameters, arguments, frames))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn mentions_rigid(&self, declaration: DeclarationId) -> bool {
        self.subject.mentions_rigid(declaration)
            || self
                .interface_arguments
                .iter()
                .any(|argument| argument.mentions_rigid(declaration))
    }
    pub fn interface_instance(&self) -> InterfaceInstance {
        InterfaceInstance {
            declaration: self.interface,
            arguments: self.interface_arguments.clone(),
        }
    }
    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| crate::DeclarationParameter {
                declaration,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }

    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        Ok(Self {
            interface: self.interface,
            subject: self.subject.instantiate_parameters(parameters, arguments)?,
            interface_arguments: self
                .interface_instance()
                .instantiate_parameters(parameters, arguments)?
                .arguments,
        })
    }
}
impl fmt::Display for TraitRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(trait-requirement (subject {}) (interface {}))",
            self.subject,
            self.interface_instance()
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    /// Driver-authorized builtin implementation packages retained across snapshots.
    pub builtin_implementation_packages: std::collections::BTreeSet<u64>,
    /// Explicit compilation target facts. None is target-independent; it must
    /// never be filled from the process host during semantic queries.
    pub target: Option<crate::target::TargetData>,
    /// Ordinary source tests, with their independent declaration identities.
    pub tests: Vec<DeclarationId>,
    /// Callable declaration telescopes; available even when a body is omitted
    /// from a required invocation snapshot. They do not authorize execution.
    pub function_parameters: Vec<crate::function_parameters::Declaration>,
    /// Completed residual bodies supplied by a concrete runtime graph. Keys
    /// retain both clause arguments and ordinary staged invocation inputs.
    pub runtime_instances: Vec<RuntimeInstance>,
    /// Checked runtime code templates retained for deterministic expansion.
    /// These are not executable functions or runtime function identities.
    pub quote_templates: Vec<Function>,
    /// Next identity in the shared hygienic-expansion namespace.
    pub quote_identity_cursor: u32,
    /// Next resolver identity after source scanning and committed generation.
    /// This includes declarations omitted from an invocation snapshot.
    pub definition_identity_cursor: u32,
    /// Compiler-owned result envelopes have no user-defined destructor. This
    /// provenance permits structural cleanup reasoning, never implicit Copy.
    pub internal_return_packets: std::collections::BTreeSet<DeclarationId>,
    /// At these generated replacement sites the private packet is still empty:
    /// every successful write immediately exits its logical return boundary.
    pub internal_return_empty_replacements: std::collections::BTreeMap<(u32, u32), DeclarationId>,
    /// Index corresponds to SourceRange::generated(index). Entries retain
    /// both the definition location and the site of each hygienic connection.
    pub expansion_sources: Vec<ExpansionSource>,
    pub effects: Vec<crate::effects::Declaration>,
    /// Body-free identities carried by required comptime evaluation. These do
    /// not authorize execution and must not substitute for runtime definitions.
    pub runtime_function_identities: Vec<RuntimeFunctionIdentity>,
    /// Declaration-ordered inputs for opaque heads; separate from executable
    /// function parameters so Type/value interleaving is never reconstructed.
    pub normal_forms: Vec<crate::normal_forms::Head>,
    pub dispatch_instances: Vec<DispatchInstance>,
    pub traits: Vec<Trait>,
    pub implementations: Vec<Implementation>,
    pub functions: Vec<Function>,
    pub structs: Vec<Struct>,
    pub newtypes: Vec<crate::newtypes::Newtype>,
    pub alias_contracts: Vec<crate::type_aliases::Contract>,
    pub enums: Vec<Enum>,
}

/// Acquired cleanup accesses, expressed relative to one residual invocation's inputs.
/// This supplements the body footprint; it is not a standalone safety certificate.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCleanupAccess {
    pub parameter: usize,
    pub input_fields: Vec<usize>,
    pub through: Vec<Vec<(FieldOwner, usize)>>,
    pub fields: Vec<(FieldOwner, usize)>,
    pub write: bool,
    pub read_value: bool,
    pub ty: Term,
    pub conditions: std::collections::BTreeMap<usize, Vec<(Vec<usize>, usize)>>,
}
impl fmt::Display for RuntimeCleanupAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-access (parameter {}) (input-fields",
            self.parameter
        )?;
        for field in &self.input_fields {
            write!(f, " {field}")?;
        }
        f.write_str(") (through")?;
        for path in &self.through {
            f.write_str(" (load-pointer")?;
            for (owner, index) in path {
                write!(f, " (field {owner} {index})")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (fields")?;
        for (owner, index) in &self.fields {
            write!(f, " (field {owner} {index})")?;
        }
        f.write_str(")")?;
        for (stage, guards) in &self.conditions {
            write!(f, " (conditions (stage {stage})")?;
            for (path, tag) in guards {
                f.write_str(" (guard (fields")?;
                for field in path {
                    write!(f, " {field}")?;
                }
                write!(f, ") (variant {tag}))")?;
            }
            f.write_str(")")?;
        }
        write!(
            f,
            " (write {}) (read-value {}) (type {}))",
            self.write, self.read_value, self.ty
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeInstance {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    pub function: Function,
    pub cleanup_accesses: Vec<RuntimeCleanupAccess>,
}
impl fmt::Display for RuntimeInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(runtime-instance {} (arguments", self.declaration)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (cleanup-accesses")?;
        for access in &self.cleanup_accesses {
            write!(f, " {access}")?;
        }
        write!(f, ") {})", self.function)
    }
}
impl Package {
    /// Return the exact residual body when present. Completed bodies have no
    /// remaining clause substitutions; their invocation identity stays in the key.
    pub fn runtime_body<'a>(
        &'a self,
        declaration: DeclarationId,
        arguments: &'a [Term],
    ) -> Option<(&'a Function, &'a [Term])> {
        self.runtime_body_checked(declaration, arguments).ok()
    }

    /// Diagnostic-preserving form of runtime_body. An invalid selected
    /// instance is an error, never permission to use the source definition.
    pub fn runtime_body_checked<'a>(
        &'a self,
        declaration: DeclarationId,
        arguments: &'a [Term],
    ) -> Result<(&'a Function, &'a [Term]), &'static str> {
        let mut matches = self.runtime_instances.iter().filter(|instance| {
            instance.declaration == declaration && instance.arguments == arguments
        });
        if let Some(instance) = matches.next() {
            if matches.next().is_some()
                || instance.function.declaration != declaration
                || !instance.function.type_parameters.is_empty()
                || instance.function.phase != FunctionPhase::Runtime
                || !matches!(&instance.function.signature, Term::Function { parameters, .. }
                    if parameters.len() == instance.function.parameters.len()
                        && parameters.iter().all(|p| p.sort == crate::Sort::Runtime))
            {
                return Err("runtime function instance is ambiguous or malformed");
            }
            return Ok((&instance.function, &[]));
        }
        let mut declarations = self
            .functions
            .iter()
            .chain(&self.quote_templates)
            .filter(|function| function.declaration == declaration);
        let function = declarations.next().ok_or("unknown function")?;
        if declarations.next().is_some() {
            return Err("function declaration is ambiguous");
        }
        if function.type_parameters.len() != arguments.len() {
            return Err("function type arguments have incorrect arity");
        }
        Ok((function, arguments))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeFunctionIdentity {
    /// The producer checks formation obligations for each use before invoking
    /// the interpreter. Staged runtime templates may carry program contracts:
    /// the identity is transport-only and calls recover the source declaration
    /// and its contracts during specialization. Ordinary contracted callable
    /// values must not be erased into this signature-only representation.
    pub declaration: DeclarationId,
    pub type_parameters: Vec<DeclarationId>,
    pub signature: Term,
}
impl fmt::Display for RuntimeFunctionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(runtime-function-identity {} (type-parameters",
            self.declaration
        )?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (signature {}))", self.signature)
    }
}

/// A compiler-selected concrete interface instance. Execution only looks up
/// this table; implementation templates are never searched at runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct DispatchInstance {
    pub interface_arguments: Vec<Term>,
    pub interface: DeclarationId,
    pub subject: Term,
    pub implementation: DeclarationId,
    pub arguments: Vec<Term>,
    pub methods: Vec<MethodInstance>,
    /// Demanded, completed associated values selected by elaboration.
    pub associated: Vec<(DeclarationId, Expr)>,
    /// Required comptime programs, kept separate from completed data values.
    pub associated_initializers: Vec<(
        DeclarationId,
        crate::associated_initializers::AssociatedInitializer,
    )>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MethodInstance {
    pub method_arguments: Vec<Term>,
    pub member: DeclarationId,
    pub body: DeclarationId,
    pub arguments: Vec<Term>,
}
impl DispatchInstance {
    pub fn interface_instance(&self) -> InterfaceInstance {
        InterfaceInstance {
            declaration: self.interface,
            arguments: self.interface_arguments.clone(),
        }
    }
}
impl Implementation {
    pub fn interface_instance(&self) -> Option<InterfaceInstance> {
        self.interface.map(|declaration| InterfaceInstance {
            declaration,
            arguments: self.interface_arguments.clone(),
        })
    }
}
impl Trait {
    pub fn parameters_with_self(&self) -> Vec<DeclarationId> {
        std::iter::once(self.self_parameter)
            .chain(self.type_parameters.iter().copied())
            .collect()
    }
}
impl fmt::Display for DispatchInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(dispatch-instance (interface {}) (subject {}) (implementation {}) (arguments",
            InterfaceInstance {
                declaration: self.interface,
                arguments: self.interface_arguments.clone()
            },
            self.subject,
            self.implementation
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (methods")?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        write!(f, ") (associated")?;
        for (member, value) in &self.associated {
            write!(f, " (value (member {member}) {value})")?;
        }
        write!(f, ") (associated-initializers")?;
        for (member, value) in &self.associated_initializers {
            write!(f, " (initializer {member} {value})")?;
        }
        write!(f, "))")
    }
}
impl fmt::Display for MethodInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method-instance (member {}) (body {}) (arguments",
            self.member, self.body
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (method-arguments")?;
        for argument in &self.method_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, "))")
    }
}

/// Interface membership, independent of a function's execution phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraitPhase {
    #[default]
    Runtime,
    Comptime,
}
impl fmt::Display for TraitPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Runtime => "runtime",
            Self::Comptime => "comptime",
        })
    }
}

/// The contributions admitted for one selected implementation relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImplementationPhases {
    pub runtime: bool,
    pub comptime: bool,
}
impl Default for ImplementationPhases {
    fn default() -> Self {
        Self {
            runtime: true,
            comptime: false,
        }
    }
}
impl ImplementationPhases {
    pub fn contains(self, phase: TraitPhase) -> bool {
        match phase {
            TraitPhase::Runtime => self.runtime,
            TraitPhase::Comptime => self.comptime,
        }
    }
}
impl fmt::Display for ImplementationPhases {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(phases (runtime {}) (comptime {}))",
            self.runtime, self.comptime
        )
    }
}

/// A declared associated value. Type is one possible value type.
#[derive(Clone, Debug, PartialEq)]
pub struct AssociatedContract {
    pub phase: TraitPhase,
    pub declaration: DeclarationId,
    pub name: String,
    pub ty: Term,
    pub default: Option<Expr>,
}
impl fmt::Display for AssociatedContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated {} (name {:?}) (phase {}) (type {}) (default",
            self.declaration, self.name, self.phase, self.ty
        )?;
        if let Some(value) = &self.default {
            write!(f, " {value}")?;
        }
        write!(f, "))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Trait {
    /// Ordered declaration input domains. Empty is the compact all-Type form;
    /// otherwise there must be exactly one domain per declaration parameter.
    pub parameter_types: Vec<Term>,
    pub associated: Vec<AssociatedContract>,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub parents: Vec<InterfaceInstance>,
    /// Rigid definition-side Self identity, substituted at an interface use.
    pub self_parameter: DeclarationId,
    pub name: String,
    pub methods: Vec<MethodContract>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MethodContract {
    pub phase: TraitPhase,
    /// Deterministic code application required by the interface; independent
    /// of execution phase and purity. Never authorizes a runtime call fallback.
    pub is_quote: bool,
    /// Runtime P4 promise; not a termination guarantee or a type predicate.
    pub pure: bool,
    /// Caller must establish an explicit unsafe boundary; independent of phase/purity.
    pub is_unsafe: bool,
    /// Concrete method arguments for a retained specialized interface entry.
    pub type_arguments: Vec<Term>,
    /// Method-local binders, excluding Self and enclosing trait parameters.
    pub type_parameters: Vec<DeclarationId>,
    pub capabilities: Vec<TraitRequirement>,
    pub program: crate::contracts::ProgramContract,
    /// Definition-side locals used while preparing interface arguments.
    pub parameter_bindings: Vec<LocalId>,
    pub by_value_receiver: bool,
    pub by_reference_receiver: bool,
    pub declaration: DeclarationId,
    pub name: String,
    pub signature: crate::code_interfaces::MethodSignature,
    pub requirements: Vec<TypePredicate>,
    pub default: Option<DeclarationId>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Implementation {
    pub phases: ImplementationPhases,
    /// Ordered declaration domains; empty denotes all Type parameters.
    pub parameter_types: Vec<Term>,
    /// Values indexed by the originating interface member, in declaration order.
    pub associated: Vec<(DeclarationId, Expr)>,
    /// Required comptime programs, kept separate from completed data values.
    pub associated_initializers: Vec<(
        DeclarationId,
        crate::associated_initializers::AssociatedInitializer,
    )>,
    pub interface_arguments: Vec<Term>,
    pub capabilities: Vec<TraitRequirement>,
    /// Rigid declaration parameters; invocation substitutes only after selection.
    pub type_parameters: Vec<DeclarationId>,
    pub requirements: Vec<TypePredicate>,
    pub declaration: DeclarationId,
    pub subject: Term,
    pub interface: Option<DeclarationId>,
    /// Contract order, with each entry retaining the selected body's identity.
    pub methods: Vec<(String, DeclarationId)>,
}

impl fmt::Display for Trait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(trait {} (self-parameter {}) (name {:?}) (type-parameters",
            self.declaration, self.self_parameter, self.name
        )?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (parameter-types")?;
        if self.parameter_types.is_empty() {
            for _ in &self.type_parameters {
                write!(f, " {}", Term::Primitive(crate::Primitive::Type))?;
            }
        } else {
            for ty in &self.parameter_types {
                write!(f, " {ty}")?;
            }
        }
        write!(f, ") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        write!(f, ") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        write!(f, ") (parents")?;
        for parent in &self.parents {
            write!(f, " {parent}")?;
        }
        write!(f, ") (methods")?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        write!(f, ") (associated")?;
        for item in &self.associated {
            write!(f, " {item}")?;
        }
        write!(f, "))")
    }
}
impl fmt::Display for MethodContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(method {} (name {:?}) (phase {}) (quote {}) (pure {}) (unsafe {}) (by-value-receiver {}) (by-reference-receiver {}) (signature {}) (requires",
            self.declaration,
            self.name,
            self.phase,
            self.is_quote,
            self.pure,
            self.is_unsafe,
            self.by_value_receiver,
            self.by_reference_receiver,
            self.signature
        )?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        write!(f, ") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (type-arguments")?;
        for argument in &self.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        write!(f, ") (parameter-bindings")?;
        for binding in &self.parameter_bindings {
            write!(f, " {binding}")?;
        }
        write!(f, ") {} (default", self.program)?;
        if let Some(default) = self.default {
            write!(f, " {default}")?;
        }
        write!(f, "))")
    }
}
impl Implementation {
    pub fn parameter_binders(&self) -> Result<Vec<crate::DeclarationParameter>, crate::TypeError> {
        crate::declaration_parameters(&self.type_parameters, &self.parameter_types)
    }
}
impl fmt::Display for Implementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(implementation {} {} (type-parameters",
            self.declaration, self.phases
        )?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (parameter-types")?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
        }
        write!(f, ") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        write!(f, ") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        write!(f, ") (subject {}) (interface", self.subject)?;
        if let Some(interface) = self.interface {
            write!(
                f,
                " {}",
                InterfaceInstance {
                    declaration: interface,
                    arguments: self.interface_arguments.clone()
                }
            )?;
        }
        write!(f, ") (methods")?;
        for (name, declaration) in &self.methods {
            write!(f, " (method {name:?} {declaration})")?;
        }
        write!(f, ") (associated")?;
        for (member, value) in &self.associated {
            write!(f, " (value {member} {value})")?;
        }
        write!(f, ") (associated-initializers")?;
        for (member, value) in &self.associated_initializers {
            write!(f, " (initializer {member} {value})")?;
        }
        write!(f, "))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Enum {
    /// Source-declared operation-label family. Target support needs a separate binding.
    pub is_intrinsic: bool,
    /// Ordered declaration input domains; empty is the compact all-Type form.
    pub parameter_types: Vec<Term>,
    pub capabilities: Vec<TraitRequirement>,
    pub requirements: Vec<TypePredicate>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub name: String,
    pub variants: Vec<Variant>,
    /// Fully qualified group paths; groups never occupy variant indices.
    pub groups: Vec<String>,
    pub encoding: Option<crate::enum_encoding::Representation>,
    pub symbol_encoding: Option<crate::enum_encoding::SymbolRepresentation>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Vec<Term>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum VariantShape {
    Unit,
    Tuple,
    Named(Vec<String>),
}
impl Enum {
    /// Check completed indices before constructing a concrete layout instance.
    pub fn validate_completed_arguments(
        &self,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), crate::nominal_parameters::AdmissionError> {
        crate::nominal_parameters::validate_completed(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            fuel,
        )
    }

    pub fn parameter_binders(&self) -> Result<Vec<crate::DeclarationParameter>, crate::TypeError> {
        crate::nominal_parameters::binders(&self.type_parameters, &self.parameter_types)
    }
    pub fn validate_encoding(&self) -> Result<(), &'static str> {
        self.validate_encoding_in(None)
    }

    pub fn validate_encoding_in(
        &self,
        target: Option<crate::target::TargetData>,
    ) -> Result<(), &'static str> {
        let groups = self
            .groups
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if groups.len() != self.groups.len() {
            return Err("duplicate enum group");
        }
        let mut leaves = std::collections::BTreeSet::new();
        for name in &self.groups {
            if name.is_empty()
                || name.split('.').any(str::is_empty)
                || name
                    .rsplit_once('.')
                    .is_some_and(|(parent, _)| !groups.contains(parent))
            {
                return Err("enum group path has no declared parent");
            }
        }
        for variant in &self.variants {
            if !leaves.insert(&variant.name) || groups.contains(variant.name.as_str()) {
                return Err("enum leaf and group names are not unique");
            }
            if variant
                .name
                .rsplit_once('.')
                .is_some_and(|(parent, _)| !groups.contains(parent))
            {
                return Err("enum leaf path has no declared group");
            }
        }
        if let Some(symbols) = &self.symbol_encoding {
            if self.encoding.is_some() {
                return Err("enum has conflicting representation domains");
            }
            if symbols.variants().len() != self.variants.len()
                || symbols
                    .variants()
                    .iter()
                    .zip(&self.variants)
                    .any(|(encoded, variant)| {
                        encoded.name != variant.name
                            || variant.shape != VariantShape::Unit
                            || !variant.fields.is_empty()
                    })
            {
                return Err("Symbol enum encoding does not describe the logical variants");
            }
        }
        let Some(layout) = &self.encoding else {
            return Ok(());
        };
        layout.validate_in(target)?;
        if layout.variants().len() != self.variants.len() {
            return Err("enum encoding variant count differs from the logical enum");
        }
        for (variant, encoded) in self.variants.iter().zip(layout.variants()) {
            if variant.name != encoded.name() || variant.fields.len() != encoded.fields().len() {
                return Err("enum encoding does not describe the logical variant");
            }
            match &variant.shape {
                VariantShape::Unit if encoded.fields().is_empty() => (),
                VariantShape::Named(names)
                    if names.len() == encoded.fields().len()
                        && names
                            .iter()
                            .zip(encoded.fields())
                            .all(|(name, field)| *name == field.name) =>
                {
                    ()
                }
                _ => return Err("enum encoding payload names differ from the logical variant"),
            }
            for (ty, field) in variant.fields.iter().zip(encoded.fields()) {
                let width = match ty {
                    Term::Primitive(crate::Primitive::BitVec(width)) => Some(*width),
                    Term::Primitive(primitive) => primitive
                        .unrestricted_scalar_width_in(target)
                        .map(|w| w as usize),
                    _ => None,
                };
                if width != Some(field.width) {
                    return Err("enum encoding payload type has a different representation");
                }
            }
        }
        Ok(())
    }
    pub fn instantiate_variants(
        &self,
        arguments: &[Term],
    ) -> Result<Vec<Variant>, crate::TypeError> {
        self.instantiate_variants_in_scope(arguments, &[])
    }
    /// Instantiate payloads in caller frames, preserving nested binder depths.
    pub fn instantiate_variants_in_scope(
        &self,
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Vec<Variant>, crate::TypeError> {
        let substitution = crate::DeclarationSubstitution::new(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            frames,
        )?;
        self.variants
            .iter()
            .map(|variant| {
                Ok(Variant {
                    name: variant.name.clone(),
                    shape: variant.shape.clone(),
                    fields: variant
                        .fields
                        .iter()
                        .map(|ty| substitution.instantiate(ty))
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect()
    }
}
impl fmt::Display for Enum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(enum (id {}) (name ", self.declaration)?;
        string(f, &self.name)?;
        write!(f, ") (intrinsic {}) (type-parameters", self.is_intrinsic)?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (parameter-types")?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
        }
        f.write_str(") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        if let Some(encoding) = &self.encoding {
            write!(f, ") (encoding {encoding}")?;
        }
        if let Some(encoding) = &self.symbol_encoding {
            write!(f, ") (encoding {encoding}")?;
        }
        if !self.groups.is_empty() {
            f.write_str(") (groups")?;
            for group in &self.groups {
                f.write_str(" ")?;
                string(f, group)?;
            }
        }
        f.write_str(") (variants")?;
        for variant in &self.variants {
            f.write_str(" (variant (name ")?;
            string(f, &variant.name)?;
            f.write_str(") (shape ")?;
            match &variant.shape {
                VariantShape::Unit => f.write_str("unit")?,
                VariantShape::Tuple => f.write_str("tuple")?,
                VariantShape::Named(names) => {
                    f.write_str("(named")?;
                    for name in names {
                        f.write_str(" ")?;
                        string(f, name)?;
                    }
                    f.write_str(")")?;
                }
            }
            f.write_str(") (fields")?;
            for ty in &variant.fields {
                write!(f, " {ty}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Struct {
    /// Ordered declaration input domains; empty is the compact all-Type form.
    pub parameter_types: Vec<Term>,
    pub capabilities: Vec<TraitRequirement>,
    pub requirements: Vec<TypePredicate>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub name: String,
    pub fields: Vec<(String, Term)>,
}
impl Struct {
    /// Check completed indices before constructing a concrete layout instance.
    pub fn validate_completed_arguments(
        &self,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), crate::nominal_parameters::AdmissionError> {
        crate::nominal_parameters::validate_completed(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            fuel,
        )
    }

    pub fn parameter_binders(&self) -> Result<Vec<crate::DeclarationParameter>, crate::TypeError> {
        crate::nominal_parameters::binders(&self.type_parameters, &self.parameter_types)
    }
    pub fn instantiate_fields(
        &self,
        arguments: &[Term],
    ) -> Result<Vec<(String, Term)>, crate::TypeError> {
        self.instantiate_fields_in_scope(arguments, &[])
    }
    /// Instantiate fields in caller frames; the closed entrypoint stays strict.
    pub fn instantiate_fields_in_scope(
        &self,
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Vec<(String, Term)>, crate::TypeError> {
        let substitution = crate::DeclarationSubstitution::new(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            frames,
        )?;
        self.fields
            .iter()
            .map(|(name, ty)| Ok((name.clone(), substitution.instantiate(ty)?)))
            .collect()
    }
}
impl fmt::Display for Struct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(struct (id {}) (name ", self.declaration)?;
        string(f, &self.name)?;
        f.write_str(") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (parameter-types")?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
        }
        f.write_str(") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (fields")?;
        for (name, ty) in &self.fields {
            f.write_str(" (field (name ")?;
            string(f, name)?;
            write!(f, ") (type {ty}))")?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(local {} {})", self.owner, self.index)
    }
}
impl fmt::Display for SourceRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(index) = self.expansion_index() {
            write!(f, "(generated-source {index})")
        } else {
            write!(f, "(source {} {})", self.start, self.end)
        }
    }
}
impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BitVec { width, words } => {
                write!(f, "(bitvec (width {width}) (words")?;
                for word in words {
                    write!(f, " {word}")?;
                }
                f.write_str("))")
            }
            Self::PendingFloat { spelling, negative } => {
                write!(f, "(pending-float (negative {negative}) (spelling ")?;
                string(f, spelling)?;
                f.write_str("))")
            }
            Self::PendingInteger { spelling, negative } => {
                write!(f, "(pending-integer (negative {negative}) (spelling ")?;
                string(f, spelling)?;
                f.write_str("))")
            }
            Self::Float16(bits) => write!(f, "(float16 (bits \"{bits:04x}\"))"),
            Self::BFloat16(bits) => write!(f, "(bfloat16 (bits \"{bits:04x}\"))"),
            Self::Float32(bits) => write!(f, "(float32 (bits \"{bits:08x}\"))"),
            Self::Float64(bits) => write!(f, "(float64 (bits \"{bits:016x}\"))"),
            Self::SignedInteger(value) => write!(f, "(signed-integer {value})"),
            Self::UnsignedInteger(value) => write!(f, "(unsigned-integer {value})"),
            Self::Bool(value) => write!(f, "(bool {value})"),
            Self::Char(value) => write!(f, "(char {})", *value as u32),
            Self::Symbol(value)
            | Self::String(value)
            | Self::Integer(value)
            | Self::Float(value) => {
                f.write_str(match self {
                    Self::String(_) => "(string ",
                    Self::Symbol(_) => "(symbol ",
                    Self::Integer(_) => "(integer ",
                    _ => "(float ",
                })?;
                string(f, value)?;
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(expr (type {}) {} {})", self.ty, self.source, self.kind)
    }
}
impl fmt::Display for ExprKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OptionalPresent(inner) => write!(f, "(optional-present {inner})"),
            Self::OptionalAbsent => f.write_str("(optional-absent)"),
            Self::OptionalType(inner) => write!(f, "(optional-type-value {inner})"),
            Self::EffectResume(value) => write!(f, "(effect-resume {value})"),
            Self::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                f.write_str("(effect-handle (outer")?;
                for operation in outer {
                    write!(f, " {operation}")?;
                }
                f.write_str(") (handlers")?;
                for handler in handlers {
                    write!(f, " {handler}")?;
                }
                write!(f, ") (computation {computation}))")
            }
            Self::EffectRun { computation, slots } => {
                f.write_str("(effect-run (slots")?;
                for slot in slots {
                    write!(f, " {slot}")?;
                }
                write!(f, ") (computation {computation}))")
            }
            Self::EffectComputation { effects, body } => {
                f.write_str("(effect-computation (allowed")?;
                for effect in effects {
                    write!(f, " {effect}")?;
                }
                write!(f, ") (body {body}))")
            }
            Self::EffectRequest {
                operation,
                abi,
                allowed_index,
                signature,
                arguments,
            } => {
                write!(
                    f,
                    "(effect-request {operation} (abi {abi}) (allowed-index {allowed_index}) (signature {signature}) (arguments"
                )?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::AssociatedValue {
                interface,
                member,
                subject,
            } => write!(
                f,
                "(associated-value (interface {interface}) (member {member}) (subject {subject}))"
            ),
            Self::Literal(x) => x.fmt(f),
            Self::Local(x) => x.fmt(f),
            Self::Function(x) => write!(f, "(function-ref {x})"),
            Self::TraitMethod {
                method_arguments,
                interface,
                interface_arguments,
                member,
                subject,
            } => {
                let interface = InterfaceInstance {
                    declaration: *interface,
                    arguments: interface_arguments.clone(),
                };
                write!(
                    f,
                    "(trait-method (interface {interface}) (member {member}) (subject {subject}) (method-arguments"
                )?;
                for argument in method_arguments {
                    write!(f, " {argument}")?;
                }
                write!(f, "))")
            }
            Self::FunctionInstance {
                declaration,
                arguments,
            } => {
                write!(f, "(function-instance {declaration} (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::Closure {
                parameters,
                captures,
                body,
            } => {
                f.write_str("(closure (parameters")?;
                for parameter in parameters {
                    write!(f, " {parameter}")?;
                }
                f.write_str(") (captures")?;
                for capture in captures {
                    write!(f, " {capture}")?;
                }
                write!(f, ") (body {body}))")
            }
            Self::Unit => f.write_str("(unit)"),
            Self::Variant {
                declaration,
                index,
                fields,
            } => {
                write!(f, "(variant {declaration} {index} (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Object { names, fields } => {
                f.write_str("(object (names")?;
                for name in names {
                    match name {
                        None => f.write_str(" (positional)")?,
                        Some(name) => {
                            f.write_str(" (property ")?;
                            crate::display::string(f, name)?;
                            f.write_str(")")?;
                        }
                    }
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::AnonymousRecord { fields } => {
                f.write_str("(anonymous-record (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Record {
                declaration,
                fields,
            } => {
                write!(f, "(record (declaration {declaration}) (fields")?;
                for value in fields {
                    write!(f, " {value}")?;
                }
                f.write_str("))")
            }
            Self::AnonymousField { value, name, index } => {
                f.write_str("(record-field-read (name ")?;
                crate::display::string(f, name)?;
                write!(f, ") (index {index}) (value {value}))")
            }
            Self::Field {
                value,
                declaration,
                index,
            } => write!(
                f,
                "(field-read (declaration {declaration}) (index {index}) (value {value}))"
            ),
            Self::Match { value, arms } => {
                write!(f, "(match (value {value}) (arms")?;
                for arm in arms {
                    write!(f, " {arm}")?;
                }
                f.write_str("))")
            }
            Self::Tuple(xs) => {
                f.write_str("(tuple")?;
                for x in xs {
                    write!(f, " {x}")?;
                }
                f.write_str(")")
            }
            Self::Block { statements, result } | Self::CallPreparation { statements, result } => {
                f.write_str(if matches!(self, Self::CallPreparation { .. }) {
                    "(call-preparation (statements"
                } else {
                    "(block (statements"
                })?;
                for x in statements {
                    write!(f, " {x}")?;
                }
                f.write_str(") (result")?;
                if let Some(x) = result {
                    write!(f, " {x}")?;
                }
                f.write_str("))")
            }
            Self::Let { binding, value } => write!(f, "(let {binding} {value})"),
            Self::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                write!(f, "(let-pattern (pattern {pattern}) (value {value}) (guard")?;
                if let Some(guard) = guard {
                    write!(f, " {guard}")?;
                }
                f.write_str(") (else")?;
                if let Some(otherwise) = otherwise {
                    write!(f, " {otherwise}")?;
                }
                f.write_str("))")
            }
            Self::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                write!(f, "(bit-write-pointer (pointer {pointer}) (selections")?;
                for selection in selections {
                    write!(f, " {selection}")?;
                }
                write!(f, ") (value {value}))")
            }
            Self::BitUpdate {
                place,
                storage_type,
                selections,
                value,
            } => {
                write!(
                    f,
                    "(bit-update (place {place}) (storage-type {storage_type}) (selections"
                )?;
                for selection in selections {
                    write!(f, " {selection}")?;
                }
                write!(f, ") (value {value}))")
            }
            Self::BitAssign {
                place,
                storage_type,
                start,
                end,
                value,
            } => write!(
                f,
                "(bit-assign (place {place}) (storage-type {storage_type}) (start {start}) (end {end}) (value {value}))"
            ),
            Self::Assign { place, value } => write!(f, "(assign {place} {value})"),
            Self::Loop { identity, body } => {
                write!(f, "(loop (identity {identity}) (body {body}))")
            }
            Self::Break(identity) => write!(f, "(break {identity})"),
            Self::Continue(identity) => write!(f, "(continue {identity})"),
            Self::RequirePredicate(value) => write!(f, "(require-predicate {value})"),
            Self::EnsurePredicate(value) => write!(f, "(ensure-predicate {value})"),
            Self::CheckPredicate(value) => write!(f, "(check-predicate (condition {value}))"),
            Self::Intrinsic {
                tag,
                inputs,
                binding,
            } => {
                write!(f, "(intrinsic {tag} (binding")?;
                if let Some(binding) = binding {
                    write!(f, " {binding}")?;
                }
                write!(f, ") (inputs")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, "))")
            }
            Self::UnsafeScope(x) => write!(f, "(unsafe-scope {x})"),
            Self::Discard(x) => write!(f, "(discard {x})"),
            Self::If {
                condition,
                then_branch,
                else_branch,
            } => {
                write!(f, "(if {condition} {then_branch}")?;
                if let Some(x) = else_branch {
                    write!(f, " {x}")?;
                }
                f.write_str(")")
            }
            Self::Return(x) => {
                f.write_str("(return")?;
                if let Some(x) = x {
                    write!(f, " {x}")?;
                }
                f.write_str(")")
            }
            Self::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                let kind = if callee.ty.is_normal_form_callable() {
                    "normal-form-apply"
                } else {
                    "call"
                };
                write!(f, "({kind} (callee {callee}) (parameter-bindings")?;
                for binding in parameter_bindings {
                    write!(f, " {binding}")?;
                }
                f.write_str(") (arguments")?;
                for x in arguments {
                    write!(f, " {x}")?;
                }
                f.write_str("))")
            }
            Self::BitWindow { value, start, end } => write!(
                f,
                "(bit-window (value {value}) (start {start}) (end {end}))"
            ),
            Self::BitIndex(value, index) => {
                write!(f, "(bit-index (value {value}) (index {index}))")
            }
            Self::BitSlice { value, start, end } => {
                write!(f, "(bit-slice (value {value}) (start {start}) (end {end}))")
            }
            Self::TypeValue(ty) => write!(f, "(type-value {ty})"),
            Self::ComptimeValue(value) => write!(f, "(comptime-value {value})"),
            Self::BuildMetadata {
                owner,
                selector,
                entries,
            } => {
                write!(
                    f,
                    "(build-metadata (owner {owner}) (selector {selector}) (entries"
                )?;
                for (alias, provider, value) in entries {
                    write!(
                        f,
                        " (entry (alias {}) (provider {provider}) (value",
                        crate::QuotedDisplay(alias)
                    )?;
                    if let Some(value) = value {
                        write!(f, " {value}")?;
                    } else {
                        f.write_str(" unavailable")?;
                    }
                    f.write_str("))")?;
                }
                f.write_str("))")
            }
            Self::TypeListValue(types) => {
                f.write_str("(type-list-value")?;
                for ty in types {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")
            }
            Self::TypeList(elements) => {
                f.write_str("(type-list (elements")?;
                for element in elements {
                    write!(f, " {element}")?;
                }
                f.write_str("))")
            }
            Self::NormalFormType {
                head,
                arguments,
                plan,
            } => {
                write!(f, "(normal-form-type-value (head {head}) (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(")")?;
                if let Some(plan) = plan {
                    write!(f, " {plan}")?;
                }
                f.write_str(")")
            }
            Self::CheckTypeAlias {
                declaration,
                arguments,
                body,
                preparations,
                bindings,
                order,
            } => {
                write!(
                    f,
                    "(check-type-alias (declaration {declaration}) (arguments"
                )?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(") (preparations")?;
                for preparation in preparations {
                    write!(
                        f,
                        " (before {} {})",
                        preparation.before, preparation.expression
                    )?;
                }
                f.write_str(") (bindings")?;
                if let Some(bindings) = bindings {
                    for binding in bindings {
                        write!(f, " {binding}")?;
                    }
                }
                f.write_str(") (order")?;
                if let Some(order) = order {
                    for slot in order {
                        write!(f, " {slot}")?;
                    }
                }
                f.write_str(") (body")?;
                if let Some(body) = body {
                    write!(f, " {body}")?;
                }
                f.write_str("))")
            }
            Self::NominalType {
                preparations,
                bindings,
                order,
                declaration,
                arguments,
            } => {
                write!(
                    f,
                    "(nominal-type-value (declaration {declaration}) (arguments"
                )?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(")")?;
                if let Some(order) = order {
                    f.write_str(" (order")?;
                    for slot in order {
                        write!(f, " {slot}")?;
                    }
                    f.write_str(")")?;
                }
                if let Some(bindings) = bindings {
                    f.write_str(" (bindings")?;
                    for binding in bindings {
                        write!(f, " {binding}")?;
                    }
                    f.write_str(")")?;
                }
                for preparation in preparations {
                    write!(
                        f,
                        " (prepare (before {}) {})",
                        preparation.before, preparation.expression
                    )?;
                }
                f.write_str(")")
            }
            Self::ScalarGreater(left, right) => {
                write!(f, "(scalar-greater (left {left}) (right {right}))")
            }
            Self::ScalarGreaterEqual(left, right) => {
                write!(f, "(scalar-greater-equal (left {left}) (right {right}))")
            }
            Self::ScalarLess(left, right) => {
                write!(f, "(scalar-less (left {left}) (right {right}))")
            }
            Self::ScalarLessEqual(left, right) => {
                write!(f, "(scalar-less-equal (left {left}) (right {right}))")
            }
            Self::ScalarEqual(left, right) => {
                write!(f, "(scalar-equal (left {left}) (right {right}))")
            }
            Self::TemplateEqual(left, right) => {
                write!(f, "(template-equal (left {left}) (right {right}))")
            }
            Self::ListLength(values) => write!(f, "(list-length {values})"),
            Self::StructFieldTypes(subject) => write!(f, "(struct-field-types {subject})"),
            Self::BuildStruct(request) => write!(f, "(build-struct {request})"),
            Self::BuildEnum(request) => write!(f, "(build-enum {request})"),
            Self::EnumStructure { subject, query } => {
                write!(f, "(enum-structure (query {query}) {subject})")
            }
            Self::ComptimeFunctionInstance {
                declaration,
                subject,
            } => {
                write!(
                    f,
                    "(comptime-function-instance (declaration {declaration}) (inputs {subject}))"
                )
            }
            Self::ObjectEdit { subject, operation } => {
                write!(f, "(object-edit (operation {operation}) {subject})")
            }
            Self::ObjectQuery { subject, query } => {
                write!(f, "(object-query (query {query}) {subject})")
            }
            Self::StructFieldNames(subject) => write!(f, "(struct-field-names {subject})"),
            Self::ListConcat(left, right) => write!(f, "(list-concat {left} {right})"),
            Self::TypeListIndex(values, index) => {
                write!(f, "(type-list-index (values {values}) (index {index}))")
            }
            Self::TypeEqual(left, right) => write!(f, "(type-equal (left {left}) (right {right}))"),
            Self::AddressOf(place) => write!(f, "(address-of (place {place}))"),
            Self::ReadPointer(pointer) => write!(f, "(read-pointer (pointer {pointer}))"),
            Self::PointerAnonymousField {
                pointer,
                name,
                index,
            } => {
                write!(f, "(pointer-record-field (pointer {pointer}) (name ")?;
                crate::display::string(f, name)?;
                write!(f, ") (index {index}))")
            }
            Self::PointerField {
                pointer,
                declaration,
                index,
            } => write!(
                f,
                "(pointer-field (pointer {pointer}) (declaration {declaration}) (index {index}))"
            ),
            Self::WritePointer { pointer, value } => {
                write!(f, "(write-pointer (pointer {pointer}) (value {value}))")
            }
            Self::PointerType(inner) => write!(f, "(pointer-type-value (pointee {inner}))"),
            Self::ClosureType {
                environment,
                signature,
            } => {
                f.write_str("(closure-type-value (environment")?;
                for field in environment {
                    write!(f, " {field}")?;
                }
                write!(f, ") (signature {signature}))")
            }
            Self::FunctionType {
                comptime,
                comptime_parameters,
                parameters,
            } => {
                write!(
                    f,
                    "(function-type-value (comptime {comptime}) (parameter-stages"
                )?;
                for staged in comptime_parameters {
                    write!(f, " {staged}")?;
                }
                f.write_str(") (parameters")?;
                for parameter in parameters {
                    write!(f, " {parameter}")?;
                }
                f.write_str("))")
            }
            Self::FunctionResultType { signature, result } => write!(
                f,
                "(function-result-type-value (signature {signature}) (result {result}))"
            ),
            Self::TupleType(fields) => {
                f.write_str("(tuple-type-value (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::BitVecType(width) => write!(f, "(bitvec-type-value (width {width}))"),
            Self::FloatNegate(value) => write!(f, "(float-negate (value {value}))"),
            Self::FloatBinary { op, left, right } => {
                write!(f, "(float-binary (op {op}) (left {left}) (right {right}))")
            }
            Self::IntegerNegate(value) => write!(f, "(integer-negate {value})"),
            Self::IntegerBinary { op, left, right } => {
                write!(f, "(integer-{op} (left {left}) (right {right}))")
            }
            Self::BitConvert(value) => write!(f, "(bit-convert {value})"),
            Self::ErrorConvert { conversion, value } => write!(
                f,
                "(error-convert (conversion {conversion}) (value {value}))"
            ),
            Self::NewtypeConvert(value) => write!(f, "(newtype-convert {value})"),
            Self::BitEqual(a, b) => write!(f, "(bit-equal {a} {b})"),
            Self::EnumName(x) => write!(f, "(enum-name {x})"),
            Self::IntegerRepresentation(x) => write!(f, "(integer-representation {x})"),
            Self::ComptimeIntegerConvert(x) => write!(f, "(comptime-integer-convert {x})"),
            Self::SymbolName(x) => write!(f, "(symbol-name {x})"),
            Self::TextQuery { subject, query } => write!(f, "(text-query {query} {subject})"),
            Self::Snapshot(x) => write!(f, "(snapshot {x})"),
            Self::TakeBits(x) => write!(f, "(take-bits {x})"),
            Self::Not(x) => write!(f, "(not {x})"),
            Self::BitConcat(a, b) => write!(f, "(bit-concat {a} {b})"),
            Self::And(a, b) => write!(f, "(and {a} {b})"),
            Self::Or(a, b) => write!(f, "(or {a} {b})"),
        }
    }
}
impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function {} (name ", self.declaration)?;
        string(f, &self.name)?;
        write!(
            f,
            ") (phase {}) (inline {}) (pure {}) (unsafe {}) (capabilities",
            self.phase, self.inline, self.pure, self.is_unsafe
        )?;
        for requirement in &self.capabilities {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (signature {}) (parameters", self.signature)?;
        for p in &self.parameters {
            write!(f, " {p}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (program-requires")?;
        for condition in &self.preconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in &self.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(binding) = self.result_binding {
            write!(f, " {binding}")?;
        }
        f.write_str(") (program-ensures")?;
        for condition in &self.postconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (defaults")?;
        for (parameter, value) in &self.defaults {
            write!(f, " (default (parameter {parameter}) {value})")?;
        }
        write!(f, ") (body {}))", self.body)
    }
}
impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(nir-package")?;
        if !self.builtin_implementation_packages.is_empty() {
            f.write_str(" (builtin-implementation-packages")?;
            for package in &self.builtin_implementation_packages {
                write!(f, " {package}")?;
            }
            f.write_str(")")?;
        }
        if self.definition_identity_cursor != 0 {
            write!(
                f,
                " (definition-identity-cursor {})",
                self.definition_identity_cursor
            )?;
        }
        if let Some(target) = self.target {
            write!(f, "\n {target}")?;
        }
        for test in &self.tests {
            write!(f, "\n (test-entry {test})")?;
        }
        for instance in &self.runtime_instances {
            write!(f, "\n {instance}")?;
        }

        if !self.quote_templates.is_empty() {
            write!(f, " (quote-identity-cursor {})", self.quote_identity_cursor)?;
        }
        for ((start, end), declaration) in &self.internal_return_empty_replacements {
            write!(
                f,
                "\n (internal-return-empty-replacement (source {start} {end}) (packet {declaration}))"
            )?;
        }
        for declaration in &self.internal_return_packets {
            write!(f, "\n (internal-return-packet {declaration})")?;
        }
        for (index, origin) in self.expansion_sources.iter().enumerate() {
            write!(f, "\n (expansion-source {index} {origin})")?;
        }
        for effect in &self.effects {
            write!(f, "\n {effect}")?;
        }
        for identity in &self.runtime_function_identities {
            write!(f, "\n {identity}")?;
        }
        for declaration in &self.function_parameters {
            write!(f, "\n {declaration}")?;
        }
        for head in &self.normal_forms {
            write!(f, "\n {head}")?;
        }
        for definition in &self.traits {
            write!(f, "\n {definition}")?;
        }
        for definition in &self.implementations {
            write!(f, "\n {definition}")?;
        }
        for definition in &self.enums {
            write!(f, "\n {definition}")?;
        }
        for contract in &self.alias_contracts {
            write!(f, "\n {contract}")?;
        }
        for definition in &self.newtypes {
            write!(f, "\n {definition}")?;
        }
        for definition in &self.structs {
            write!(f, "\n {definition}")?;
        }
        for template in &self.quote_templates {
            write!(f, "\n (quote-template {template})")?;
        }
        for function in &self.functions {
            write!(f, "\n {function}")?;
        }
        for instance in &self.dispatch_instances {
            write!(f, "\n {instance}")?;
        }
        f.write_str(")\n")
    }
}

impl Expr {
    /// A discarded void expression has no separate value to retire. Lowering
    /// commonly gives the statement wrapper and its child the same range; the
    /// child's full-expression hook owns that boundary exactly once.
    pub fn shares_discard_temporary_boundary(&self) -> bool {
        matches!(&self.kind, ExprKind::Discard(inner)
            if self.source == inner.source && matches!(inner.ty, Term::Primitive(crate::Primitive::Void))
                && !matches!(inner.kind, ExprKind::Block {..} | ExprKind::CallPreparation {..}
                    | ExprKind::EffectComputation {..} | ExprKind::EffectRun {..}))
    }

    /// Specialize type-bearing structure using explicitly sorted declaration
    /// inputs. Value locals and runtime erasure remain the instantiation pass's
    /// responsibility; this operation does not invent runtime values.
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        let mut value = self.clone();
        value.rewrite_types(&mut |ty| ty.instantiate_parameters(parameters, arguments))?;
        Ok(value)
    }

    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters_in_scope(parameters, arguments, frames)?;
        let mut value = self.clone();
        value.rewrite_types(&mut |ty| {
            ty.instantiate_parameters_in_scope(parameters, arguments, frames)
        })?;
        Ok(value)
    }

    /// Specialize a declaration-owned template using completed parameter
    /// domains. Full value/formation admission belongs to the caller.
    pub fn instantiate_declaration_parameters_in_scope(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        let substitution =
            crate::DeclarationSubstitution::new(parameters, domains, arguments, frames)?;
        let mut value = self.clone();
        value.rewrite_types(&mut |ty| substitution.instantiate(ty))?;
        Ok(value)
    }

    /// Substitute type annotations throughout a checked template, including
    /// closure captures, defaults and nested generic function references.
    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let mut value = self.clone();
        value.rewrite_types(&mut |ty| ty.instantiate_types(parameters, arguments))?;
        Ok(value)
    }
    /// Substitute annotations with explicit caller frames without changing term identity.
    pub fn instantiate_types_in_scope(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        let mut value = self.clone();
        value.rewrite_types(&mut |ty| {
            ty.instantiate_types_in_scope(parameters, arguments, frames)
        })?;
        Ok(value)
    }
    /// Rewrite every type-bearing annotation and operand in deterministic structural order.
    /// The callback owns recursive Term rewriting. On error this working copy
    /// may be partially changed; callers must discard it rather than publish it.
    pub fn rewrite_types<E>(
        &mut self,
        transform: &mut impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<(), E> {
        self.rewrite_with(&mut TypeRewriter(transform))
    }
    /// Rewrite a private expression tree, including binding/control identities.
    /// Callbacks run before children. This deliberately shares the exhaustive
    /// traversal used by type substitution so new operations cannot be skipped.
    pub fn rewrite_with<R: Rewriter>(&mut self, rewriter: &mut R) -> Result<(), R::Error> {
        // Generated code can contain long chains of scopes and operations.
        // Visit ordinary children using a heap frontier in the same preorder
        // as recursive rewriting. Specialized nodes retain their own handling
        // when child rewrites affect a parent's metadata or pattern domain.
        enum Work<'a> {
            Expr(&'a mut Expr),
            Capture(&'a mut Capture),
            ClosureBody(&'a mut Arc<Expr>),
        }
        let mut work = vec![Work::Expr(self)];
        while let Some(next) = work.pop() {
            let expr = match next {
                Work::Expr(expr) => expr,
                Work::Capture(capture) => {
                    capture.ty = rewriter.ty(&capture.ty)?;
                    if let Some(value) = &mut capture.value {
                        work.push(Work::Expr(value));
                    }
                    continue;
                }
                // Clone shared code only once all earlier captures succeeded.
                Work::ClosureBody(body) => Arc::make_mut(body),
            };
            if !rewriter.expression(expr)? {
                continue;
            }
            expr.ty = rewriter.ty(&expr.ty)?;
            if let ExprKind::Intrinsic {
                binding: Some(binding),
                ..
            } = &mut expr.kind
            {
                binding.rewrite_pending(&mut |ty| rewriter.ty(ty))?;
            }
            if !matches!(
                &expr.kind,
                ExprKind::Block { .. }
                    | ExprKind::CallPreparation { .. }
                    | ExprKind::AnonymousField { .. }
                    | ExprKind::Field { .. }
                    | ExprKind::Let { .. }
                    | ExprKind::Assign { .. }
                    | ExprKind::UnsafeScope(_)
                    | ExprKind::Discard(_)
                    | ExprKind::RequirePredicate(_)
                    | ExprKind::EnsurePredicate(_)
                    | ExprKind::CheckPredicate(_)
                    | ExprKind::BitSlice { .. }
                    | ExprKind::AddressOf(_)
                    | ExprKind::ReadPointer(_)
                    | ExprKind::PointerAnonymousField { .. }
                    | ExprKind::PointerField { .. }
                    | ExprKind::OptionalPresent(_)
                    | ExprKind::OptionalType(_)
                    | ExprKind::PointerType(_)
                    | ExprKind::ListLength(_)
                    | ExprKind::StructFieldTypes(_)
                    | ExprKind::BuildStruct(_)
                    | ExprKind::BuildEnum(_)
                    | ExprKind::EnumStructure { .. }
                    | ExprKind::ObjectQuery { .. }
                    | ExprKind::ObjectEdit { .. }
                    | ExprKind::ComptimeFunctionInstance { .. }
                    | ExprKind::StructFieldNames(_)
                    | ExprKind::BitVecType(_)
                    | ExprKind::IntegerNegate(_)
                    | ExprKind::FloatNegate(_)
                    | ExprKind::BitConvert(_)
                    | ExprKind::NewtypeConvert(_)
                    | ExprKind::Snapshot(_)
                    | ExprKind::TakeBits(_)
                    | ExprKind::TextQuery { subject: _, .. }
                    | ExprKind::IntegerRepresentation(_)
                    | ExprKind::ComptimeIntegerConvert(_)
                    | ExprKind::SymbolName(_)
                    | ExprKind::EnumName(_)
                    | ExprKind::Not(_)
                    | ExprKind::Loop { .. }
                    | ExprKind::WritePointer { .. }
                    | ExprKind::FunctionResultType { .. }
                    | ExprKind::IntegerBinary { .. }
                    | ExprKind::FloatBinary { .. }
                    | ExprKind::ScalarLess(_, _)
                    | ExprKind::ScalarGreater(_, _)
                    | ExprKind::ScalarGreaterEqual(_, _)
                    | ExprKind::ScalarLessEqual(_, _)
                    | ExprKind::TemplateEqual(_, _)
                    | ExprKind::ScalarEqual(_, _)
                    | ExprKind::TypeEqual(_, _)
                    | ExprKind::ListConcat(_, _)
                    | ExprKind::TypeListIndex(_, _)
                    | ExprKind::BitEqual(_, _)
                    | ExprKind::BitConcat(_, _)
                    | ExprKind::BitIndex(_, _)
                    | ExprKind::And(_, _)
                    | ExprKind::Or(_, _)
                    | ExprKind::BitWindow { .. }
                    | ExprKind::If { .. }
                    | ExprKind::Return(_)
                    | ExprKind::Call { .. }
                    | ExprKind::FunctionType { .. }
                    | ExprKind::TypeList(_)
                    | ExprKind::TupleType(_)
                    | ExprKind::Intrinsic { .. }
                    | ExprKind::Tuple(_)
                    | ExprKind::Object { .. }
                    | ExprKind::AnonymousRecord { .. }
                    | ExprKind::Record { .. }
                    | ExprKind::Variant { .. }
                    | ExprKind::ClosureType { .. }
                    | ExprKind::Closure { .. }
                    | ExprKind::EffectResume(_)
                    | ExprKind::EffectRun { .. }
                    | ExprKind::EffectComputation { .. }
                    | ExprKind::EffectRequest { .. }
            ) {
                expr.rewrite_operation(rewriter)?;
                continue;
            }
            match &mut expr.kind {
                ExprKind::Block { statements, result }
                | ExprKind::CallPreparation { statements, result } => {
                    if let Some(result) = result {
                        work.push(Work::Expr(result));
                    }
                    work.extend(statements.iter_mut().rev().map(Work::Expr));
                }
                ExprKind::AnonymousField { value, .. }
                | ExprKind::Field { value, .. }
                | ExprKind::Let { value, .. }
                | ExprKind::Assign { value, .. }
                | ExprKind::UnsafeScope(value)
                | ExprKind::Discard(value)
                | ExprKind::RequirePredicate(value)
                | ExprKind::EnsurePredicate(value)
                | ExprKind::CheckPredicate(value)
                | ExprKind::BitSlice { value, .. }
                | ExprKind::AddressOf(value)
                | ExprKind::ReadPointer(value)
                | ExprKind::PointerAnonymousField { pointer: value, .. }
                | ExprKind::PointerField { pointer: value, .. }
                | ExprKind::OptionalPresent(value)
                | ExprKind::OptionalType(value)
                | ExprKind::PointerType(value)
                | ExprKind::ListLength(value)
                | ExprKind::StructFieldTypes(value)
                | ExprKind::BuildStruct(value)
                | ExprKind::BuildMetadata {
                    selector: value, ..
                }
                | ExprKind::BuildEnum(value)
                | ExprKind::EnumStructure { subject: value, .. }
                | ExprKind::ObjectQuery { subject: value, .. }
                | ExprKind::ObjectEdit { subject: value, .. }
                | ExprKind::ComptimeFunctionInstance { subject: value, .. }
                | ExprKind::StructFieldNames(value)
                | ExprKind::BitVecType(value)
                | ExprKind::IntegerNegate(value)
                | ExprKind::FloatNegate(value)
                | ExprKind::BitConvert(value)
                | ExprKind::NewtypeConvert(value)
                | ExprKind::Snapshot(value)
                | ExprKind::TakeBits(value)
                | ExprKind::TextQuery { subject: value, .. }
                | ExprKind::IntegerRepresentation(value)
                | ExprKind::ComptimeIntegerConvert(value)
                | ExprKind::SymbolName(value)
                | ExprKind::EnumName(value)
                | ExprKind::Not(value)
                | ExprKind::Loop { body: value, .. } => work.push(Work::Expr(value)),
                ExprKind::WritePointer {
                    pointer: a,
                    value: b,
                }
                | ExprKind::FunctionResultType {
                    signature: a,
                    result: b,
                }
                | ExprKind::IntegerBinary {
                    left: a, right: b, ..
                }
                | ExprKind::FloatBinary {
                    left: a, right: b, ..
                }
                | ExprKind::ScalarLess(a, b)
                | ExprKind::ScalarGreater(a, b)
                | ExprKind::ScalarGreaterEqual(a, b)
                | ExprKind::ScalarLessEqual(a, b)
                | ExprKind::TemplateEqual(a, b)
                | ExprKind::ScalarEqual(a, b)
                | ExprKind::TypeEqual(a, b)
                | ExprKind::ListConcat(a, b)
                | ExprKind::TypeListIndex(a, b)
                | ExprKind::BitEqual(a, b)
                | ExprKind::BitConcat(a, b)
                | ExprKind::BitIndex(a, b)
                | ExprKind::And(a, b)
                | ExprKind::Or(a, b) => {
                    work.push(Work::Expr(b));
                    work.push(Work::Expr(a));
                }
                ExprKind::BitWindow { value, start, end } => {
                    work.push(Work::Expr(end));
                    work.push(Work::Expr(start));
                    work.push(Work::Expr(value));
                }
                ExprKind::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    if let Some(value) = else_branch {
                        work.push(Work::Expr(value));
                    }
                    work.push(Work::Expr(then_branch));
                    work.push(Work::Expr(condition));
                }
                ExprKind::Return(value) => {
                    if let Some(value) = value {
                        work.push(Work::Expr(value));
                    }
                }
                ExprKind::Call {
                    callee, arguments, ..
                } => {
                    work.extend(
                        arguments
                            .iter_mut()
                            .rev()
                            .map(|argument| Work::Expr(&mut argument.value)),
                    );
                    work.push(Work::Expr(callee));
                }
                ExprKind::FunctionType {
                    parameters: fields, ..
                }
                | ExprKind::TypeList(fields)
                | ExprKind::TupleType(fields)
                | ExprKind::Intrinsic { inputs: fields, .. }
                | ExprKind::Tuple(fields)
                | ExprKind::Object { fields, .. }
                | ExprKind::AnonymousRecord { fields }
                | ExprKind::Record { fields, .. }
                | ExprKind::Variant { fields, .. } => {
                    work.extend(fields.iter_mut().rev().map(Work::Expr))
                }
                ExprKind::Closure { captures, body, .. } => {
                    work.push(Work::ClosureBody(body));
                    work.extend(captures.iter_mut().rev().map(Work::Capture));
                }
                ExprKind::ClosureType {
                    environment,
                    signature,
                } => {
                    work.push(Work::Expr(signature));
                    work.extend(environment.iter_mut().rev().map(Work::Expr));
                }
                ExprKind::EffectResume(value)
                | ExprKind::EffectRun {
                    computation: value, ..
                } => work.push(Work::Expr(value)),
                ExprKind::EffectComputation { effects, body } => {
                    for operation in effects {
                        for argument in &mut operation.arguments {
                            *argument = rewriter.ty(argument)?;
                        }
                    }
                    work.push(Work::Expr(body));
                }
                ExprKind::EffectRequest {
                    operation,
                    signature,
                    arguments,
                    ..
                } => {
                    for argument in &mut operation.arguments {
                        *argument = rewriter.ty(argument)?;
                    }
                    **signature = rewriter.ty(signature)?;
                    work.extend(arguments.iter_mut().rev().map(Work::Expr));
                }
                _ => unreachable!("specialized nodes were dispatched before borrowing children"),
            }
        }
        Ok(())
    }

    #[inline(never)]
    fn rewrite_operation<R: Rewriter>(&mut self, rewriter: &mut R) -> Result<(), R::Error> {
        match &mut self.kind {
            ExprKind::ErrorConvert { conversion, value } => {
                let was_error = matches!(value.ty, Term::ErrorResult { .. });
                value.rewrite_with(rewriter)?;
                if *conversion == crate::error_results::Conversion::Success
                    && !was_error
                    && matches!(value.ty, Term::ErrorResult { .. })
                {
                    // A generic success domain can itself acquire an error
                    // qualifier. Preserve its selected branch after flattening.
                    *conversion = crate::error_results::Conversion::Embed;
                }
            }

            ExprKind::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                computation.rewrite_with(rewriter)?;
                for operation in outer {
                    for argument in &mut operation.arguments {
                        *argument = rewriter.ty(argument)?;
                    }
                }
                for handler in handlers {
                    for argument in &mut handler.operation.arguments {
                        *argument = rewriter.ty(argument)?;
                    }
                    handler.signature = rewriter.ty(&handler.signature)?;
                    for arm in &mut handler.arms {
                        arm.pattern.rewrite_with(rewriter)?;
                        if let Some(guard) = &mut arm.guard {
                            guard.rewrite_with(rewriter)?;
                        }
                        arm.body.rewrite_with(rewriter)?;
                    }
                }
            }
            ExprKind::EffectResume(computation) | ExprKind::EffectRun { computation, .. } => {
                computation.rewrite_with(rewriter)?
            }
            ExprKind::EffectComputation { effects, body } => {
                for operation in effects {
                    for argument in &mut operation.arguments {
                        *argument = rewriter.ty(argument)?;
                    }
                }
                body.rewrite_with(rewriter)?;
            }
            ExprKind::EffectRequest {
                operation,
                signature,
                arguments,
                ..
            } => {
                for argument in &mut operation.arguments {
                    *argument = rewriter.ty(argument)?;
                }
                **signature = rewriter.ty(signature)?;
                for argument in arguments {
                    argument.rewrite_with(rewriter)?;
                }
            }
            ExprKind::AssociatedValue {
                interface, subject, ..
            } => {
                *subject = rewriter.ty(subject)?;
                for argument in &mut interface.arguments {
                    *argument = rewriter.ty(argument)?;
                }
            }
            ExprKind::BuildMetadata { entries, .. } => {
                for (_, _, value) in entries {
                    if let Some(value) = value {
                        *value = rewriter.ty(value)?;
                    }
                }
            }
            ExprKind::TypeValue(ty) => {
                *ty = rewriter.ty(ty)?;
            }
            ExprKind::ComptimeValue(value) => {
                *value = rewriter.ty(value)?;
                // A dependent value input can acquire Type as its domain after
                // specialization. Publish the same representation as any other
                // Type value; do not leave a data-value node around a Type term.
                if self.ty == Term::Primitive(crate::Primitive::Type)
                    && value.sort() == crate::Sort::Type
                {
                    self.kind = ExprKind::TypeValue(value.clone());
                }
            }
            ExprKind::TypeListValue(types) => {
                for ty in types {
                    *ty = rewriter.ty(ty)?;
                }
            }

            ExprKind::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                pointer.rewrite_with(rewriter)?;
                for input in selections
                    .iter_mut()
                    .flat_map(BitSelection::expressions_mut)
                {
                    input.rewrite_with(rewriter)?;
                }
                value.rewrite_with(rewriter)?;
            }
            ExprKind::BitUpdate {
                storage_type,
                selections,
                value,
                ..
            } => {
                *storage_type = rewriter.ty(storage_type)?;
                for expression in selections
                    .iter_mut()
                    .flat_map(BitSelection::expressions_mut)
                {
                    expression.rewrite_with(rewriter)?;
                }
                value.rewrite_with(rewriter)?;
            }
            ExprKind::BitAssign {
                storage_type,
                value,
                ..
            } => {
                *storage_type = rewriter.ty(storage_type)?;
                (value).rewrite_with(rewriter)?;
            }
            ExprKind::TraitMethod {
                subject,
                interface_arguments,
                method_arguments,
                ..
            } => {
                *subject = rewriter.ty(subject)?;
                for argument in interface_arguments.iter_mut().chain(method_arguments) {
                    *argument = rewriter.ty(argument)?;
                }
            }
            ExprKind::FunctionInstance {
                arguments: types, ..
            } => {
                for ty in types {
                    *ty = rewriter.ty(ty)?;
                }
            }
            ExprKind::Closure { captures, body, .. } => {
                for capture in captures {
                    capture.ty = rewriter.ty(&capture.ty)?;
                    if let Some(value) = &mut capture.value {
                        (value).rewrite_with(rewriter)?;
                    }
                }
                Arc::make_mut(body).rewrite_with(rewriter)?;
            }
            ExprKind::NormalFormType {
                arguments: inputs,
                plan,
                ..
            } => {
                for preparation in plan.iter_mut().flat_map(|p| &mut p.preparations) {
                    preparation.expression.rewrite_with(rewriter)?;
                }
                for pattern in inputs.iter_mut().flat_map(NormalFormArgument::patterns_mut) {
                    pattern.rewrite_with(rewriter)?;
                }
                for field in inputs
                    .iter_mut()
                    .flat_map(NormalFormArgument::expressions_mut)
                {
                    (field).rewrite_with(rewriter)?;
                }
            }
            ExprKind::ClosureType {
                environment,
                signature,
            } => {
                for field in environment {
                    (field).rewrite_with(rewriter)?;
                }
                (signature).rewrite_with(rewriter)?;
            }
            ExprKind::NominalType {
                arguments,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    preparation.expression.rewrite_with(rewriter)?;
                }
                for pattern in arguments
                    .iter_mut()
                    .flat_map(NormalFormArgument::patterns_mut)
                {
                    pattern.rewrite_with(rewriter)?;
                }
                for field in arguments
                    .iter_mut()
                    .flat_map(NormalFormArgument::expressions_mut)
                {
                    field.rewrite_with(rewriter)?;
                }
            }
            ExprKind::CheckTypeAlias {
                arguments,
                body,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    preparation.expression.rewrite_with(rewriter)?;
                }
                for argument in arguments {
                    for pattern in argument.patterns_mut() {
                        pattern.rewrite_with(rewriter)?;
                    }
                    for expression in argument.expressions_mut() {
                        expression.rewrite_with(rewriter)?;
                    }
                }
                if let Some(body) = body {
                    body.rewrite_with(rewriter)?;
                }
            }
            ExprKind::FunctionType {
                parameters: fields, ..
            }
            | ExprKind::TypeList(fields)
            | ExprKind::TupleType(fields)
            | ExprKind::Intrinsic { inputs: fields, .. }
            | ExprKind::Tuple(fields)
            | ExprKind::Object { fields, .. }
            | ExprKind::AnonymousRecord { fields }
            | ExprKind::Record { fields, .. }
            | ExprKind::Variant { fields, .. } => {
                for field in fields {
                    (field).rewrite_with(rewriter)?;
                }
            }
            ExprKind::AnonymousField { value, .. }
            | ExprKind::Field { value, .. }
            | ExprKind::Let { value, .. }
            | ExprKind::Assign { value, .. }
            | ExprKind::UnsafeScope(value)
            | ExprKind::Discard(value)
            | ExprKind::RequirePredicate(value)
            | ExprKind::EnsurePredicate(value)
            | ExprKind::CheckPredicate(value)
            | ExprKind::BitSlice { value, .. }
            | ExprKind::AddressOf(value)
            | ExprKind::ReadPointer(value)
            | ExprKind::PointerAnonymousField { pointer: value, .. }
            | ExprKind::PointerField { pointer: value, .. }
            | ExprKind::OptionalPresent(value)
            | ExprKind::OptionalType(value)
            | ExprKind::PointerType(value)
            | ExprKind::ListLength(value)
            | ExprKind::StructFieldTypes(value)
            | ExprKind::BuildStruct(value)
            | ExprKind::BuildMetadata {
                selector: value, ..
            }
            | ExprKind::BuildEnum(value)
            | ExprKind::EnumStructure { subject: value, .. }
            | ExprKind::ObjectQuery { subject: value, .. }
            | ExprKind::ObjectEdit { subject: value, .. }
            | ExprKind::ComptimeFunctionInstance { subject: value, .. }
            | ExprKind::StructFieldNames(value)
            | ExprKind::BitVecType(value)
            | ExprKind::IntegerNegate(value)
            | ExprKind::FloatNegate(value)
            | ExprKind::BitConvert(value)
            | ExprKind::NewtypeConvert(value)
            | ExprKind::Snapshot(value)
            | ExprKind::TakeBits(value)
            | ExprKind::TextQuery { subject: value, .. }
            | ExprKind::IntegerRepresentation(value)
            | ExprKind::ComptimeIntegerConvert(value)
            | ExprKind::SymbolName(value)
            | ExprKind::EnumName(value)
            | ExprKind::Not(value)
            | ExprKind::Loop { body: value, .. } => (value).rewrite_with(rewriter)?,
            ExprKind::WritePointer {
                pointer: a,
                value: b,
            }
            | ExprKind::FunctionResultType {
                signature: a,
                result: b,
            }
            | ExprKind::IntegerBinary {
                left: a, right: b, ..
            }
            | ExprKind::FloatBinary {
                left: a, right: b, ..
            }
            | ExprKind::ScalarLess(a, b)
            | ExprKind::ScalarGreater(a, b)
            | ExprKind::ScalarGreaterEqual(a, b)
            | ExprKind::ScalarLessEqual(a, b)
            | ExprKind::TemplateEqual(a, b)
            | ExprKind::ScalarEqual(a, b)
            | ExprKind::TypeEqual(a, b)
            | ExprKind::ListConcat(a, b)
            | ExprKind::TypeListIndex(a, b)
            | ExprKind::BitEqual(a, b)
            | ExprKind::BitConcat(a, b)
            | ExprKind::BitIndex(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b) => {
                (a).rewrite_with(rewriter)?;
                (b).rewrite_with(rewriter)?;
            }
            ExprKind::BitWindow { value, start, end } => {
                value.rewrite_with(rewriter)?;
                start.rewrite_with(rewriter)?;
                end.rewrite_with(rewriter)?;
            }
            ExprKind::Match { value, arms } => {
                let pattern_domain = value.ty.clone();
                let (mut known_errors, projections) = match &value.ty {
                    Term::ErrorResult { errors, result }
                        if !matches!(result.as_ref(), Term::ErrorResult { .. }) =>
                    {
                        let projections = arms
                            .iter()
                            .enumerate()
                            .filter_map(|(index, arm)| {
                                crate::error_results::specialize::projection_operand(&arm.body)
                                    .is_some_and(|operand| operand.ty == **result)
                                    .then_some(index)
                            })
                            .collect::<Vec<_>>();
                        (errors.clone(), projections)
                    }
                    _ => (Vec::new(), Vec::new()),
                };
                (value).rewrite_with(rewriter)?;
                for arm in arms.iter_mut() {
                    arm.pattern
                        .rewrite_with_domain(rewriter, Some(&pattern_domain))?;
                    if let Some(g) = &mut arm.guard {
                        (g).rewrite_with(rewriter)?;
                    }
                    (&mut arm.body).rewrite_with(rewriter)?;
                }
                if projections.iter().any(|index| {
                    crate::error_results::specialize::projection_operand(&arms[*index].body)
                        .is_some_and(|operand| matches!(operand.ty, Term::ErrorResult { .. }))
                }) {
                    for label in &mut known_errors {
                        *label = rewriter.ty(label)?;
                    }
                    // Preserve the user's arm order. Newly exposed inner
                    // branches belong beside their original success arm.
                    for index in projections.into_iter().rev() {
                        let additional =
                            crate::error_results::specialize::complete_success_projection(
                                &value.ty,
                                &known_errors,
                                &mut arms[index],
                            );
                        arms.splice(index + 1..index + 1, additional);
                    }
                }
            }
            ExprKind::Block { .. } | ExprKind::CallPreparation { .. } => {
                unreachable!("lexical nodes are handled by rewrite_with")
            }
            ExprKind::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                let pattern_domain = value.ty.clone();
                pattern.rewrite_with_domain(rewriter, Some(&pattern_domain))?;
                (value).rewrite_with(rewriter)?;
                if let Some(g) = guard {
                    (g).rewrite_with(rewriter)?;
                }
                if let Some(v) = otherwise {
                    (v).rewrite_with(rewriter)?;
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                (condition).rewrite_with(rewriter)?;
                (then_branch).rewrite_with(rewriter)?;
                if let Some(v) = else_branch {
                    (v).rewrite_with(rewriter)?;
                }
            }
            ExprKind::Return(value) => {
                if let Some(v) = value {
                    (v).rewrite_with(rewriter)?;
                }
            }
            ExprKind::Call {
                callee, arguments, ..
            } => {
                (callee).rewrite_with(rewriter)?;
                for arg in arguments {
                    (&mut arg.value).rewrite_with(rewriter)?;
                }
            }
            ExprKind::Literal(_)
            | ExprKind::Local(_)
            | ExprKind::Function(_)
            | ExprKind::OptionalAbsent
            | ExprKind::Unit
            | ExprKind::Break(_)
            | ExprKind::Continue(_) => {}
        }
        Ok(())
    }
}

impl Term {
    /// Whether this type contains a specific definition-side parameter.
    pub fn mentions_rigid(&self, declaration: DeclarationId) -> bool {
        let mut terms = vec![self];
        while let Some(term) = terms.pop() {
            match term {
                Term::ErrorResult { errors, result } => {
                    terms.push(result);
                    terms.extend(errors);
                }
                Term::Computation { effects, result } => {
                    terms.push(result);
                    for operation in effects {
                        terms.extend(&operation.arguments);
                    }
                }
                Term::AssociatedType {
                    arguments, subject, ..
                } => {
                    terms.extend(arguments);
                    terms.push(subject);
                }
                Term::Rigid {
                    declaration: id, ..
                } if *id == declaration => return true,
                Term::Nominal { arguments, .. }
                | Term::NormalForm { arguments, .. }
                | Term::Tuple(arguments)
                | Term::ClosureContextValue(arguments)
                | Term::TypeListValue(arguments)
                | Term::TypeListConcat(arguments)
                | Term::TupleValue(arguments) => terms.extend(arguments),
                Term::OptionalValue { ty, value } => {
                    terms.push(ty);
                    if let Some(value) = value {
                        terms.push(value);
                    }
                }

                Term::ErrorValue { ty, label, value } => {
                    terms.push(ty);
                    if let Some(label) = label {
                        terms.push(label);
                    }
                    terms.push(value);
                }
                Term::NominalValue { ty, fields, .. } => {
                    terms.push(ty);
                    terms.extend(fields);
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    terms.extend(fields.iter().map(|(_, ty)| ty))
                }
                Term::Optional(t)
                | Term::Pointer(t)
                | Term::ComptimeFunction(t)
                | Term::NormalFormFunction(t, _)
                | Term::UnsafeFunction(t) => terms.push(t),
                Term::Function { parameters, result } => {
                    terms.extend(parameters.iter().map(|p| &p.ty));
                    terms.push(result);
                }
                Term::ContextualClosure { context, signature } => {
                    terms.push(context);
                    terms.push(signature);
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    terms.extend(environment);
                    terms.push(signature);
                }
                _ => {}
            }
        }
        false
    }
}
/// Type-only requirements retained with a normalized definition. Runtime
/// contracts belong to program verification and are not represented here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypePredicate {
    Bool(bool),
    Trait(TraitRequirement),
    Equal(Term, Term),
    Not(Box<TypePredicate>),
    And(Vec<TypePredicate>),
    Or(Vec<TypePredicate>),
}
impl TypePredicate {
    /// Instantiate a type proposition inside an explicit caller telescope.
    /// Even constant propositions validate supplied arguments. This operation
    /// preserves the proposition; it neither proves it nor admits state facts.
    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters_in_scope(parameters, arguments, frames)?;
        Ok(match self {
            Self::Bool(value) => Self::Bool(*value),
            Self::Trait(requirement) => Self::Trait(
                requirement.instantiate_parameters_in_scope(parameters, arguments, frames)?,
            ),
            Self::Equal(left, right) => Self::Equal(
                left.instantiate_parameters_in_scope(parameters, arguments, frames)?,
                right.instantiate_parameters_in_scope(parameters, arguments, frames)?,
            ),
            Self::Not(inner) => Self::Not(Box::new(
                inner.instantiate_parameters_in_scope(parameters, arguments, frames)?,
            )),
            Self::And(parts) | Self::Or(parts) => {
                let parts = parts
                    .iter()
                    .map(|part| part.instantiate_parameters_in_scope(parameters, arguments, frames))
                    .collect::<Result<_, _>>()?;
                if matches!(self, Self::And(_)) {
                    Self::And(parts)
                } else {
                    Self::Or(parts)
                }
            }
        })
    }

    /// A bounded proof shortcut, not a disequality theory. Distinct terms remain
    /// unknown; callers must use their type solver for nontrivial relations.
    pub fn trivial_truth(&self) -> Option<bool> {
        fn visit(predicate: &TypePredicate, depth: usize, fuel: &mut usize) -> Option<bool> {
            if depth >= 128 || *fuel == 0 {
                return None;
            }
            *fuel -= 1;
            match predicate {
                TypePredicate::Bool(value) => Some(*value),
                TypePredicate::Trait(_) => None,
                TypePredicate::Equal(left, right) => (left == right).then_some(true),
                TypePredicate::Not(value) => visit(value, depth + 1, fuel).map(|v| !v),
                TypePredicate::And(values) | TypePredicate::Or(values) => {
                    let is_and = matches!(predicate, TypePredicate::And(_));
                    let mut unknown = false;
                    for value in values {
                        match visit(value, depth + 1, fuel) {
                            Some(value) if value != is_and => return Some(value),
                            None => unknown = true,
                            _ => {}
                        }
                    }
                    (!unknown).then_some(is_and)
                }
            }
        }
        visit(self, 0, &mut 10_000)
    }

    pub fn mentions_rigid(&self, declaration: DeclarationId) -> bool {
        let mut predicates = vec![self];
        let mut terms = Vec::new();
        while let Some(predicate) = predicates.pop() {
            match predicate {
                Self::Trait(requirement) => {
                    terms.push(&requirement.subject);
                    terms.extend(&requirement.interface_arguments);
                }
                Self::Equal(a, b) => {
                    terms.push(a);
                    terms.push(b);
                }
                Self::Not(p) => predicates.push(p),
                Self::And(ps) | Self::Or(ps) => predicates.extend(ps),
                Self::Bool(_) => {}
            }
        }
        terms
            .into_iter()
            .any(|term| term.mentions_rigid(declaration))
    }
    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| crate::DeclarationParameter {
                declaration,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }

    /// Substitute declaration inputs in type propositions. Compile-time values
    /// may occur inside opaque type heads; this neither evaluates those heads
    /// nor admits program-state predicates into the type solver.
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        Ok(match self {
            Self::Bool(v) => Self::Bool(*v),
            Self::Trait(requirement) => Self::Trait(TraitRequirement {
                interface: requirement.interface,
                subject: requirement
                    .subject
                    .instantiate_parameters(parameters, arguments)?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|term| term.instantiate_parameters(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            }),
            Self::Equal(a, b) => Self::Equal(
                a.instantiate_parameters(parameters, arguments)?,
                b.instantiate_parameters(parameters, arguments)?,
            ),
            Self::Not(p) => Self::Not(Box::new(p.instantiate_parameters(parameters, arguments)?)),
            Self::And(ps) => Self::And(
                ps.iter()
                    .map(|p| p.instantiate_parameters(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Or(ps) => Self::Or(
                ps.iter()
                    .map(|p| p.instantiate_parameters(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
    pub fn instantiate_rigids(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Ok(match self {
            Self::Bool(v) => Self::Bool(*v),
            Self::Trait(requirement) => Self::Trait(TraitRequirement {
                interface: requirement.interface,
                subject: requirement
                    .subject
                    .instantiate_rigids(parameters, arguments)?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|term| term.instantiate_rigids(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            }),
            Self::Equal(a, b) => Self::Equal(
                a.instantiate_rigids(parameters, arguments)?,
                b.instantiate_rigids(parameters, arguments)?,
            ),
            Self::Not(p) => Self::Not(Box::new(p.instantiate_rigids(parameters, arguments)?)),
            Self::And(ps) => Self::And(
                ps.iter()
                    .map(|p| p.instantiate_rigids(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Or(ps) => Self::Or(
                ps.iter()
                    .map(|p| p.instantiate_rigids(parameters, arguments))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}
impl fmt::Display for TypePredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "(type-proposition {value})"),
            Self::Trait(requirement) => write!(f, "(trait-proposition {requirement})"),
            Self::Equal(a, b) => write!(f, "(type-equal {a} {b})"),
            Self::Not(p) => write!(f, "(not {p})"),
            Self::And(ps) | Self::Or(ps) => {
                f.write_str(if matches!(self, Self::And(_)) {
                    "(and"
                } else {
                    "(or"
                })?;
                for p in ps {
                    write!(f, " {p}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Scope-aware, fallible rewriting of a private typed tree. Returning false
/// from expression delegates its entire subtree (including types) to the
/// rewriter. This supports defaults and closures with independent binding frames.
pub trait Rewriter {
    type Error;
    fn ty(&mut self, ty: &Term) -> Result<Term, Self::Error>;
    fn expression(&mut self, _expression: &mut Expr) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn pattern(&mut self, _pattern: &mut Pattern) -> Result<(), Self::Error> {
        Ok(())
    }
}
struct TypeRewriter<'a, T>(&'a mut T);
impl<E, T: FnMut(&Term) -> Result<Term, E>> Rewriter for TypeRewriter<'_, T> {
    type Error = E;
    fn ty(&mut self, ty: &Term) -> Result<Term, E> {
        (self.0)(ty)
    }
}

impl Pattern {
    /// Visit embedded expressions and nested patterns using the same type
    /// rewrite as Expr. Failure leaves a private, partially rewritten copy.
    pub fn rewrite_types<E>(
        &mut self,
        transform: &mut impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<(), E> {
        self.rewrite_with(&mut TypeRewriter(transform))
    }
    pub fn rewrite_with<R: Rewriter>(&mut self, rewriter: &mut R) -> Result<(), R::Error> {
        self.rewrite_with_domain(rewriter, None)
    }
    /// Carry the original subject domain through structural patterns. Only
    /// explicit field layouts are used; opaque nominal/normal heads stay opaque.
    fn rewrite_with_domain<R: Rewriter>(
        &mut self,
        rewriter: &mut R,
        domain: Option<&Term>,
    ) -> Result<(), R::Error> {
        rewriter.pattern(self)?;
        let completion = match (&*self, domain) {
            (Self::ErrorResult { label: None, .. }, Some(Term::ErrorResult { errors, result }))
                if matches!(
                    result.as_ref(),
                    Term::Rigid { .. }
                        | Term::Bound { .. }
                        | Term::Hole { .. }
                        | Term::AssociatedType { .. }
                        | Term::NormalForm { .. }
                ) =>
            {
                Some((errors, result))
            }
            _ => None,
        };
        match self {
            Self::OptionalPresent(inner) => {
                let payload = match domain {
                    Some(Term::Optional(payload)) => Some(payload.as_ref()),
                    _ => None,
                };
                inner.rewrite_with_domain(rewriter, payload)?;
            }
            Self::ErrorResult { label, inner } => {
                let inner_domain = label.clone().or_else(|| match domain {
                    Some(Term::ErrorResult { result, .. }) => Some((**result).clone()),
                    _ => None,
                });
                if let Some(ty) = label {
                    *ty = rewriter.ty(ty)?;
                }
                inner.rewrite_with_domain(rewriter, inner_domain.as_ref())?;
            }
            Self::CodeEvidence { outputs } => {
                for (_, ty) in outputs {
                    *ty = rewriter.ty(ty)?;
                }
            }

            Self::Then { first, value, next } => {
                first.rewrite_with_domain(rewriter, domain)?;
                let computed_domain = value.ty.clone();
                value.rewrite_with(rewriter)?;
                next.rewrite_with_domain(rewriter, Some(&computed_domain))?;
                // An identity observation used to retain a generic error
                // field's domain is no new value or ownership transfer. Once
                // its success domain is concrete, keep matching the subject
                // directly if the temporary alias is unused by the pattern.
                if let Self::Observe { binding, inner } = first.as_ref() {
                    if matches!(inner.as_ref(), Self::Wild)
                        && matches!(&value.kind, ExprKind::Local(id) if id == binding)
                        && matches!(&value.ty, Term::ErrorResult { result, .. }
                            if !matches!(result.as_ref(), Term::Rigid { .. }
                                | Term::Bound { .. } | Term::Hole { .. }
                                | Term::AssociatedType { .. }))
                        && crate::free_bindings::collect_pattern(next, &mut 4096)
                            .is_ok_and(|free| !free.contains(binding))
                    {
                        *self = *std::mem::replace(next, Box::new(Self::Wild));
                    }
                }
            }
            Self::Guard { pattern, condition } => {
                pattern.rewrite_with_domain(rewriter, domain)?;
                condition.rewrite_with(rewriter)?;
            }
            Self::Observe { inner: pattern, .. }
            | Self::Remap { pattern, .. }
            | Self::Binding { inner: pattern, .. } => {
                pattern.rewrite_with_domain(rewriter, domain)?
            }
            Self::Or(alternatives) => {
                for pattern in alternatives {
                    pattern.rewrite_with_domain(rewriter, domain)?;
                }
            }
            Self::Tuple(fields) => {
                let domains = match domain {
                    Some(Term::Tuple(types)) if types.len() == fields.len() => Some(types),
                    _ => None,
                };
                for (index, pattern) in fields.iter_mut().enumerate() {
                    pattern.rewrite_with_domain(rewriter, domains.map(|types| &types[index]))?;
                }
            }
            Self::AnonymousRecord { names, fields } => {
                for (index, pattern) in fields.iter_mut().enumerate() {
                    let field_domain = match (domain, names.get(index)) {
                        (Some(Term::Record(types)), Some(name)) => types
                            .iter()
                            .find(|(field, _)| field == name)
                            .map(|(_, ty)| ty),
                        _ => None,
                    };
                    pattern.rewrite_with_domain(rewriter, field_domain)?;
                }
            }
            Self::NormalForm {
                arguments: fields, ..
            }
            | Self::TypeTuple(fields)
            | Self::NominalType { fields, .. }
            | Self::Record { fields, .. }
            | Self::Variant { fields, .. } => {
                for pattern in fields {
                    pattern.rewrite_with(rewriter)?;
                }
            }
            _ => {}
        }
        if let Some((errors, result)) = completion {
            let payload = rewriter.ty(result)?;
            if matches!(payload, Term::ErrorResult { .. }) {
                let labels = errors
                    .iter()
                    .map(|ty| rewriter.ty(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                crate::error_results::specialize::complete_ignored_success_pattern(
                    self, &labels, &payload,
                );
            }
        }
        Ok(())
    }
}

/// The current closed, read-only decision fragment for runtime checks. This
/// verifies executable shape, not truth or pointer validity; resource checking
/// establishes access permission separately. Calls need a predicate/effect contract
/// before admission; type propositions use a separate interface.
pub fn checkable_predicate(expression: &Expr) -> bool {
    if expression.ty != Term::Primitive(crate::Primitive::Bool) {
        return false;
    }
    let mut pending = vec![expression];
    let mut remaining = 10_000usize;
    while let Some(expr) = pending.pop() {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        if matches!(&expr.kind, ExprKind::UnsafeScope(body) if body.ty != expr.ty) {
            return false;
        }
        match &expr.kind {
            ExprKind::BitWindow { value, start, end } => {
                pending.extend([value.as_ref(), start.as_ref(), end.as_ref()])
            }
            ExprKind::Literal(_)
            | ExprKind::Local(_)
            | ExprKind::OptionalAbsent
            | ExprKind::Unit => {}
            ExprKind::Intrinsic {
                tag,
                inputs,
                binding,
            } => {
                // Target evidence establishes a scalar operation with no memory
                // or owning-result effects. It never grants permission for its
                // operands: continue visiting every input, including unsafe scopes.
                // Pending evidence only admits template shape; concrete execution
                // still calls intrinsics::checked after specialization.
                if !binding.as_ref().is_some_and(|binding| {
                    binding.validate_template(*tag, inputs, &expr.ty).is_ok()
                }) {
                    return false;
                }
                pending.extend(inputs);
            }
            // A checked associated value is initialized at comptime. Reading
            // it is a predicate operand; invoking a callable stored in it is
            // still subject to the separate call admission rules below.
            ExprKind::AssociatedValue { .. } => {}
            ExprKind::ComptimeValue(value) => {
                if crate::comptime_values::validate(value, &expr.ty, &mut remaining).is_err() {
                    return false;
                }
            }
            ExprKind::ReadPointer(value) => {
                if !intrinsic_pointer_value(&expr.ty) || pointer_operation_error(expr).is_some() {
                    return false;
                }
                pending.push(value);
            }
            ExprKind::AddressOf(value)
            | ExprKind::PointerField { pointer: value, .. }
            | ExprKind::PointerAnonymousField { pointer: value, .. } => {
                if pointer_operation_error(expr).is_some() {
                    return false;
                }
                pending.push(value);
            }
            ExprKind::Not(value)
            | ExprKind::IntegerNegate(value)
            | ExprKind::FloatNegate(value)
            | ExprKind::BitConvert(value)
            | ExprKind::OptionalPresent(value)
            | ExprKind::ErrorConvert { value: value, .. }
            | ExprKind::NewtypeConvert(value)
            | ExprKind::Snapshot(value)
            | ExprKind::TakeBits(value)
            | ExprKind::TextQuery { subject: value, .. }
            | ExprKind::IntegerRepresentation(value)
            | ExprKind::ComptimeIntegerConvert(value)
            | ExprKind::SymbolName(value)
            | ExprKind::EnumName(value)
            | ExprKind::UnsafeScope(value)
            | ExprKind::Discard(value)
            | ExprKind::AnonymousField { value, .. }
            | ExprKind::Field { value, .. }
            | ExprKind::BitSlice { value, .. }
            | ExprKind::Let { value, .. } => pending.push(value),
            ExprKind::ScalarEqual(a, b)
            | ExprKind::ScalarLess(a, b)
            | ExprKind::ScalarLessEqual(a, b)
            | ExprKind::ScalarGreater(a, b)
            | ExprKind::ScalarGreaterEqual(a, b)
            | ExprKind::BitEqual(a, b)
            | ExprKind::BitConcat(a, b)
            | ExprKind::BitIndex(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b)
            | ExprKind::IntegerBinary {
                left: a, right: b, ..
            }
            | ExprKind::FloatBinary {
                left: a, right: b, ..
            } => pending.extend([a.as_ref(), b.as_ref()]),
            // Staging reconstructs immutable aggregate inputs inside each
            // contract scope. These nodes only assemble data; calls, writes,
            // and cleanup remain rejected when visiting their children.
            ExprKind::Tuple(fields)
            | ExprKind::Record { fields, .. }
            | ExprKind::Object { fields, .. }
            | ExprKind::AnonymousRecord { fields }
            | ExprKind::Variant { fields, .. } => pending.extend(fields),
            ExprKind::Match { value, arms } => {
                pending.push(value);
                for arm in arms {
                    if !predicate_pattern(&arm.pattern, &mut pending, &mut remaining) {
                        return false;
                    }
                    pending.extend(arm.guard.as_ref());
                    pending.push(&arm.body);
                }
            }
            ExprKind::LetPattern {
                pattern,
                value,
                guard: None,
                otherwise: None,
            } => {
                if !predicate_pattern(pattern, &mut pending, &mut remaining) {
                    return false;
                }
                pending.push(value);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                pending.extend([condition.as_ref(), then_branch.as_ref()]);
                pending.extend(else_branch.as_deref());
            }
            ExprKind::Block { statements, result }
            | ExprKind::CallPreparation { statements, result } => {
                pending.extend(statements);
                pending.extend(result.as_deref());
            }
            _ => return false,
        }
    }
    true
}

// Source elaboration proves let-pattern irrefutability; this traversal checks
// every embedded observation using the same expression budget as match.
fn predicate_pattern<'a>(
    pattern: &'a Pattern,
    pending: &mut Vec<&'a Expr>,
    remaining: &mut usize,
) -> bool {
    let mut patterns = vec![pattern];
    while let Some(pattern) = patterns.pop() {
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        match pattern {
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
            Pattern::Then { first, value, next } => {
                patterns.extend([first.as_ref(), next.as_ref()]);
                pending.push(value);
            }
            Pattern::Remap { pattern, bindings } => {
                if bindings.len() > *remaining {
                    return false;
                }
                *remaining -= bindings.len();
                patterns.push(pattern);
            }
            Pattern::Guard { pattern, condition } => {
                patterns.push(pattern);
                pending.push(condition);
            }
            Pattern::OptionalPresent(inner)
            | Pattern::Observe { inner, .. }
            | Pattern::Binding { inner, .. } => patterns.push(inner),
            Pattern::Tuple(fields)
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Or(fields)
            | Pattern::Variant { fields, .. } => {
                if fields.len() > *remaining {
                    return false;
                }
                patterns.extend(fields);
            }
            _ => return false,
        }
    }
    true
}

/// Representation selected for an integer operation, not a safety proof.
pub fn machine_integer_type(expr: &Expr) -> Option<crate::Primitive> {
    machine_integer_type_in(expr, None)
}

pub fn machine_integer_type_in(
    expr: &Expr,
    target: Option<crate::target::TargetData>,
) -> Option<crate::Primitive> {
    let supported = |p: crate::Primitive| match target {
        Some(target) => target.integer_layout(p).is_some(),
        None => p.fixed_integer_layout().is_some(),
    };
    let operand = match &expr.kind {
        ExprKind::IntegerBinary { left, right, .. } => {
            if left.ty == Term::Primitive(crate::Primitive::Never) {
                &right.ty
            } else {
                &left.ty
            }
        }
        ExprKind::IntegerNegate(value) => &value.ty,
        _ => return None,
    };
    match operand {
        Term::Primitive(p) if supported(*p) => Some(*p),
        _ => match expr.ty {
            Term::Primitive(p) if supported(p) => Some(p),
            _ => None,
        },
    }
}

/// Structural pointer-operation checks, independent of memory access evidence.
pub fn pointer_operation_error(expr: &Expr) -> Option<&'static str> {
    pointer_operation_error_in(expr, None)
}

pub fn pointer_operation_error_in(
    expr: &Expr,
    target: Option<crate::target::TargetData>,
) -> Option<&'static str> {
    use ExprKind as K;
    match &expr.kind {
        K::AddressOf(place) => {
            if expr.ty != Term::Pointer(Box::new(place.ty.clone())) {
                return Some("address-of result does not match its place type");
            }
            let mut place = place.as_ref();
            let mut depth = 0;
            while let K::AnonymousField { value, .. } | K::Field { value, .. } = &place.kind {
                if depth >= 128 {
                    return Some("address place depth exhausted");
                }
                depth += 1;
                place = value;
            }
            if !matches!(place.kind, K::Local(_)) {
                return Some("address-of requires a local or field place");
            }
        }
        K::ReadPointer(pointer) => match &pointer.ty {
            Term::Pointer(inner) if **inner == expr.ty => (),
            _ => return Some("pointer read type mismatch"),
        },
        K::PointerAnonymousField {
            pointer,
            name,
            index,
        } => {
            let (Term::Pointer(base), Term::Pointer(result)) = (&pointer.ty, &expr.ty) else {
                return Some("record field pointer requires pointer operands");
            };
            match base.as_ref() {
                Term::Record(fields)
                    if fields
                        .get(*index)
                        .is_some_and(|(label, ty)| label == name && ty == result.as_ref()) =>
                {
                    ()
                }
                Term::AssociatedType { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::Bound { .. }
                | Term::Hole { .. } => (),
                _ => return Some("record field pointer label, index or type mismatch"),
            }
        }
        K::PointerField { pointer, .. } => {
            if !matches!(pointer.ty, Term::Pointer(_)) || !matches!(expr.ty, Term::Pointer(_)) {
                return Some("field pointer requires pointer operands");
            }
        }
        K::BitWritePointer { pointer, .. } => match &pointer.ty {
            Term::Pointer(inner)
                if matches!(inner.as_ref(),Term::Primitive(p)
                if matches!(p,crate::Primitive::BitVec(_)) || p.unrestricted_scalar_width_in(target).is_some()) =>
            {
                ()
            }
            _ => {
                return Some(
                    "bit writeback pointer requires an unrestricted scalar representation",
                );
            }
        },
        K::WritePointer { pointer, value } => match &pointer.ty {
            Term::Pointer(inner)
                if (**inner == value.ty && expr.ty == Term::Primitive(crate::Primitive::Void))
                    || (value.ty == Term::Primitive(crate::Primitive::Never)
                        && expr.ty == Term::Primitive(crate::Primitive::Never)) =>
            {
                ()
            }
            _ => return Some("pointer write type mismatch"),
        },
        _ => (),
    }
    None
}

/// Structural representation rules for normalized bit-position updates. This
/// checks widths/types only; availability and evaluation order belong to the
/// resource and execution layers.
pub fn bit_writeback_error(
    storage: &Term,
    start: usize,
    end: usize,
    value: &Term,
    result: &Term,
) -> Option<&'static str> {
    bit_writeback_error_in(storage, start, end, value, result, None)
}

pub fn bit_writeback_error_in(
    storage: &Term,
    start: usize,
    end: usize,
    value: &Term,
    result: &Term,
    target: Option<crate::target::TargetData>,
) -> Option<&'static str> {
    use crate::Primitive;
    let width = match storage {
        Term::Primitive(Primitive::BitVec(width)) => Some(*width),
        Term::Primitive(primitive) => primitive
            .unrestricted_scalar_width_in(target)
            .map(|n| n as usize),
        _ => None,
    };
    let Some(width) = width else {
        return Some("bit writeback requires an unrestricted scalar representation");
    };
    let never = Term::Primitive(Primitive::Never);
    if start > end
        || end > width
        || (*result != Term::Primitive(Primitive::Void) && !(*result == never && *value == never))
        || (*value != never && *value != Term::Primitive(Primitive::BitVec(end - start)))
    {
        return Some("bit writeback width or result type mismatch");
    }
    None
}

/// Current pointer interpreter subset: representation reads/writes whose Copy
/// behavior is intrinsic. Nominal ownership transfer needs resource evidence.
/// Bounded shape check for scalar-only observation values. Product types
/// still require separate Copy evidence; their structure does not grant
/// copying or authorize user-defined cleanup inside a failed candidate.
pub fn scalar_observation_shape(ty: &Term) -> bool {
    scalar_observation_shape_in_package(None, ty)
}

/// Registered Tuple layouts may be inspected without unfolding normal-form
/// identity. This checks representation only; callers still require Copy.
pub fn scalar_observation_shape_in_package(package: Option<&Package>, ty: &Term) -> bool {
    let mut pending = vec![ty];
    let mut remaining = 4096usize;
    while let Some(ty) = pending.pop() {
        let Some(next) = remaining.checked_sub(1) else {
            return false;
        };
        remaining = next;
        if let Some(fields) = crate::normal_forms::tuple_fields(package, ty) {
            if fields.len().saturating_add(pending.len()) > remaining {
                return false;
            }
            pending.extend(fields);
            continue;
        }
        match ty {
            Term::Primitive(crate::Primitive::Void) => {}
            Term::Primitive(_) if intrinsic_pointer_value(ty) => {}
            Term::Tuple(fields) if fields.len().saturating_add(pending.len()) <= remaining => {
                pending.extend(fields)
            }
            Term::Record(fields) if fields.len().saturating_add(pending.len()) <= remaining => {
                pending.extend(fields.iter().map(|(_, ty)| ty))
            }
            _ => return false,
        }
    }
    true
}

pub fn intrinsic_pointer_value(ty: &Term) -> bool {
    matches!(
        ty,
        Term::Pointer(_)
            | Term::Primitive(
                crate::Primitive::Void
                    | crate::Primitive::Usize
                    | crate::Primitive::Isize
                    | crate::Primitive::Bool
                    | crate::Primitive::Integer
                    | crate::Primitive::Float
                    | crate::Primitive::Symbol
                    | crate::Primitive::Char
                    | crate::Primitive::BitVec(_)
                    | crate::Primitive::F16
                    | crate::Primitive::Bf16
                    | crate::Primitive::F32
                    | crate::Primitive::F64
            )
    ) || matches!(ty,Term::Primitive(p) if p.fixed_integer_layout().is_some())
}

pub fn bit_update_error(
    storage: &Term,
    selections: &[BitSelection],
    value: &Term,
    result: &Term,
) -> Option<&'static str> {
    bit_update_error_in(storage, selections, value, result, None)
}

/// Validate a writeback using the selected target for platform-integer indices.
/// The storage representation is checked independently of the index width.
pub fn bit_update_error_in(
    storage: &Term,
    selections: &[BitSelection],
    value: &Term,
    result: &Term,
    target: Option<crate::target::TargetData>,
) -> Option<&'static str> {
    let width = match storage {
        Term::Primitive(crate::Primitive::BitVec(width)) => Some(*width),
        Term::Primitive(p) => p.unrestricted_scalar_width_in(target).map(|n| n as usize),
        _ => None,
    };
    let Some(mut width) = width else {
        return Some("bit writeback requires an unrestricted scalar representation");
    };
    let mut divergent = false;
    for selection in selections {
        let Some(next) = selection.output_width(width) else {
            return Some("bit writeback selection is out of bounds");
        };
        for expression in selection.expressions() {
            divergent |= expression.ty == Term::Primitive(crate::Primitive::Never);
            if matches!(expression.ty, Term::Primitive(p) if !matches!(p,crate::Primitive::Integer | crate::Primitive::Never) && target.map_or_else(|| p.fixed_integer_layout(), |target| target.integer_layout(p)).is_none())
            {
                return Some("bit writeback index must be an integer");
            }
        }
        width = next;
    }
    if divergent {
        if *result != Term::Primitive(crate::Primitive::Never) {
            return Some("divergent bit writeback must have Never type");
        }
        if *value != Term::Primitive(crate::Primitive::Never)
            && *value != Term::Primitive(crate::Primitive::BitVec(width))
        {
            return Some("bit writeback width or result type mismatch");
        }
        return None;
    }
    bit_writeback_error_in(storage, 0, width, value, result, target)
}

impl ExprKind {
    /// Validate constructor-local binding ownership, including packet producers.
    pub fn nominal_local_bindings(&self) -> Option<Vec<LocalId>> {
        let (Self::NominalType {
            arguments,
            bindings,
            preparations,
            ..
        }
        | Self::CheckTypeAlias {
            arguments,
            bindings,
            preparations,
            ..
        }) = self
        else {
            return None;
        };
        if bindings
            .as_ref()
            .is_some_and(|bindings| bindings.len() != arguments.len())
            || preparations.iter().any(|p| p.before > arguments.len())
            || preparations.windows(2).any(|p| p[0].before > p[1].before)
        {
            return None;
        }
        let mut locals = bindings.clone().unwrap_or_default();
        for preparation in preparations {
            locals.extend(preparation.local_bindings()?);
        }
        if locals
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != locals.len()
        {
            return None;
        }
        Some(locals)
    }
}
