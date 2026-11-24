//! Immediate structural edges of type and immutable value terms.
use crate::Term;

// Both borrow modes expose the same structural edges in the same order.
macro_rules! visit_children {
    ($term:expr, $visit:ident, $method_terms:ident $(, $borrow:ident)?) => {{
        match $term {
            Self::ErrorResult { errors, result } => {
                $visit(result)?;
                for error in errors {
                    $visit(error)?;
                }
            }
            Self::Computation { effects, result } => {
                $visit(result)?;
                for operation in effects {
                    for argument in & $($borrow)? operation.arguments {
                        $visit(argument)?;
                    }
                }
            }
            Self::AssociatedType {
                arguments, subject, ..
            } => {
                $visit(subject)?;
                for argument in arguments {
                    $visit(argument)?;
                }
            }
            Self::MethodValue(method) => {
                for argument in method.$method_terms() {
                    $visit(argument)?;
                }
            }
            Self::ClosureValue { ty, captures, .. } => {
                $visit(ty)?;
                for capture in captures {
                    $visit(capture)?;
                }
            }
            Self::Nominal { arguments, .. }
            | Self::TraitObject { arguments, .. }
            | Self::NormalForm { arguments, .. }
            | Self::FunctionValue { arguments, .. }
            | Self::ClosureSnapshot { arguments, .. }
            | Self::ClosureContextValue(arguments)
            | Self::Tuple(arguments)
            | Self::TupleValue(arguments) => {
                for argument in arguments {
                    $visit(argument)?;
                }
            }
            Self::ContextualClosure { context, signature } => {
                $visit(context)?;
                $visit(signature)?;
            }
            Self::Closure {
                environment,
                signature,
            } => {
                for capture in environment {
                    $visit(capture)?;
                }
                $visit(signature)?;
            }
            Self::Record(fields) | Self::RecordValue(fields) => {
                for (_, field) in fields {
                    $visit(field)?;
                }
            }
            Self::ListConcat { ty, parts: fields } | Self::NominalValue { ty, fields, .. } => {
                $visit(ty)?;
                for field in fields {
                    $visit(field)?;
                }
            }
            Self::OptionalValue { ty, value } => {
                $visit(ty)?;
                if let Some(value) = value {
                    $visit(value)?;
                }
            }
            Self::ErrorValue { ty, label, value } => {
                $visit(ty)?;
                if let Some(label) = label {
                    $visit(label)?;
                }
                $visit(value)?;
            }
            Self::RuntimeField { base: inner, .. }
            | Self::Optional(inner)
            | Self::Pointer(inner)
            | Self::Lift(inner)
            | Self::ComptimeFunction(inner)
            | Self::NormalFormFunction(inner, _)
            | Self::UnsafeFunction(inner) => $visit(inner)?,
            Self::Function { parameters, result } => {
                for parameter in parameters {
                    $visit(& $($borrow)? parameter.ty)?;
                }
                $visit(result)?;
            }
            Self::Primitive(_)
            | Self::ClosureContextType
            | Self::EffectRowType
            | Self::ErrorRowType
            | Self::ReachabilityType
            | Self::AnyType
            | Self::Hole { .. }
            | Self::Bound { .. }
            | Self::Rigid { .. }
            | Self::Runtime(_)
            | Self::Unit
            | Self::Bool(_)
            | Self::Char(_)
            | Self::BitVec(_)
            | Self::Float16(_)
            | Self::BFloat16(_)
            | Self::Float32(_)
            | Self::Float64(_)
            | Self::String(_)
            | Self::Symbol(_)
            | Self::Integer(_)
            | Self::Float(_)
            | Self::Value { .. } => {}
        }
        Ok(())
    }};
}

impl Term {
    /// Borrow stored terms for unordered dependency queries. Declaration heads
    /// are identities; their supplied arguments are structural children.
    pub(crate) fn structural_terms(&self) -> impl Iterator<Item = &Term> {
        let mut root = Some(self);
        let mut pending = Vec::new();
        std::iter::from_fn(move || {
            let term = root.take().or_else(|| pending.pop())?;
            term.visit_children(|child| {
                pending.push(child);
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap_or_else(|never| match never {});
            Some(term)
        })
    }

    /// Whether an argument or domain contains this definition-side parameter.
    pub fn mentions_rigid(&self, declaration: crate::DeclarationId) -> bool {
        self.structural_terms()
            .any(|term| matches!(term, Self::Rigid { declaration: id, .. } if *id == declaration))
    }

    /// Visit immediate children without cloning terms or allocating a worklist.
    /// Order is stable: product fields and arguments keep their stored order;
    /// function parameter types precede the result. Error/effect results precede
    /// their labels/arguments. The first callback failure stops visitation.
    ///
    /// This is structural inspection, not scope or formation checking. A caller
    /// entering function parameters/results must establish the appropriate binder
    /// frames itself. Callers also own traversal depth and work budgets.
    pub fn visit_children<'a, E>(
        &'a self,
        mut visit: impl FnMut(&'a Term) -> Result<(), E>,
    ) -> Result<(), E> {
        visit_children!(self, visit, terms)
    }

    pub(crate) fn visit_children_mut<'a, E>(
        &'a mut self,
        mut visit: impl FnMut(&'a mut Term) -> Result<(), E>,
    ) -> Result<(), E> {
        visit_children!(self, visit, terms_mut, mut)
    }
}
