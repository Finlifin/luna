//! Readable dumps of candidate-local contexts; no settling or publication.
use super::{CallableContext, InputContext};
use std::fmt;

impl fmt::Display for InputContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("(unknown-type)"),
            Self::UnknownValue => f.write_str("(unknown-value)"),
            Self::UnknownDependentValue => f.write_str("(unknown-dependent-value)"),
            Self::Known(ty) => write!(f, "(known {ty})"),
            Self::Optional(value) => write!(f, "(optional {value})"),
            Self::ErrorResult { errors, result } => {
                f.write_str("(error-result (errors")?;
                for error in errors {
                    write!(f, " {error}")?;
                }
                write!(f, ") (result {result}))")
            }
            Self::Computation { effects, result } => {
                f.write_str("(computation (effects")?;
                for effect in effects {
                    write!(f, " {effect}")?;
                }
                write!(f, ") (result {result}))")
            }
            Self::Tuple(fields) => {
                f.write_str("(tuple")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")
            }
            Self::NormalTuple {
                head,
                fields,
                list_domain,
            } => {
                write!(f, "(normal-tuple (head {head}) (list-domain")?;
                if let Some(domain) = list_domain {
                    write!(f, " {domain}")?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Nominal {
                declaration,
                arguments,
            } => {
                write!(f, "(nominal (declaration {declaration}) (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::Record(fields) => {
                f.write_str("(record")?;
                for (name, field) in fields {
                    // Debug string escaping keeps source names quoted and prevents
                    // parentheses or newlines in names from becoming dump syntax.
                    write!(f, " (field {name:?} {field})")?;
                }
                f.write_str(")")
            }
            Self::Callable(callable) => callable.fmt(f),
        }
    }
}

impl fmt::Display for CallableContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-context (comptime {}) (open-environment {}) (environment",
            self.comptime, self.open_environment
        )?;
        if let Some(environment) = &self.environment {
            f.write_str(" (bindings")?;
            for binding in environment {
                match binding {
                    Some(ty) => write!(f, " (known {ty})")?,
                    None => f.write_str(" (unknown-type)")?,
                }
            }
            f.write_str(")")?;
        }
        f.write_str(") (parameters")?;
        for parameter in &self.parameters {
            match parameter {
                Some(ty) => write!(f, " (known {ty})")?,
                None => f.write_str(" (unknown-type)")?,
            }
        }
        f.write_str(") (result")?;
        if let Some(result) = &self.result {
            write!(f, " {result}")?;
        }
        f.write_str(") (result-context")?;
        if let Some(context) = &self.result_context {
            write!(f, " {context}")?;
        }
        f.write_str("))")
    }
}
