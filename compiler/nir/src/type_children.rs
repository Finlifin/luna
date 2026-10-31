//! Immediate structural edges of type and immutable value terms.
use crate::Term;

impl Term {
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
        match self {
            Self::ErrorResult { errors, result } => {
                visit(result)?;
                for error in errors {
                    visit(error)?;
                }
            }
            Self::Computation { effects, result } => {
                visit(result)?;
                for operation in effects {
                    for argument in &operation.arguments {
                        visit(argument)?;
                    }
                }
            }
            Self::AssociatedType {
                arguments, subject, ..
            } => {
                visit(subject)?;
                for argument in arguments {
                    visit(argument)?;
                }
            }
            Self::Nominal { arguments, .. }
            | Self::NormalForm { arguments, .. }
            | Self::FunctionValue { arguments, .. }
            | Self::ClosureSnapshot { arguments, .. }
            | Self::ClosureContextValue(arguments)
            | Self::Tuple(arguments)
            | Self::TupleValue(arguments) => {
                for argument in arguments {
                    visit(argument)?;
                }
            }
            Self::ContextualClosure { context, signature } => {
                visit(context)?;
                visit(signature)?;
            }
            Self::Closure {
                environment,
                signature,
            } => {
                for capture in environment {
                    visit(capture)?;
                }
                visit(signature)?;
            }
            Self::Record(fields) | Self::RecordValue(fields) => {
                for (_, field) in fields {
                    visit(field)?;
                }
            }
            Self::ListConcat { ty, parts: fields } | Self::NominalValue { ty, fields, .. } => {
                visit(ty)?;
                for field in fields {
                    visit(field)?;
                }
            }
            Self::OptionalValue { ty, value } => {
                visit(ty)?;
                if let Some(value) = value {
                    visit(value)?;
                }
            }
            Self::ErrorValue { ty, label, value } => {
                visit(ty)?;
                if let Some(label) = label {
                    visit(label)?;
                }
                visit(value)?;
            }
            Self::Optional(inner)
            | Self::Pointer(inner)
            | Self::ComptimeFunction(inner)
            | Self::NormalFormFunction(inner, _)
            | Self::UnsafeFunction(inner) => visit(inner)?,
            Self::Function { parameters, result } => {
                for parameter in parameters {
                    visit(&parameter.ty)?;
                }
                visit(result)?;
            }
            Self::Primitive(_)
            | Self::ClosureContextType
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
    }
}
