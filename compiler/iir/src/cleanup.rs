//! Concrete cleanup operations. Paths are relative to a single owner storage.
//! Publication requires resource verification; this IR does not select Drop.
use crate::InstanceKey;
use nir::Term;
use std::fmt;
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub ty: Term,
    /// Known variant evidence must match the concrete input before any effect.
    pub guards: Vec<(Vec<usize>, usize)>,
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VariantCase {
    pub guards: Vec<(Vec<usize>, usize)>,
    pub steps: Vec<Step>,
}
impl fmt::Display for VariantCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(variant-case (guards")?;
        for (fields, tag) in &self.guards {
            f.write_str(" (guard ")?;
            path(f, fields)?;
            write!(f, " (tag {tag}))")?;
        }
        f.write_str(") (steps")?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Exactly one case must match, before any case executes. Guards are
    /// ordered from enclosing tags to tags inside their active payloads.
    Select {
        cases: Vec<VariantCase>,
    },
    IfConstructed {
        path: Vec<usize>,
        steps: Vec<Step>,
    },
    Invoke {
        path: Vec<usize>,
        subject: Term,
        method: InstanceKey,
    },
    Switch {
        path: Vec<usize>,
        branches: Vec<Vec<Step>>,
    },
    Release {
        path: Vec<usize>,
        ty: Term,
    },
}
fn path(f: &mut fmt::Formatter<'_>, indices: &[usize]) -> fmt::Result {
    f.write_str("(path")?;
    for index in indices {
        write!(f, " {index}")?;
    }
    f.write_str(")")
}
impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Select { cases } => {
                f.write_str("(cleanup-select")?;
                for case in cases {
                    write!(f, " {case}")?;
                }
                f.write_str(")")
            }
            Self::IfConstructed {
                path: indices,
                steps,
            } => {
                f.write_str("(cleanup-if-constructed ")?;
                path(f, indices)?;
                f.write_str(" (steps")?;
                for step in steps {
                    write!(f, " {step}")?;
                }
                f.write_str("))")
            }
            Self::Invoke {
                path: indices,
                subject,
                method,
            } => {
                f.write_str("(cleanup-invoke ")?;
                path(f, indices)?;
                write!(f, " (subject {subject}) {method})")
            }
            Self::Release { path: indices, ty } => {
                f.write_str("(cleanup-release ")?;
                path(f, indices)?;
                write!(f, " (type {ty}))")
            }
            Self::Switch {
                path: indices,
                branches,
            } => {
                f.write_str("(cleanup-switch ")?;
                path(f, indices)?;
                for (index, steps) in branches.iter().enumerate() {
                    write!(f, " (branch (tag {index}) (steps")?;
                    for step in steps {
                        write!(f, " {step}")?;
                    }
                    f.write_str("))")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(cleanup-program (type {}) (variant-guards", self.ty)?;
        for (indices, tag) in &self.guards {
            f.write_str(" (guard ")?;
            path(f, indices)?;
            write!(f, " (tag {tag}))")?;
        }
        f.write_str(") (steps")?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Full current-resource-scope unwind when the effect targets another
    /// scope. The selected installer's return/loop boundary has its own plan.
    EffectEscape,
    /// Target invocation cleanup after checking the handler-supplied result.
    EffectReturn,
    EffectBreak(nir::program::LocalId),
    EffectContinue(nir::program::LocalId),
    HandlerEnd,
    EffectResume,
    ScopeEnd,
    TemporaryEnd,
    FunctionEnd,
    Return,
    Break,
    Continue,
    Replacement,
    PredicateFailure,
    InterfacePreconditionFailure,
    /// Invocation inputs have transferred; its result is still pending delivery.
    InterfacePostconditionFailure,
    DefaultFailure(usize),
    PostconditionFailure(nir::program::SourceRange),
}
impl fmt::Display for Boundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::EffectBreak(id) | Self::EffectContinue(id) = self {
            let kind = if matches!(self, Self::EffectBreak(_)) {
                "effect-break"
            } else {
                "effect-continue"
            };
            return write!(f, "({kind} (loop {} {}))", id.owner, id.index);
        }
        if let Self::PostconditionFailure(origin) = self {
            return write!(f, "(postcondition-failure (return-source {origin}))");
        }
        if let Self::DefaultFailure(parameter) = self {
            return write!(f, "(default-failure (parameter {parameter}))");
        }
        f.write_str(match self {
            Self::EffectEscape => "effect-escape",
            Self::EffectReturn => "effect-return",
            Self::EffectBreak(_) | Self::EffectContinue(_) => unreachable!(),
            Self::HandlerEnd => "handler-end",
            Self::EffectResume => "effect-resume",
            Self::ScopeEnd => "scope-end",
            Self::TemporaryEnd => "temporary-end",
            Self::FunctionEnd => "function-end",
            Self::Return => "return",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::Replacement => "replacement",
            Self::PredicateFailure => "predicate-failure",
            Self::InterfacePreconditionFailure => "interface-precondition-failure",
            Self::InterfacePostconditionFailure => "interface-postcondition-failure",
            Self::DefaultFailure(_) | Self::PostconditionFailure(_) => unreachable!(),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Temporary {
        source: nir::program::SourceRange,
        optional: bool,
    },
    Local {
        binding: nir::program::LocalId,
        fields: Vec<usize>,
    },
    Prepared {
        construction: nir::program::SourceRange,
        slot: usize,
        value_source: nir::program::SourceRange,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub target: Target,
    pub program: Program,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub source: nir::program::SourceRange,
    pub boundary: Boundary,
    pub actions: Vec<Action>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextKey {
    EffectHandler {
        installation: nir::program::SourceRange,
        handler: usize,
        arm: usize,
    },
    Default {
        construction: nir::program::SourceRange,
        parameter: usize,
    },
    Function {
        call: nir::program::SourceRange,
        callee: InstanceKey,
    },
    Closure {
        call: nir::program::SourceRange,
        definition: nir::program::SourceRange,
    },
}
impl fmt::Display for ContextKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EffectHandler {
                installation,
                handler,
                arm,
            } => write!(
                f,
                "(effect-handler-context (installation {installation}) (handler {handler}) (arm {arm}))"
            ),
            Self::Default {
                construction,
                parameter,
            } => write!(
                f,
                "(default-context (construction {construction}) (parameter {parameter}))"
            ),
            Self::Function { call, callee } => {
                write!(f, "(function-context (call {call}) (callee {callee}))")
            }
            Self::Closure { call, definition } => write!(
                f,
                "(closure-context (call {call}) (definition {definition}))"
            ),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionSchedule {
    pub instance: InstanceKey,
    pub context: Vec<ContextKey>,
    pub sites: Vec<Site>,
    pub discards: Vec<DiscardCleanup>,
    pub transfers: Vec<Transfer>,
    pub copies: Vec<Transfer>,
    /// Successful pattern binding positions; applied only to retained subjects.
    pub pattern_transfers: Vec<Transfer>,
    pub pattern_orders: Vec<BindingOrder>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingOrder {
    pub source: nir::program::SourceRange,
    pub alternatives: Vec<Vec<nir::program::LocalId>>,
}
impl fmt::Display for BindingOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pattern-order (source {})", self.source)?;
        for order in &self.alternatives {
            f.write_str(" (bindings")?;
            for binding in order {
                write!(f, " {binding}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub source: nir::program::SourceRange,
    pub binding: nir::program::LocalId,
    pub fields: Vec<usize>,
}
impl fmt::Display for Transfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(ownership-transfer (source {}) (binding {}) ",
            self.source, self.binding
        )?;
        path(f, &self.fields)?;
        f.write_str(")")
    }
}
/// A whole result discarded at its own full-expression boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscardCleanup {
    pub source: nir::program::SourceRange,
    pub value_source: nir::program::SourceRange,
    pub program: Program,
}
impl fmt::Display for DiscardCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(discard-cleanup (source {}) (value-source {}) {})",
            self.source, self.value_source, self.program
        )
    }
}
/// A recursive call with the same validated callable/enum premises as an
/// ancestor reuses that ancestor's finite cleanup specialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextAlias {
    pub instance: InstanceKey,
    pub context: Vec<ContextKey>,
    pub target: Vec<ContextKey>,
}
impl fmt::Display for ContextAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-context-alias (instance {}) (context",
            self.instance
        )?;
        for key in &self.context {
            write!(f, " {key}")?;
        }
        f.write_str(") (target")?;
        for key in &self.target {
            write!(f, " {key}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, Default)]
pub struct Schedule {
    /// Concrete closure evidence for an invocation scope. Source ranges alone
    /// cannot distinguish bodies produced by different generic instances.
    pub closure_definitions: Vec<ClosureDefinition>,
    /// Instances whose borrowed callable identities require per-call cleanup evidence.
    pub borrowed_callable_functions: Vec<InstanceKey>,
    pub functions: Vec<FunctionSchedule>,
    pub aliases: Vec<ContextAlias>,
}

#[derive(Clone, Debug)]
pub struct ClosureDefinition {
    pub instance: InstanceKey,
    pub context: Vec<ContextKey>,
    pub definition: nir::program::Expr,
}
impl fmt::Display for ClosureDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(closure-definition (instance {}) (context",
            self.instance
        )?;
        for key in &self.context {
            write!(f, " {key}")?;
        }
        write!(f, ") (definition {}))", self.definition)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeLookupError {
    Missing,
    Ambiguous,
}
impl fmt::Display for ScopeLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "(cleanup-scope-error missing)",
            Self::Ambiguous => "(cleanup-scope-error ambiguous)",
        })
    }
}
impl std::error::Error for ScopeLookupError {}

impl Schedule {
    /// Resolve one concrete resource scope, including its complete instance and
    /// lexical context. Callers charge the scan before entering this helper.
    /// Aliases are intentionally not followed: handler/default activations need
    /// their own published scope, not another activation's cleanup evidence.
    pub fn exact_function(
        &self,
        instance: &InstanceKey,
        context: &[ContextKey],
    ) -> Result<&FunctionSchedule, ScopeLookupError> {
        let mut matches = self
            .functions
            .iter()
            .filter(|candidate| &candidate.instance == instance && candidate.context == context);
        let result = matches.next().ok_or(ScopeLookupError::Missing)?;
        if matches.next().is_some() {
            return Err(ScopeLookupError::Ambiguous);
        }
        Ok(result)
    }
}
impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Temporary { source, optional } => write!(
                f,
                "(temporary-target (source {source}) (optional {optional}))"
            ),
            Self::Local { binding, fields } => {
                write!(f, "(local-target (binding {binding}) ")?;
                path(f, fields)?;
                f.write_str(")")
            }
            Self::Prepared {
                construction,
                slot,
                value_source,
            } => write!(
                f,
                "(prepared-target (construction {construction}) (slot {slot}) (value-source {value_source}))"
            ),
        }
    }
}
impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(scheduled-cleanup {} {})", self.target, self.program)
    }
}
impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-site (source {}) (boundary {}) (actions",
            self.source, self.boundary
        )?;
        for action in &self.actions {
            write!(f, " {action}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for FunctionSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-cleanup {} (context", self.instance)?;
        for context in &self.context {
            write!(f, " {context}")?;
        }
        f.write_str(") (transfers")?;
        for transfer in &self.transfers {
            write!(f, " {transfer}")?;
        }
        f.write_str(") (pattern-orders")?;
        for order in &self.pattern_orders {
            write!(f, " {order}")?;
        }
        f.write_str(") (pattern-transfers")?;
        for transfer in &self.pattern_transfers {
            write!(f, " {transfer}")?;
        }
        f.write_str(") (copies")?;
        for copy in &self.copies {
            write!(
                f,
                " (representation-copy (source {}) (binding {}) ",
                copy.source, copy.binding
            )?;
            path(f, &copy.fields)?;
            f.write_str(")")?;
        }
        f.write_str(") (sites")?;
        for site in &self.sites {
            write!(f, " {site}")?;
        }
        f.write_str(") (discards")?;
        for discard in &self.discards {
            write!(f, " {discard}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(cleanup-schedule (borrowed-callable-functions")?;
        for key in &self.borrowed_callable_functions {
            write!(f, " {key}")?;
        }
        f.write_str(")")?;
        for definition in &self.closure_definitions {
            write!(f, " {definition}")?;
        }
        for function in &self.functions {
            write!(f, " {function}")?;
        }
        for alias in &self.aliases {
            write!(f, " {alias}")?;
        }
        f.write_str(")")
    }
}
