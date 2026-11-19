//! Program specifications, kept separate from type predicates and trait selection.
mod alignment;
mod input_frame;
mod program_view;
use crate::program::{
    EntrySnapshot, Expr, ExprKind as K, FunctionDeclaration, FunctionPhase, LocalId, MethodContract,
};
use crate::{DeclarationId, Term, TypeError};
pub use alignment::BindingAlignment;
pub use input_frame::{InputFrameError, InputFrameErrorKind};
pub use program_view::{CallableProgram, ProgramParts};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProgramContract {
    pub preconditions: Vec<Expr>,
    pub postconditions: Vec<Expr>,
    pub result_binding: Option<LocalId>,
    pub entry_snapshots: Vec<EntrySnapshot>,
}
impl ProgramContract {
    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, TypeError> {
        let parameters = parameters
            .iter()
            .map(|id| crate::DeclarationParameter {
                declaration: *id,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, TypeError> {
        let substitution =
            crate::types::ParameterSubstitution::for_fragments(parameters, arguments)?;
        Ok(Self {
            preconditions: self
                .preconditions
                .iter()
                .map(|e| e.with_parameter_substitution(&substitution))
                .collect::<Result<_, _>>()?,
            postconditions: self
                .postconditions
                .iter()
                .map(|e| e.with_parameter_substitution(&substitution))
                .collect::<Result<_, _>>()?,
            result_binding: self.result_binding,
            entry_snapshots: self.entry_snapshots.clone(),
        })
    }
    /// Specialize declaration inputs while preserving ordinary contract scope.
    pub fn instantiate_declaration_parameters(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        frames: &[Vec<crate::Sort>],
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        let substitution =
            crate::DeclarationSubstitution::bounded(parameters, domains, arguments, frames, fuel)?;
        instantiate_program_parts(
            &self.preconditions,
            &self.postconditions,
            self.result_binding,
            &self.entry_snapshots,
            |value, fuel| substitution.instantiate_expression_bounded(value, fuel),
            fuel,
        )
    }
    pub fn instantiate_admitted<T: std::borrow::Borrow<Term>>(
        &self,
        inputs: &crate::AdmittedArguments<'_, T>,
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        instantiate_program_parts(
            &self.preconditions,
            &self.postconditions,
            self.result_binding,
            &self.entry_snapshots,
            |value, fuel| inputs.instantiate_expression(value, fuel),
            fuel,
        )
    }
    pub fn is_empty(&self) -> bool {
        self.preconditions.is_empty() && self.postconditions.is_empty()
    }
}
/// The owned contract and borrowed callable view use the same copy boundary.
fn instantiate_program_parts(
    preconditions: &[Expr],
    postconditions: &[Expr],
    result_binding: Option<LocalId>,
    entry_snapshots: &[EntrySnapshot],
    mut instantiate: impl FnMut(&Expr, &mut usize) -> Result<Expr, crate::ExpressionInstantiationError>,
    fuel: &mut usize,
) -> Result<ProgramContract, crate::type_computations::Error> {
    use crate::type_computations::Error;
    let mut expression = |value: &Expr| instantiate(value, fuel).map_err(Error::from);
    let preconditions = preconditions
        .iter()
        .map(&mut expression)
        .collect::<Result<_, _>>()?;
    let postconditions = postconditions
        .iter()
        .map(&mut expression)
        .collect::<Result<_, _>>()?;
    *fuel = fuel
        .checked_sub(entry_snapshots.len())
        .ok_or(Error::Budget)?;
    Ok(ProgramContract {
        preconditions,
        postconditions,
        result_binding,
        entry_snapshots: entry_snapshots.to_vec(),
    })
}

impl fmt::Display for ProgramContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(program-contract (requires")?;
        for e in &self.preconditions {
            write!(f, " {e}")?;
        }
        f.write_str(") (ensures")?;
        for e in &self.postconditions {
            write!(f, " {e}")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(binding) = self.result_binding {
            write!(f, " {binding}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in &self.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str("))")
    }
}
impl MethodContract {
    /// Specialize a body-free interface summary using declaration domains, keeping
    /// caller type frames and the method's ordinary input frame distinct.
    pub fn program_summary_declaration_inputs(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        caller_frames: &[Vec<crate::Sort>],
        fuel: &mut usize,
    ) -> Result<FunctionDeclaration, crate::type_computations::Error> {
        use crate::type_computations::Error;
        let signature = self.signature.ordinary().map_err(Error::Scope)?;
        let substitution = crate::DeclarationSubstitution::bounded(
            parameters,
            domains,
            arguments,
            caller_frames,
            fuel,
        )?;
        let completed_signature = substitution.instantiate_bounded(signature, fuel)?;
        let mut frames = caller_frames.to_vec();
        let scoped_arguments =
            if let Term::Function { parameters, .. } = signature.callable_signature() {
                *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
                frames.push(parameters.iter().map(|p| p.sort).collect());
                arguments
                    .iter()
                    .map(|argument| {
                        crate::type_computations::charge_term(argument, fuel)?;
                        argument.abstract_parameters(&[]).map_err(Error::Scope)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                return Err(Error::Scope(crate::TypeError::InvalidFunctionSignature));
            };
        let scoped = crate::DeclarationSubstitution::bounded(
            parameters,
            domains,
            &scoped_arguments,
            &frames,
            fuel,
        )?;
        let program = self.program.instantiate_declaration_parameters(
            parameters,
            domains,
            &scoped_arguments,
            &frames,
            fuel,
        )?;
        let capabilities = self
            .capabilities
            .iter()
            .map(|r| scoped.instantiate_requirement_bounded(r, fuel))
            .collect::<Result<_, _>>()?;
        let requirements = self
            .requirements
            .iter()
            .map(|r| scoped.instantiate_predicate_bounded(r, fuel))
            .collect::<Result<_, _>>()?;
        *fuel = fuel
            .checked_sub(
                self.name
                    .len()
                    .saturating_add(self.parameter_bindings.len())
                    .saturating_add(
                        self.type_parameters
                            .len()
                            .saturating_mul(parameters.len().saturating_add(1)),
                    ),
            )
            .ok_or(Error::Budget)?;
        let phase = if completed_signature.is_comptime_callable() {
            FunctionPhase::Comptime
        } else {
            FunctionPhase::Runtime
        };
        crate::type_computations::charge_term(completed_signature.callable_signature(), fuel)?;
        let signature = completed_signature.callable_signature().clone();
        Ok(FunctionDeclaration {
            source: self.source,
            declaration: self.declaration,
            name: self.name.clone(),
            phase,
            inline: false,
            pure: self.pure,
            is_unsafe: self.is_unsafe,
            signature,
            parameters: self.parameter_bindings.clone(),
            preconditions: program.preconditions,
            postconditions: program.postconditions,
            result_binding: program.result_binding,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: program.entry_snapshots,
            capabilities,
            requirements,
            type_parameters: self
                .type_parameters
                .iter()
                .copied()
                .filter(|id| !parameters.contains(id))
                .collect(),
            defaults: vec![],
        })
    }
}
/// Rebind an effect-free predicate, retaining its expression tree and source.
/// Unsupported executable forms cannot silently bypass substitution.
pub fn remap_predicate(
    expr: &mut Expr,
    bindings: &BTreeMap<LocalId, LocalId>,
) -> Result<(), &'static str> {
    remap_predicate_metered(expr, bindings, &mut 10_000)
}

/// Rewrite a private contract with the caller's shared traversal budget.
/// Failure can leave this private expression partially remapped.
pub fn remap_predicate_metered(
    expr: &mut Expr,
    bindings: &BTreeMap<LocalId, LocalId>,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    let mut pending = vec![expr];
    while let Some(expr) = pending.pop() {
        if *remaining == 0 {
            return Err("program contract substitution budget exhausted");
        }
        *remaining -= 1;
        match &mut expr.kind {
            K::ComptimeValue(value) => {
                crate::comptime_values::validate(value, &expr.ty, remaining)
                    .map_err(|error| error.message())?;
            }
            K::Local(id) => {
                if let Some(to) = bindings.get(id) {
                    *id = *to;
                }
            }
            K::Let { binding, value } => {
                if let Some(to) = bindings.get(binding) {
                    *binding = *to;
                }
                pending.push(value);
            }
            K::LetPattern {
                pattern,
                value,
                guard: None,
                otherwise: None,
            } => {
                remap_predicate_pattern(pattern, bindings, &mut pending, remaining)?;
                pending.push(value);
            }
            // Associated identities contain declaration inputs, not local
            // bindings. Type/input substitution is handled before this remap.
            K::OptionalAbsent | K::AssociatedValue { .. } | K::Literal(_) | K::Unit => {}
            K::ReadPointer(v)
            | K::AddressOf(v)
            | K::PointerField { pointer: v, .. }
            | K::PointerAnonymousField { pointer: v, .. }
            | K::Not(v)
            | K::IntegerNegate(v)
            | K::FloatNegate(v)
            | K::BitConvert(v)
            | K::OptionalPresent(v)
            | K::OptionalType(v)
            | K::ErrorConvert { value: v, .. }
            | K::NewtypeConvert(v)
            | K::Snapshot(v)
            | K::TakeBits(v)
            | K::TextQuery { subject: v, .. }
            | K::IntegerRepresentation(v)
            | K::ComptimeIntegerConvert(v)
            | K::SymbolName(v)
            | K::EnumName(v)
            | K::UnsafeScope(v)
            | K::Discard(v)
            | K::AnonymousField { value: v, .. }
            | K::Field { value: v, .. }
            | K::BitSlice { value: v, .. } => pending.push(v),
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::BitEqual(a, b)
            | K::BitConcat(a, b)
            | K::BitIndex(a, b)
            | K::And(a, b)
            | K::Or(a, b)
            | K::IntegerBinary {
                left: a, right: b, ..
            }
            | K::FloatBinary {
                left: a, right: b, ..
            } => {
                pending.push(a);
                pending.push(b);
            }
            K::BitWindow { value, start, end } => {
                pending.push(value);
                pending.push(start);
                pending.push(end);
            }
            K::Tuple(fields)
            | K::Record { fields, .. }
            | K::AnonymousRecord { fields, .. }
            | K::Variant { fields, .. } => pending.extend(fields),
            K::Match { value, arms } => {
                pending.push(value);
                for arm in arms {
                    remap_predicate_pattern(&mut arm.pattern, bindings, &mut pending, remaining)?;
                    pending.extend(arm.guard.as_mut());
                    pending.push(&mut arm.body);
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                pending.push(condition);
                pending.push(then_branch);
                pending.extend(else_branch.as_deref_mut());
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                pending.extend(statements);
                pending.extend(result.as_deref_mut());
            }
            _ => return Err("invalid expression in program contract substitution"),
        }
    }
    Ok(())
}

fn remap_predicate_pattern<'a>(
    pattern: &'a mut crate::program::Pattern,
    bindings: &BTreeMap<LocalId, LocalId>,
    pending: &mut Vec<&'a mut Expr>,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    let mut patterns = vec![pattern];
    while let Some(pattern) = patterns.pop() {
        if *remaining == 0 {
            return Err("program contract substitution budget exhausted");
        }
        *remaining -= 1;
        use crate::program::Pattern;
        match pattern {
            Pattern::OptionalPresent(inner) | Pattern::ErrorResult { inner, .. } => {
                patterns.push(inner)
            }
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
            Pattern::Then { first, value, next } => {
                patterns.push(first);
                patterns.push(next);
                pending.push(value);
            }
            Pattern::Remap {
                pattern,
                bindings: aliases,
            } => {
                if aliases.len() > *remaining {
                    return Err("program contract substitution budget exhausted");
                }
                *remaining -= aliases.len();
                for (from, to) in aliases {
                    if let Some(mapped) = bindings.get(from) {
                        *from = *mapped;
                    }
                    if let Some(mapped) = bindings.get(to) {
                        *to = *mapped;
                    }
                }
                patterns.push(pattern);
            }
            Pattern::Guard { pattern, condition } => {
                patterns.push(pattern);
                pending.push(condition);
            }
            Pattern::Observe { binding, inner } | Pattern::Binding { binding, inner } => {
                if let Some(to) = bindings.get(binding) {
                    *binding = *to;
                }
                patterns.push(inner);
            }
            Pattern::Tuple(fields)
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Or(fields)
            | Pattern::Variant { fields, .. } => {
                if fields.len() > *remaining {
                    return Err("program contract substitution budget exhausted");
                }
                patterns.extend(fields);
            }
            _ => return Err("invalid pattern in program contract substitution"),
        }
    }
    Ok(())
}
