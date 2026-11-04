//! Lexical admission of responsibility-bearing calls. This is independent of
//! phase, purity, resource legality and the callee's program contract.
use crate::program::{ExprKind as K, Function, SourceRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget(&'static str),
    Invalid {
        source: Option<SourceRange>,
        message: &'static str,
    },
}
impl From<crate::visit::Error> for Error {
    fn from(error: crate::visit::Error) -> Self {
        match error {
            crate::visit::Error::Budget(message) => Self::Budget(message),
            crate::visit::Error::Invalid(message) => Self::Invalid {
                source: None,
                message,
            },
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(message)
            | Self::Invalid {
                source: None,
                message,
            } => f.write_str(message),
            Self::Invalid {
                source: Some(source),
                message,
            } => {
                write!(f, "{message} at {}..{}", source.start, source.end)
            }
        }
    }
}
impl std::error::Error for Error {}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

pub fn validate(function: &Function, fuel: &mut usize) -> Result<(), Error> {
    validate_program(crate::contracts::CallableProgram::function(function), fuel)
}

pub fn validate_program(
    function: crate::contracts::CallableProgram<'_>,
    fuel: &mut usize,
) -> Result<(), Error> {
    if function.signature.is_unsafe_callable() {
        return Err(Error::Invalid {
            source: None,
            message: "function declaration requires an unqualified parameter/result telescope",
        });
    }
    let mut roots = function.roots().rev();
    let mut pending = Vec::new();
    while let Some((root, permitted)) = pending
        .pop()
        .or_else(|| roots.next().map(|root| (root, function.is_unsafe)))
    {
        crate::visit::expression_pruned_checked(root, fuel, |expr, _| {
            if let K::UnsafeScope(body) = &expr.kind {
                pending.push((body, true));
                return Ok(false);
            }
            if matches!(expr.kind, K::Intrinsic { .. }) && !permitted {
                return Err(Error::Invalid {
                    source: Some(expr.source),
                    message: "intrinsic requires an unsafe boundary",
                });
            }
            if let K::Call { callee, .. } = &expr.kind {
                if callee.ty.is_unsafe_callable() && !permitted {
                    return Err(Error::Invalid {
                        source: Some(expr.source),
                        message: "unsafe function call requires an unsafe boundary",
                    });
                }
            }
            Ok(true)
        })?;
    }
    Ok(())
}
