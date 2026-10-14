//! Program specifications, kept separate from type predicates and trait selection.
use crate::program::{
    EntrySnapshot, Expr, ExprKind as K, Function, FunctionPhase, LocalId, MethodContract,
    SourceRange,
};
use crate::{DeclarationId, Term, TypeError};
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
        Ok(Self {
            preconditions: self
                .preconditions
                .iter()
                .map(|e| e.instantiate_types(parameters, arguments))
                .collect::<Result<_, _>>()?,
            postconditions: self
                .postconditions
                .iter()
                .map(|e| e.instantiate_types(parameters, arguments))
                .collect::<Result<_, _>>()?,
            result_binding: self.result_binding,
            entry_snapshots: self.entry_snapshots.clone(),
        })
    }
    pub fn is_empty(&self) -> bool {
        self.preconditions.is_empty() && self.postconditions.is_empty()
    }
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
    /// An interface-only callable summary. The unit body is never invoked.
    pub fn program_summary(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Function, TypeError> {
        let signature = self.signature.instantiate_types(parameters, arguments)?;
        let program = self.program.instantiate_types(parameters, arguments)?;
        Ok(Function {
            declaration: self.declaration,
            name: self.name.clone(),
            phase: if signature.is_comptime_callable() {
                FunctionPhase::Comptime
            } else {
                FunctionPhase::Runtime
            },
            inline: false, pure: self.pure,
            signature: signature.callable_signature().clone(),
            parameters: self.parameter_bindings.clone(),
            preconditions: program.preconditions,
            postconditions: program.postconditions,
            result_binding: program.result_binding,
            entry_snapshots: program.entry_snapshots,
            capabilities: self.capabilities.iter()
                .map(|requirement| requirement.instantiate_types(parameters, arguments))
                .collect::<Result<_, _>>()?,
            requirements: self.requirements.iter()
                .map(|requirement| requirement.instantiate_types(parameters, arguments))
                .collect::<Result<_, _>>()?,
            type_parameters: self.type_parameters.iter().copied()
                .filter(|parameter| !parameters.contains(parameter)).collect(),
            defaults: vec![],
            body: Expr {
                source: SourceRange { start: 0, end: 0 },
                ty: Term::Primitive(crate::Primitive::Void),
                kind: K::Unit,
            },
        })
    }
}
/// Rebind an effect-free predicate, retaining its expression tree and source.
/// Unsupported executable forms cannot silently bypass substitution.
pub fn remap_predicate(
    expr: &mut Expr,
    bindings: &BTreeMap<LocalId, LocalId>,
) -> Result<(), &'static str> {
    let mut pending = vec![expr];
    let mut remaining = 10_000;
    while let Some(expr) = pending.pop() {
        if remaining == 0 {
            return Err("program contract substitution budget exhausted");
        }
        remaining -= 1;
        match &mut expr.kind {
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
            K::Literal(_) | K::Unit => {}
            K::ReadPointer(v)
            | K::AddressOf(v)
            | K::PointerField {pointer: v, ..}
            | K::PointerAnonymousField {pointer: v, ..}
            | K::Not(v)
            | K::IntegerNegate(v)
            | K::FloatNegate(v)
            | K::BitConvert(v)
            | K::Snapshot(v) | K::TakeBits(v) | K::SymbolName(v) | K::EnumName(v)
            | K::Discard(v)
            | K::AnonymousField { value: v, .. } | K::Field { value: v, .. }
            | K::BitSlice { value: v, .. } => pending.push(v),
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::BitEqual(a, b)
            | K::BitConcat(a, b) | K::BitIndex(a, b)
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
            K::BitWindow {value, start, end} => {pending.push(value);pending.push(start);pending.push(end);}
            K::Tuple(fields) => pending.extend(fields),
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
